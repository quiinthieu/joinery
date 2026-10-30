use crate::{domain::*, drivers::RowMutation};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::Path,
};

pub const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_IMPORT_ROWS: usize = 5_000;
pub const IMPORT_BATCH_ROWS: usize = 500;
pub const MAX_SAMPLE_ROWS: usize = 50;
pub const MAX_REPORTED_ISSUES: usize = 100;
pub const MAX_CELL_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_REVIEW_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCsvField {
    pub text: String,
    pub quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCsvRecord {
    pub line: usize,
    pub fields: Vec<ParsedCsvField>,
}

pub fn read_import_utf8(path: &Path, max_bytes: u64) -> AppResult<String> {
    let cap = max_bytes.min(MAX_IMPORT_BYTES);
    let meta = std::fs::metadata(path).map_err(|_| {
        AppError::invalid("Choose a readable regular file before starting CSV import")
    })?;
    if !meta.is_file() {
        return Err(AppError::invalid(
            "Choose a regular file rather than a directory or device",
        ));
    }
    if meta.len() > cap {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            format!("Import files are limited to {} MiB", cap / (1024 * 1024)),
        ));
    }
    let file = std::fs::File::open(path).map_err(|_| {
        AppError::invalid("Choose a readable regular file before starting CSV import")
    })?;
    let file_meta = file.metadata().map_err(|_| {
        AppError::invalid("Choose a readable regular file before starting CSV import")
    })?;
    if !file_meta.is_file() || file_meta.len() > cap {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            format!("Import files are limited to {} MiB", cap / (1024 * 1024)),
        ));
    }
    let mut bytes = Vec::with_capacity(file_meta.len() as usize);
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::invalid("Could not read the selected CSV file"))?;
    if bytes.len() as u64 > cap {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            format!("Import files are limited to {} MiB", cap / (1024 * 1024)),
        ));
    }
    if bytes.contains(&0) {
        return Err(AppError::invalid(
            "Binary files containing NUL bytes cannot be imported as CSV",
        ));
    }
    let mut text = String::from_utf8(bytes)
        .map_err(|_| AppError::invalid("CSV import files must be valid UTF-8 text"))?;
    if text.starts_with('\u{feff}') {
        text.drain(..'\u{feff}'.len_utf8());
    }
    Ok(text)
}

pub fn parse_csv_records(
    content: &str,
    delimiter: CsvDelimiter,
) -> Result<Vec<ParsedCsvRecord>, CsvImportIssue> {
    if content.is_empty() {
        return Ok(Vec::new());
    }
    let delim = delimiter.byte() as char;
    let mut chars = content.chars().peekable();
    let mut current_line = 1usize;
    let mut records = Vec::new();

    while chars.peek().is_some() {
        let record_line = current_line;
        let mut fields = Vec::new();

        loop {
            if chars.peek() == Some(&'"') {
                chars.next();
                let quote_start_line = current_line;
                let mut text = String::new();
                loop {
                    match chars.next() {
                        None => {
                            return Err(CsvImportIssue {
                                line: quote_start_line,
                                column: None,
                                message: format!(
                                    "Unterminated quoted field starting on line {quote_start_line}"
                                ),
                            });
                        }
                        Some('"') => {
                            if chars.peek() == Some(&'"') {
                                chars.next();
                                text.push('"');
                            } else {
                                break;
                            }
                        }
                        Some('\r') => {
                            if chars.peek() == Some(&'\n') {
                                chars.next();
                                text.push_str("\r\n");
                            } else {
                                text.push('\r');
                            }
                            current_line += 1;
                        }
                        Some('\n') => {
                            text.push('\n');
                            current_line += 1;
                        }
                        Some(ch) => text.push(ch),
                    }
                }

                match chars.peek().copied() {
                    Some(next) if next == delim => {
                        chars.next();
                        fields.push(ParsedCsvField { text, quoted: true });
                    }
                    Some('\r') => {
                        chars.next();
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                        current_line += 1;
                        fields.push(ParsedCsvField { text, quoted: true });
                        break;
                    }
                    Some('\n') => {
                        chars.next();
                        current_line += 1;
                        fields.push(ParsedCsvField { text, quoted: true });
                        break;
                    }
                    None => {
                        fields.push(ParsedCsvField { text, quoted: true });
                        break;
                    }
                    Some(_) => {
                        return Err(CsvImportIssue {
                            line: current_line,
                            column: None,
                            message: format!(
                                "Unexpected characters after closing quote on line {current_line}"
                            ),
                        });
                    }
                }
            } else {
                let mut text = String::new();
                let finished_record = loop {
                    match chars.peek().copied() {
                        Some(ch) if ch == delim => {
                            chars.next();
                            fields.push(ParsedCsvField {
                                text,
                                quoted: false,
                            });
                            break false;
                        }
                        Some('"') => {
                            return Err(CsvImportIssue {
                                line: current_line,
                                column: None,
                                message: format!(
                                    "Unescaped quote inside unquoted field on line {current_line}"
                                ),
                            });
                        }
                        Some('\r') => {
                            chars.next();
                            if chars.peek() == Some(&'\n') {
                                chars.next();
                            }
                            current_line += 1;
                            fields.push(ParsedCsvField {
                                text,
                                quoted: false,
                            });
                            break true;
                        }
                        Some('\n') => {
                            chars.next();
                            current_line += 1;
                            fields.push(ParsedCsvField {
                                text,
                                quoted: false,
                            });
                            break true;
                        }
                        None => {
                            fields.push(ParsedCsvField {
                                text,
                                quoted: false,
                            });
                            break true;
                        }
                        Some(ch) => {
                            chars.next();
                            text.push(ch);
                        }
                    }
                };
                if finished_record {
                    break;
                }
            }
        }

        records.push(ParsedCsvRecord {
            line: record_line,
            fields,
        });
    }

    Ok(records)
}

