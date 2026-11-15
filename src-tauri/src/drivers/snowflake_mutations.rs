//! Pure generated mutation plans, without execution or edit authority.
//!
//! A caller must retain and corroborate inspection/result revisions separately,
//! own the transaction, check each actual DML cardinality, and roll back on a
//! mismatch. Quoted names are mutable names, not immutable object identities.
//! Original-value fingerprints only detect locally recognizable overlaps;
//! duplicate server rows can still match the same predicate.
//!
//! Supports UTF-8 text, exact fixed-point decimals (precision 1..=38, scale
//! 0..=38), IEEE-754 float, Boolean, hex binary, proleptic Gregorian date,
//! nanosecond-scale time and NTZ/LTZ/TZ timestamps, and strict RFC 8259 JSON
//! semi-structured types (VARIANT, ARRAY, OBJECT) on up to 1,000 columns. Text
//! stale predicates compare explicit UTF-8 binary conversions, not SQL
//! collation. UPDATE DEFAULT and mixed batches with unknown INSERT postimages
//! remain explicit compiler limits rather than guessed server effects. Quoted
//! identifier resolution requires corroborating the session's
//! QUOTED_IDENTIFIERS_IGNORE_CASE setting; quoting is not an identity proof.
use crate::domain::{AppError, AppResult, ErrorCode};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const MAX_ROWS: usize = 500;
pub const MAX_COLUMNS: usize = 1000;
pub const MAX_CELL_BYTES: usize = 1024 * 1024;
pub const MAX_UNIT_SQL_BYTES: usize = 768 * 1024;
pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_JSON_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedType {
    Geography,
    Geometry,
    Vector,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarType {
    Text { max_bytes: usize },
    Decimal { precision: u8, scale: u8 },
    Float,
    Boolean,
    Binary { max_bytes: usize },
    Date,
    Time { scale: u8 },
    TimestampNtz { scale: u8 },
    TimestampLtz { scale: u8 },
    TimestampTz { scale: u8 },
    Variant,
    Array,
    Object,
    Unsupported(UnsupportedType),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    Writable,
    Generated,
}

#[derive(Clone, Debug)]
pub struct MutationColumn {
    name: String,
    quoted: String,
    data_type: ScalarType,
    nullable: bool,
    mode: WriteMode,
    has_default: bool,
}
impl MutationColumn {
    pub fn new(
        name: &str,
        data_type: ScalarType,
        nullable: bool,
        mode: WriteMode,
        has_default: bool,
    ) -> AppResult<Self> {
        validate_identifier(name)?;
        match data_type {
            ScalarType::Text { max_bytes } if max_bytes == 0 || max_bytes > MAX_CELL_BYTES => {
                return Err(invalid());
            }
            ScalarType::Decimal { precision, scale }
                if precision == 0 || precision > 38 || scale > precision =>
            {
                return Err(invalid());
            }
            ScalarType::Binary { max_bytes }
                if max_bytes == 0 || max_bytes > MAX_CELL_BYTES / 2 =>
            {
                return Err(invalid());
            }
            ScalarType::Time { scale }
            | ScalarType::TimestampNtz { scale }
            | ScalarType::TimestampLtz { scale }
            | ScalarType::TimestampTz { scale }
                if scale > 9 =>
            {
                return Err(invalid());
            }
            ScalarType::Unsupported(_) => return Err(unsupported()),
            _ => {}
        }
        Ok(Self {
            name: name.into(),
            quoted: quote(name),
            data_type,
            nullable,
            mode,
            has_default,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn data_type(&self) -> ScalarType {
        self.data_type
    }
}

/// Validated compilation inputs only. This descriptor proves no permission,
/// immutable identity, snapshot freshness, key enforcement or row uniqueness.
#[derive(Clone, Debug)]
pub struct MutationTarget {
    qualified_name: String,
    columns: Vec<MutationColumn>,
}
impl MutationTarget {
    pub fn new(
        database: &str,
        schema: &str,
        table: &str,
        columns: Vec<MutationColumn>,
    ) -> AppResult<Self> {
        for name in [database, schema, table] {
            validate_identifier(name)?;
        }
        if columns.is_empty() || columns.len() > MAX_COLUMNS {
            return Err(invalid());
        }
        let mut names = HashSet::new();
        for column in &columns {
            if !names.insert(column.name.as_str()) {
                return Err(invalid());
            }
        }
        Ok(Self {
            qualified_name: format!("{}.{}.{}", quote(database), quote(schema), quote(table)),
            columns,
        })
    }
    pub fn columns(&self) -> &[MutationColumn] {
        &self.columns
    }
    pub fn qualified_name(&self) -> &str {
        &self.qualified_name
    }
}

/// Text is exact input for the column's closed scalar codec, never SQL syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellValue {
    Default,
    Null,
    Text(String),
}

/// Full ordered values of all target columns, including generated columns.
/// The runtime obtains originals from its own retained results.
#[derive(Clone, Debug)]
pub struct OriginalRow {
    values: Vec<CellValue>,
}
impl OriginalRow {
    pub fn new(target: &MutationTarget, values: Vec<CellValue>) -> AppResult<Self> {
        validate_original(target, &values)?;
        Ok(Self { values })
    }
    pub fn values(&self) -> &[CellValue] {
        &self.values
    }
}

#[derive(Clone, Debug)]
pub enum Mutation {
    Insert {
        values: Vec<(usize, CellValue)>,
    },
    Update {
        original: OriginalRow,
        changes: Vec<(usize, CellValue)>,
    },
    Delete {
        original: OriginalRow,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    Text,
    Decimal,
    Float,
    Boolean,
    Binary,
    Date,
    Time,
    TimestampNtz,
    TimestampLtz,
    TimestampTz,
    Variant,
    Array,
    Object,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    kind: BindingKind,
    value: Option<String>,
}
impl Binding {
    pub fn kind(&self) -> BindingKind {
        self.kind
    }
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationKind {
    Insert,
    Update,
    Delete,
}
#[derive(Debug)]
pub struct CompiledUnit {
    kind: MutationKind,
    sql: String,
    bindings: Vec<Binding>,
    expected_rows: u64,
}
impl CompiledUnit {
    pub fn kind(&self) -> MutationKind {
        self.kind
    }
    pub fn sql(&self) -> &str {
        &self.sql
    }
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
    pub fn expected_rows(&self) -> u64 {
        self.expected_rows
    }
}
#[derive(Debug)]
pub struct CompiledBatch {
    units: Vec<CompiledUnit>,
    accounted_bytes: usize,
}
impl CompiledBatch {
    pub fn units(&self) -> &[CompiledUnit] {
        &self.units
    }
    /// Conservative logical input/output/SQL/identifier/binding accounting;
    /// this is not an allocator-capacity or whole-process RSS measurement.
    pub fn accounted_bytes(&self) -> usize {
        self.accounted_bytes
    }
    pub fn into_units(self) -> Vec<CompiledUnit> {
        self.units
    }
}

fn invalid() -> AppError {
    AppError::invalid("Generated Snowflake mutation inputs are invalid")
}
fn unsupported() -> AppError {
    AppError::new(
        ErrorCode::Unsupported,
        "This generated Snowflake mutation requires an unsupported exact codec or batch shape",
    )
}
fn overlap() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Generated mutations contain duplicate or overlapping original-row targets",
    )
}
fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Generated mutations exceed the bounded compiler budget",
    )
}
fn validate_identifier(name: &str) -> AppResult<()> {
    // Snowflake quoted identifiers allow at most 255 characters. The byte
    // ceiling rejects oversized input before scanning; UTF-8 uses <=4 bytes.
    if name.is_empty()
        || name.len() > 255 * 4
        || name.chars().count() > 255
        || name.trim().is_empty()
        || name.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
        })
    {
        return Err(invalid());
    }
    Ok(())
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn decimal_parts(text: &str, precision: u8, scale: u8) -> AppResult<(&str, &str, bool)> {
    let (negative, unsigned) = text.strip_prefix('-').map_or((false, text), |s| (true, s));
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if integer.is_empty()
        || !integer.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || (unsigned.contains('.') && fraction.is_empty())
        || fraction.len() > usize::from(scale)
        || integer.trim_start_matches('0').len() > usize::from(precision - scale)
    {
        return Err(invalid());
    }
    Ok((integer, fraction, negative))
}

