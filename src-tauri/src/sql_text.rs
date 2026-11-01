use crate::domain::SqlSpan;

/// Records how a rewritten SQL statement (for example after placeholder
/// substitution) maps byte offsets back to the user's original statement text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewriteMap {
    pieces: Vec<RewritePiece>,
    rewritten_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewritePiece {
    Verbatim {
        orig_start: usize,
        rew_start: usize,
        len: usize,
    },
    Replaced {
        orig_start: usize,
        orig_end: usize,
        rew_start: usize,
        rew_end: usize,
    },
}

impl RewriteMap {
    pub fn identity(sql: &str) -> Self {
        let len = sql.len();
        Self {
            pieces: if len > 0 {
                vec![RewritePiece::Verbatim {
                    orig_start: 0,
                    rew_start: 0,
                    len,
                }]
            } else {
                Vec::new()
            },
            rewritten_len: len,
        }
    }

    pub fn push_verbatim(&mut self, orig_start: usize, len: usize) {
        if len == 0 {
            return;
        }
        let rew_start = self.rewritten_len;
        self.rewritten_len += len;
        if let Some(RewritePiece::Verbatim {
            orig_start: prev_orig,
            rew_start: prev_rew,
            len: prev_len,
        }) = self.pieces.last_mut()
            && *prev_orig + *prev_len == orig_start
            && *prev_rew + *prev_len == rew_start
        {
            *prev_len += len;
            return;
        }
        self.pieces.push(RewritePiece::Verbatim {
            orig_start,
            rew_start,
            len,
        });
    }

    pub fn push_replaced(
        &mut self,
        orig_start: usize,
        orig_end: usize,
        rewritten_piece_len: usize,
    ) {
        let rew_start = self.rewritten_len;
        let rew_end = rew_start + rewritten_piece_len;
        self.rewritten_len = rew_end;
        self.pieces.push(RewritePiece::Replaced {
            orig_start,
            orig_end,
            rew_start,
            rew_end,
        });
    }

    /// Maps a byte offset in `rewritten_sql` back to a `SqlSpan` in `original_sql`.
    pub fn span_at_rewritten_byte(&self, original_sql: &str, rew_byte: usize) -> SqlSpan {
        if rew_byte >= self.rewritten_len {
            return eof_token_span(original_sql);
        }
        for piece in &self.pieces {
            match *piece {
                RewritePiece::Verbatim {
                    orig_start,
                    rew_start,
                    len,
                } if rew_byte >= rew_start && rew_byte < rew_start + len => {
                    return token_span_at_byte(original_sql, orig_start + (rew_byte - rew_start));
                }
                RewritePiece::Replaced {
                    orig_start,
                    orig_end,
                    rew_start,
                    rew_end,
                } if rew_byte >= rew_start && rew_byte < rew_end => {
                    return span_for_byte_range(original_sql, orig_start, orig_end);
                }
                _ => {}
            }
        }
        eof_token_span(original_sql)
    }

    /// Maps a 1-based Unicode scalar (`char`) index in `rewritten_sql` (such as
    /// PostgreSQL `ErrorPosition::Original`) to a `SqlSpan` in `original_sql`.
    pub fn span_from_rewritten_pg_char_pos(
        &self,
        original_sql: &str,
        rewritten_sql: &str,
        char_pos_1_based: u32,
    ) -> SqlSpan {
        if char_pos_1_based == 0 {
            return trimmed_span(original_sql);
        }
        match rewritten_sql
            .char_indices()
            .nth((char_pos_1_based - 1) as usize)
        {
            Some((rew_byte, _)) => self.span_at_rewritten_byte(original_sql, rew_byte),
            None => eof_token_span(original_sql),
        }
    }
}

fn clamp_char_boundary(sql: &str, mut idx: usize, round_up: bool) -> usize {
    idx = idx.min(sql.len());
    if round_up {
        while idx < sql.len() && !sql.is_char_boundary(idx) {
            idx += 1;
        }
    } else {
        while idx > 0 && !sql.is_char_boundary(idx) {
            idx -= 1;
        }
    }
    idx
}

