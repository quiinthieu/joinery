use crate::domain::*;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

pub const MAX_EXPORT_ROWS: u64 = 1_000_000;
pub const MAX_EXPORT_BYTES: u64 = 512 * 1024 * 1024;
const EXPORT_BUFFER_BYTES: usize = 64 * 1024;
const SQL_INSERT_BATCH_ROWS: u64 = 500;

pub fn resolve_export_columns(
    columns: &[ColumnInfo],
    requested_ids: &[String],
) -> AppResult<(Vec<ColumnInfo>, Vec<usize>)> {
    if columns.is_empty() {
        return Err(AppError::invalid("No columns are available to export"));
    }
    if requested_ids.is_empty() {
        let indices = (0..columns.len()).collect();
        return Ok((columns.to_vec(), indices));
    }
    let mut selected = Vec::with_capacity(requested_ids.len());
    let mut indices = Vec::with_capacity(requested_ids.len());
    for requested in requested_ids {
        let idx = columns
            .iter()
            .position(|col| &col.id == requested)
            .ok_or_else(|| AppError::invalid(format!("Unknown export column id: {requested}")))?;
        if indices.contains(&idx) {
            return Err(AppError::invalid(
                "Duplicate export column selection is not allowed",
            ));
        }
        selected.push(columns[idx].clone());
        indices.push(idx);
    }
    if selected.is_empty() {
        return Err(AppError::invalid("Select at least one column to export"));
    }
    Ok((selected, indices))
}

pub fn partial_export_path(destination: &Path) -> AppResult<(PathBuf, String, String)> {
    let file_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::invalid("Choose a destination file with a valid UTF-8 name"))?
        .to_owned();
    let partial_name = format!("{file_name}.partial");
    let partial_path = destination.with_file_name(&partial_name);
    Ok((partial_path, file_name, partial_name))
}

pub struct ExportWriterOptions {
    pub format: ExportFormat,
    pub columns: Vec<ColumnInfo>,
    pub include_headers: bool,
    pub engine: Option<EngineKind>,
    pub target_table: Option<TableRef>,
    pub max_rows: u64,
    pub max_bytes: u64,
}

impl ExportWriterOptions {
    pub fn new(
        format: ExportFormat,
        columns: Vec<ColumnInfo>,
        include_headers: bool,
        engine: Option<EngineKind>,
        target_table: Option<TableRef>,
    ) -> Self {
        Self {
            format,
            columns,
            include_headers,
            engine,
            target_table,
            max_rows: MAX_EXPORT_ROWS,
            max_bytes: MAX_EXPORT_BYTES,
        }
    }
}

pub struct ExportStreamWriter<W: Write> {
    out: W,
    options: ExportWriterOptions,
    rows_written: u64,
    bytes_written: u64,
    header_emitted: bool,
    delimited_has_line: bool,
    last_byte_was_newline: bool,
    json_keys: Vec<String>,
    sql_prefix: String,
}