fn float_bits(text: &str) -> AppResult<u64> {
    match text {
        "NaN" => return Ok(0x7ff8_0000_0000_0000),
        "inf" | "Infinity" => return Ok(f64::INFINITY.to_bits()),
        "-inf" | "-Infinity" => return Ok(f64::NEG_INFINITY.to_bits()),
        _ => {}
    }
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (significand, exponent) = unsigned
        .split_once(['e', 'E'])
        .map_or((unsigned, None), |(s, e)| (s, Some(e)));
    let (integer, fraction) = significand
        .split_once('.')
        .map_or((significand, None), |(i, f)| (i, Some(f)));
    if integer.is_empty()
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    if let Some(exp) = exponent {
        let exp_digits = exp.strip_prefix(['+', '-']).unwrap_or(exp);
        if exp_digits.is_empty() || !exp_digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
    }
    let value: f64 = text.parse().map_err(|_| invalid())?;
    if !value.is_finite() {
        return Err(invalid());
    }
    Ok(if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    })
}

fn validate_hex_binary(text: &str, max_bytes: usize) -> AppResult<()> {
    if !text.len().is_multiple_of(2)
        || text.len() / 2 > max_bytes
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    Ok(())
}

fn two_digits(bytes: &[u8]) -> AppResult<u8> {
    if bytes.len() != 2 || !bytes[0].is_ascii_digit() || !bytes[1].is_ascii_digit() {
        return Err(invalid());
    }
    Ok((bytes[0] - b'0') * 10 + (bytes[1] - b'0'))
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn parse_ymd(text: &str) -> AppResult<(i32, u8, u8)> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(invalid());
    }
    if !bytes[..4].iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    let year: i32 = text[..4].parse().map_err(|_| invalid())?;
    let month = two_digits(&bytes[5..7])?;
    let day = two_digits(&bytes[8..10])?;
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || day == 0 {
        return Err(invalid());
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => unreachable!(),
    };
    if day > max_day {
        return Err(invalid());
    }
    Ok((year, month, day))
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_hms_nanos(text: &str, scale: u8) -> AppResult<(u8, u8, u8, u32, bool)> {
    let (hms, fraction) = text
        .split_once('.')
        .map_or((text, None), |(h, f)| (h, Some(f)));
    let bytes = hms.as_bytes();
    if bytes.len() != 8 || bytes[2] != b':' || bytes[5] != b':' {
        return Err(invalid());
    }
    let hour = two_digits(&bytes[0..2])?;
    let minute = two_digits(&bytes[3..5])?;
    let second = two_digits(&bytes[6..8])?;
    if hour > 23 || minute > 59 || second > 59 {
        return Err(invalid());
    }
    let Some(frac) = fraction else {
        return Ok((hour, minute, second, 0, false));
    };
    if frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    if frac.len() > usize::from(scale)
        && frac.as_bytes()[usize::from(scale)..]
            .iter()
            .any(|&b| b != b'0')
    {
        return Err(invalid());
    }
    let raw: u32 = frac.parse().map_err(|_| invalid())?;
    let nanos = raw * 10u32.pow(9 - frac.len() as u32);
    Ok((hour, minute, second, nanos, true))
}

