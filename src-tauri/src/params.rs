use crate::{
    domain::{AppError, AppResult, EngineKind, QueryParamInput, QueryParamKind, QueryParamSlot},
    sql_text::RewriteMap,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_QUERY_PARAMS: usize = 64;
pub const MAX_PARAM_VALUE_BYTES: usize = 256 * 1024;
pub const MAX_POSITIONAL_INDEX: usize = 256;
pub const MAX_PARAM_OCCURRENCES: usize = 1024;
pub const MAX_BOUND_PLAN_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedBind {
    pub key: String,
    pub kind: QueryParamKind,
    /// Canonical validated string representation, or `None` for SQL `NULL`.
    pub value: Option<String>,
    /// For `Decimal`, `(precision, scale)` within `1..=65` and `0..=38`.
    pub decimal_dims: Option<(u8, u8)>,
    /// For `Timestamp`, MySQL UTC `YYYY-MM-DD HH:MM:SS[.ffffff]` representation.
    pub mysql_datetime: Option<String>,
    /// Whether the original timestamp included an explicit timezone (`Z` or `[+-]HH:MM`).
    pub timestamp_has_tz: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundStatement {
    pub rewritten_sql: String,
    pub binds: Vec<TypedBind>,
    #[serde(skip)]
    pub rewrite_map: RewriteMap,
}

#[derive(Debug, Clone)]
struct RawToken {
    start: usize,
    end: usize,
    key: String,
    positional: bool,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn prev_blocks_placeholder(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return false;
    }
    let prev = bytes[idx - 1];
    prev.is_ascii_alphanumeric() || matches!(prev, b'_' | b'$' | b'\'' | b'"' | b'`' | b'@' | b':')
}

fn scan_raw_tokens(engine: EngineKind, sql: &str) -> AppResult<Vec<RawToken>> {
    if engine == EngineKind::Redis {
        return Ok(Vec::new());
    }
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut tokens = Vec::new();
    let mut question_counter = 0usize;
    let allow_backslash_escape = matches!(engine, EngineKind::Mysql | EngineKind::Snowflake);
    let allow_dollar_quote = matches!(engine, EngineKind::Postgres | EngineKind::Snowflake);
    let allow_dollar_params = engine != EngineKind::Redis;
    let allow_question_params = matches!(engine, EngineKind::Mysql | EngineKind::Snowflake);

    while i < len {
        let b = bytes[i];
        match b {
            // Line comment `--`
            b'-' if bytes.get(i + 1) == Some(&b'-')
                && (engine != EngineKind::Mysql
                    || bytes
                        .get(i + 2)
                        .is_none_or(|c| c.is_ascii_whitespace() || c.is_ascii_control())) =>
            {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // MySQL `#` line comment
            b'#' if engine == EngineKind::Mysql => {
                i += 1;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // Block comment `/* ... */` (nested on Postgres/Snowflake)
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                if engine == EngineKind::Mysql
                    && (bytes.get(i + 2) == Some(&b'!')
                        || (matches!(bytes.get(i + 2), Some(b'M' | b'm'))
                            && bytes.get(i + 3) == Some(&b'!')))
                {
                    return Err(AppError::new(
                        crate::domain::ErrorCode::Unsupported,
                        "MySQL executable comments are not supported. Submit their SQL explicitly for review.",
                    ));
                }
                i += 2;
                let mut depth = 1usize;
                while i < len && depth > 0 {
                    if engine != EngineKind::Mysql
                        && bytes[i] == b'/'
                        && bytes.get(i + 1) == Some(&b'*')
                    {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            // Single-quoted string `'...'` (plus PG `E'...'` when preceded by E/e)
            b'\'' => {
                let pg_escape = engine == EngineKind::Postgres
                    && i > 0
                    && matches!(bytes[i - 1], b'e' | b'E')
                    && (i == 1 || !is_ident_continue(bytes[i - 2]));
                i += 1;
                while i < len {
                    if (allow_backslash_escape || pg_escape) && bytes[i] == b'\\' {
                        i = (i + 2).min(len);
                    } else if bytes[i] == b'\'' {
                        if bytes.get(i + 1) == Some(&b'\'') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            // Double-quoted identifier/string `"..."`
            b'"' => {
                i += 1;
                while i < len {
                    if engine == EngineKind::Mysql && bytes[i] == b'\\' {
                        i = (i + 2).min(len);
                    } else if bytes[i] == b'"' {
                        if bytes.get(i + 1) == Some(&b'"') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            // Backtick-quoted identifier `` `...` `` (MySQL)
            b'`' if engine == EngineKind::Mysql => {
                i += 1;
                while i < len {
                    if bytes[i] == b'`' {
                        if bytes.get(i + 1) == Some(&b'`') {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            // Dollar-quoted string `$$...$$` / `$tag$...$tag$` OR positional `$1..$256`
            b'$' => {
                if allow_dollar_quote && !prev_blocks_placeholder(bytes, i) {
                    let tag_start = i;
                    let mut end = i + 1;
                    if end < len && bytes[end] == b'$' {
                        let tag = &bytes[tag_start..=end];
                        i = end + 1;
                        while i + tag.len() <= len && &bytes[i..i + tag.len()] != tag {
                            i += 1;
                        }
                        i = (i + tag.len()).min(len);
                        continue;
                    } else if end < len && is_ident_start(bytes[end]) {
                        while end < len && is_ident_continue(bytes[end]) {
                            end += 1;
                        }
                        if end < len && bytes[end] == b'$' {
                            let tag = &bytes[tag_start..=end];
                            i = end + 1;
                            while i + tag.len() <= len && &bytes[i..i + tag.len()] != tag {
                                i += 1;
                            }
                            i = (i + tag.len()).min(len);
                            continue;
                        }
                    }
                }
                if allow_dollar_params
                    && !prev_blocks_placeholder(bytes, i)
                    && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                {
                    let start = i;
                    let mut end = i + 1;
                    while end < len && bytes[end].is_ascii_digit() {
                        end += 1;
                    }
                    if end < len && (is_ident_continue(bytes[end]) || bytes[end] == b'$') {
                        i = end;
                        continue;
                    }
                    let digits = &sql[start + 1..end];
                    let index: usize = digits.parse().map_err(|_| {
                        AppError::invalid(format!(
                            "Positional parameter ${digits} must be between $1 and ${MAX_POSITIONAL_INDEX}"
                        ))
                    })?;
                    if index == 0 || index > MAX_POSITIONAL_INDEX {
                        return Err(AppError::invalid(format!(
                            "Positional parameter ${digits} must be between $1 and ${MAX_POSITIONAL_INDEX}"
                        )));
                    }
                    tokens.push(RawToken {
                        start,
                        end,
                        key: format!("${index}"),
                        positional: true,
                    });
                    i = end;
                    continue;
                }
                if allow_dollar_params
                    && !prev_blocks_placeholder(bytes, i)
                    && bytes.get(i + 1).is_some_and(|c| is_ident_start(*c))
                {
                    let start = i;
                    let mut end = i + 2;
                    while end < len && is_ident_continue(bytes[end]) {
                        end += 1;
                    }
                    let name = &sql[start..end];
                    if name.len() > 65 {
                        return Err(AppError::invalid(
                            "Parameter names must be 64 characters or fewer",
                        ));
                    }
                    tokens.push(RawToken {
                        start,
                        end,
                        key: name.to_owned(),
                        positional: false,
                    });
                    i = end;
                    continue;
                }
                i += 1;
            }
            // Named parameter `:name` (skipping `::` casts and `:=` assignments)
            b':' => {
                if bytes.get(i + 1) == Some(&b':') {
                    while i < len && bytes[i] == b':' {
                        i += 1;
                    }
                    continue;
                }
                if bytes.get(i + 1) == Some(&b'=') {
                    i += 2;
                    continue;
                }
                if !prev_blocks_placeholder(bytes, i)
                    && bytes.get(i + 1).is_some_and(|c| is_ident_start(*c))
                {
                    let start = i;
                    let mut end = i + 2;
                    while end < len && is_ident_continue(bytes[end]) {
                        end += 1;
                    }
                    let name = &sql[start + 1..end];
                    if name.len() > 64 {
                        return Err(AppError::invalid(
                            "Parameter names must be 64 characters or fewer",
                        ));
                    }
                    tokens.push(RawToken {
                        start,
                        end,
                        key: name.to_owned(),
                        positional: false,
                    });
                    i = end;
                    continue;
                }
                i += 1;
            }
            // Positional `?` (MySQL / MariaDB and Snowflake only; never PostgreSQL JSONB `?`)
            b'?' if allow_question_params => {
                question_counter += 1;
                tokens.push(RawToken {
                    start: i,
                    end: i + 1,
                    key: format!("?{question_counter}"),
                    positional: true,
                });
                i += 1;
            }
            _ => i += 1,
        }
    }

    Ok(tokens)
}

pub fn extract_query_params(engine: EngineKind, sql: &str) -> AppResult<Vec<QueryParamSlot>> {
    if sql.len() > 1024 * 1024 {
        return Err(AppError::invalid(
            "A query batch must be smaller than 1 MiB",
        ));
    }
    let tokens = scan_raw_tokens(engine, sql)?;
    if tokens.len() > MAX_PARAM_OCCURRENCES {
        return Err(AppError::invalid(
            "Use at most 1024 parameter occurrences in one query batch",
        ));
    }
    let mut slots: Vec<QueryParamSlot> = Vec::new();
    let mut index_by_key: HashMap<String, usize> = HashMap::new();

    for token in tokens {
        if let Some(&idx) = index_by_key.get(&token.key) {
            slots[idx].occurrences += 1;
        } else {
            if slots.len() >= MAX_QUERY_PARAMS {
                return Err(AppError::invalid(format!(
                    "Use at most {MAX_QUERY_PARAMS} distinct parameters in one query batch"
                )));
            }
            let idx = slots.len();
            index_by_key.insert(token.key.clone(), idx);
            slots.push(QueryParamSlot {
                key: token.key,
                positional: token.positional,
                occurrences: 1,
            });
        }
    }

    Ok(slots)
}

/// Rewrites placeholder tokens to dialect-native syntax (`$1` on PostgreSQL,
/// `?` on MySQL/Snowflake) purely so `sqlparser` can classify statements,
/// returning `(classification_sql, token_ranges)` where every token's byte
/// offset in `classification_sql` maps back to `[orig_start, orig_end)`.
pub struct ClassificationRewrite {
    pub sql: String,
    pub map: RewriteMap,
    pieces: Vec<(usize, usize, usize, usize)>, // (orig_start, orig_end, rew_start, rew_end)
}

impl ClassificationRewrite {
    /// Maps a byte offset in `classification.sql` back to the exact byte offset
    /// in `original_sql`.
    pub fn to_original_byte(&self, orig_len: usize, rew_byte: usize) -> usize {
        if rew_byte >= self.sql.len() {
            return orig_len;
        }
        for &(orig_start, orig_end, rew_start, rew_end) in &self.pieces {
            if rew_byte >= rew_start && rew_byte < rew_end {
                if orig_end - orig_start == rew_end - rew_start {
                    return orig_start + (rew_byte - rew_start);
                }
                return orig_start;
            }
        }
        orig_len
    }

    /// Maps an end byte offset (exclusive) in `classification.sql` back to the
    /// exclusive end byte offset in `original_sql`.
    pub fn to_original_end_byte(&self, orig_len: usize, rew_end_byte: usize) -> usize {
        if rew_end_byte == 0 {
            return 0;
        }
        if rew_end_byte >= self.sql.len() {
            return orig_len;
        }
        for &(orig_start, orig_end, rew_start, rew_end) in &self.pieces {
            if rew_end_byte > rew_start && rew_end_byte <= rew_end {
                if orig_end - orig_start == rew_end - rew_start {
                    return orig_start + (rew_end_byte - rew_start);
                }
                return orig_end;
            }
        }
        orig_len
    }
}

pub fn rewrite_for_classification(
    engine: EngineKind,
    sql: &str,
) -> AppResult<ClassificationRewrite> {
    let tokens = scan_raw_tokens(engine, sql)?;
    let replacement = if engine == EngineKind::Postgres {
        "$1"
    } else {
        "?"
    };
    let mut out = String::with_capacity(sql.len());
    let mut map = RewriteMap::default();
    let mut pieces = Vec::new();
    let mut cursor = 0usize;

    for token in &tokens {
        if token.start > cursor {
            let len = token.start - cursor;
            let rew_start = out.len();
            out.push_str(&sql[cursor..token.start]);
            map.push_verbatim(cursor, len);
            pieces.push((cursor, token.start, rew_start, out.len()));
        }
        let rew_start = out.len();
        out.push_str(replacement);
        map.push_replaced(token.start, token.end, replacement.len());
        pieces.push((token.start, token.end, rew_start, out.len()));
        cursor = token.end;
    }
    if cursor < sql.len() {
        let len = sql.len() - cursor;
        let rew_start = out.len();
        out.push_str(&sql[cursor..]);
        map.push_verbatim(cursor, len);
        pieces.push((cursor, sql.len(), rew_start, out.len()));
    }

    Ok(ClassificationRewrite {
        sql: out,
        map,
        pieces,
    })
}

pub fn validate_inputs(
    slots: &[QueryParamSlot],
    inputs: Option<&[QueryParamInput]>,
) -> AppResult<(Vec<QueryParamInput>, HashMap<String, TypedBind>)> {
    if slots.is_empty() {
        if inputs.is_some_and(|list| !list.is_empty()) {
            return Err(AppError::invalid(
                "This query contains no parameter placeholders",
            ));
        }
        return Ok((Vec::new(), HashMap::new()));
    }
    if inputs.is_some_and(|list| list.len() > MAX_QUERY_PARAMS) {
        return Err(AppError::invalid("Provide at most 64 query parameters"));
    }
    let provided = inputs.ok_or_else(|| {
        AppError::invalid(format!(
            "Provide a value or NULL for parameter {}",
            slots[0].key
        ))
    })?;

    let mut by_key: HashMap<&str, &QueryParamInput> = HashMap::with_capacity(provided.len());
    let mut total_bytes = 0usize;
    for item in provided {
        if by_key.insert(item.key.as_str(), item).is_some() {
            return Err(AppError::invalid(format!(
                "Parameter {} was provided more than once",
                item.key
            )));
        }
        if let Some(val) = &item.value {
            if val.contains('\0') {
                return Err(AppError::invalid(format!(
                    "Parameter {} cannot contain NUL bytes",
                    item.key
                )));
            }
            total_bytes = total_bytes.saturating_add(val.len());
            if total_bytes > MAX_PARAM_VALUE_BYTES {
                return Err(AppError::new(
                    crate::domain::ErrorCode::ResultLimitReached,
                    "Bound parameter values exceed the 256 KiB limit",
                ));
            }
        }
    }

    if by_key.len() > slots.len() {
        for key in by_key.keys() {
            if !slots.iter().any(|s| s.key == *key) {
                return Err(AppError::invalid(format!(
                    "Unknown query parameter {key} is not present in the SQL"
                )));
            }
        }
    }

    let mut canonical_inputs = Vec::with_capacity(slots.len());
    let mut typed_map = HashMap::with_capacity(slots.len());

    for slot in slots {
        let input = by_key.get(slot.key.as_str()).copied().ok_or_else(|| {
            AppError::invalid(format!(
                "Provide a value or NULL for parameter {}",
                slot.key
            ))
        })?;
        let bind = validate_single_bind(&slot.key, input.kind, input.value.as_deref())?;
        canonical_inputs.push(QueryParamInput {
            key: slot.key.clone(),
            kind: input.kind,
            value: bind.value.clone(),
        });
        typed_map.insert(slot.key.clone(), bind);
    }

    Ok((canonical_inputs, typed_map))
}

fn validate_single_bind(
    key: &str,
    kind: QueryParamKind,
    raw: Option<&str>,
) -> AppResult<TypedBind> {
    let Some(raw) = raw else {
        return Ok(TypedBind {
            key: key.to_owned(),
            kind,
            value: None,
            decimal_dims: None,
            mysql_datetime: None,
            timestamp_has_tz: false,
        });
    };

    match kind {
        QueryParamKind::Text => Ok(TypedBind {
            key: key.to_owned(),
            kind,
            value: Some(raw.to_owned()),
            decimal_dims: None,
            mysql_datetime: None,
            timestamp_has_tz: false,
        }),
        QueryParamKind::Integer => {
            let trimmed = raw.trim();
            let parsed: i64 = trimmed.parse().map_err(|_| {
                AppError::invalid(format!(
                    "Parameter {key} must be a 64-bit integer (or use Decimal for larger numbers)"
                ))
            })?;
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(parsed.to_string()),
                decimal_dims: None,
                mysql_datetime: None,
                timestamp_has_tz: false,
            })
        }
        QueryParamKind::Decimal => {
            let (canonical, dims) = parse_decimal(key, raw)?;
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(canonical),
                decimal_dims: Some(dims),
                mysql_datetime: None,
                timestamp_has_tz: false,
            })
        }
        QueryParamKind::Boolean => {
            let canonical = match raw.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "t" | "yes" => "true",
                "false" | "0" | "f" | "no" => "false",
                _ => {
                    return Err(AppError::invalid(format!(
                        "Parameter {key} must be a boolean (true or false)"
                    )));
                }
            };
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(canonical.to_owned()),
                decimal_dims: None,
                mysql_datetime: None,
                timestamp_has_tz: false,
            })
        }
        QueryParamKind::Uuid => {
            let parsed = uuid::Uuid::parse_str(raw.trim())
                .map_err(|_| AppError::invalid(format!("Parameter {key} must be a valid UUID")))?;
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(parsed.to_string()),
                decimal_dims: None,
                mysql_datetime: None,
                timestamp_has_tz: false,
            })
        }
        QueryParamKind::Date => {
            let trimmed = raw.trim();
            parse_ymd(trimmed).ok_or_else(|| {
                AppError::invalid(format!(
                    "Parameter {key} must be a valid YYYY-MM-DD calendar date"
                ))
            })?;
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(trimmed.to_owned()),
                decimal_dims: None,
                mysql_datetime: None,
                timestamp_has_tz: false,
            })
        }
        QueryParamKind::Timestamp => {
            let (canonical, mysql_dt, has_tz) = parse_timestamp(key, raw)?;
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(canonical),
                decimal_dims: None,
                mysql_datetime: Some(mysql_dt),
                timestamp_has_tz: has_tz,
            })
        }
        QueryParamKind::Json => {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|_| AppError::invalid(format!("Parameter {key} must be valid JSON")))?;
            Ok(TypedBind {
                key: key.to_owned(),
                kind,
                value: Some(raw.to_owned()),
                decimal_dims: None,
                mysql_datetime: None,
                timestamp_has_tz: false,
            })
        }
    }
}

fn parse_decimal(key: &str, raw: &str) -> AppResult<(String, (u8, u8))> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid(format!(
            "Parameter {key} must be a valid decimal number"
        )));
    }
    let (sign, rest) = if let Some(s) = trimmed.strip_prefix('-') {
        ("-", s)
    } else if let Some(s) = trimmed.strip_prefix('+') {
        ("", s)
    } else {
        ("", trimmed)
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    if (int_part.is_empty() && frac_part.is_none_or(str::is_empty))
        || !int_part.bytes().all(|b| b.is_ascii_digit())
        || frac_part.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(AppError::invalid(format!(
            "Parameter {key} must be a valid decimal number without exponent notation"
        )));
    }
    let normalized_int = {
        let stripped = int_part.trim_start_matches('0');
        if stripped.is_empty() { "0" } else { stripped }
    };
    let scale = frac_part.map_or(0usize, str::len);
    let int_digits = if normalized_int == "0" {
        1
    } else {
        normalized_int.len()
    };
    let precision = int_digits + scale;
    if precision > 65 || scale > 38 {
        return Err(AppError::invalid(format!(
            "Parameter {key} exceeds maximum decimal precision (65 digits, scale 38)"
        )));
    }
    let canonical = match frac_part {
        Some(f) => format!("{sign}{normalized_int}.{f}"),
        None => format!("{sign}{normalized_int}"),
    };
    let is_zero = normalized_int == "0" && frac_part.is_none_or(|f| f.bytes().all(|b| b == b'0'));
    let final_str = if is_zero && sign == "-" {
        canonical.trim_start_matches('-').to_owned()
    } else {
        canonical
    };
    Ok((final_str, (precision as u8, scale as u8)))
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return None,
    })
}

fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    if !s.is_ascii() || s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    if year < 1 || day < 1 || day > days_in_month(year, month)? {
        return None;
    }
    Some((year, month, day))
}

/// Howard Hinnant civil-from-days / days-from-civil algorithms for exact UTC
/// offset normalization without external chrono dependency.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(m <= 2);
    (year, m as u32, d as u32)
}

fn parse_timestamp(key: &str, raw: &str) -> AppResult<(String, String, bool)> {
    let err = || {
        AppError::invalid(format!(
            "Parameter {key} must be an ISO-8601 timestamp (e.g. 2026-10-31 12:30:00 or 2026-10-31T12:30:00Z)"
        ))
    };
    let trimmed = raw.trim();
    if !trimmed.is_ascii() || trimmed.len() < 19 {
        return Err(err());
    }
    let (year, month, day) = parse_ymd(&trimmed[0..10]).ok_or_else(err)?;
    let sep = trimmed.as_bytes()[10];
    if !matches!(sep, b'T' | b't' | b' ') {
        return Err(err());
    }
    let time_part = &trimmed[11..];
    if time_part.len() < 8 || time_part.as_bytes()[2] != b':' || time_part.as_bytes()[5] != b':' {
        return Err(err());
    }
    let hour: u32 = time_part[0..2].parse().map_err(|_| err())?;
    let minute: u32 = time_part[3..5].parse().map_err(|_| err())?;
    let second: u32 = time_part[6..8].parse().map_err(|_| err())?;
    if hour > 23 || minute > 59 || second > 59 {
        return Err(err());
    }
    let mut rest = &time_part[8..];
    let mut frac = "";
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits = after_dot.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return Err(err());
        }
        frac = &after_dot[..digits];
        rest = &after_dot[digits..];
    }

    let rest = rest.trim_start();
    let (has_tz, offset_seconds): (bool, i64) = if rest.is_empty() {
        (false, 0)
    } else if matches!(rest, "Z" | "z") {
        (true, 0)
    } else {
        let sign = match rest.as_bytes().first() {
            Some(b'+') => 1i64,
            Some(b'-') => -1i64,
            _ => return Err(err()),
        };
        let tz = &rest[1..];
        let (tz_h, tz_m) = if tz.len() == 5 && tz.as_bytes()[2] == b':' {
            (
                tz[0..2].parse::<i64>().map_err(|_| err())?,
                tz[3..5].parse::<i64>().map_err(|_| err())?,
            )
        } else if tz.len() == 4 {
            (
                tz[0..2].parse::<i64>().map_err(|_| err())?,
                tz[2..4].parse::<i64>().map_err(|_| err())?,
            )
        } else if tz.len() == 2 {
            (tz[0..2].parse::<i64>().map_err(|_| err())?, 0)
        } else {
            return Err(err());
        };
        if tz_h > 14 || tz_m > 59 || (tz_h == 14 && tz_m != 0) {
            return Err(err());
        }
        (true, sign * (tz_h * 3600 + tz_m * 60))
    };

    let day_num = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    let total_secs =
        day_num * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)
            - offset_seconds;
    let utc_days = total_secs.div_euclid(86_400);
    let rem_secs = total_secs.rem_euclid(86_400);
    let (uy, um, ud) = civil_from_days(utc_days);
    if !(1..=9999).contains(&uy) {
        return Err(err());
    }
    let uh = rem_secs / 3600;
    let umin = (rem_secs % 3600) / 60;
    let us = rem_secs % 60;

    let mysql_frac = if frac.is_empty() {
        String::new()
    } else {
        format!(".{}", &frac[..frac.len().min(6)])
    };
    let mysql_dt = format!("{uy:04}-{um:02}-{ud:02} {uh:02}:{umin:02}:{us:02}{mysql_frac}");

    Ok((trimmed.to_owned(), mysql_dt, has_tz))
}