struct ValidatedImport {
    headers: Vec<String>,
    mappings: Vec<CsvColumnMapping>,
    unmapped_defaults: Vec<String>,
    unmapped_nulls: Vec<String>,
    sample_rows: Vec<Vec<Option<String>>>,
    validated_rows: Vec<Vec<(usize, Option<String>)>>,
    total_rows: usize,
    issues: Vec<CsvImportIssue>,
    total_issues: usize,
}

fn push_issue(issues: &mut Vec<CsvImportIssue>, total_issues: &mut usize, issue: CsvImportIssue) {
    *total_issues += 1;
    if issues.len() < MAX_REPORTED_ISSUES {
        issues.push(issue);
    }
}

fn decode_raw_field(field: &ParsedCsvField, options: &CsvImportOptions) -> Option<String> {
    if !field.quoted {
        if field.text.is_empty() {
            return None;
        }
        if !options.null_token.is_empty() && field.text == options.null_token {
            return None;
        }
    }
    Some(field.text.clone())
}

fn resolve_mappings(
    headers: &[String],
    has_header: bool,
    schema: &TableSchema,
    requested: &[CsvColumnMapping],
) -> Vec<CsvColumnMapping> {
    if requested.is_empty() {
        let mut used_targets = HashSet::new();
        let insertable_cols: Vec<&ColumnInfo> =
            schema.columns.iter().filter(|c| c.insertable).collect();
        return headers
            .iter()
            .enumerate()
            .map(|(csv_index, header)| {
                let target_column = if has_header {
                    let trimmed = header.trim();
                    let exact = insertable_cols
                        .iter()
                        .find(|c| c.name == trimmed && !used_targets.contains(c.name.as_str()));
                    let matched = exact.or_else(|| {
                        insertable_cols.iter().find(|c| {
                            c.name.eq_ignore_ascii_case(trimmed)
                                && !used_targets.contains(c.name.as_str())
                        })
                    });
                    matched.map(|col| {
                        used_targets.insert(col.name.as_str());
                        col.name.clone()
                    })
                } else {
                    insertable_cols.get(csv_index).map(|col| col.name.clone())
                };
                CsvColumnMapping {
                    csv_index,
                    target_column,
                }
            })
            .collect();
    }

    let mut by_index: HashMap<usize, Option<String>> = HashMap::new();
    for entry in requested {
        by_index.insert(
            entry.csv_index,
            entry
                .target_column
                .as_ref()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
        );
    }

    (0..headers.len())
        .map(|csv_index| CsvColumnMapping {
            csv_index,
            target_column: by_index.remove(&csv_index).flatten(),
        })
        .collect()
}