struct ParsedTimestampNtz {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nanos: u32,
    separator: char,
    has_fraction: bool,
}

fn parse_timestamp_ntz(text: &str, scale: u8) -> AppResult<ParsedTimestampNtz> {
    if text.len() < 19 || !text.is_char_boundary(10) || !text.is_char_boundary(11) {
        return Err(invalid());
    }
    let (year, month, day) = parse_ymd(&text[..10])?;
    let separator = match text.as_bytes()[10] {
        b' ' => ' ',
        b'T' => 'T',
        _ => return Err(invalid()),
    };
    let (hour, minute, second, nanos, has_fraction) = parse_hms_nanos(&text[11..], scale)?;
    Ok(ParsedTimestampNtz {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanos,
        separator,
        has_fraction,
    })
}

struct ParsedTimestampTz {
    utc_seconds: i64,
    nanos: u32,
    separator: char,
    has_fraction: bool,
    tz_space: bool,
}

fn parse_timestamp_tz(text: &str, scale: u8) -> AppResult<ParsedTimestampTz> {
    let bytes = text.as_bytes();
    if bytes.len() < 25 || !text.is_char_boundary(10) || !text.is_char_boundary(11) {
        return Err(invalid());
    }
    let (year, month, day) = parse_ymd(&text[..10])?;
    let separator = match bytes[10] {
        b' ' => ' ',
        b'T' => 'T',
        _ => return Err(invalid()),
    };
    let tz_start = bytes.len() - 6;
    if !text.is_char_boundary(tz_start) {
        return Err(invalid());
    }
    let tz_bytes = &bytes[tz_start..];
    let sign = match tz_bytes[0] {
        b'+' => 1i64,
        b'-' => -1i64,
        _ => return Err(invalid()),
    };
    if tz_bytes[3] != b':' {
        return Err(invalid());
    }
    let tz_hour = two_digits(&tz_bytes[1..3])?;
    let tz_minute = two_digits(&tz_bytes[4..6])?;
    if tz_hour > 23 || tz_minute > 59 {
        return Err(invalid());
    }
    let (time_end, tz_space) = if tz_start > 11 && bytes[tz_start - 1] == b' ' {
        (tz_start - 1, true)
    } else {
        (tz_start, false)
    };
    if !text.is_char_boundary(time_end) {
        return Err(invalid());
    }
    let (hour, minute, second, nanos, has_fraction) = parse_hms_nanos(&text[11..time_end], scale)?;
    let civil_seconds = days_from_civil(year, month, day) * 86_400
        + i64::from(hour) * 3_600
        + i64::from(minute) * 60
        + i64::from(second);
    let offset_seconds = sign * (i64::from(tz_hour) * 3_600 + i64::from(tz_minute) * 60);
    Ok(ParsedTimestampTz {
        utc_seconds: civil_seconds - offset_seconds,
        nanos,
        separator,
        has_fraction,
        tz_space,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonKind {
    Any,
    Array,
    Object,
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn consume_literal(&mut self, expected: &[u8]) -> AppResult<()> {
        if self.bytes.get(self.pos..self.pos + expected.len()) == Some(expected) {
            self.pos += expected.len();
            Ok(())
        } else {
            Err(invalid())
        }
    }

    fn parse_hex4(&mut self) -> AppResult<u16> {
        let slice = self.bytes.get(self.pos..self.pos + 4).ok_or_else(invalid)?;
        let mut value = 0u16;
        for &b in slice {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(invalid()),
            };
            value = (value << 4) | u16::from(digit);
        }
        self.pos += 4;
        Ok(value)
    }

    fn parse_string(&mut self) -> AppResult<String> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(invalid());
        }
        self.pos += 1;
        let mut out = String::new();
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                0x00..=0x1f => return Err(invalid()),
                b'\\' => {
                    let esc = *self.bytes.get(self.pos).ok_or_else(invalid)?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.parse_hex4()?;
                            let code = if (0xD800..=0xDBFF).contains(&first) {
                                self.consume_literal(b"\\u")?;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(invalid());
                                }
                                0x10000
                                    + ((u32::from(first - 0xD800) << 10) | u32::from(low - 0xDC00))
                            } else if (0xDC00..=0xDFFF).contains(&first) {
                                return Err(invalid());
                            } else {
                                u32::from(first)
                            };
                            out.push(char::from_u32(code).ok_or_else(invalid)?);
                        }
                        _ => return Err(invalid()),
                    }
                }
                _ => {
                    let start = self.pos - 1;
                    // Advance to end of UTF-8 codepoint validated by `&str` input.
                    while self
                        .bytes
                        .get(self.pos)
                        .is_some_and(|&next| (next & 0b1100_0000) == 0b1000_0000)
                    {
                        self.pos += 1;
                    }
                    let ch =
                        std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| invalid())?;
                    out.push_str(ch);
                }
            }
        }
        Err(invalid())
    }

    fn parse_number(&mut self, out: &mut Vec<u8>) -> AppResult<()> {
        let negative = if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
            true
        } else {
            false
        };
        let int_start = self.pos;
        match self.bytes.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                if self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                    return Err(invalid());
                }
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                    self.pos += 1;
                }
            }
            _ => return Err(invalid()),
        }
        let int_slice = &self.bytes[int_start..self.pos];
        let mut frac_slice: &[u8] = &[];
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            let frac_start = self.pos;
            while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(invalid());
            }
            frac_slice = &self.bytes[frac_start..self.pos];
        }
        let mut exp_val: i64 = 0;
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            let exp_sign = match self.bytes.get(self.pos) {
                Some(b'+') => {
                    self.pos += 1;
                    1i64
                }
                Some(b'-') => {
                    self.pos += 1;
                    -1i64
                }
                _ => 1i64,
            };
            let exp_start = self.pos;
            while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(invalid());
            }
            let exp_str =
                std::str::from_utf8(&self.bytes[exp_start..self.pos]).map_err(|_| invalid())?;
            let magnitude: i64 = exp_str.parse().map_err(|_| invalid())?;
            exp_val = exp_sign.checked_mul(magnitude).ok_or_else(invalid)?;
        }
        let mut digits = Vec::with_capacity(int_slice.len() + frac_slice.len());
        digits.extend_from_slice(int_slice);
        digits.extend_from_slice(frac_slice);
        let first_nz = digits.iter().position(|&b| b != b'0');
        let Some(first) = first_nz else {
            out.push(b'N');
            out.push(0);
            out.extend_from_slice(&0u64.to_be_bytes());
            out.extend_from_slice(&0i64.to_be_bytes());
            return Ok(());
        };
        let last = digits.iter().rposition(|&b| b != b'0').unwrap_or(first);
        let sig = &digits[first..=last];
        // Normalized scientific exponent: value = 0.<sig> * 10^norm_exp
        let norm_exp = exp_val
            .checked_add(int_slice.len() as i64)
            .and_then(|e| e.checked_sub(first as i64))
            .ok_or_else(invalid)?;
        out.push(b'N');
        out.push(u8::from(negative));
        out.extend_from_slice(&(sig.len() as u64).to_be_bytes());
        out.extend_from_slice(sig);
        out.extend_from_slice(&norm_exp.to_be_bytes());
        Ok(())
    }

    fn parse_value(&mut self, depth: usize, out: &mut Vec<u8>) -> AppResult<JsonKind> {
        if depth > MAX_JSON_DEPTH {
            return Err(invalid());
        }
        self.skip_ws();
        let b = *self.bytes.get(self.pos).ok_or_else(invalid)?;
        let kind = match b {
            b'n' => {
                self.consume_literal(b"null")?;
                out.push(b'0');
                JsonKind::Any
            }
            b't' => {
                self.consume_literal(b"true")?;
                out.push(b'T');
                JsonKind::Any
            }
            b'f' => {
                self.consume_literal(b"false")?;
                out.push(b'F');
                JsonKind::Any
            }
            b'"' => {
                let s = self.parse_string()?;
                out.push(b'S');
                out.extend_from_slice(&(s.len() as u64).to_be_bytes());
                out.extend_from_slice(s.as_bytes());
                JsonKind::Any
            }
            b'[' => {
                self.pos += 1;
                self.skip_ws();
                let mut elements: Vec<Vec<u8>> = Vec::new();
                if self.bytes.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                } else {
                    loop {
                        let mut elem = Vec::new();
                        self.parse_value(depth + 1, &mut elem)?;
                        elements.push(elem);
                        self.skip_ws();
                        match self.bytes.get(self.pos) {
                            Some(b',') => self.pos += 1,
                            Some(b']') => {
                                self.pos += 1;
                                break;
                            }
                            _ => return Err(invalid()),
                        }
                    }
                }
                out.push(b'[');
                out.extend_from_slice(&(elements.len() as u64).to_be_bytes());
                for elem in elements {
                    out.extend_from_slice(&elem);
                }
                JsonKind::Array
            }
            b'{' => {
                self.pos += 1;
                self.skip_ws();
                let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
                if self.bytes.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                } else {
                    let mut seen_keys = HashSet::new();
                    loop {
                        self.skip_ws();
                        let key = self.parse_string()?;
                        if !seen_keys.insert(key.clone()) {
                            return Err(invalid());
                        }
                        self.skip_ws();
                        if self.bytes.get(self.pos) != Some(&b':') {
                            return Err(invalid());
                        }
                        self.pos += 1;
                        let mut val = Vec::new();
                        self.parse_value(depth + 1, &mut val)?;
                        entries.push((key, val));
                        self.skip_ws();
                        match self.bytes.get(self.pos) {
                            Some(b',') => self.pos += 1,
                            Some(b'}') => {
                                self.pos += 1;
                                break;
                            }
                            _ => return Err(invalid()),
                        }
                    }
                }
                entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
                out.push(b'{');
                out.extend_from_slice(&(entries.len() as u64).to_be_bytes());
                for (key, val) in entries {
                    out.extend_from_slice(&(key.len() as u64).to_be_bytes());
                    out.extend_from_slice(key.as_bytes());
                    out.extend_from_slice(&val);
                }
                JsonKind::Object
            }
            b'-' | b'0'..=b'9' => {
                self.parse_number(out)?;
                JsonKind::Any
            }
            _ => return Err(invalid()),
        };
        Ok(kind)
    }
}