/// Converts a half-open UTF-8 byte range `[start_byte, end_byte)` in `sql` into
/// UTF-16 code-unit offsets and 1-based `(line, char_column)` coordinates.
pub fn span_for_byte_range(sql: &str, start_byte: usize, end_byte: usize) -> SqlSpan {
    let start_byte = clamp_char_boundary(sql, start_byte, false);
    let end_byte = clamp_char_boundary(sql, end_byte.max(start_byte), true);

    let mut utf16 = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let mut start_utf16 = None;
    let mut start_line = 1u32;
    let mut start_col = 1u32;
    let mut end_utf16 = None;

    for (byte_idx, ch) in sql.char_indices() {
        if byte_idx == start_byte {
            start_utf16 = Some(utf16);
            start_line = line;
            start_col = col;
        }
        if byte_idx == end_byte {
            end_utf16 = Some(utf16);
            break;
        }
        utf16 += ch.len_utf16();
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    let s_utf16 = start_utf16.unwrap_or_else(|| {
        start_line = line;
        start_col = col;
        utf16
    });
    let e_utf16 = end_utf16.unwrap_or(utf16).max(s_utf16);

    SqlSpan {
        start_utf16: s_utf16,
        end_utf16: e_utf16,
        line: start_line,
        column: start_col,
    }
}

/// Returns the `SqlSpan` covering the trimmed non-whitespace body of `sql`.
pub fn trimmed_span(sql: &str) -> SqlSpan {
    let leading = sql.len() - sql.trim_start().len();
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return span_for_byte_range(sql, 0, sql.len());
    }
    span_for_byte_range(sql, leading, leading + trimmed.len())
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphanumeric()
}

/// Expands a starting UTF-8 byte index in `sql` across the offending token so
/// editor error decorations underline a visible token range (`start < end`).
pub fn token_span_at_byte(sql: &str, byte_idx: usize) -> SqlSpan {
    if sql.is_empty() {
        return SqlSpan {
            start_utf16: 0,
            end_utf16: 0,
            line: 1,
            column: 1,
        };
    }
    let start = clamp_char_boundary(sql, byte_idx, false);
    if start >= sql.len() {
        return eof_token_span(sql);
    }
    let rest = &sql[start..];
    let mut chars = rest.char_indices();
    let Some((_, first)) = chars.next() else {
        return eof_token_span(sql);
    };

    let token_len = match first {
        '\r' | '\n' => return eof_token_before_byte(sql, start),
        c if c.is_whitespace() => {
            // Advance to next non-whitespace token on the same line if present.
            let mut ws_end = first.len_utf8();
            for (idx, ch) in chars {
                if ch == '\r' || ch == '\n' || !ch.is_whitespace() {
                    ws_end = idx;
                    break;
                }
                ws_end = idx + ch.len_utf8();
            }
            if start + ws_end < sql.len()
                && !matches!(sql[start + ws_end..].chars().next(), Some('\r' | '\n'))
            {
                return token_span_at_byte(sql, start + ws_end);
            }
            first.len_utf8()
        }
        '\'' | '"' | '`' => {
            let quote = first;
            let mut end = quote.len_utf8();
            while let Some((idx, ch)) = chars.next() {
                end = idx + ch.len_utf8();
                if ch == '\\' && quote != '"' && quote != '`' {
                    if let Some((next_idx, next_ch)) = chars.next() {
                        end = next_idx + next_ch.len_utf8();
                    }
                } else if ch == quote {
                    if rest[end..].starts_with(quote) {
                        let (_, dup) = chars.next().unwrap();
                        end += dup.len_utf8();
                    } else {
                        break;
                    }
                } else if ch == '\n' {
                    end = idx;
                    break;
                }
            }
            end.max(first.len_utf8())
        }
        ':' | '$' => {
            let mut end = first.len_utf8();
            for (idx, ch) in chars {
                if is_ident_char(ch) {
                    end = idx + ch.len_utf8();
                } else {
                    break;
                }
            }
            end
        }
        c if is_ident_char(c) => {
            let mut end = c.len_utf8();
            for (idx, ch) in chars {
                if is_ident_char(ch) {
                    end = idx + ch.len_utf8();
                } else {
                    break;
                }
            }
            end
        }
        _ => first.len_utf8(),
    };

    span_for_byte_range(sql, start, start + token_len)
}