pub fn validate_and_coerce_import_cell(
    engine: EngineKind,
    column: &ColumnInfo,
    raw: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return if column.nullable {
            Ok(None)
        } else if column.has_default {
            Err("does not allow NULL (set mapping to <skip> to use its database default)".into())
        } else {
            Err("does not allow NULL".into())
        };
    };

    if raw.len() > MAX_CELL_BYTES {
        return Err("exceeds the 1 MiB cell size limit".into());
    }

    match column.kind {
        ColumnKind::Binary => Err("is a binary column and cannot be imported from CSV text".into()),
        ColumnKind::Integer => {
            let trimmed = raw.trim();
            let normalized = if engine == EngineKind::Mysql
                && column
                    .database_type
                    .to_ascii_lowercase()
                    .starts_with("tinyint(1)")
            {
                let lower = trimmed.to_ascii_lowercase();
                if matches!(lower.as_str(), "true" | "t") {
                    "1"
                } else if matches!(lower.as_str(), "false" | "f") {
                    "0"
                } else {
                    trimmed
                }
            } else {
                trimmed
            };

            let n = normalized.parse::<i128>().map_err(|_| {
                "expects a whole integer without fractional or exponent notation".to_owned()
            })?;

            let db_type = column.database_type.to_ascii_lowercase();
            let (min, max) = match engine {
                EngineKind::Mysql => {
                    let base = db_type.split(['(', ' ']).next().unwrap_or("");
                    let bits = match base {
                        "tinyint" => 8,
                        "smallint" | "year" => 16,
                        "mediumint" => 24,
                        "int" | "integer" => 32,
                        _ => 64,
                    };
                    if db_type.contains("unsigned") {
                        (0, (1i128 << bits) - 1)
                    } else {
                        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
                    }
                }
                _ => match db_type.as_str() {
                    "smallint" | "int2" => (i16::MIN as i128, i16::MAX as i128),
                    "integer" | "int4" => (i32::MIN as i128, i32::MAX as i128),
                    "oid" => (0, u32::MAX as i128),
                    _ => (i64::MIN as i128, i64::MAX as i128),
                },
            };

            if n < min || n > max {
                return Err(format!(
                    "integer is outside the range of {}",
                    column.database_type
                ));
            }
            Ok(Some(normalized.to_owned()))
        }
        ColumnKind::Decimal => {
            let trimmed = raw.trim();
            validate_decimal_token(engine, column, trimmed)?;
            Ok(Some(trimmed.to_owned()))
        }
        ColumnKind::Boolean => {
            let lower = raw.trim().to_ascii_lowercase();
            match lower.as_str() {
                "true" | "t" | "1" | "yes" | "y" => Ok(Some("true".to_owned())),
                "false" | "f" | "0" | "no" | "n" => Ok(Some("false".to_owned())),
                _ => Err("expects a boolean (true/false, t/f, 1/0)".into()),
            }
        }
        ColumnKind::Json => {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|_| "expects valid JSON".to_owned())?;
            Ok(Some(raw.to_owned()))
        }
        ColumnKind::Temporal => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err("expects a date or timestamp value".into());
            }
            Ok(Some(trimmed.to_owned()))
        }
        ColumnKind::Text | ColumnKind::Unknown => Ok(Some(raw.to_owned())),
    }
}

fn parse_decimal_dims(database_type: &str) -> Option<(usize, usize)> {
    let lower = database_type.to_ascii_lowercase();
    let (_, rest) = lower.split_once('(')?;
    let inside = rest.split(')').next()?;
    let (p_str, s_str) = inside.split_once(',')?;
    let p = p_str.trim().parse::<usize>().ok()?;
    let s = s_str.trim().parse::<usize>().ok()?;
    if p == 0 || s > p {
        return None;
    }
    Some((p, s))
}