fn canonicalize_json(text: &str, expected: JsonKind) -> AppResult<Vec<u8>> {
    let mut parser = JsonParser::new(text);
    let mut out = Vec::with_capacity(text.len() + 16);
    let kind = parser.parse_value(0, &mut out)?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(invalid());
    }
    match expected {
        JsonKind::Any => {}
        JsonKind::Array if kind != JsonKind::Array => return Err(invalid()),
        JsonKind::Object if kind != JsonKind::Object => return Err(invalid()),
        _ => {}
    }
    Ok(out)
}

fn validate_cell(column: &MutationColumn, value: &CellValue, original: bool) -> AppResult<()> {
    match value {
        CellValue::Default if !original && (column.has_default || column.nullable) => Ok(()),
        CellValue::Default => Err(invalid()),
        CellValue::Null if column.nullable => Ok(()),
        CellValue::Null => Err(invalid()),
        CellValue::Text(text) => {
            if text.len() > MAX_CELL_BYTES {
                return Err(limit());
            }
            match column.data_type {
                ScalarType::Text { max_bytes } if text.len() <= max_bytes => Ok(()),
                ScalarType::Decimal { precision, scale } => {
                    decimal_parts(text, precision, scale).map(|_| ())
                }
                ScalarType::Float => float_bits(text).map(|_| ()),
                ScalarType::Boolean if matches!(text.as_str(), "true" | "false") => Ok(()),
                ScalarType::Binary { max_bytes } => validate_hex_binary(text, max_bytes),
                ScalarType::Date => parse_ymd(text).map(|_| ()),
                ScalarType::Time { scale } => parse_hms_nanos(text, scale).map(|_| ()),
                ScalarType::TimestampNtz { scale } => parse_timestamp_ntz(text, scale).map(|_| ()),
                ScalarType::TimestampLtz { scale } | ScalarType::TimestampTz { scale } => {
                    parse_timestamp_tz(text, scale).map(|_| ())
                }
                ScalarType::Variant => canonicalize_json(text, JsonKind::Any).map(|_| ()),
                ScalarType::Array => canonicalize_json(text, JsonKind::Array).map(|_| ()),
                ScalarType::Object => canonicalize_json(text, JsonKind::Object).map(|_| ()),
                ScalarType::Unsupported(_) => Err(unsupported()),
                _ => Err(invalid()),
            }
        }
    }
}
fn validate_original(target: &MutationTarget, values: &[CellValue]) -> AppResult<()> {
    if values.len() != target.columns.len() {
        return Err(invalid());
    }
    for (column, value) in target.columns.iter().zip(values) {
        validate_cell(column, value, true)?;
    }
    Ok(())
}
fn validate_assignments(
    target: &MutationTarget,
    values: &[(usize, CellValue)],
    update: bool,
) -> AppResult<()> {
    if values.is_empty() || values.len() > target.columns.len() {
        return Err(invalid());
    }
    let mut seen = [false; MAX_COLUMNS];
    for (index, value) in values {
        let column = target.columns.get(*index).ok_or_else(invalid)?;
        if seen[*index] || column.mode != WriteMode::Writable {
            return Err(invalid());
        }
        seen[*index] = true;
        if update && *value == CellValue::Default {
            return Err(unsupported());
        }
        validate_cell(column, value, false)?;
    }
    if !update
        && target.columns.iter().enumerate().any(|(i, c)| {
            !seen[i] && c.mode == WriteMode::Writable && !c.nullable && !c.has_default
        })
    {
        return Err(invalid());
    }
    Ok(())
}