impl<W: Write> ExportStreamWriter<W> {
    pub fn try_new(out: W, options: ExportWriterOptions) -> AppResult<Self> {
        if options.columns.is_empty() {
            return Err(AppError::invalid("Select at least one column to export"));
        }
        let json_keys = options
            .columns
            .iter()
            .map(|c| {
                serde_json::to_string(&c.name)
                    .map_err(|_| AppError::internal("Could not encode JSON column name"))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let sql_prefix = if options.format == ExportFormat::SqlInsert {
            let table_name = format_sql_table_name(options.target_table.as_ref(), options.engine);
            let cols = options
                .columns
                .iter()
                .map(|c| quote_sql_identifier(&c.name, options.engine))
                .collect::<Vec<_>>()
                .join(", ");
            format!("INSERT INTO {table_name} ({cols})\nVALUES\n")
        } else {
            String::new()
        };
        Ok(Self {
            out,
            options,
            rows_written: 0,
            bytes_written: 0,
            header_emitted: false,
            delimited_has_line: false,
            last_byte_was_newline: true,
            json_keys,
            sql_prefix,
        })
    }

    pub fn rows_written(&self) -> u64 {
        self.rows_written
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    fn emit(&mut self, text: &str) -> AppResult<()> {
        if text.is_empty() {
            return Ok(());
        }
        let next_bytes = self
            .bytes_written
            .checked_add(text.len() as u64)
            .ok_or_else(byte_limit_error)?;
        if next_bytes > self.options.max_bytes {
            return Err(byte_limit_error());
        }
        self.out
            .write_all(text.as_bytes())
            .map_err(|_| AppError::internal("Could not write to export file"))?;
        self.bytes_written = next_bytes;
        self.last_byte_was_newline = text.as_bytes().last() == Some(&b'\n');
        Ok(())
    }

    fn ensure_preamble(&mut self) -> AppResult<()> {
        if self.header_emitted {
            return Ok(());
        }
        self.header_emitted = true;
        match self.options.format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                if self.options.include_headers {
                    let (delimiter, null_token) = delimited_tokens(self.options.format);
                    let col_count = self.options.columns.len();
                    for idx in 0..col_count {
                        if idx > 0 {
                            self.emit(delimiter)?;
                        }
                        let cell = format_delimited_header(
                            &self.options.columns[idx].name,
                            delimiter,
                            null_token,
                        );
                        self.emit(&cell)?;
                    }
                    self.delimited_has_line = true;
                }
            }
            ExportFormat::Markdown => {
                self.emit("| ")?;
                let col_count = self.options.columns.len();
                for idx in 0..col_count {
                    if idx > 0 {
                        self.emit(" | ")?;
                    }
                    let escaped = escape_markdown_text(&self.options.columns[idx].name);
                    self.emit(&escaped)?;
                }
                self.emit(" |\n| ")?;
                for idx in 0..col_count {
                    if idx > 0 {
                        self.emit(" | ")?;
                    }
                    let align = if matches!(
                        self.options.columns[idx].kind,
                        ColumnKind::Integer | ColumnKind::Decimal
                    ) {
                        "---:"
                    } else {
                        "---"
                    };
                    self.emit(align)?;
                }
                self.emit(" |")?;
            }
            ExportFormat::Json | ExportFormat::Ndjson | ExportFormat::SqlInsert => {}
        }
        Ok(())
    }

    pub fn write_row(&mut self, row: &[Option<String>]) -> AppResult<()> {
        if row.len() != self.options.columns.len() {
            return Err(AppError::internal(
                "Export row width did not match column metadata",
            ));
        }
        if self.rows_written >= self.options.max_rows {
            return Err(row_limit_error(self.options.max_rows));
        }
        self.ensure_preamble()?;

        match self.options.format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                let (delimiter, null_token) = delimited_tokens(self.options.format);
                if self.delimited_has_line {
                    self.emit("\n")?;
                }
                for (idx, value) in row.iter().enumerate() {
                    if idx > 0 {
                        self.emit(delimiter)?;
                    }
                    let cell = format_delimited_cell(value.as_deref(), delimiter, null_token);
                    self.emit(&cell)?;
                }
                self.delimited_has_line = true;
            }
            ExportFormat::Json => {
                if self.rows_written == 0 {
                    self.emit("[\n  {\n")?;
                } else {
                    self.emit(",\n  {\n")?;
                }
                let col_count = self.options.columns.len();
                for (idx, cell_value) in row.iter().enumerate().take(col_count) {
                    let key = self.json_keys[idx].clone();
                    let formatted =
                        format_json_cell(cell_value.as_deref(), &self.options.columns[idx], true)?;
                    self.emit("    ")?;
                    self.emit(&key)?;
                    self.emit(": ")?;
                    self.emit(&formatted)?;
                    if idx + 1 < col_count {
                        self.emit(",\n")?;
                    } else {
                        self.emit("\n")?;
                    }
                }
                self.emit("  }")?;
            }
            ExportFormat::Ndjson => {
                self.emit("{")?;
                let col_count = self.options.columns.len();
                for (idx, cell_value) in row.iter().enumerate().take(col_count) {
                    if idx > 0 {
                        self.emit(",")?;
                    }
                    let key = self.json_keys[idx].clone();
                    let formatted =
                        format_json_cell(cell_value.as_deref(), &self.options.columns[idx], false)?;
                    self.emit(&key)?;
                    self.emit(":")?;
                    self.emit(&formatted)?;
                }
                self.emit("}\n")?;
            }
            ExportFormat::SqlInsert => {
                if self.rows_written == 0 {
                    let prefix = self.sql_prefix.clone();
                    self.emit(&prefix)?;
                } else if self.rows_written.is_multiple_of(SQL_INSERT_BATCH_ROWS) {
                    let prefix = self.sql_prefix.clone();
                    self.emit(");\n\n")?;
                    self.emit(&prefix)?;
                } else {
                    self.emit("),\n")?;
                }
                self.emit("  (")?;
                let col_count = self.options.columns.len();
                for (idx, cell_value) in row.iter().enumerate().take(col_count) {
                    if idx > 0 {
                        self.emit(", ")?;
                    }
                    let formatted = format_sql_cell(
                        cell_value.as_deref(),
                        &self.options.columns[idx],
                        self.options.engine,
                    );
                    self.emit(&formatted)?;
                }
            }
            ExportFormat::Markdown => {
                self.emit("\n| ")?;
                let col_count = self.options.columns.len();
                for (idx, cell_value) in row.iter().enumerate().take(col_count) {
                    if idx > 0 {
                        self.emit(" | ")?;
                    }
                    let cell = match cell_value.as_deref() {
                        None => "NULL".to_owned(),
                        Some(raw) => escape_markdown_text(raw),
                    };
                    self.emit(&cell)?;
                }
                self.emit(" |")?;
            }
        }