fn validate_decimal_token(
    engine: EngineKind,
    column: &ColumnInfo,
    trimmed: &str,
) -> Result<(), String> {
    if trimmed.is_empty() {
        return Err("expects a decimal number".into());
    }
    let db_type = column.database_type.to_ascii_lowercase();
    let dims = parse_decimal_dims(&db_type);
    let is_fixed_decimal = engine == EngineKind::Mysql
        || db_type.starts_with("numeric(")
        || db_type.starts_with("decimal(");

    if is_fixed_decimal && let Some((precision, scale)) = dims {
        let unsigned = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        if (whole.is_empty() && fraction.is_empty())
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("expects a fixed-point decimal without exponent notation".into());
        }
        if whole.trim_start_matches('0').len() > precision - scale
            || fraction.trim_end_matches('0').len() > scale
        {
            return Err(format!(
                "would lose precision in {} (max {} integer digits, {} fractional digits)",
                column.database_type,
                precision - scale,
                scale
            ));
        }
        if db_type.contains("unsigned") && trimmed.starts_with('-') {
            return Err("column is unsigned and does not allow negative numbers".into());
        }
        return Ok(());
    }

    if engine == EngineKind::Mysql {
        return Err("this floating-point column cannot be safely imported via typed casts".into());
    }

    // PostgreSQL unbounded numeric or float4/float8
    let (mantissa, exponent_ok) = match trimmed.split_once(['e', 'E']) {
        Some((m, exp)) => {
            let exp_digits = exp.strip_prefix(['-', '+']).unwrap_or(exp);
            (
                m,
                !exp_digits.is_empty() && exp_digits.bytes().all(|b| b.is_ascii_digit()),
            )
        }
        None => (trimmed, true),
    };
    if !exponent_ok {
        return Err("expects a valid decimal number".into());
    }
    let unsigned = mantissa.strip_prefix(['-', '+']).unwrap_or(mantissa);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("expects a valid decimal number".into());
    }
    Ok(())
}