fn fingerprint<'a>(
    target: &MutationTarget,
    values: impl Iterator<Item = &'a CellValue>,
) -> AppResult<[u8; 32]> {
    let mut digest = Sha256::new();
    for (column, value) in target.columns.iter().zip(values) {
        match value {
            CellValue::Default => return Err(unsupported()),
            CellValue::Null => digest.update([0]),
            CellValue::Text(text) => {
                digest.update([1]);
                match column.data_type {
                    ScalarType::Decimal { precision, scale } => {
                        let (integer, fraction, negative) = decimal_parts(text, precision, scale)?;
                        let integer = integer.trim_start_matches('0');
                        let fraction = fraction.trim_end_matches('0');
                        digest.update([u8::from(
                            negative && (!integer.is_empty() || !fraction.is_empty()),
                        )]);
                        digest.update((integer.len() as u64).to_be_bytes());
                        digest.update(integer.as_bytes());
                        digest.update((fraction.len() as u64).to_be_bytes());
                        digest.update(fraction.as_bytes());
                    }
                    ScalarType::Float => {
                        digest.update(float_bits(text)?.to_be_bytes());
                    }
                    ScalarType::Binary { max_bytes } => {
                        validate_hex_binary(text, max_bytes)?;
                        digest.update((text.len() as u64).to_be_bytes());
                        for b in text.bytes() {
                            digest.update([b.to_ascii_uppercase()]);
                        }
                    }
                    ScalarType::Date => {
                        let (year, month, day) = parse_ymd(text)?;
                        digest.update(year.to_be_bytes());
                        digest.update([month, day]);
                    }
                    ScalarType::Time { scale } => {
                        let (hour, minute, second, nanos, _) = parse_hms_nanos(text, scale)?;
                        digest.update([hour, minute, second]);
                        digest.update(nanos.to_be_bytes());
                    }
                    ScalarType::TimestampNtz { scale } => {
                        let parsed = parse_timestamp_ntz(text, scale)?;
                        digest.update(parsed.year.to_be_bytes());
                        digest.update([
                            parsed.month,
                            parsed.day,
                            parsed.hour,
                            parsed.minute,
                            parsed.second,
                        ]);
                        digest.update(parsed.nanos.to_be_bytes());
                    }
                    ScalarType::TimestampLtz { scale } | ScalarType::TimestampTz { scale } => {
                        let parsed = parse_timestamp_tz(text, scale)?;
                        digest.update(parsed.utc_seconds.to_be_bytes());
                        digest.update(parsed.nanos.to_be_bytes());
                    }
                    ScalarType::Variant => {
                        let canon = canonicalize_json(text, JsonKind::Any)?;
                        digest.update((canon.len() as u64).to_be_bytes());
                        digest.update(&canon);
                    }
                    ScalarType::Array => {
                        let canon = canonicalize_json(text, JsonKind::Array)?;
                        digest.update((canon.len() as u64).to_be_bytes());
                        digest.update(&canon);
                    }
                    ScalarType::Object => {
                        let canon = canonicalize_json(text, JsonKind::Object)?;
                        digest.update((canon.len() as u64).to_be_bytes());
                        digest.update(&canon);
                    }
                    _ => {
                        digest.update((text.len() as u64).to_be_bytes());
                        digest.update(text.as_bytes());
                    }
                }
            }
        }
    }
    Ok(digest.finalize().into())
}
struct Budget(usize);
impl Budget {
    fn add(&mut self, amount: usize) -> AppResult<()> {
        self.0 = self
            .0
            .checked_add(amount)
            .filter(|n| *n <= MAX_BATCH_BYTES)
            .ok_or_else(limit)?;
        Ok(())
    }
    fn cell(&mut self, value: &CellValue) -> AppResult<()> {
        // Input cell/index/Vec overhead plus one owned output binding. Payload
        // bytes are charged twice because compilation retains exact binds.
        self.add(96)?;
        if let CellValue::Text(text) = value {
            self.add(text.len().checked_mul(2).ok_or_else(limit)?)?;
        }
        Ok(())
    }
}