/// Compiles a single statement's template SQL (and a global `?` counter across
/// statements in a batch) into a dialect-rewritten `BoundStatement`.
pub fn compile_statement(
    engine: EngineKind,
    statement_sql: &str,
    validated: &HashMap<String, TypedBind>,
    question_offset: &mut usize,
) -> AppResult<Option<BoundStatement>> {
    let mut budget = MAX_BOUND_PLAN_BYTES;
    compile_statement_with_budget(
        engine,
        statement_sql,
        validated,
        question_offset,
        &mut budget,
    )
}

pub fn compile_statement_with_budget(
    engine: EngineKind,
    statement_sql: &str,
    validated: &HashMap<String, TypedBind>,
    question_offset: &mut usize,
    remaining_bytes: &mut usize,
) -> AppResult<Option<BoundStatement>> {
    let mut tokens = scan_raw_tokens(engine, statement_sql)?;
    if tokens.len() > MAX_PARAM_OCCURRENCES {
        return Err(AppError::invalid(
            "Use at most 1024 parameter occurrences in one query batch",
        ));
    }
    if tokens.is_empty() {
        return Ok(None);
    }
    for token in &mut tokens {
        if token.key.starts_with('?') {
            *question_offset += 1;
            token.key = format!("?{}", *question_offset);
        }
    }

    let mut rewritten = String::with_capacity(statement_sql.len() + 32);
    let mut map = RewriteMap::default();
    let mut binds: Vec<TypedBind> = Vec::new();
    let mut pg_slot_by_key: HashMap<String, usize> = HashMap::new();
    let mut cursor = 0usize;

    for token in &tokens {
        if token.start > cursor {
            rewritten.push_str(&statement_sql[cursor..token.start]);
            map.push_verbatim(cursor, token.start - cursor);
        }
        let bind = validated.get(&token.key).ok_or_else(|| {
            AppError::invalid(format!(
                "Provide a value or NULL for parameter {}",
                token.key
            ))
        })?;

        let duplicate_pg =
            engine == EngineKind::Postgres && pg_slot_by_key.contains_key(&token.key);
        if !duplicate_pg {
            let bytes = bind.key.len()
                + bind.value.as_ref().map_or(4, String::len)
                + bind.mysql_datetime.as_ref().map_or(0, String::len)
                + 128;
            *remaining_bytes = remaining_bytes.checked_sub(bytes).ok_or_else(|| {
                AppError::new(
                    crate::domain::ErrorCode::ResultLimitReached,
                    "Prepared parameters exceed the 8 MiB review budget",
                )
            })?;
        }
        if engine == EngineKind::Snowflake && bind.decimal_dims.is_some_and(|(p, _)| p > 38) {
            return Err(AppError::invalid(format!(
                "Parameter {} exceeds Snowflake's 38-digit decimal precision",
                bind.key
            )));
        }
        if engine == EngineKind::Mysql
            && bind.kind == QueryParamKind::Timestamp
            && bind.value.as_deref().is_some_and(|s| {
                s.split_once('.')
                    .is_some_and(|(_, f)| f.bytes().take_while(u8::is_ascii_digit).count() > 6)
            })
        {
            return Err(AppError::invalid(format!(
                "Parameter {} exceeds MySQL's six fractional timestamp digits",
                bind.key
            )));
        }
        let piece = match engine {
            EngineKind::Postgres => {
                let slot_num = if let Some(&existing) = pg_slot_by_key.get(&token.key) {
                    existing
                } else {
                    binds.push(bind.clone());
                    let next = binds.len();
                    pg_slot_by_key.insert(token.key.clone(), next);
                    next
                };
                format!("${slot_num}")
            }
            EngineKind::Mysql => {
                binds.push(bind.clone());
                mysql_placeholder_expr(bind)
            }
            EngineKind::Snowflake => {
                binds.push(bind.clone());
                snowflake_placeholder_expr(bind)
            }
            EngineKind::Redis => unreachable!(),
        };

        rewritten.push_str(&piece);
        map.push_replaced(token.start, token.end, piece.len());
        cursor = token.end;
    }

    if cursor < statement_sql.len() {
        rewritten.push_str(&statement_sql[cursor..]);
        map.push_verbatim(cursor, statement_sql.len() - cursor);
    }

    Ok(Some(BoundStatement {
        rewritten_sql: rewritten,
        binds,
        rewrite_map: map,
    }))
}

