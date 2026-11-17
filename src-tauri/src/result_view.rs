//! Bounded in-memory filtering and multi-column sorting over retained `ResultStore` snapshots.
//!
//! Produces a compact `Vec<u32>` display-to-actual row index map without duplicating Arrow
//! result batches, writing temporary files, or re-executing database SQL.

use crate::domain::*;
use crate::results::ResultStore;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::Instant;

pub const MAX_RESULT_FILTER_BYTES: usize = 1024;
pub const MAX_RESULT_SORT_KEYS: usize = 4;
pub const MAX_RESULT_VIEW_ROWS: usize = 100_000;
pub const MAX_SORT_KEY_MEMORY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct ParsedDecimal<'a> {
    negative: bool,
    int_digits: &'a [u8],
    frac_digits: &'a [u8],
}

fn parse_exact_decimal(raw: &str) -> Option<ParsedDecimal<'_>> {
    let trimmed = raw.trim().as_bytes();
    if trimmed.is_empty() {
        return None;
    }
    let (neg_flag, rest) = match trimmed[0] {
        b'-' => (true, &trimmed[1..]),
        b'+' => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    if rest.is_empty() {
        return None;
    }
    let (int_raw, frac_raw) = match rest.iter().position(|&b| b == b'.') {
        Some(dot) => (&rest[..dot], &rest[dot + 1..]),
        None => (rest, &[][..]),
    };
    if int_raw.is_empty() && frac_raw.is_empty() {
        return None;
    }
    if !int_raw.iter().all(u8::is_ascii_digit) || !frac_raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut start = 0usize;
    while start < int_raw.len() && int_raw[start] == b'0' {
        start += 1;
    }
    let int_digits = &int_raw[start..];

    let mut end = frac_raw.len();
    while end > 0 && frac_raw[end - 1] == b'0' {
        end -= 1;
    }
    let frac_digits = &frac_raw[..end];

    let is_zero = int_digits.is_empty() && frac_digits.is_empty();
    Some(ParsedDecimal {
        negative: neg_flag && !is_zero,
        int_digits,
        frac_digits,
    })
}

pub fn compare_exact_decimal(left: &str, right: &str) -> Option<Ordering> {
    let a = parse_exact_decimal(left)?;
    let b = parse_exact_decimal(right)?;
    match (a.negative, b.negative) {
        (true, false) => return Some(Ordering::Less),
        (false, true) => return Some(Ordering::Greater),
        _ => {}
    }
    let mag_cmp = a
        .int_digits
        .len()
        .cmp(&b.int_digits.len())
        .then_with(|| a.int_digits.cmp(b.int_digits))
        .then_with(|| a.frac_digits.cmp(b.frac_digits));
    Some(if a.negative {
        mag_cmp.reverse()
    } else {
        mag_cmp
    })
}

fn parse_canonical_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" => Some(true),
        "false" | "f" | "0" | "no" => Some(false),
        _ => None,
    }
}