/// Compile a nonempty bounded batch. No query, transaction or capability is
/// created. Actual affected-row counts must equal expected_rows before commit.
pub fn compile(target: &MutationTarget, mutations: &[Mutation]) -> AppResult<CompiledBatch> {
    if mutations.is_empty() {
        return Err(invalid());
    }
    if mutations.len() > MAX_ROWS {
        return Err(limit());
    }
    let mut budget = Budget(0);
    budget.add(
        target.qualified_name.len()
            + target
                .columns
                .iter()
                .map(|c| c.name.len() + c.quoted.len() + 128)
                .sum::<usize>(),
    )?;
    let mut originals = HashSet::new();
    let mut original_by_unit = Vec::with_capacity(mutations.len());
    let mut postimages = Vec::with_capacity(mutations.len());
    let mut capacities = Vec::with_capacity(mutations.len());
    let mut unknown_insert = false;
    for mutation in mutations {
        let (original, assignments) = match mutation {
            Mutation::Insert { values } => {
                validate_assignments(target, values, false)?;
                (None, values.as_slice())
            }
            Mutation::Update { original, changes } => {
                validate_original(target, &original.values)?;
                validate_assignments(target, changes, true)?;
                (Some(original), changes.as_slice())
            }
            Mutation::Delete { original } => {
                validate_original(target, &original.values)?;
                (Some(original), &[][..])
            }
        };
        let mut assigned = [None; MAX_COLUMNS];
        for (index, value) in assignments {
            assigned[*index] = Some(value);
            budget.cell(value)?;
        }
        if let Some(original) = original {
            for value in &original.values {
                budget.cell(value)?;
            }
        }
        let original_key = original
            .map(|r| fingerprint(target, r.values.iter()))
            .transpose()?;
        if let Some(key) = original_key
            && !originals.insert(key)
        {
            return Err(overlap());
        }
        original_by_unit.push(original_key);
        let postimage = match mutation {
            Mutation::Delete { .. } => None,
            Mutation::Update { original, .. } => Some(fingerprint(
                target,
                original
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| assigned[i].unwrap_or(v)),
            )?),
            Mutation::Insert { .. }
                if (0..target.columns.len())
                    .all(|i| assigned[i].is_some_and(|v| *v != CellValue::Default)) =>
            {
                Some(fingerprint(
                    target,
                    (0..target.columns.len()).map(|i| assigned[i].expect("complete validated row")),
                )?)
            }
            Mutation::Insert { .. } => {
                unknown_insert = true;
                None
            }
        };
        postimages.push(postimage);
        // Conservative upper bound includes syntax, quoted identifiers, casts,
        // bind-vector storage and bounded overlap bookkeeping before allocation.
        let sql_capacity = 160
            + target.qualified_name.len()
            + assignments
                .iter()
                .map(|(i, _)| target.columns[*i].quoted.len() + 144)
                .sum::<usize>()
            + original.map_or(0, |_| {
                target
                    .columns
                    .iter()
                    .map(|c| c.quoted.len() + 160)
                    .sum::<usize>()
            });
        budget.add(sql_capacity + 256)?;
        capacities.push(sql_capacity);
    }
    if unknown_insert && !originals.is_empty() {
        return Err(unsupported());
    }
    for (i, postimage) in postimages.iter().enumerate() {
        if let Some(key) = postimage
            && originals.contains(key)
            && original_by_unit[i].as_ref() != Some(key)
        {
            return Err(overlap());
        }
    }
    let units: Vec<_> = mutations
        .iter()
        .zip(capacities)
        .map(|(mutation, capacity)| emit(target, mutation, capacity))
        .collect();
    if units.iter().any(|u| u.sql.len() > MAX_UNIT_SQL_BYTES) {
        return Err(limit());
    }
    Ok(CompiledBatch {
        units,
        accounted_bytes: budget.0,
    })
}