fn mysql_placeholder_expr(bind: &TypedBind) -> String {
    match bind.kind {
        QueryParamKind::Integer | QueryParamKind::Boolean => "?".into(),
        QueryParamKind::Text | QueryParamKind::Uuid | QueryParamKind::Json => {
            "CONVERT(? USING utf8mb4)".into()
        }
        QueryParamKind::Decimal => {
            let (p, s) = bind.decimal_dims.unwrap_or((38, 10));
            let p = p.max(s).clamp(1, 65);
            format!("CAST(? AS DECIMAL({p},{s}))")
        }
        QueryParamKind::Date => "CAST(? AS DATE)".into(),
        QueryParamKind::Timestamp => {
            let has_frac = bind
                .mysql_datetime
                .as_deref()
                .is_some_and(|dt| dt.contains('.'));
            if has_frac {
                "CAST(? AS DATETIME(6))".into()
            } else {
                "CAST(? AS DATETIME)".into()
            }
        }
    }
}

fn snowflake_placeholder_expr(bind: &TypedBind) -> String {
    match bind.kind {
        QueryParamKind::Text
        | QueryParamKind::Integer
        | QueryParamKind::Boolean
        | QueryParamKind::Uuid => "?".into(),
        QueryParamKind::Decimal => {
            let (p, s) = bind.decimal_dims.unwrap_or((38, 10));
            let p = p.clamp(1, 38);
            let s = s.min(p);
            format!("CAST(? AS NUMBER({p},{s}))")
        }
        QueryParamKind::Date => "CAST(? AS DATE)".into(),
        QueryParamKind::Timestamp => {
            if bind.timestamp_has_tz {
                "CAST(? AS TIMESTAMP_TZ)".into()
            } else {
                "CAST(? AS TIMESTAMP_NTZ)".into()
            }
        }
        QueryParamKind::Json => "PARSE_JSON(?)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unicode_calendar_inputs_and_lossy_engine_conversions() {
        for (kind, value) in [
            (QueryParamKind::Date, "🧪🧪xx"),
            (QueryParamKind::Timestamp, "2026-11-01T🧪:00:00"),
            (QueryParamKind::Timestamp, "2026-11-01T12:00:00+14:01"),
        ] {
            assert!(validate_single_bind("value", kind, Some(value)).is_err());
        }
        for (engine, kind, value) in [
            (
                EngineKind::Snowflake,
                QueryParamKind::Decimal,
                "123456789012345678901234567890123456789",
            ),
            (
                EngineKind::Mysql,
                QueryParamKind::Timestamp,
                "2026-11-01T12:00:00.1234567Z",
            ),
        ] {
            let slots = extract_query_params(engine, "SELECT :value").unwrap();
            let (_, values) = validate_inputs(
                &slots,
                Some(&[QueryParamInput {
                    key: "value".into(),
                    kind,
                    value: Some(value.into()),
                }]),
            )
            .unwrap();
            assert!(compile_statement(engine, "SELECT :value", &values, &mut 0).is_err());
        }
    }

    #[test]
    fn bounds_repeated_values_and_keeps_question_numbering_across_statements() {
        let sql = "SELECT :value, :value";
        let slots = extract_query_params(EngineKind::Mysql, sql).unwrap();
        let (_, values) = validate_inputs(
            &slots,
            Some(&[QueryParamInput {
                key: "value".into(),
                kind: QueryParamKind::Text,
                value: Some("x".repeat(1000)),
            }]),
        )
        .unwrap();
        assert!(
            compile_statement_with_budget(EngineKind::Mysql, sql, &values, &mut 0, &mut 1500)
                .is_err()
        );
        let sql = format!("SELECT {}", vec![":v"; MAX_PARAM_OCCURRENCES + 1].join(","));
        assert!(extract_query_params(EngineKind::Mysql, &sql).is_err());
        let slots = extract_query_params(EngineKind::Mysql, "SELECT ?; SELECT ?").unwrap();
        let (_, values) = validate_inputs(
            &slots,
            Some(&[
                QueryParamInput {
                    key: "?1".into(),
                    kind: QueryParamKind::Text,
                    value: Some("first".into()),
                },
                QueryParamInput {
                    key: "?2".into(),
                    kind: QueryParamKind::Text,
                    value: Some("second".into()),
                },
            ]),
        )
        .unwrap();
        let mut offset = 0;
        let first = compile_statement(EngineKind::Mysql, "SELECT ?", &values, &mut offset)
            .unwrap()
            .unwrap();
        let second = compile_statement(EngineKind::Mysql, "SELECT ?", &values, &mut offset)
            .unwrap()
            .unwrap();
        assert_eq!(first.binds[0].value.as_deref(), Some("first"));
        assert_eq!(second.binds[0].value.as_deref(), Some("second"));
    }

    #[test]
    fn postgres_scanner_ignores_strings_dollar_quotes_comments_casts_assignments_and_jsonb_question()
     {
        let sql = r#"
            -- :ignored_line_comment $9
            /* outer /* :ignored_nested_comment $8 */ still comment */
            SELECT ':ignored_str',
                   E'escaped \' :ignored_escape',
                   ":ignored_ident",
                   $$ :ignored_dollar $7 $$,
                   $body$ :ignored_tagged $6 $body$,
                   col$1,
                   payload::jsonb ? 'key' AS has_key,
                   payload::jsonb ?| ARRAY['a', 'b'] AS has_any,
                   payload::jsonb ?& ARRAY['a', 'b'] AS has_all,
                   (:customer_id)::bigint,
                   $2,
                   :customer_id
            FROM orders
            WHERE status = :order_status
        "#;
        let slots = extract_query_params(EngineKind::Postgres, sql).unwrap();
        assert_eq!(
            slots,
            vec![
                QueryParamSlot {
                    key: "customer_id".into(),
                    positional: false,
                    occurrences: 2,
                },
                QueryParamSlot {
                    key: "$2".into(),
                    positional: true,
                    occurrences: 1,
                },
                QueryParamSlot {
                    key: "order_status".into(),
                    positional: false,
                    occurrences: 1,
                },
            ]
        );

        let plpgsql = "DO $$ DECLARE x int := 10; BEGIN PERFORM x; END $$";
        assert!(
            extract_query_params(EngineKind::Postgres, plpgsql)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn mysql_and_snowflake_scanners_handle_question_marks_escapes_comments_and_assignments() {
        let mysql_sql = "SELECT 'it\\'s ? :no', `:no_bt`, \"a\\\"?b\", @v := 1, :status, ? # ? :hash_comment\nFROM orders WHERE id = ? -- :line_comment ?";
        let mysql_slots = extract_query_params(EngineKind::Mysql, mysql_sql).unwrap();
        assert_eq!(
            mysql_slots,
            vec![
                QueryParamSlot {
                    key: "status".into(),
                    positional: false,
                    occurrences: 1,
                },
                QueryParamSlot {
                    key: "?1".into(),
                    positional: true,
                    occurrences: 1,
                },
                QueryParamSlot {
                    key: "?2".into(),
                    positional: true,
                    occurrences: 1,
                },
            ]
        );

        let sf_sql = "SELECT $$ :skip ? $1 $$, payload::VARIANT, :region, $1, ? FROM SALES";
        let sf_slots = extract_query_params(EngineKind::Snowflake, sf_sql).unwrap();
        assert_eq!(sf_slots.len(), 3);
        assert_eq!(sf_slots[0].key, "region");
        assert_eq!(sf_slots[1].key, "$1");
        assert_eq!(sf_slots[2].key, "?1");
    }

    #[test]
    fn validates_all_typed_kinds_nulls_limits_and_rejects_malformed_values() {
        let sql = "SELECT :txt, :num, :dec, :flag, :uid, :dt, :ts, :js, :nil";
        let slots = extract_query_params(EngineKind::Postgres, sql).unwrap();
        let inputs = vec![
            QueryParamInput {
                key: "txt".into(),
                kind: QueryParamKind::Text,
                value: Some("O'Reilly \\ 🧪".into()),
            },
            QueryParamInput {
                key: "num".into(),
                kind: QueryParamKind::Integer,
                value: Some(" -9223372036854775808 ".into()),
            },
            QueryParamInput {
                key: "dec".into(),
                kind: QueryParamKind::Decimal,
                value: Some("+00123.4500".into()),
            },
            QueryParamInput {
                key: "flag".into(),
                kind: QueryParamKind::Boolean,
                value: Some("YES".into()),
            },
            QueryParamInput {
                key: "uid".into(),
                kind: QueryParamKind::Uuid,
                value: Some("550E8400-E29B-41D4-A716-446655440000".into()),
            },
            QueryParamInput {
                key: "dt".into(),
                kind: QueryParamKind::Date,
                value: Some("2024-02-29".into()),
            },
            QueryParamInput {
                key: "ts".into(),
                kind: QueryParamKind::Timestamp,
                value: Some("2026-10-31T23:30:00.123456+07:00".into()),
            },
            QueryParamInput {
                key: "js".into(),
                kind: QueryParamKind::Json,
                value: Some(r#"{"ok":true,"n":1}"#.into()),
            },
            QueryParamInput {
                key: "nil".into(),
                kind: QueryParamKind::Integer,
                value: None,
            },
        ];
        let (canonical, map) = validate_inputs(&slots, Some(&inputs)).unwrap();
        assert_eq!(canonical[2].value.as_deref(), Some("123.4500"));
        assert_eq!(map["dec"].decimal_dims, Some((7, 4)));
        assert_eq!(canonical[3].value.as_deref(), Some("true"));
        assert_eq!(
            canonical[4].value.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(
            map["ts"].mysql_datetime.as_deref(),
            Some("2026-10-31 16:30:00.123456")
        );
        assert!(map["ts"].timestamp_has_tz);
        assert_eq!(canonical[8].value, None);

        // Non-leap Feb 29 rejected
        let mut bad_date = inputs.clone();
        bad_date[5].value = Some("2025-02-29".into());
        assert!(validate_inputs(&slots, Some(&bad_date)).is_err());

        // NUL byte rejected
        let mut bad_nul = inputs.clone();
        bad_nul[0].value = Some("ab\0cd".into());
        assert!(validate_inputs(&slots, Some(&bad_nul)).is_err());

        // Invalid JSON rejected
        let mut bad_json = inputs.clone();
        bad_json[7].value = Some("{broken".into());
        assert!(validate_inputs(&slots, Some(&bad_json)).is_err());

        // Missing slot rejected
        assert!(validate_inputs(&slots, Some(&inputs[..8])).is_err());
    }

    #[test]
    fn compiles_per_statement_rewrites_and_preserves_utf16_error_spans() {
        let orig = "SELECT '🧪', :very_long_param, bad_col, :very_long_param FROM orders";
        let slots = extract_query_params(EngineKind::Postgres, orig).unwrap();
        let (_, map) = validate_inputs(
            &slots,
            Some(&[QueryParamInput {
                key: "very_long_param".into(),
                kind: QueryParamKind::Integer,
                value: Some("42".into()),
            }]),
        )
        .unwrap();
        let mut q = 0;
        let bound = compile_statement(EngineKind::Postgres, orig, &map, &mut q)
            .unwrap()
            .unwrap();
        assert_eq!(
            bound.rewritten_sql,
            "SELECT '🧪', $1, bad_col, $1 FROM orders"
        );
        assert_eq!(bound.binds.len(), 1);
        let rew_bad_char = (bound.rewritten_sql[..bound.rewritten_sql.find("bad_col").unwrap()]
            .chars()
            .count()
            + 1) as u32;
        let span = bound.rewrite_map.span_from_rewritten_pg_char_pos(
            orig,
            &bound.rewritten_sql,
            rew_bad_char,
        );
        let units: Vec<u16> = orig.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[span.start_utf16..span.end_utf16]).unwrap(),
            "bad_col"
        );
    }

    #[test]
    fn scans_named_dollar_parameters_across_dialects_and_preserves_dollar_quotes() {
        for engine in [
            EngineKind::Postgres,
            EngineKind::Snowflake,
            EngineKind::Mysql,
        ] {
            let sql = "SELECT * FROM orders WHERE customer_id = $cust_id AND region = $region AND total > $cust_id";
            let slots = extract_query_params(engine, sql).unwrap();
            assert_eq!(slots.len(), 2);
            assert_eq!(slots[0].key, "$cust_id");
            assert_eq!(slots[0].occurrences, 2);
            assert!(!slots[0].positional);
            assert_eq!(slots[1].key, "$region");
            assert_eq!(slots[1].occurrences, 1);
            assert!(!slots[1].positional);
        }

        let quote_sql = "SELECT $$ $ignored_in_quote $$, $real_param FROM dual";
        let slots = extract_query_params(EngineKind::Snowflake, quote_sql).unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].key, "$real_param");
    }
}