        self.rows_written += 1;
        Ok(())
    }

    pub fn write_rows(&mut self, rows: &[TextRow]) -> AppResult<()> {
        for row in rows {
            self.write_row(row)?;
        }
        Ok(())
    }

    pub fn write_projected_rows(
        &mut self,
        rows: &[TextRow],
        column_indices: &[usize],
    ) -> AppResult<()> {
        let mut projected = Vec::with_capacity(column_indices.len());
        for row in rows {
            projected.clear();
            for &idx in column_indices {
                let value = row
                    .get(idx)
                    .ok_or_else(|| AppError::internal("Export column projection out of bounds"))?
                    .clone();
                projected.push(value);
            }
            self.write_row(&projected)?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> AppResult<(W, u64, u64)> {
        self.ensure_preamble()?;
        match self.options.format {
            ExportFormat::Json => {
                if self.rows_written == 0 {
                    self.emit("[]")?;
                } else {
                    self.emit("\n]")?;
                }
            }
            ExportFormat::SqlInsert => {
                if self.rows_written > 0 {
                    self.emit(");")?;
                }
            }
            ExportFormat::Csv
            | ExportFormat::Tsv
            | ExportFormat::Ndjson
            | ExportFormat::Markdown => {}
        }
        self.out
            .flush()
            .map_err(|_| AppError::internal("Could not flush export file"))?;
        Ok((self.out, self.rows_written, self.bytes_written))
    }

    pub fn abort_incomplete(mut self, reason: &str) -> (W, u64, u64) {
        let clean_reason: String = reason
            .chars()
            .map(|ch| {
                if ch == '\r' || ch == '\n' || ch.is_control() {
                    ' '
                } else {
                    ch
                }
            })
            .take(240)
            .collect();
        let prefix = if self.bytes_written == 0 || self.last_byte_was_newline {
            ""
        } else {
            "\n"
        };
        let trailer = match self.options.format {
            ExportFormat::Csv | ExportFormat::Tsv => format!(
                "{prefix}# JOINERY EXPORT INCOMPLETE ({clean_reason}; {} rows written)\n",
                self.rows_written
            ),
            ExportFormat::Json => format!(
                "{prefix}/* JOINERY EXPORT INCOMPLETE ({clean_reason}; {} rows written) */\n",
                self.rows_written
            ),
            ExportFormat::Ndjson => {
                let encoded_reason =
                    serde_json::to_string(&clean_reason).unwrap_or_else(|_| "\"aborted\"".into());
                format!(
                    "{prefix}{{\"__joinery_export_incomplete\":true,\"reason\":{encoded_reason},\"rowsWritten\":{}}}\n",
                    self.rows_written
                )
            }
            ExportFormat::SqlInsert => format!(
                "{prefix}-- JOINERY EXPORT INCOMPLETE ({clean_reason}; {} rows written)\n",
                self.rows_written
            ),
            ExportFormat::Markdown => format!(
                "{prefix}\n> JOINERY EXPORT INCOMPLETE ({clean_reason}; {} rows written)\n",
                self.rows_written
            ),
        };
        if self.out.write_all(trailer.as_bytes()).is_ok() {
            self.bytes_written = self.bytes_written.saturating_add(trailer.len() as u64);
        }
        let _ = self.out.flush();
        (self.out, self.rows_written, self.bytes_written)
    }
}

pub struct ExportFileSession {
    destination: PathBuf,
    partial_path: PathBuf,
    output_file_name: String,
    partial_file_name: String,
    writer: ExportStreamWriter<BufWriter<File>>,
}

impl ExportFileSession {
    pub fn create(destination: PathBuf, options: ExportWriterOptions) -> AppResult<Self> {
        if let Ok(meta) = std::fs::symlink_metadata(&destination)
            && meta.file_type().is_symlink()
        {
            return Err(AppError::invalid(
                "Refusing to overwrite a symbolic link during export",
            ));
        }
        let (partial_path, output_file_name, partial_file_name) =
            partial_export_path(&destination)?;
        if let Ok(meta) = std::fs::symlink_metadata(&partial_path)
            && meta.file_type().is_symlink()
        {
            return Err(AppError::invalid(
                "Refusing to write through a symbolic link partial file",
            ));
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&partial_path)
            .map_err(|_| AppError::internal("Could not open export destination file"))?;
        let buffered = BufWriter::with_capacity(EXPORT_BUFFER_BYTES, file);
        let writer = ExportStreamWriter::try_new(buffered, options)?;
        Ok(Self {
            destination,
            partial_path,
            output_file_name,
            partial_file_name,
            writer,
        })
    }

    pub fn file_name(&self) -> &str {
        &self.output_file_name
    }

    pub fn rows_written(&self) -> u64 {
        self.writer.rows_written()
    }

    pub fn bytes_written(&self) -> u64 {
        self.writer.bytes_written()
    }

    pub fn write_rows(&mut self, rows: &[TextRow]) -> AppResult<()> {
        self.writer.write_rows(rows)
    }

    pub fn write_projected_rows(
        &mut self,
        rows: &[TextRow],
        column_indices: &[usize],
    ) -> AppResult<()> {
        self.writer.write_projected_rows(rows, column_indices)
    }

    pub fn complete(self) -> AppResult<(u64, u64, String)> {
        let (buffered, rows, bytes) = self.writer.finish()?;
        let file = buffered
            .into_inner()
            .map_err(|_| AppError::internal("Could not flush export buffer"))?;
        file.sync_data()
            .map_err(|_| AppError::internal("Could not sync export file to disk"))?;
        drop(file);
        std::fs::rename(&self.partial_path, &self.destination)
            .map_err(|_| AppError::internal("Could not finalize exported file"))?;
        Ok((rows, bytes, self.output_file_name))
    }

    pub fn abort(self, reason: &str) -> (u64, u64, String) {
        let (buffered, rows, bytes) = self.writer.abort_incomplete(reason);
        if let Ok(file) = buffered.into_inner() {
            let _ = file.sync_data();
        }
        (rows, bytes, self.partial_file_name)
    }
}

fn byte_limit_error() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Export exceeded the 512 MiB file size limit. Narrow the query or filter before exporting.",
    )
}