fn eof_token_before_byte(sql: &str, limit_byte: usize) -> SqlSpan {
    let prefix = &sql[..clamp_char_boundary(sql, limit_byte, false)];
    let trimmed = prefix.trim_end();
    if trimmed.is_empty() {
        let end = sql.chars().next().map_or(0, char::len_utf8);
        return span_for_byte_range(sql, 0, end);
    }
    let end_byte = trimmed.len();
    let mut start_byte = end_byte;
    for (idx, ch) in trimmed.char_indices().rev() {
        if is_ident_char(ch) {
            start_byte = idx;
        } else {
            if start_byte == end_byte {
                start_byte = idx;
            }
            break;
        }
    }
    span_for_byte_range(sql, start_byte, end_byte)
}

pub fn eof_token_span(sql: &str) -> SqlSpan {
    if sql.is_empty() {
        return SqlSpan {
            start_utf16: 0,
            end_utf16: 0,
            line: 1,
            column: 1,
        };
    }
    eof_token_before_byte(sql, sql.len())
}

/// Maps a 1-based PostgreSQL `ErrorPosition::Original` Unicode scalar (`char`)
/// index into a UTF-16 `SqlSpan`.
pub fn span_from_pg_char_pos(sql: &str, char_pos_1_based: u32) -> SqlSpan {
    if char_pos_1_based == 0 {
        return trimmed_span(sql);
    }
    match sql.char_indices().nth((char_pos_1_based - 1) as usize) {
        Some((byte_idx, _)) => token_span_at_byte(sql, byte_idx),
        None => eof_token_span(sql),
    }
}

/// Maps 1-based `(line, char_column)` coordinates (such as `sqlparser`
/// `Location` or Snowflake compilation positions) into a UTF-16 `SqlSpan`.
pub fn span_from_line_char_col(sql: &str, line_1_based: u64, col_1_based: u64) -> SqlSpan {
    if sql.is_empty() {
        return SqlSpan {
            start_utf16: 0,
            end_utf16: 0,
            line: 1,
            column: 1,
        };
    }
    let target_line = line_1_based.max(1) as usize;
    let target_col = col_1_based.max(1) as usize;
    let mut line_start = 0usize;
    for (current_line, segment) in (1usize..).zip(sql.split_inclusive('\n')) {
        if current_line == target_line {
            let line_body = segment.trim_end_matches(['\r', '\n']);
            return match line_body.char_indices().nth(target_col - 1) {
                Some((rel_byte, _)) => token_span_at_byte(sql, line_start + rel_byte),
                None => eof_token_before_byte(sql, line_start + line_body.len()),
            };
        }
        line_start += segment.len();
    }
    eof_token_span(sql)
}

/// Extracts `Line: <L>, Column: <C>` coordinates from a `sqlparser` error message.
pub fn span_from_sqlparser_error(sql: &str, message: &str) -> Option<SqlSpan> {
    let marker = "Line: ";
    let idx = message.rfind(marker)?;
    let after_line = &message[idx + marker.len()..];
    let comma = after_line.find(", Column: ")?;
    let line: u64 = after_line[..comma].trim().parse().ok()?;
    let after_col = &after_line[comma + ", Column: ".len()..];
    let digits_len = after_col.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    let col: u64 = after_col[..digits_len].parse().ok()?;
    Some(span_from_line_char_col(sql, line, col))
}