fn null_cast_type(data_type: ScalarType) -> String {
    match data_type {
        ScalarType::Text { .. } => "VARCHAR".into(),
        ScalarType::Decimal { precision, scale } => format!("NUMBER({precision},{scale})"),
        ScalarType::Float => "FLOAT".into(),
        ScalarType::Boolean => "BOOLEAN".into(),
        ScalarType::Binary { .. } => "BINARY".into(),
        ScalarType::Date => "DATE".into(),
        ScalarType::Time { scale } => format!("TIME({scale})"),
        ScalarType::TimestampNtz { scale } => format!("TIMESTAMP_NTZ({scale})"),
        ScalarType::TimestampLtz { scale } => format!("TIMESTAMP_LTZ({scale})"),
        ScalarType::TimestampTz { scale } => format!("TIMESTAMP_TZ({scale})"),
        ScalarType::Variant => "VARIANT".into(),
        ScalarType::Array => "ARRAY".into(),
        ScalarType::Object => "OBJECT".into(),
        ScalarType::Unsupported(_) => unreachable!("validated target"),
    }
}

fn binding_kind(data_type: ScalarType) -> BindingKind {
    match data_type {
        ScalarType::Text { .. } => BindingKind::Text,
        ScalarType::Decimal { .. } => BindingKind::Decimal,
        ScalarType::Float => BindingKind::Float,
        ScalarType::Boolean => BindingKind::Boolean,
        ScalarType::Binary { .. } => BindingKind::Binary,
        ScalarType::Date => BindingKind::Date,
        ScalarType::Time { .. } => BindingKind::Time,
        ScalarType::TimestampNtz { .. } => BindingKind::TimestampNtz,
        ScalarType::TimestampLtz { .. } => BindingKind::TimestampLtz,
        ScalarType::TimestampTz { .. } => BindingKind::TimestampTz,
        ScalarType::Variant => BindingKind::Variant,
        ScalarType::Array => BindingKind::Array,
        ScalarType::Object => BindingKind::Object,
        ScalarType::Unsupported(_) => unreachable!("validated target"),
    }
}

fn timestamp_ntz_format(text: &str) -> &'static str {
    let parsed = parse_timestamp_ntz(text, 9).expect("validated timestamp_ntz");
    match (parsed.separator, parsed.has_fraction) {
        (' ', true) => "YYYY-MM-DD HH24:MI:SS.FF",
        (' ', false) => "YYYY-MM-DD HH24:MI:SS",
        ('T', true) => "YYYY-MM-DD\"T\"HH24:MI:SS.FF",
        ('T', false) => "YYYY-MM-DD\"T\"HH24:MI:SS",
        _ => unreachable!(),
    }
}

fn timestamp_tz_format(text: &str) -> &'static str {
    let parsed = parse_timestamp_tz(text, 9).expect("validated timestamp_tz");
    match (parsed.separator, parsed.has_fraction, parsed.tz_space) {
        (' ', true, true) => "YYYY-MM-DD HH24:MI:SS.FF TZH:TZM",
        (' ', false, true) => "YYYY-MM-DD HH24:MI:SS TZH:TZM",
        (' ', true, false) => "YYYY-MM-DD HH24:MI:SS.FFTZH:TZM",
        (' ', false, false) => "YYYY-MM-DD HH24:MI:SSTZH:TZM",
        ('T', true, true) => "YYYY-MM-DD\"T\"HH24:MI:SS.FF TZH:TZM",
        ('T', false, true) => "YYYY-MM-DD\"T\"HH24:MI:SS TZH:TZM",
        ('T', true, false) => "YYYY-MM-DD\"T\"HH24:MI:SS.FFTZH:TZM",
        ('T', false, false) => "YYYY-MM-DD\"T\"HH24:MI:SSTZH:TZM",
        _ => unreachable!(),
    }
}