fn row_limit_error(max_rows: u64) -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        format!(
            "Export exceeded the {} row limit. Narrow the query or filter before exporting.",
            max_rows
        ),
    )
}

fn delimited_tokens(format: ExportFormat) -> (&'static str, &'static str) {
    match format {
        ExportFormat::Tsv => ("\t", "\\N"),
        _ => (",", ""),
    }
}

fn quote_delimited(raw: &str) -> String {
    format!("\"{}\"", raw.replace('"', "\"\""))
}

fn needs_delimited_quotes(raw: &str, delimiter: &str, null_token: &str) -> bool {
    raw == "\\N"
        || (!null_token.is_empty() && raw == null_token)
        || raw.contains(delimiter)
        || raw.contains('"')
        || raw.contains('\n')
        || raw.contains('\r')
}

fn format_delimited_header(name: &str, delimiter: &str, null_token: &str) -> String {
    if name.is_empty() || needs_delimited_quotes(name, delimiter, null_token) {
        quote_delimited(name)
    } else {
        name.to_owned()
    }
}

fn format_delimited_cell(cell: Option<&str>, delimiter: &str, null_token: &str) -> String {
    let Some(raw) = cell else {
        return null_token.to_owned();
    };
    if raw.is_empty() {
        return "\"\"".to_owned();
    }
    if needs_delimited_quotes(raw, delimiter, null_token) {
        quote_delimited(raw)
    } else {
        raw.to_owned()
    }
}