/// Parses MySQL / MariaDB `... [near '<snippet>'] at line <N>` syntax error
/// coordinates into an intra-statement UTF-16 `SqlSpan`.
pub fn span_from_mysql_error(sql: &str, message: &str) -> Option<SqlSpan> {
    let marker = " at line ";
    let at_idx = message.rfind(marker)?;
    let after = &message[at_idx + marker.len()..];
    let digits_len = after.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    let line_num: usize = after[..digits_len].parse().ok()?;
    if line_num == 0 {
        return None;
    }

    let snippet = message[..at_idx].rfind(" near '").and_then(|near_idx| {
        let candidate = &message[near_idx + " near '".len()..at_idx];
        candidate.strip_suffix('\'')
    });

    let mut line_start = 0usize;
    for (current_line, segment) in (1usize..).zip(sql.split_inclusive('\n')) {
        if current_line == line_num {
            let line_body = segment.trim_end_matches(['\r', '\n']);
            if let Some(near) = snippet {
                if near.is_empty() {
                    return Some(eof_token_before_byte(sql, line_start + line_body.len()));
                }
                let first_line_near = near.lines().next().unwrap_or(near);
                if !first_line_near.is_empty()
                    && let Some(rel) = line_body.find(first_line_near)
                {
                    return Some(token_span_at_byte(sql, line_start + rel));
                }
            }
            let leading_ws = line_body.len() - line_body.trim_start().len();
            let trimmed = line_body.trim();
            if trimmed.is_empty() {
                return Some(eof_token_before_byte(sql, line_start + line_body.len()));
            }
            return Some(span_for_byte_range(
                sql,
                line_start + leading_ws,
                line_start + leading_ws + trimmed.len(),
            ));
        }
        line_start += segment.len();
    }
    None
}