fn bind(sql: &mut String, bindings: &mut Vec<Binding>, column: &MutationColumn, value: &CellValue) {
    match value {
        CellValue::Default => {
            sql.push_str("DEFAULT");
        }
        CellValue::Null => {
            sql.push_str("CAST(? AS ");
            sql.push_str(&null_cast_type(column.data_type));
            sql.push(')');
            bindings.push(Binding {
                kind: binding_kind(column.data_type),
                value: None,
            });
        }
        CellValue::Text(text) => {
            match column.data_type {
                ScalarType::Text { .. }
                | ScalarType::Decimal { .. }
                | ScalarType::Float
                | ScalarType::Boolean => {
                    sql.push_str("CAST(? AS ");
                    sql.push_str(&null_cast_type(column.data_type));
                    sql.push(')');
                }
                ScalarType::Binary { .. } => {
                    sql.push_str("TO_BINARY(CAST(? AS VARCHAR), 'HEX')");
                }
                ScalarType::Date => {
                    sql.push_str("TO_DATE(CAST(? AS VARCHAR), 'YYYY-MM-DD')");
                }
                ScalarType::Time { scale } => {
                    let fmt = if text.contains('.') {
                        "HH24:MI:SS.FF"
                    } else {
                        "HH24:MI:SS"
                    };
                    sql.push_str("CAST(TO_TIME(CAST(? AS VARCHAR), '");
                    sql.push_str(fmt);
                    sql.push_str("') AS TIME(");
                    sql.push_str(&scale.to_string());
                    sql.push_str("))");
                }
                ScalarType::TimestampNtz { scale } => {
                    sql.push_str("CAST(TO_TIMESTAMP_NTZ(CAST(? AS VARCHAR), '");
                    sql.push_str(timestamp_ntz_format(text));
                    sql.push_str("') AS TIMESTAMP_NTZ(");
                    sql.push_str(&scale.to_string());
                    sql.push_str("))");
                }
                ScalarType::TimestampLtz { scale } => {
                    sql.push_str("CAST(TO_TIMESTAMP_LTZ(CAST(? AS VARCHAR), '");
                    sql.push_str(timestamp_tz_format(text));
                    sql.push_str("') AS TIMESTAMP_LTZ(");
                    sql.push_str(&scale.to_string());
                    sql.push_str("))");
                }
                ScalarType::TimestampTz { scale } => {
                    sql.push_str("CAST(TO_TIMESTAMP_TZ(CAST(? AS VARCHAR), '");
                    sql.push_str(timestamp_tz_format(text));
                    sql.push_str("') AS TIMESTAMP_TZ(");
                    sql.push_str(&scale.to_string());
                    sql.push_str("))");
                }
                ScalarType::Variant => {
                    sql.push_str("PARSE_JSON(CAST(? AS VARCHAR))");
                }
                ScalarType::Array => {
                    sql.push_str("TO_ARRAY(PARSE_JSON(CAST(? AS VARCHAR)))");
                }
                ScalarType::Object => {
                    sql.push_str("TO_OBJECT(PARSE_JSON(CAST(? AS VARCHAR)))");
                }
                ScalarType::Unsupported(_) => unreachable!("validated target"),
            }
            bindings.push(Binding {
                kind: binding_kind(column.data_type),
                value: Some(text.clone()),
            });
        }
    }
}

fn emit(target: &MutationTarget, mutation: &Mutation, capacity: usize) -> CompiledUnit {
    let mut sql = String::with_capacity(capacity);
    let mut bindings = Vec::new();
    let (kind, original) = match mutation {
        Mutation::Insert { values } => {
            let needs_select = values.iter().any(|(i, v)| {
                *v != CellValue::Default
                    && matches!(
                        target.columns[*i].data_type,
                        ScalarType::Variant | ScalarType::Array | ScalarType::Object
                    )
            });
            sql.push_str("INSERT INTO ");
            sql.push_str(&target.qualified_name);
            sql.push_str(" (");
            if needs_select {
                let non_default: Vec<_> = values
                    .iter()
                    .filter(|(_, v)| *v != CellValue::Default)
                    .collect();
                for (n, (i, _)) in non_default.iter().enumerate() {
                    if n > 0 {
                        sql.push_str(", ");
                    }
                    sql.push_str(&target.columns[*i].quoted);
                }
                sql.push_str(") SELECT ");
                for (n, (i, v)) in non_default.iter().enumerate() {
                    if n > 0 {
                        sql.push_str(", ");
                    }
                    bind(&mut sql, &mut bindings, &target.columns[*i], v);
                }
            } else {
                for (n, (i, _)) in values.iter().enumerate() {
                    if n > 0 {
                        sql.push_str(", ");
                    }
                    sql.push_str(&target.columns[*i].quoted);
                }
                sql.push_str(") VALUES (");
                for (n, (i, v)) in values.iter().enumerate() {
                    if n > 0 {
                        sql.push_str(", ");
                    }
                    bind(&mut sql, &mut bindings, &target.columns[*i], v);
                }
                sql.push(')');
            }
            (MutationKind::Insert, None)
        }
        Mutation::Update { original, changes } => {
            sql.push_str("UPDATE ");
            sql.push_str(&target.qualified_name);
            sql.push_str(" SET ");
            for (n, (i, v)) in changes.iter().enumerate() {
                if n > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&target.columns[*i].quoted);
                sql.push_str(" = ");
                bind(&mut sql, &mut bindings, &target.columns[*i], v);
            }
            (MutationKind::Update, Some(original))
        }
        Mutation::Delete { original } => {
            sql.push_str("DELETE FROM ");
            sql.push_str(&target.qualified_name);
            (MutationKind::Delete, Some(original))
        }
    };
    if let Some(original) = original {
        sql.push_str(" WHERE ");
        for (n, (column, value)) in target.columns.iter().zip(&original.values).enumerate() {
            if n > 0 {
                sql.push_str(" AND ");
            }
            let binary_text = matches!(column.data_type, ScalarType::Text { .. });
            if binary_text {
                sql.push_str("TO_BINARY(");
            }
            sql.push_str(&column.quoted);
            if binary_text {
                sql.push_str(", 'UTF-8')");
            }
            sql.push_str(" IS NOT DISTINCT FROM ");
            if binary_text {
                sql.push_str("TO_BINARY(");
            }
            bind(&mut sql, &mut bindings, column, value);
            if binary_text {
                sql.push_str(", 'UTF-8')");
            }
        }
    }
    debug_assert!(sql.len() <= capacity);
    CompiledUnit {
        kind,
        sql,
        bindings,
        expected_rows: 1,
    }
}

#[cfg(test)]
#[path = "snowflake_mutations_tests.rs"]
mod tests;