pub fn is_sql_integer(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let rest = match bytes.first() {
        Some(b'-') => &bytes[1..],
        Some(_) => bytes,
        None => return false,
    };
    if rest.is_empty() {
        return false;
    }
    if rest[0] == b'0' {
        return rest.len() == 1;
    }
    rest[0].is_ascii_digit() && rest[1..].iter().all(u8::is_ascii_digit)
}

pub fn is_rfc8259_number(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut i = 0;
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    match bytes.get(i) {
        Some(b'0') => {
            i += 1;
            if bytes.get(i).is_some_and(u8::is_ascii_digit) {
                return false;
            }
        }
        Some(b'1'..=b'9') => {
            i += 1;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        if !bytes.get(i).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !bytes.get(i).is_some_and(u8::is_ascii_digit) {
            return false;
        }
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    i == bytes.len()
}

fn is_json_column(column: &ColumnInfo) -> bool {
    column.kind == ColumnKind::Json && !column.database_type.trim().ends_with("[]")
}

/// Validates RFC 8259 JSON syntax without float coercion and formats tokens losslessly.
fn format_embedded_json_lossless(raw: &str, pretty: bool) -> Option<String> {
    let trimmed = raw.trim();
    if serde_json::from_str::<serde::de::IgnoredAny>(trimmed).is_err() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut out = String::with_capacity(trimmed.len() + if pretty { 32 } else { 0 });
    let mut depth = 0usize;
    let mut i = 0usize;

    let next_non_space = |mut idx: usize| -> usize {
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        idx
    };

    while i < bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if ch == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push_str(&trimmed[start..i.min(bytes.len())]);
            continue;
        }
        if ch == b'{' || ch == b'[' {
            let close = if ch == b'{' { b'}' } else { b']' };
            let peek = next_non_space(i + 1);
            if bytes.get(peek) == Some(&close) {
                out.push(ch as char);
                out.push(close as char);
                i = peek + 1;
                continue;
            }
            out.push(ch as char);
            if pretty {
                depth += 1;
                out.push_str("\n    ");
                for _ in 0..depth {
                    out.push_str("  ");
                }
            }
            i += 1;
            continue;
        }
        if ch == b'}' || ch == b']' {
            if pretty {
                depth = depth.saturating_sub(1);
                out.push_str("\n    ");
                for _ in 0..depth {
                    out.push_str("  ");
                }
            }
            out.push(ch as char);
            i += 1;
            continue;
        }
        if ch == b',' {
            out.push(',');
            if pretty {
                out.push_str("\n    ");
                for _ in 0..depth {
                    out.push_str("  ");
                }
            }
            i += 1;
            continue;
        }
        if ch == b':' {
            if pretty {
                out.push_str(": ");
            } else {
                out.push(':');
            }
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b',' | b'}' | b']' | b':')
        {
            i += 1;
        }
        out.push_str(&trimmed[start..i]);
    }
    Some(out)
}

fn format_json_cell(cell: Option<&str>, column: &ColumnInfo, pretty: bool) -> AppResult<String> {
    let Some(raw) = cell else {
        return Ok("null".to_owned());
    };
    match column.kind {
        ColumnKind::Integer | ColumnKind::Decimal => {
            let trimmed = raw.trim();
            if is_rfc8259_number(trimmed) {
                return Ok(trimmed.to_owned());
            }
        }
        ColumnKind::Boolean => {
            let lower = raw.trim().to_ascii_lowercase();
            if matches!(lower.as_str(), "true" | "t" | "1") {
                return Ok("true".to_owned());
            }
            if matches!(lower.as_str(), "false" | "f" | "0") {
                return Ok("false".to_owned());
            }
        }
        ColumnKind::Json if is_json_column(column) => {
            if let Some(lossless) = format_embedded_json_lossless(raw, pretty) {
                return Ok(lossless);
            }
        }
        _ => {}
    }
    serde_json::to_string(raw).map_err(|_| AppError::internal("Could not encode JSON string cell"))
}