fn analyze_csv_import(
    file_content: &str,
    engine: EngineKind,
    schema: &TableSchema,
    options: &CsvImportOptions,
    requested_mappings: &[CsvColumnMapping],
    collect_mutations: bool,
) -> AppResult<ValidatedImport> {
    let records = match parse_csv_records(file_content, options.delimiter) {
        Ok(records) => records,
        Err(syntax_issue) => {
            return Ok(ValidatedImport {
                headers: Vec::new(),
                mappings: Vec::new(),
                unmapped_defaults: Vec::new(),
                unmapped_nulls: Vec::new(),
                sample_rows: Vec::new(),
                validated_rows: Vec::new(),
                total_rows: 0,
                issues: vec![syntax_issue],
                total_issues: 1,
            });
        }
    };

    if records.is_empty() {
        let issue = CsvImportIssue {
            line: 1,
            column: None,
            message: "The selected CSV file is empty".into(),
        };
        return Ok(ValidatedImport {
            headers: Vec::new(),
            mappings: Vec::new(),
            unmapped_defaults: Vec::new(),
            unmapped_nulls: Vec::new(),
            sample_rows: Vec::new(),
            validated_rows: Vec::new(),
            total_rows: 0,
            issues: vec![issue],
            total_issues: 1,
        });
    }

    let (headers, data_records): (Vec<String>, &[ParsedCsvRecord]) = if options.has_header {
        let hdrs = records[0]
            .fields
            .iter()
            .enumerate()
            .map(|(idx, f)| {
                let trimmed = f.text.trim();
                if trimmed.is_empty() {
                    format!("Column {}", idx + 1)
                } else {
                    trimmed.to_owned()
                }
            })
            .collect();
        (hdrs, &records[1..])
    } else {
        let hdrs = (0..records[0].fields.len())
            .map(|idx| format!("Column {}", idx + 1))
            .collect();
        (hdrs, &records[..])
    };

    if data_records.len() > MAX_IMPORT_ROWS {
        return Err(AppError::new(
            ErrorCode::ResultLimitReached,
            format!(
                "CSV import is limited to {MAX_IMPORT_ROWS} data rows per file (found {})",
                data_records.len()
            ),
        ));
    }

    let expected_width = headers.len();
    let mappings = resolve_mappings(&headers, options.has_header, schema, requested_mappings);
    let mut issues = Vec::new();
    let mut total_issues = 0usize;

    // Resolve and validate column mappings against TableSchema
    let mut mapped_targets: Vec<(usize, usize)> = Vec::new();
    let mut seen_target_indices = HashSet::new();

    for mapping in &mappings {
        let Some(target_name) = &mapping.target_column else {
            continue;
        };
        match schema.columns.iter().position(|c| &c.name == target_name) {
            None => push_issue(
                &mut issues,
                &mut total_issues,
                CsvImportIssue {
                    line: 1,
                    column: Some(target_name.clone()),
                    message: format!("Target column \"{target_name}\" does not exist in table"),
                },
            ),
            Some(schema_idx) => {
                let col = &schema.columns[schema_idx];
                if !col.insertable {
                    push_issue(
                        &mut issues,
                        &mut total_issues,
                        CsvImportIssue {
                            line: 1,
                            column: Some(col.name.clone()),
                            message: format!(
                                "Column \"{}\" is generated or read-only and cannot be set on insert",
                                col.name
                            ),
                        },
                    );
                } else if !seen_target_indices.insert(schema_idx) {
                    push_issue(
                        &mut issues,
                        &mut total_issues,
                        CsvImportIssue {
                            line: 1,
                            column: Some(col.name.clone()),
                            message: format!("Column \"{}\" is mapped more than once", col.name),
                        },
                    );
                } else {
                    mapped_targets.push((mapping.csv_index, schema_idx));
                }
            }
        }
    }

    if mapped_targets.is_empty() {
        push_issue(
            &mut issues,
            &mut total_issues,
            CsvImportIssue {
                line: 1,
                column: None,
                message: "Map at least one CSV column to an insertable table column".into(),
            },
        );
    }

    let mut unmapped_defaults = Vec::new();
    let mut unmapped_nulls = Vec::new();
    for (idx, col) in schema.columns.iter().enumerate() {
        if seen_target_indices.contains(&idx) {
            continue;
        }
        if col.has_default {
            unmapped_defaults.push(col.name.clone());
        } else if col.nullable {
            unmapped_nulls.push(col.name.clone());
        } else {
            push_issue(
                &mut issues,
                &mut total_issues,
                CsvImportIssue {
                    line: 1,
                    column: Some(col.name.clone()),
                    message: format!(
                        "Required column \"{}\" is not mapped (NOT NULL without a default)",
                        col.name
                    ),
                },
            );
        }
    }

    if data_records.is_empty() {
        push_issue(
            &mut issues,
            &mut total_issues,
            CsvImportIssue {
                line: if options.has_header { 2 } else { 1 },
                column: None,
                message: "No data rows were found to import".into(),
            },
        );
    }

    let mut sample_rows = Vec::with_capacity(data_records.len().min(MAX_SAMPLE_ROWS));
    let mut validated_rows = if collect_mutations {
        Vec::with_capacity(data_records.len())
    } else {
        Vec::new()
    };

    for record in data_records {
        if sample_rows.len() < MAX_SAMPLE_ROWS {
            sample_rows.push(
                record
                    .fields
                    .iter()
                    .map(|field| decode_raw_field(field, options))
                    .collect(),
            );
        }

        if record.fields.len() != expected_width {
            push_issue(
                &mut issues,
                &mut total_issues,
                CsvImportIssue {
                    line: record.line,
                    column: None,
                    message: format!(
                        "Expected {expected_width} column{} on line {}, found {}",
                        if expected_width == 1 { "" } else { "s" },
                        record.line,
                        record.fields.len()
                    ),
                },
            );
            continue;
        }

        let mut row_values = if collect_mutations {
            Vec::with_capacity(mapped_targets.len())
        } else {
            Vec::new()
        };

        for &(csv_idx, schema_idx) in &mapped_targets {
            let column = &schema.columns[schema_idx];
            let decoded = decode_raw_field(&record.fields[csv_idx], options);
            match validate_and_coerce_import_cell(engine, column, decoded.as_deref()) {
                Ok(coerced) => {
                    if collect_mutations {
                        row_values.push((schema_idx, coerced));
                    }
                }
                Err(reason) => {
                    push_issue(
                        &mut issues,
                        &mut total_issues,
                        CsvImportIssue {
                            line: record.line,
                            column: Some(column.name.clone()),
                            message: format!("Column \"{}\" {reason}", column.name),
                        },
                    );
                }
            }
        }

        if collect_mutations && total_issues == 0 {
            row_values.sort_by_key(|(schema_idx, _)| *schema_idx);
            validated_rows.push(row_values);
        }
    }

    Ok(ValidatedImport {
        headers,
        mappings,
        unmapped_defaults,
        unmapped_nulls,
        sample_rows,
        validated_rows,
        total_rows: data_records.len(),
        issues,
        total_issues,
    })
}