/// Parses Snowflake `line <L> at position <P>` compilation coordinates (where
/// `L` is 1-based and `P` is 0-based) into an intra-statement UTF-16 `SqlSpan`.
pub fn span_from_snowflake_error(sql: &str, message: &str) -> Option<SqlSpan> {
    let lower = message.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find("line ") {
        let start = search_from + rel + "line ".len();
        let line_digits = lower[start..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if line_digits > 0 {
            let after_line = start + line_digits;
            if let Some(after_pos_kw) = lower[after_line..].strip_prefix(" at position ") {
                let pos_digits = after_pos_kw.bytes().take_while(u8::is_ascii_digit).count();
                if pos_digits > 0
                    && let (Ok(line), Ok(pos0)) = (
                        lower[start..after_line].parse::<u64>(),
                        after_pos_kw[..pos_digits].parse::<u64>(),
                    )
                {
                    return Some(span_from_line_char_col(sql, line, pos0.saturating_add(1)));
                }
            }
        }
        search_from = start;
    }
    None
}

/// Shifts an intra-statement `child` span by its enclosing `statement_span` in
/// a multi-statement SQL script.
pub fn shift_child_span(statement_span: SqlSpan, child: SqlSpan) -> SqlSpan {
    let start_utf16 = statement_span.start_utf16.saturating_add(child.start_utf16);
    let end_utf16 = statement_span
        .start_utf16
        .saturating_add(child.end_utf16)
        .min(statement_span.end_utf16.max(start_utf16));
    let line = statement_span
        .line
        .saturating_add(child.line.saturating_sub(1));
    let column = if child.line <= 1 {
        statement_span
            .column
            .saturating_add(child.column.saturating_sub(1))
    } else {
        child.column
    };
    SqlSpan {
        start_utf16,
        end_utf16: end_utf16.max(start_utf16),
        line,
        column,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16_slice(sql: &str, span: SqlSpan) -> String {
        let units: Vec<u16> = sql.encode_utf16().collect();
        String::from_utf16(&units[span.start_utf16..span.end_utf16]).unwrap()
    }

    #[test]
    fn maps_pg_char_positions_across_bmp_cjk_and_emoji_surrogate_pairs() {
        // 'café' (4 chars, 5 UTF-8 bytes, 4 UTF-16 units)
        // '日本語' (3 chars, 9 UTF-8 bytes, 3 UTF-16 units)
        // '🧪🚀' (2 chars, 8 UTF-8 bytes, 4 UTF-16 units!)
        let sql = "SELECT 'café', '日本語', '🧪🚀', bad_col FROM orders";
        let bad_char_pos = (sql[..sql.find("bad_col").unwrap()].chars().count() + 1) as u32;
        let span = span_from_pg_char_pos(sql, bad_char_pos);
        assert_eq!(utf16_slice(sql, span), "bad_col");
        assert_eq!(span.line, 1);
        assert_eq!(span.column, bad_char_pos);
        // Because '🧪🚀' is 2 chars = 4 UTF-16 code units, UTF-16 offset is char_index + 2!
        assert_eq!(span.start_utf16, (bad_char_pos as usize - 1) + 2);
        assert_eq!(span.end_utf16, span.start_utf16 + "bad_col".len());
    }

    #[test]
    fn maps_crlf_multiline_and_eof_positions_cleanly() {
        let sql = "SELECT '🧪'\r\nFROM orders\r\nWHERE missing_col = 1";
        let char_pos = (sql[..sql.find("missing_col").unwrap()].chars().count() + 1) as u32;
        let span = span_from_pg_char_pos(sql, char_pos);
        assert_eq!(utf16_slice(sql, span), "missing_col");
        assert_eq!(span.line, 3);
        assert_eq!(span.column, 7);

        let eof_sql = "SELECT * FROM";
        let eof_span = span_from_pg_char_pos(eof_sql, 14);
        assert_eq!(utf16_slice(eof_sql, eof_span), "FROM");
    }

    #[test]
    fn parses_mysql_and_snowflake_and_sqlparser_error_positions() {
        let mysql_sql = "SELECT '🧪',\r\n  FRM sample";
        let mysql_msg = "You have an error in your SQL syntax; check the manual that corresponds to your MySQL server version for the right syntax to use near 'FRM sample' at line 2";
        let mysql_span = span_from_mysql_error(mysql_sql, mysql_msg).unwrap();
        assert_eq!(utf16_slice(mysql_sql, mysql_span), "FRM");
        assert_eq!(mysql_span.line, 2);
        assert_eq!(mysql_span.column, 3);

        let sf_sql = "SELECT '🧪'\nFROM ORDERS\nWHERE BAD_TOKEN = 1";
        let sf_msg = "Snowflake rejected the request: SQL compilation error: error line 3 at position 6 invalid identifier 'BAD_TOKEN'";
        let sf_span = span_from_snowflake_error(sf_sql, sf_msg).unwrap();
        assert_eq!(utf16_slice(sf_sql, sf_span), "BAD_TOKEN");
        assert_eq!(sf_span.line, 3);
        assert_eq!(sf_span.column, 7);

        let parser_msg =
            "sql parser error: Expected: an SQL statement, found: SELEC at Line: 1, Column: 1";
        let p_span = span_from_sqlparser_error("SELEC 1", parser_msg).unwrap();
        assert_eq!(utf16_slice("SELEC 1", p_span), "SELEC");
    }

    #[test]
    fn shifts_child_spans_in_same_line_and_multiline_scripts_and_rewrite_maps() {
        let batch = "SELECT '🧪'; SELECT bad_col FROM orders";
        let stmt2_start = batch.find("SELECT bad_col").unwrap();
        let stmt2 = &batch[stmt2_start..];
        let stmt2_span = span_for_byte_range(batch, stmt2_start, batch.len());
        let child = span_from_pg_char_pos(stmt2, 8);
        let shifted = shift_child_span(stmt2_span, child);
        assert_eq!(utf16_slice(batch, shifted), "bad_col");
        assert_eq!(shifted.line, 1);
        assert_eq!(shifted.column, 20); // '🧪' is 1 char (col 9), so 'b' is char 20

        // RewriteMap shrinking ':very_long_param' -> '$1' before 'bad_col'
        let orig = "SELECT '🧪', :very_long_param, bad_col FROM orders";
        let rewritten = "SELECT '🧪', $1, bad_col FROM orders";
        let p_start = orig.find(":very_long_param").unwrap();
        let p_end = p_start + ":very_long_param".len();
        let mut map = RewriteMap::default();
        map.push_verbatim(0, p_start);
        map.push_replaced(p_start, p_end, "$1".len());
        map.push_verbatim(p_end, orig.len() - p_end);

        let rew_bad_char = (rewritten[..rewritten.find("bad_col").unwrap()]
            .chars()
            .count()
            + 1) as u32;
        let mapped_bad = map.span_from_rewritten_pg_char_pos(orig, rewritten, rew_bad_char);
        assert_eq!(utf16_slice(orig, mapped_bad), "bad_col");

        let rew_param_char =
            (rewritten[..rewritten.find("$1").unwrap()].chars().count() + 1) as u32;
        let mapped_param = map.span_from_rewritten_pg_char_pos(orig, rewritten, rew_param_char);
        assert_eq!(utf16_slice(orig, mapped_param), ":very_long_param");
    }
}