pub fn quote_sql_identifier(name: &str, engine: Option<EngineKind>) -> String {
    if engine == Some(EngineKind::Mysql) {
        format!("`{}`", name.replace('`', "``"))
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

pub fn format_sql_table_name(table: Option<&TableRef>, engine: Option<EngineKind>) -> String {
    let Some(table) = table else {
        return quote_sql_identifier("result_rows", engine);
    };
    let table_name = table.name.trim();
    if table_name.is_empty() {
        return quote_sql_identifier("result_rows", engine);
    }
    let schema_name = table.schema.trim();
    if schema_name.is_empty() {
        quote_sql_identifier(table_name, engine)
    } else {
        format!(
            "{}.{}",
            quote_sql_identifier(schema_name, engine),
            quote_sql_identifier(table_name, engine)
        )
    }
}

fn quote_sql_string(raw: &str, engine: Option<EngineKind>) -> String {
    if matches!(engine, Some(EngineKind::Mysql | EngineKind::Snowflake)) {
        let mut escaped = String::with_capacity(raw.len() + 2);
        escaped.push('\'');
        for ch in raw.chars() {
            match ch {
                '\\' => escaped.push_str("\\\\"),
                '\'' => escaped.push_str("''"),
                '\0' => escaped.push_str("\\0"),
                '\r' => escaped.push_str("\\r"),
                '\n' => escaped.push_str("\\n"),
                _ => escaped.push(ch),
            }
        }
        escaped.push('\'');
        escaped
    } else {
        format!("'{}'", raw.replace('\'', "''"))
    }
}

fn parse_hex_binary(raw: &str) -> Option<&str> {
    let hex = raw
        .strip_prefix("\\x")
        .or_else(|| raw.strip_prefix("\\X"))
        .or_else(|| raw.strip_prefix("0x"))
        .or_else(|| raw.strip_prefix("0X"))?;
    if hex.len().is_multiple_of(2) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex)
    } else {
        None
    }
}

fn format_sql_cell(cell: Option<&str>, column: &ColumnInfo, engine: Option<EngineKind>) -> String {
    let Some(raw) = cell else {
        return "NULL".to_owned();
    };
    match column.kind {
        ColumnKind::Integer => {
            let trimmed = raw.trim();
            if is_sql_integer(trimmed) {
                return trimmed.to_owned();
            }
        }
        ColumnKind::Decimal => {
            let trimmed = raw.trim();
            if is_rfc8259_number(trimmed) {
                return trimmed.to_owned();
            }
        }
        ColumnKind::Boolean => {
            let lower = raw.trim().to_ascii_lowercase();
            if matches!(lower.as_str(), "true" | "t" | "1") {
                return "TRUE".to_owned();
            }
            if matches!(lower.as_str(), "false" | "f" | "0") {
                return "FALSE".to_owned();
            }
        }
        ColumnKind::Binary => {
            if let Some(hex) = parse_hex_binary(raw.trim()) {
                return if matches!(engine, Some(EngineKind::Mysql | EngineKind::Snowflake)) {
                    format!("X'{hex}'")
                } else {
                    format!("'\\x{hex}'::bytea")
                };
            }
        }
        _ => {}
    }
    quote_sql_string(raw, engine)
}