pub fn build_csv_import_preview(
    path: &Path,
    max_bytes: u64,
    file_name: String,
    engine: EngineKind,
    schema: &TableSchema,
    options: &CsvImportOptions,
    requested_mappings: &[CsvColumnMapping],
) -> AppResult<CsvImportPreview> {
    let content = read_import_utf8(path, max_bytes)?;
    let analyzed =
        analyze_csv_import(&content, engine, schema, options, requested_mappings, false)?;
    let batch_count = if analyzed.total_rows == 0 {
        0
    } else {
        analyzed.total_rows.div_ceil(IMPORT_BATCH_ROWS)
    };
    let can_prepare = analyzed.total_rows > 0 && analyzed.total_issues == 0;
    Ok(CsvImportPreview {
        file_name,
        headers: analyzed.headers,
        mappings: analyzed.mappings,
        unmapped_defaults: analyzed.unmapped_defaults,
        unmapped_nulls: analyzed.unmapped_nulls,
        sample_rows: analyzed.sample_rows,
        total_rows: analyzed.total_rows,
        batch_size: IMPORT_BATCH_ROWS,
        batch_count,
        issues: analyzed.issues,
        total_issues: analyzed.total_issues,
        can_prepare,
    })
}

pub struct PreparedImportBatch {
    pub mutations: Vec<RowMutation>,
    pub batch_count: usize,
    pub approx_bytes: usize,
}