fn compare_ascii_ci_or_utf8(left: &str, right: &str) -> Ordering {
    if left.is_ascii() && right.is_ascii() {
        for (a, b) in left.bytes().zip(right.bytes()) {
            let ca = a.to_ascii_lowercase();
            let cb = b.to_ascii_lowercase();
            if ca != cb {
                return ca.cmp(&cb);
            }
        }
        return left.len().cmp(&right.len()).then_with(|| left.cmp(right));
    }
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn compare_non_null_values(
    left: &str,
    right: &str,
    kind: ColumnKind,
    case_sensitive: bool,
) -> Ordering {
    match kind {
        ColumnKind::Integer | ColumnKind::Decimal => {
            if let Some(ord) = compare_exact_decimal(left, right) {
                return ord.then_with(|| left.cmp(right));
            }
            if let (Ok(fa), Ok(fb)) = (left.trim().parse::<f64>(), right.trim().parse::<f64>()) {
                return fa.total_cmp(&fb).then_with(|| left.cmp(right));
            }
        }
        ColumnKind::Boolean => {
            if let (Some(ba), Some(bb)) = (parse_canonical_bool(left), parse_canonical_bool(right))
            {
                return ba.cmp(&bb).then_with(|| left.cmp(right));
            }
        }
        _ => {}
    }
    if case_sensitive {
        left.cmp(right)
    } else {
        compare_ascii_ci_or_utf8(left, right)
    }
}

fn contains_ascii_ci(haystack: &str, needle_lower_ascii: &[u8]) -> bool {
    let hb = haystack.as_bytes();
    if needle_lower_ascii.is_empty() {
        return true;
    }
    if hb.len() < needle_lower_ascii.len() {
        return false;
    }
    hb.windows(needle_lower_ascii.len()).any(|win| {
        win.iter()
            .zip(needle_lower_ascii.iter())
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

fn cell_matches_filter(
    cell: Option<&str>,
    needle_raw: &str,
    needle_lower: &str,
    case_sensitive: bool,
    needle_is_ascii: bool,
) -> bool {
    let text = cell.unwrap_or("NULL");
    if case_sensitive {
        text.contains(needle_raw)
    } else if needle_is_ascii && text.is_ascii() {
        contains_ascii_ci(text, needle_lower.as_bytes())
    } else {
        text.to_lowercase().contains(needle_lower)
    }
}

fn resolve_column_index(columns: &[ColumnInfo], id_or_name: &str) -> AppResult<usize> {
    columns
        .iter()
        .position(|c| c.id == id_or_name)
        .or_else(|| columns.iter().position(|c| c.name == id_or_name))
        .ok_or_else(|| {
            AppError::invalid(format!(
                "Column {id_or_name:?} does not exist in this result snapshot"
            ))
        })
}

pub fn validate_result_view_request(request: &ResultViewRequest) -> AppResult<()> {
    let rid = request.result_id.trim();
    if rid.is_empty() || rid.len() > 128 {
        return Err(AppError::invalid("Provide a valid result ID"));
    }
    if let Some(ft) = &request.filter_text {
        if ft.contains('\0') {
            return Err(AppError::invalid("Result filter cannot contain NUL bytes"));
        }
        if ft.len() > MAX_RESULT_FILTER_BYTES {
            return Err(AppError::invalid(format!(
                "Result filter is limited to {MAX_RESULT_FILTER_BYTES} bytes"
            )));
        }
    }
    if request.sort.len() > MAX_RESULT_SORT_KEYS {
        return Err(AppError::invalid(format!(
            "Sort by at most {MAX_RESULT_SORT_KEYS} columns at a time"
        )));
    }
    let mut seen_sort = HashSet::with_capacity(request.sort.len());
    for spec in &request.sort {
        let cid = spec.column_id.trim();
        if cid.is_empty() {
            return Err(AppError::invalid("Sort column ID cannot be empty"));
        }
        if !seen_sort.insert(cid) {
            return Err(AppError::invalid(format!(
                "Duplicate sort column {cid:?} in result view request"
            )));
        }
    }
    Ok(())
}

pub fn compute_result_view(
    results: &ResultStore,
    request: &ResultViewRequest,
) -> AppResult<ResultViewResponse> {
    let started = Instant::now();
    validate_result_view_request(request)?;
    let result_id = request.result_id.trim();
    let info = results.info(result_id)?;

    let filter_needle = request
        .filter_text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let filter_col_indices: Vec<usize> = if filter_needle.is_some() {
        if let Some(col_id) = request
            .filter_column_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            vec![resolve_column_index(&info.columns, col_id)?]
        } else {
            (0..info.columns.len()).collect()
        }
    } else {
        Vec::new()
    };

    let mut sort_resolved: Vec<(usize, ColumnKind, bool)> = Vec::with_capacity(request.sort.len());
    for spec in &request.sort {
        let idx = resolve_column_index(&info.columns, spec.column_id.trim())?;
        let col = &info.columns[idx];
        if col.kind == ColumnKind::Binary {
            return Err(AppError::invalid(format!(
                "Binary column {:?} cannot be sorted",
                col.name
            )));
        }
        sort_resolved.push((idx, col.kind, spec.descending));
    }

    // Collect the minimal set of column indices needed from ResultStore batches
    let mut needed_cols: Vec<usize> = Vec::new();
    for &ci in &filter_col_indices {
        if !needed_cols.contains(&ci) {
            needed_cols.push(ci);
        }
    }
    for &(ci, _, _) in &sort_resolved {
        if !needed_cols.contains(&ci) {
            needed_cols.push(ci);
        }
    }
    needed_cols.sort_unstable();

    let map_needed = |orig_col: usize| -> usize {
        needed_cols
            .binary_search(&orig_col)
            .expect("needed_cols contains all filter and sort columns")
    };
    let filter_mapped: Vec<usize> = filter_col_indices.iter().map(|&c| map_needed(c)).collect();
    let sort_mapped: Vec<(usize, ColumnKind, bool)> = sort_resolved
        .iter()
        .map(|&(c, kind, desc)| (map_needed(c), kind, desc))
        .collect();

    let needle_lower = filter_needle.map(str::to_lowercase).unwrap_or_default();
    let needle_is_ascii = filter_needle.is_some_and(str::is_ascii);

    let batch_count = results.batch_count(result_id)?;
    let mut scanned_rows = 0usize;
    let mut matched_rows = 0usize;
    let mut truncated = false;

    if sort_mapped.is_empty() {
        let mut row_indices: Vec<u32> = Vec::new();
        if needed_cols.is_empty() {
            let total = info.retained_rows;
            scanned_rows = total;
            matched_rows = total;
            let take = total.min(MAX_RESULT_VIEW_ROWS);
            if total > MAX_RESULT_VIEW_ROWS {
                truncated = true;
            }
            row_indices.extend(0..take as u32);
        } else {
            let needle_raw = filter_needle.unwrap_or("");
            for batch_idx in 0..batch_count {
                let Some(batch) = results.batch_rows(result_id, batch_idx, &needed_cols)? else {
                    break;
                };
                for row in &batch {
                    let actual_idx = scanned_rows;
                    scanned_rows += 1;
                    let matched = filter_mapped.iter().any(|&mi| {
                        cell_matches_filter(
                            row[mi].as_deref(),
                            needle_raw,
                            &needle_lower,
                            request.case_sensitive,
                            needle_is_ascii,
                        )
                    });
                    if matched {
                        matched_rows += 1;
                        if row_indices.len() < MAX_RESULT_VIEW_ROWS
                            && let Ok(idx_u32) = u32::try_from(actual_idx)
                        {
                            row_indices.push(idx_u32);
                        } else {
                            truncated = true;
                        }
                    }
                }
            }
        }
        return Ok(ResultViewResponse {
            result_id: result_id.to_owned(),
            row_indices,
            matched_rows,
            scanned_rows,
            truncated,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        });
    }

    // Sorting active: retain only the sort-key columns for up to MAX_RESULT_VIEW_ROWS matching rows
    let mut keyed_rows: Vec<(u32, Vec<Option<String>>)> = Vec::new();
    let mut sort_memory_bytes = 0usize;
    let needle_raw = filter_needle.unwrap_or("");

    for batch_idx in 0..batch_count {
        let Some(batch) = results.batch_rows(result_id, batch_idx, &needed_cols)? else {
            break;
        };
        for mut row in batch {
            let actual_idx = scanned_rows;
            scanned_rows += 1;
            let matched = filter_needle.is_none()
                || filter_mapped.iter().any(|&mi| {
                    cell_matches_filter(
                        row[mi].as_deref(),
                        needle_raw,
                        &needle_lower,
                        request.case_sensitive,
                        needle_is_ascii,
                    )
                });
            if !matched {
                continue;
            }
            matched_rows += 1;
            if keyed_rows.len() >= MAX_RESULT_VIEW_ROWS {
                truncated = true;
                continue;
            }
            let Ok(idx_u32) = u32::try_from(actual_idx) else {
                truncated = true;
                continue;
            };
            let mut keys = Vec::with_capacity(sort_mapped.len());
            let mut row_mem = 32usize;
            for &(mi, _, _) in &sort_mapped {
                let val = if filter_mapped.is_empty() && sort_mapped.len() == 1 {
                    row[mi].take()
                } else {
                    row[mi].clone()
                };
                row_mem = row_mem.saturating_add(16 + val.as_ref().map_or(0, String::len));
                keys.push(val);
            }
            if sort_memory_bytes.saturating_add(row_mem) > MAX_SORT_KEY_MEMORY_BYTES {
                truncated = true;
                continue;
            }
            sort_memory_bytes += row_mem;
            keyed_rows.push((idx_u32, keys));
        }
    }

    keyed_rows.sort_by(|(idx_a, keys_a), (idx_b, keys_b)| {
        for (pos, &(_, kind, descending)) in sort_mapped.iter().enumerate() {
            let ord = match (keys_a[pos].as_deref(), keys_b[pos].as_deref()) {
                (None, None) => Ordering::Equal,
                // Match Joinery's server-side SQL browse contract: ASC NULLS LAST, DESC NULLS FIRST
                (None, Some(_)) => {
                    if descending {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (Some(_), None) => {
                    if descending {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                (Some(va), Some(vb)) => {
                    let cmp = compare_non_null_values(va, vb, kind, request.case_sensitive);
                    if descending { cmp.reverse() } else { cmp }
                }
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        idx_a.cmp(idx_b)
    });

    let row_indices = keyed_rows.into_iter().map(|(idx, _)| idx).collect();

    Ok(ResultViewResponse {
        result_id: result_id.to_owned(),
        row_indices,
        matched_rows,
        scanned_rows,
        truncated,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_col(id: &str, name: &str, kind: ColumnKind) -> ColumnInfo {
        ColumnInfo {
            id: id.into(),
            name: name.into(),
            database_type: "text".into(),
            kind,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }

    #[test]
    fn exact_decimal_comparison_preserves_38_digit_precision_and_negative_order() {
        assert_eq!(
            compare_exact_decimal("9007199254740993", "9007199254740995"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_exact_decimal(
                "99999999999999999999999999999999999998",
                "99999999999999999999999999999999999999"
            ),
            Some(Ordering::Less)
        );
        assert_eq!(compare_exact_decimal("-100", "-20"), Some(Ordering::Less));
        assert_eq!(
            compare_exact_decimal("-0.000", "0.0"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_exact_decimal("0012.3400", "12.34"),
            Some(Ordering::Equal)
        );
        assert_eq!(compare_exact_decimal("not-a-number", "10"), None);
    }

    #[test]
    fn filters_and_sorts_retained_results_with_null_semantics_and_multi_key_tie_breaking() {
        let store = ResultStore::default();
        let created = store
            .create(
                "run-view-1",
                0,
                vec![
                    make_col("c_dept", "dept", ColumnKind::Text),
                    make_col("c_amount", "amount", ColumnKind::Decimal),
                    make_col("c_big", "big_id", ColumnKind::Integer),
                ],
                None,
            )
            .unwrap();
        assert!(
            store
                .append(
                    &created.id,
                    vec![
                        vec![
                            Some("Sales".into()),
                            Some("100.5".into()),
                            Some("9007199254740995".into()),
                        ],
                        vec![
                            Some("Engineering".into()),
                            None,
                            Some("9007199254740993".into()),
                        ],
                        vec![
                            Some("sales".into()),
                            Some("20.25".into()),
                            Some("9007199254740999".into()),
                        ],
                        vec![Some("Sales".into()), None, Some("9007199254740991".into()),],
                    ],
                )
                .unwrap()
        );
        let info = store.finish(&created.id, false).unwrap();

        // 1. Case-insensitive substring filter on "sales" + ASC sort on amount (NULLS LAST)
        let asc_sales = compute_result_view(
            &store,
            &ResultViewRequest {
                result_id: info.id.clone(),
                filter_text: Some("sales".into()),
                filter_column_id: Some("c_dept".into()),
                case_sensitive: false,
                sort: vec![ResultSortSpec {
                    column_id: "c_amount".into(),
                    descending: false,
                }],
            },
        )
        .unwrap();
        assert_eq!(asc_sales.scanned_rows, 4);
        assert_eq!(asc_sales.matched_rows, 3);
        // Row 2 (20.25) < Row 0 (100.5) < Row 3 (NULL last)
        assert_eq!(asc_sales.row_indices, vec![2, 0, 3]);

        // 2. DESC sort on amount puts NULLS FIRST, then 100.5, then 20.25
        let desc_all = compute_result_view(
            &store,
            &ResultViewRequest {
                result_id: info.id.clone(),
                filter_text: None,
                filter_column_id: None,
                case_sensitive: false,
                sort: vec![
                    ResultSortSpec {
                        column_id: "c_amount".into(),
                        descending: true,
                    },
                    ResultSortSpec {
                        column_id: "c_big".into(),
                        descending: false,
                    },
                ],
            },
        )
        .unwrap();
        // Rows 1 & 3 both have amount = NULL; secondary sort c_big ASC orders 9007199254740991 (row 3) before 9007199254740993 (row 1)!
        assert_eq!(desc_all.row_indices, vec![3, 1, 0, 2]);

        // 3. Searching "null" matches NULL cells when filtering across columns
        let null_hits = compute_result_view(
            &store,
            &ResultViewRequest {
                result_id: info.id,
                filter_text: Some("null".into()),
                filter_column_id: None,
                case_sensitive: false,
                sort: vec![],
            },
        )
        .unwrap();
        assert_eq!(null_hits.row_indices, vec![1, 3]);
    }
}