fn escape_markdown_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("<br>");
            }
            '\n' => out.push_str("<br>"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(id: &str, name: &str, kind: ColumnKind, db_type: &str) -> ColumnInfo {
        ColumnInfo {
            id: id.into(),
            name: name.into(),
            database_type: db_type.into(),
            kind,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }

    fn sample_columns() -> Vec<ColumnInfo> {
        vec![
            col("c0", "id", ColumnKind::Integer, "bigint"),
            col("c1", "amount", ColumnKind::Decimal, "numeric(38,10)"),
            col("c2", "note", ColumnKind::Text, "text"),
            col("c3", "active", ColumnKind::Boolean, "boolean"),
            col("c4", "payload", ColumnKind::Json, "jsonb"),
            col("c5", "raw_bytes", ColumnKind::Binary, "bytea"),
        ]
    }

    fn sample_rows() -> Vec<TextRow> {
        vec![
            vec![
                Some("9223372036854775807".into()),
                Some("12345678901234567890.1234567890".into()),
                None,
                Some("t".into()),
                Some("{\"seq\": 9223372036854775807,\n\"rate\": 0.0000000001}".into()),
                Some("\\xdeadbeef".into()),
            ],
            vec![
                Some("-9223372036854775808".into()),
                Some("-0.0000000005".into()),
                Some("".into()),
                Some("0".into()),
                None,
                Some("0x00ff".into()),
            ],
            vec![
                Some("42".into()),
                Some("10.50".into()),
                Some("\\N".into()),
                Some("true".into()),
                Some("[]".into()),
                None,
            ],
        ]
    }

    #[test]
    fn csv_and_tsv_distinguish_null_empty_string_and_literal_backslash_n() {
        let mut csv_buf = Vec::new();
        let mut writer = ExportStreamWriter::try_new(
            &mut csv_buf,
            ExportWriterOptions::new(ExportFormat::Csv, sample_columns(), true, None, None),
        )
        .unwrap();
        writer.write_rows(&sample_rows()).unwrap();
        writer.finish().unwrap();
        let csv_text = String::from_utf8(csv_buf).unwrap();
        let lines: Vec<&str> = csv_text.lines().collect();
        assert_eq!(lines[0], "id,amount,note,active,payload,raw_bytes");
        // Row 1 has unquoted empty for SQL NULL in `note`
        assert!(lines[1].starts_with("9223372036854775807,12345678901234567890.1234567890,,t,"));
        // Row 2 (after multiline JSON line) has quoted "" for empty string
        assert!(csv_text.contains("-9223372036854775808,-0.0000000005,\"\",0,,0x00ff"));
        // Row 3 quotes literal "\N"
        assert!(csv_text.contains("42,10.50,\"\\N\",true,[],"));

        let mut tsv_buf = Vec::new();
        let mut tsv_writer = ExportStreamWriter::try_new(
            &mut tsv_buf,
            ExportWriterOptions::new(
                ExportFormat::Tsv,
                sample_columns()[0..3].to_vec(),
                false,
                None,
                None,
            ),
        )
        .unwrap();
        tsv_writer
            .write_projected_rows(&sample_rows(), &[0, 1, 2])
            .unwrap();
        tsv_writer.finish().unwrap();
        let tsv_text = String::from_utf8(tsv_buf).unwrap();
        assert_eq!(
            tsv_text,
            "9223372036854775807\t12345678901234567890.1234567890\t\\N\n\
             -9223372036854775808\t-0.0000000005\t\"\"\n\
             42\t10.50\t\"\\N\""
        );
    }

    #[test]
    fn json_and_ndjson_preserve_exact_int64_and_high_scale_decimals_without_float_coercion() {
        let mut json_buf = Vec::new();
        let mut writer = ExportStreamWriter::try_new(
            &mut json_buf,
            ExportWriterOptions::new(ExportFormat::Json, sample_columns(), true, None, None),
        )
        .unwrap();
        writer.write_rows(&sample_rows()).unwrap();
        writer.finish().unwrap();
        let json_text = String::from_utf8(json_buf).unwrap();
        assert!(json_text.contains("\"id\": 9223372036854775807,"));
        assert!(json_text.contains("\"amount\": 12345678901234567890.1234567890,"));
        assert!(json_text.contains("\"note\": null,"));
        assert!(json_text.contains("\"note\": \"\","));
        assert!(json_text.contains("\"seq\": 9223372036854775807"));
        assert!(serde_json::from_str::<serde::de::IgnoredAny>(&json_text).is_ok());

        let mut ndjson_buf = Vec::new();
        let mut nd_writer = ExportStreamWriter::try_new(
            &mut ndjson_buf,
            ExportWriterOptions::new(ExportFormat::Ndjson, sample_columns(), true, None, None),
        )
        .unwrap();
        nd_writer.write_rows(&sample_rows()).unwrap();
        nd_writer.finish().unwrap();
        let nd_text = String::from_utf8(ndjson_buf).unwrap();
        let nd_lines: Vec<&str> = nd_text.lines().collect();
        // Multiline JSON cell must be compacted onto a single NDJSON line without losing precision
        assert_eq!(nd_lines.len(), 3);
        for line in &nd_lines {
            assert!(serde_json::from_str::<serde::de::IgnoredAny>(line).is_ok());
        }
        assert!(
            nd_lines[0].contains("\"payload\":{\"seq\":9223372036854775807,\"rate\":0.0000000001}")
        );
    }

    #[test]
    fn sql_insert_and_markdown_handle_dialects_batches_and_escaping() {
        let table = TableRef {
            schema: "sales".into(),
            name: "order`items".into(),
        };
        let mut sql_buf = Vec::new();
        let mut sql_writer = ExportStreamWriter::try_new(
            &mut sql_buf,
            ExportWriterOptions::new(
                ExportFormat::SqlInsert,
                sample_columns(),
                true,
                Some(EngineKind::Mysql),
                Some(table),
            ),
        )
        .unwrap();
        sql_writer.write_rows(&sample_rows()).unwrap();
        sql_writer.finish().unwrap();
        let sql_text = String::from_utf8(sql_buf).unwrap();
        assert!(sql_text.starts_with(
            "INSERT INTO `sales`.`order``items` (`id`, `amount`, `note`, `active`, `payload`, `raw_bytes`)\nVALUES\n"
        ));
        assert!(sql_text.contains("X'deadbeef'"));
        assert!(sql_text.contains("'\\\\N'"));
        assert!(sql_text.ends_with(");"));

        let md_cols = vec![
            col("c0", "id", ColumnKind::Integer, "bigint"),
            col("c1", "label|pipe", ColumnKind::Text, "text"),
        ];
        let mut md_buf = Vec::new();
        let mut md_writer = ExportStreamWriter::try_new(
            &mut md_buf,
            ExportWriterOptions::new(ExportFormat::Markdown, md_cols, true, None, None),
        )
        .unwrap();
        md_writer
            .write_row(&[Some("7".into()), Some("a|b\nc\\d".into())])
            .unwrap();
        md_writer.finish().unwrap();
        let md_text = String::from_utf8(md_buf).unwrap();
        assert_eq!(
            md_text,
            "| id | label\\|pipe |\n| ---: | --- |\n| 7 | a\\|b<br>c\\\\d |"
        );
    }

    #[test]
    fn file_session_publishes_atomically_on_complete_and_leaves_marked_partial_on_abort() {
        let dir = tempfile::tempdir().unwrap();
        let cols = vec![col("c0", "id", ColumnKind::Integer, "bigint")];

        // 1. Clean completion publishes final file and removes .partial
        let ok_dest = dir.path().join("complete.csv");
        let mut session = ExportFileSession::create(
            ok_dest.clone(),
            ExportWriterOptions::new(ExportFormat::Csv, cols.clone(), true, None, None),
        )
        .unwrap();
        session
            .write_rows(&[vec![Some("1".into())], vec![Some("2".into())]])
            .unwrap();
        assert!(!ok_dest.exists());
        assert!(dir.path().join("complete.csv.partial").exists());
        let (rows, _, out_name) = session.complete().unwrap();
        assert_eq!(rows, 2);
        assert_eq!(out_name, "complete.csv");
        assert!(ok_dest.exists());
        assert!(!dir.path().join("complete.csv.partial").exists());
        assert_eq!(std::fs::read_to_string(&ok_dest).unwrap(), "id\n1\n2");

        // 2. Abort / cancellation never publishes final file and marks .partial
        let cancelled_dest = dir.path().join("cancelled.csv");
        let mut cancelled_session = ExportFileSession::create(
            cancelled_dest.clone(),
            ExportWriterOptions::new(ExportFormat::Csv, cols.clone(), true, None, None),
        )
        .unwrap();
        cancelled_session
            .write_rows(&[vec![Some("10".into())]])
            .unwrap();
        let (aborted_rows, _, partial_name) = cancelled_session.abort("cancelled by user");
        assert_eq!(aborted_rows, 1);
        assert_eq!(partial_name, "cancelled.csv.partial");
        assert!(!cancelled_dest.exists());
        let partial_contents =
            std::fs::read_to_string(dir.path().join("cancelled.csv.partial")).unwrap();
        assert!(partial_contents.contains("id\n10\n# JOINERY EXPORT INCOMPLETE"));

        // 3. Row and byte limits reject cleanly
        let mut limited_opts = ExportWriterOptions::new(ExportFormat::Csv, cols, true, None, None);
        limited_opts.max_rows = 1;
        let mut limited = Vec::new();
        let mut limited_writer = ExportStreamWriter::try_new(&mut limited, limited_opts).unwrap();
        limited_writer.write_row(&[Some("1".into())]).unwrap();
        assert_eq!(
            limited_writer
                .write_row(&[Some("2".into())])
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
    }
}