pub fn prepare_csv_batch_mutations(
    path: &Path,
    max_bytes: u64,
    engine: EngineKind,
    schema: &TableSchema,
    options: &CsvImportOptions,
    requested_mappings: &[CsvColumnMapping],
    batch_index: usize,
) -> AppResult<PreparedImportBatch> {
    let content = read_import_utf8(path, max_bytes)?;
    let analyzed = analyze_csv_import(&content, engine, schema, options, requested_mappings, true)?;
    if analyzed.total_issues > 0 {
        let first = analyzed
            .issues
            .first()
            .map(|issue| format!("Line {}: {}", issue.line, issue.message))
            .unwrap_or_else(|| "CSV validation failed".to_owned());
        return Err(AppError::invalid(format!(
            "Fix CSV validation issues before preparing import batches ({first})"
        )));
    }
    if analyzed.total_rows == 0 {
        return Err(AppError::invalid("No data rows were found to import"));
    }
    let batch_count = analyzed.total_rows.div_ceil(IMPORT_BATCH_ROWS);
    if batch_index >= batch_count {
        return Err(AppError::invalid(format!(
            "Batch index {} is out of range (total batches: {batch_count})",
            batch_index + 1
        )));
    }

    let start = batch_index * IMPORT_BATCH_ROWS;
    let end = ((batch_index + 1) * IMPORT_BATCH_ROWS).min(analyzed.total_rows);
    let mut approx_bytes = 0usize;
    let mut mutations = Vec::with_capacity(end - start);

    for row_values in analyzed
        .validated_rows
        .into_iter()
        .skip(start)
        .take(end - start)
    {
        for (_, val) in &row_values {
            approx_bytes += val.as_ref().map_or(0, String::len);
        }
        if approx_bytes > MAX_BATCH_REVIEW_BYTES {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "This CSV import batch exceeds the 8 MiB review budget",
            ));
        }
        mutations.push(RowMutation::Insert { values: row_values });
    }

    Ok(PreparedImportBatch {
        mutations,
        batch_count,
        approx_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_schema() -> TableSchema {
        TableSchema {
            table: TableRef {
                schema: "public".into(),
                name: "customers".into(),
            },
            columns: vec![
                ColumnInfo {
                    id: "c0".into(),
                    name: "id".into(),
                    database_type: "bigint".into(),
                    kind: ColumnKind::Integer,
                    nullable: false,
                    primary_key: true,
                    foreign_key: false,
                    editable: false,
                    insertable: true,
                    has_default: true,
                },
                ColumnInfo {
                    id: "c1".into(),
                    name: "full_name".into(),
                    database_type: "text".into(),
                    kind: ColumnKind::Text,
                    nullable: false,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c2".into(),
                    name: "note".into(),
                    database_type: "text".into(),
                    kind: ColumnKind::Text,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c3".into(),
                    name: "credit".into(),
                    database_type: "numeric(12,4)".into(),
                    kind: ColumnKind::Decimal,
                    nullable: false,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c4".into(),
                    name: "active".into(),
                    database_type: "boolean".into(),
                    kind: ColumnKind::Boolean,
                    nullable: false,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
                ColumnInfo {
                    id: "c5".into(),
                    name: "meta".into(),
                    database_type: "jsonb".into(),
                    kind: ColumnKind::Json,
                    nullable: true,
                    primary_key: false,
                    foreign_key: false,
                    editable: true,
                    insertable: true,
                    has_default: false,
                },
            ],
            primary_key: vec!["id".into()],
            foreign_keys: vec![],
            editable: true,
        }
    }

    #[test]
    fn rfc4180_parser_preserves_multiline_quotes_null_vs_empty_and_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("customers.csv");
        // Includes UTF-8 BOM, multiline quoted note, empty quoted string "", unquoted NULL, and quoted literal "\N"
        let csv = "\u{feff}FULL_NAME,note,credit,active,meta\r\n\"Ada, \"\"Lovelace\"\"\",\"line 1\nline 2\",12345.6789,true,\"{\"\"tier\"\":1}\"\r\nGrace,,0.0000,0,\r\nLinus,\"\",99.50,yes,\"\\N\"\r\n";
        std::fs::write(&path, csv.as_bytes()).unwrap();

        let schema = sample_schema();
        let options = CsvImportOptions::default();
        // note has "\N" on row 3? Wait: meta is jsonb, let's check meta on row 3 vs note!
        // On row 3 above, meta is "\N" which is invalid JSON — let's verify that line 5 is flagged!
        let preview_bad_json = build_csv_import_preview(
            &path,
            MAX_IMPORT_BYTES,
            "customers.csv".into(),
            EngineKind::Postgres,
            &schema,
            &options,
            &[],
        )
        .unwrap();
        assert!(!preview_bad_json.can_prepare);
        assert_eq!(preview_bad_json.issues.len(), 1);
        assert_eq!(preview_bad_json.issues[0].line, 5); // line 1 header, line 2-3 Ada (multiline!), line 4 Grace, line 5 Linus!
        assert_eq!(preview_bad_json.issues[0].column.as_deref(), Some("meta"));

        // Now put quoted literal "\N" in `note` (text) and unquoted \N in `meta` (NULL jsonb)
        let valid_csv = "\u{feff}FULL_NAME,note,credit,active,meta\r\n\"Ada, \"\"Lovelace\"\"\",\"line 1\nline 2\",12345.6789,true,\"{\"\"tier\"\":1}\"\r\nGrace,,0.0000,0,\\N\r\nLinus,\"\",99.50,yes,\r\nAlan,\"\\N\",-12.25,false,\r\n";
        std::fs::write(&path, valid_csv.as_bytes()).unwrap();

        let preview = build_csv_import_preview(
            &path,
            MAX_IMPORT_BYTES,
            "customers.csv".into(),
            EngineKind::Postgres,
            &schema,
            &options,
            &[],
        )
        .unwrap();
        assert!(preview.can_prepare);
        assert_eq!(preview.total_rows, 4);
        assert_eq!(preview.batch_count, 1);
        assert_eq!(preview.unmapped_defaults, vec!["id"]);
        // Row 1 (Grace): unquoted empty note -> None, unquoted \N meta -> None
        assert_eq!(preview.sample_rows[1][1], None);
        assert_eq!(preview.sample_rows[1][4], None);
        // Row 2 (Linus): quoted "" note -> Some("")
        assert_eq!(preview.sample_rows[2][1].as_deref(), Some(""));
        // Row 3 (Alan): quoted "\N" note -> Some("\\N")
        assert_eq!(preview.sample_rows[3][1].as_deref(), Some("\\N"));

        let prepared = prepare_csv_batch_mutations(
            &path,
            MAX_IMPORT_BYTES,
            EngineKind::Postgres,
            &schema,
            &options,
            &preview.mappings,
            0,
        )
        .unwrap();
        assert_eq!(prepared.mutations.len(), 4);
        match &prepared.mutations[2] {
            RowMutation::Insert { values } => {
                // `id` (index 0) is omitted so DB default applies!
                assert!(values.iter().all(|(idx, _)| *idx != 0));
                assert_eq!(values[1], (2, Some(String::new())));
            }
            _ => panic!("expected Insert mutation"),
        }
    }

    #[test]
    fn export_to_import_csv_and_tsv_roundtrip_exact_int64_null_and_empty_strings() {
        use crate::export::{ExportStreamWriter, ExportWriterOptions};

        let schema = sample_schema();
        let dir = tempfile::tempdir().unwrap();
        let rows: Vec<TextRow> = vec![
            vec![
                Some("9223372036854775807".into()),
                Some("Aria".into()),
                None,
                Some("99999999.1234".into()),
                Some("true".into()),
                Some("{\"ok\":true}".into()),
            ],
            vec![
                Some("-9223372036854775808".into()),
                Some("".into()),
                Some("\\N".into()),
                Some("0.0001".into()),
                Some("false".into()),
                None,
            ],
        ];

        for (export_fmt, delim) in [
            (ExportFormat::Csv, CsvDelimiter::Comma),
            (ExportFormat::Tsv, CsvDelimiter::Tab),
        ] {
            let mut buf = Vec::new();
            let mut writer = ExportStreamWriter::try_new(
                &mut buf,
                ExportWriterOptions::new(export_fmt, schema.columns.clone(), true, None, None),
            )
            .unwrap();
            writer.write_rows(&rows).unwrap();
            writer.finish().unwrap();

            let path = dir
                .path()
                .join(format!("roundtrip.{}", export_fmt.extension()));
            std::fs::write(&path, &buf).unwrap();

            let options = CsvImportOptions {
                delimiter: delim,
                has_header: true,
                null_token: "\\N".into(),
            };
            let preview = build_csv_import_preview(
                &path,
                MAX_IMPORT_BYTES,
                "roundtrip".into(),
                EngineKind::Postgres,
                &schema,
                &options,
                &[],
            )
            .unwrap();
            assert!(preview.can_prepare, "issues: {:?}", preview.issues);
            assert_eq!(preview.sample_rows, rows);
        }
    }

    #[test]
    fn enforces_row_cap_binary_rejection_and_batch_slicing() {
        let dir = tempfile::tempdir().unwrap();
        let bin_path = dir.path().join("bad.bin");
        std::fs::write(&bin_path, b"full_name\nAda\0bad\n").unwrap();
        assert_eq!(
            read_import_utf8(&bin_path, MAX_IMPORT_BYTES)
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );

        let schema = sample_schema();
        let batch_path = dir.path().join("batches.csv");
        let mut csv = String::from("full_name,credit,active\n");
        for idx in 0..620 {
            csv.push_str(&format!("User {idx},10.25,true\n"));
        }
        std::fs::write(&batch_path, csv.as_bytes()).unwrap();

        let options = CsvImportOptions::default();
        let preview = build_csv_import_preview(
            &batch_path,
            MAX_IMPORT_BYTES,
            "batches.csv".into(),
            EngineKind::Postgres,
            &schema,
            &options,
            &[],
        )
        .unwrap();
        assert!(preview.can_prepare);
        assert_eq!(preview.total_rows, 620);
        assert_eq!(preview.batch_count, 2);
        assert_eq!(preview.sample_rows.len(), MAX_SAMPLE_ROWS);

        let b0 = prepare_csv_batch_mutations(
            &batch_path,
            MAX_IMPORT_BYTES,
            EngineKind::Postgres,
            &schema,
            &options,
            &preview.mappings,
            0,
        )
        .unwrap();
        let b1 = prepare_csv_batch_mutations(
            &batch_path,
            MAX_IMPORT_BYTES,
            EngineKind::Postgres,
            &schema,
            &options,
            &preview.mappings,
            1,
        )
        .unwrap();
        assert_eq!(b0.mutations.len(), 500);
        assert_eq!(b1.mutations.len(), 120);

        let too_many_path = dir.path().join("too_many.csv");
        let mut huge = String::from("full_name,credit,active\n");
        for idx in 0..=MAX_IMPORT_ROWS {
            huge.push_str(&format!("U{idx},1.0,1\n"));
        }
        std::fs::write(&too_many_path, huge.as_bytes()).unwrap();
        assert_eq!(
            build_csv_import_preview(
                &too_many_path,
                MAX_IMPORT_BYTES,
                "too_many.csv".into(),
                EngineKind::Postgres,
                &schema,
                &options,
                &[],
            )
            .unwrap_err()
            .code,
            ErrorCode::ResultLimitReached
        );
    }
}
