use crate::domain::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MAX_COMPARE_SIDE_ROWS: usize = 25_000;
pub const MAX_COMPARE_SIDE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_REPORT_DIFF_ROWS: usize = 1_000;
pub const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ANOMALY_SAMPLES: usize = 25;

#[derive(Debug, Clone)]
pub struct CompareDataset {
    pub label: String,
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<TextRow>,
    pub truncated: bool,
}

pub struct CompareDatasetBuilder {
    side_name: &'static str,
    label: String,
    columns: Vec<ColumnInfo>,
    rows: Vec<TextRow>,
    bytes: usize,
    truncated: bool,
    max_rows: usize,
    max_bytes: usize,
}

impl CompareDatasetBuilder {
    pub fn new(
        side_name: &'static str,
        label: String,
        columns: Vec<ColumnInfo>,
        truncated: bool,
    ) -> Self {
        Self::with_limits(
            side_name,
            label,
            columns,
            truncated,
            MAX_COMPARE_SIDE_ROWS,
            MAX_COMPARE_SIDE_BYTES,
        )
    }

    pub fn with_limits(
        side_name: &'static str,
        label: String,
        columns: Vec<ColumnInfo>,
        truncated: bool,
        max_rows: usize,
        max_bytes: usize,
    ) -> Self {
        Self {
            side_name,
            label,
            columns,
            rows: Vec::new(),
            bytes: 0,
            truncated,
            max_rows,
            max_bytes,
        }
    }

    pub fn set_truncated(&mut self, truncated: bool) {
        self.truncated |= truncated;
    }

    pub fn append_rows(&mut self, batch: Vec<TextRow>) -> AppResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let col_count = self.columns.len();
        if batch.iter().any(|row| row.len() != col_count) {
            return Err(AppError::internal(format!(
                "{} row column count did not match schema",
                self.side_name
            )));
        }
        if self.rows.len().saturating_add(batch.len()) > self.max_rows {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                format!(
                    "{} exceeds the {}-row comparison limit. Filter the dataset before comparing.",
                    self.side_name, self.max_rows
                ),
            ));
        }
        let added_bytes: usize = batch.iter().map(|r| estimate_row_bytes(r)).sum();
        if self.bytes.saturating_add(added_bytes) > self.max_bytes {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                format!(
                    "{} exceeds the {} MiB comparison memory budget. Filter the dataset before comparing.",
                    self.side_name,
                    (self.max_bytes / (1024 * 1024)).max(1)
                ),
            ));
        }
        self.bytes += added_bytes;
        self.rows.extend(batch);
        Ok(())
    }

    pub fn finish(self) -> CompareDataset {
        CompareDataset {
            label: self.label,
            columns: self.columns,
            rows: self.rows,
            truncated: self.truncated,
        }
    }
}

fn estimate_row_bytes(row: &[Option<String>]) -> usize {
    32 + row
        .iter()
        .map(|cell| 16 + cell.as_ref().map_or(0, String::len))
        .sum::<usize>()
}

fn estimate_diff_bytes(diff: &ComparedRowDiff) -> usize {
    let key_bytes: usize = diff.key_values.iter().map(|s| 16 + s.len()).sum();
    let cell_bytes: usize = diff
        .cells
        .iter()
        .map(|c| {
            32 + c.column.len()
                + c.left.as_ref().map_or(0, String::len)
                + c.right.as_ref().map_or(0, String::len)
        })
        .sum();
    let left_bytes = diff.left_row.as_deref().map_or(0, estimate_row_bytes);
    let right_bytes = diff.right_row.as_deref().map_or(0, estimate_row_bytes);
    96 + key_bytes + cell_bytes + left_bytes + right_bytes
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyAtom {
    Int(i128, String),
    Text(String),
}

impl KeyAtom {
    fn from_raw(value: String, integer_sort: bool) -> Self {
        if integer_sort && let Ok(parsed) = value.parse::<i128>() {
            return Self::Int(parsed, value);
        }
        Self::Text(value)
    }
}

impl Ord for KeyAtom {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Int(a_num, a_raw), Self::Int(b_num, b_raw)) => {
                a_num.cmp(b_num).then_with(|| a_raw.cmp(b_raw))
            }
            (Self::Int(..), Self::Text(..)) => Ordering::Less,
            (Self::Text(..), Self::Int(..)) => Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for KeyAtom {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CompareKey {
    atoms: Vec<KeyAtom>,
    raw: Vec<String>,
}

struct AlignedInternal {
    meta: AlignedCompareColumn,
    left_index: usize,
    right_index: usize,
    integer_key_capable: bool,
}

enum KeyBucket {
    Single(TextRow),
    Duplicate(usize),
}

struct IndexedSide {
    unique: BTreeMap<CompareKey, TextRow>,
    duplicates: BTreeMap<CompareKey, usize>,
    null_key_count: usize,
    null_key_samples: Vec<TextRow>,
    duplicate_key_rows: usize,
}

fn normalize_name(name: &str, ignore_case: bool) -> String {
    if ignore_case {
        name.to_ascii_lowercase()
    } else {
        name.to_owned()
    }
}

fn check_unique_columns(
    side_name: &str,
    columns: &[ColumnInfo],
    ignore_case: bool,
) -> AppResult<HashMap<String, usize>> {
    let mut map = HashMap::with_capacity(columns.len());
    for (idx, col) in columns.iter().enumerate() {
        let key = normalize_name(&col.name, ignore_case);
        if let Some(prev_idx) = map.insert(key, idx) {
            let prev_name = &columns[prev_idx].name;
            return Err(AppError::invalid(
                if ignore_case && prev_name != &col.name {
                    format!(
                        "{side_name} columns \"{prev_name}\" and \"{}\" collide when case-insensitive column matching is enabled",
                        col.name
                    )
                } else {
                    format!(
                        "{side_name} contains duplicate column name \"{}\". Alias duplicate columns before comparing.",
                        col.name
                    )
                },
            ));
        }
    }
    Ok(map)
}

fn index_side(
    rows: Vec<TextRow>,
    aligned: &[AlignedInternal],
    key_positions: &[(usize, bool)],
) -> IndexedSide {
    let mut buckets: BTreeMap<CompareKey, KeyBucket> = BTreeMap::new();
    let mut null_key_count = 0usize;
    let mut null_key_samples = Vec::new();

    for row in rows {
        let projected: TextRow = aligned
            .iter()
            .map(|col| row[col.left_index].clone())
            .collect();
        let mut atoms = Vec::with_capacity(key_positions.len());
        let mut raw = Vec::with_capacity(key_positions.len());
        let mut has_null = false;
        for &(aligned_idx, integer_sort) in key_positions {
            match &projected[aligned_idx] {
                Some(val) => {
                    raw.push(val.clone());
                    atoms.push(KeyAtom::from_raw(val.clone(), integer_sort));
                }
                None => {
                    has_null = true;
                    break;
                }
            }
        }
        if has_null {
            null_key_count += 1;
            if null_key_samples.len() < MAX_ANOMALY_SAMPLES {
                null_key_samples.push(projected);
            }
            continue;
        }
        let key = CompareKey { atoms, raw };
        match buckets.get_mut(&key) {
            None => {
                buckets.insert(key, KeyBucket::Single(projected));
            }
            Some(entry) => {
                let next_count = match entry {
                    KeyBucket::Single(_) => 2,
                    KeyBucket::Duplicate(c) => *c + 1,
                };
                *entry = KeyBucket::Duplicate(next_count);
            }
        }
    }

    let mut unique = BTreeMap::new();
    let mut duplicates = BTreeMap::new();
    let mut duplicate_key_rows = 0usize;
    for (key, bucket) in buckets {
        match bucket {
            KeyBucket::Single(row) => {
                unique.insert(key, row);
            }
            KeyBucket::Duplicate(count) => {
                duplicate_key_rows += count;
                duplicates.insert(key, count);
            }
        }
    }

    IndexedSide {
        unique,
        duplicates,
        null_key_count,
        null_key_samples,
        duplicate_key_rows,
    }
}

pub fn compare_datasets(
    left: CompareDataset,
    right: CompareDataset,
    requested_keys: &[String],
    ignore_column_case: bool,
) -> AppResult<DataCompareReport> {
    if left.columns.is_empty() || right.columns.is_empty() {
        return Err(AppError::invalid(
            "Both compared datasets must have at least one column",
        ));
    }

    let _left_map = check_unique_columns("Left dataset", &left.columns, ignore_column_case)?;
    let right_map = check_unique_columns("Right dataset", &right.columns, ignore_column_case)?;

    let mut aligned = Vec::new();
    let mut unaligned_left_columns = Vec::new();
    let mut matched_right_indices = HashSet::new();

    for (left_idx, left_col) in left.columns.iter().enumerate() {
        let lookup = normalize_name(&left_col.name, ignore_column_case);
        if let Some(&right_idx) = right_map.get(&lookup) {
            let right_col = &right.columns[right_idx];
            matched_right_indices.insert(right_idx);
            aligned.push(AlignedInternal {
                meta: AlignedCompareColumn {
                    name: left_col.name.clone(),
                    left_name: left_col.name.clone(),
                    right_name: right_col.name.clone(),
                    left_type: left_col.database_type.clone(),
                    right_type: right_col.database_type.clone(),
                    kind_mismatch: left_col.kind != right_col.kind,
                    is_key: false,
                },
                left_index: left_idx,
                right_index: right_idx,
                integer_key_capable: left_col.kind == ColumnKind::Integer
                    && right_col.kind == ColumnKind::Integer,
            });
        } else {
            unaligned_left_columns.push(left_col.name.clone());
        }
    }

    if aligned.is_empty() {
        return Err(AppError::invalid(
            "Left and right datasets do not share any matching column names",
        ));
    }

    let unaligned_right_columns: Vec<String> = right
        .columns
        .iter()
        .enumerate()
        .filter(|(idx, _)| !matched_right_indices.contains(idx))
        .map(|(_, col)| col.name.clone())
        .collect();

    // Resolve comparison key indices into `aligned`
    let aligned_lookup: HashMap<String, usize> = aligned
        .iter()
        .enumerate()
        .map(|(idx, col)| (normalize_name(&col.meta.name, ignore_column_case), idx))
        .collect();

    let resolved_key_indices: Vec<usize> = if !requested_keys.is_empty() {
        let mut indices = Vec::with_capacity(requested_keys.len());
        let mut seen = HashSet::new();
        for raw_key in requested_keys {
            let trimmed = raw_key.trim();
            if trimmed.is_empty() {
                continue;
            }
            let norm = normalize_name(trimmed, ignore_column_case);
            let Some(&aligned_idx) = aligned_lookup.get(&norm) else {
                return Err(AppError::invalid(format!(
                    "Key column \"{trimmed}\" is not shared by both datasets"
                )));
            };
            if !seen.insert(aligned_idx) {
                return Err(AppError::invalid(format!(
                    "Key column \"{trimmed}\" was specified more than once"
                )));
            }
            indices.push(aligned_idx);
        }
        indices
    } else {
        let shared_pks: Vec<usize> = aligned
            .iter()
            .enumerate()
            .filter(|(_, col)| {
                left.columns[col.left_index].primary_key
                    && right.columns[col.right_index].primary_key
            })
            .map(|(idx, _)| idx)
            .collect();
        if !shared_pks.is_empty() {
            shared_pks
        } else {
            let left_pk_total = left.columns.iter().filter(|c| c.primary_key).count();
            let right_pk_total = right.columns.iter().filter(|c| c.primary_key).count();
            if left_pk_total > 0 && right_pk_total == 0 {
                let left_aligned_pks: Vec<usize> = aligned
                    .iter()
                    .enumerate()
                    .filter(|(_, col)| left.columns[col.left_index].primary_key)
                    .map(|(idx, _)| idx)
                    .collect();
                if left_aligned_pks.len() == left_pk_total {
                    left_aligned_pks
                } else {
                    Vec::new()
                }
            } else if right_pk_total > 0 && left_pk_total == 0 {
                let right_aligned_pks: Vec<usize> = aligned
                    .iter()
                    .enumerate()
                    .filter(|(_, col)| right.columns[col.right_index].primary_key)
                    .map(|(idx, _)| idx)
                    .collect();
                if right_aligned_pks.len() == right_pk_total {
                    right_aligned_pks
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        }
    };

    if resolved_key_indices.is_empty() {
        return Err(AppError::invalid(
            "Select at least one shared key column to compare rows",
        ));
    }

    for &idx in &resolved_key_indices {
        aligned[idx].meta.is_key = true;
    }

    let key_columns: Vec<String> = resolved_key_indices
        .iter()
        .map(|&idx| aligned[idx].meta.name.clone())
        .collect();
    let key_positions: Vec<(usize, bool)> = resolved_key_indices
        .iter()
        .map(|&idx| (idx, aligned[idx].integer_key_capable))
        .collect();

    let left_total_rows = left.rows.len();
    let right_total_rows = right.rows.len();

    let mut left_indexed = index_side(left.rows, &aligned, &key_positions);

    // For Right, remap `left_index` projection to `right_index`
    let right_projection_specs: Vec<AlignedInternal> = aligned
        .iter()
        .map(|col| AlignedInternal {
            meta: col.meta.clone(),
            left_index: col.right_index,
            right_index: col.right_index,
            integer_key_capable: col.integer_key_capable,
        })
        .collect();
    let mut right_indexed = index_side(right.rows, &right_projection_specs, &key_positions);

    // Exclude keys that are duplicated on either side from 1:1 unique matching so a
    // duplicated key on Left never produces a phantom `OnlyRight` on Right (or vice versa).
    let mut left_excluded_counterpart_rows = 0usize;
    let mut right_excluded_counterpart_rows = 0usize;

    let mut left_dup_samples = Vec::new();
    for (dup_key, &left_occ) in &left_indexed.duplicates {
        let counterpart_occ = if let Some(&right_occ) = right_indexed.duplicates.get(dup_key) {
            right_occ
        } else if right_indexed.unique.remove(dup_key).is_some() {
            right_excluded_counterpart_rows += 1;
            1
        } else {
            0
        };
        if left_dup_samples.len() < MAX_ANOMALY_SAMPLES {
            left_dup_samples.push(DuplicateKeySample {
                key_values: dup_key.raw.clone(),
                occurrences: left_occ,
                counterpart_occurrences: counterpart_occ,
            });
        }
    }

    let mut right_dup_samples = Vec::new();
    for (dup_key, &right_occ) in &right_indexed.duplicates {
        let counterpart_occ = if let Some(&left_occ) = left_indexed.duplicates.get(dup_key) {
            left_occ
        } else if left_indexed.unique.remove(dup_key).is_some() {
            left_excluded_counterpart_rows += 1;
            1
        } else {
            0
        };
        if right_dup_samples.len() < MAX_ANOMALY_SAMPLES {
            right_dup_samples.push(DuplicateKeySample {
                key_values: dup_key.raw.clone(),
                occurrences: right_occ,
                counterpart_occurrences: counterpart_occ,
            });
        }
    }

    let left_anomalies = CompareSideAnomalies {
        null_key_count: left_indexed.null_key_count,
        null_key_samples: left_indexed.null_key_samples,
        duplicate_key_rows: left_indexed.duplicate_key_rows,
        duplicate_distinct_keys: left_indexed.duplicates.len(),
        excluded_counterpart_rows: left_excluded_counterpart_rows,
        duplicate_key_samples: left_dup_samples,
    };
    let right_anomalies = CompareSideAnomalies {
        null_key_count: right_indexed.null_key_count,
        null_key_samples: right_indexed.null_key_samples,
        duplicate_key_rows: right_indexed.duplicate_key_rows,
        duplicate_distinct_keys: right_indexed.duplicates.len(),
        excluded_counterpart_rows: right_excluded_counterpart_rows,
        duplicate_key_samples: right_dup_samples,
    };

    let mut identical_count = 0usize;
    let mut different_count = 0usize;
    let mut only_left_count = 0usize;
    let mut only_right_count = 0usize;
    let mut diffs = Vec::new();
    let mut diffs_truncated = false;
    let mut report_bytes = 0usize;

    let mut push_diff = |diff: ComparedRowDiff| {
        if diffs_truncated || diffs.len() >= MAX_REPORT_DIFF_ROWS {
            diffs_truncated = true;
            return;
        }
        let est = estimate_diff_bytes(&diff);
        if report_bytes.saturating_add(est) > MAX_REPORT_BYTES {
            diffs_truncated = true;
            return;
        }
        report_bytes += est;
        diffs.push(diff);
    };

    let mut left_iter = left_indexed.unique.into_iter().peekable();
    let mut right_iter = right_indexed.unique.into_iter().peekable();

    loop {
        match (left_iter.peek(), right_iter.peek()) {
            (Some((l_key, _)), Some((r_key, _))) => match l_key.cmp(r_key) {
                Ordering::Less => {
                    let (l_key, l_row) = left_iter.next().unwrap();
                    only_left_count += 1;
                    push_diff(ComparedRowDiff {
                        kind: RowDiffKind::OnlyLeft,
                        key_values: l_key.raw,
                        cells: Vec::new(),
                        left_row: Some(l_row),
                        right_row: None,
                    });
                }
                Ordering::Greater => {
                    let (r_key, r_row) = right_iter.next().unwrap();
                    only_right_count += 1;
                    push_diff(ComparedRowDiff {
                        kind: RowDiffKind::OnlyRight,
                        key_values: r_key.raw,
                        cells: Vec::new(),
                        left_row: None,
                        right_row: Some(r_row),
                    });
                }
                Ordering::Equal => {
                    let (l_key, l_row) = left_iter.next().unwrap();
                    let (_, r_row) = right_iter.next().unwrap();
                    let mut cells = Vec::new();
                    for (col_idx, col) in aligned.iter().enumerate() {
                        if l_row[col_idx] != r_row[col_idx] {
                            cells.push(CellDiff {
                                column: col.meta.name.clone(),
                                left: l_row[col_idx].clone(),
                                right: r_row[col_idx].clone(),
                                type_mismatch: col.meta.kind_mismatch,
                            });
                        }
                    }
                    if cells.is_empty() {
                        identical_count += 1;
                    } else {
                        different_count += 1;
                        push_diff(ComparedRowDiff {
                            kind: RowDiffKind::Different,
                            key_values: l_key.raw,
                            cells,
                            left_row: Some(l_row),
                            right_row: Some(r_row),
                        });
                    }
                }
            },
            (Some(_), None) => {
                let (l_key, l_row) = left_iter.next().unwrap();
                only_left_count += 1;
                push_diff(ComparedRowDiff {
                    kind: RowDiffKind::OnlyLeft,
                    key_values: l_key.raw,
                    cells: Vec::new(),
                    left_row: Some(l_row),
                    right_row: None,
                });
            }
            (None, Some(_)) => {
                let (r_key, r_row) = right_iter.next().unwrap();
                only_right_count += 1;
                push_diff(ComparedRowDiff {
                    kind: RowDiffKind::OnlyRight,
                    key_values: r_key.raw,
                    cells: Vec::new(),
                    left_row: None,
                    right_row: Some(r_row),
                });
            }
            (None, None) => break,
        }
    }

    let aligned_columns = aligned.into_iter().map(|col| col.meta).collect();

    Ok(DataCompareReport {
        left_label: left.label,
        right_label: right.label,
        left_truncated: left.truncated,
        right_truncated: right.truncated,
        key_columns,
        aligned_columns,
        unaligned_left_columns,
        unaligned_right_columns,
        left_total_rows,
        right_total_rows,
        identical_count,
        different_count,
        only_left_count,
        only_right_count,
        left_anomalies,
        right_anomalies,
        diffs,
        diffs_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, kind: ColumnKind, db_type: &str, pk: bool) -> ColumnInfo {
        ColumnInfo {
            id: format!("c_{name}"),
            name: name.into(),
            database_type: db_type.into(),
            kind,
            nullable: !pk,
            primary_key: pk,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }
    }

    #[test]
    fn compares_composite_keys_null_vs_empty_numeric_order_and_anomalies_without_phantom_diffs() {
        let left_cols = vec![
            col("tenant_id", ColumnKind::Integer, "bigint", true),
            col("order_no", ColumnKind::Integer, "bigint", true),
            col("note", ColumnKind::Text, "text", false),
            col("amount", ColumnKind::Decimal, "numeric(18,4)", false),
            col("left_extra", ColumnKind::Text, "text", false),
        ];
        // Right has reordered columns, different case on `AMOUNT`, type mismatch on `amount` (Text vs Decimal), and an extra column
        let right_cols = vec![
            col("right_extra", ColumnKind::Text, "varchar(32)", false),
            col("order_no", ColumnKind::Integer, "bigint", true),
            col("tenant_id", ColumnKind::Integer, "bigint", true),
            col("AMOUNT", ColumnKind::Text, "varchar(64)", false),
            col("note", ColumnKind::Text, "varchar(128)", false),
        ];

        let left = CompareDataset {
            label: "pg.orders".into(),
            columns: left_cols,
            truncated: false,
            rows: vec![
                // Key (1, 2): identical
                vec![
                    Some("1".into()),
                    Some("2".into()),
                    Some("same".into()),
                    Some("9223372036854775807.0001".into()),
                    Some("ignored-l".into()),
                ],
                // Key (1, 10): NULL vs "" on `note` + decimal difference on `amount` (numeric key 10 sorts AFTER 2!)
                vec![
                    Some("1".into()),
                    Some("10".into()),
                    None,
                    Some("10.5000".into()),
                    None,
                ],
                // Key (1, 3): only in left
                vec![
                    Some("1".into()),
                    Some("3".into()),
                    Some("only-l".into()),
                    Some("1.0000".into()),
                    None,
                ],
                // Key (2, 1): duplicated on Left (2 rows), single on Right -> must NOT report OnlyRight!
                vec![
                    Some("2".into()),
                    Some("1".into()),
                    Some("dup-a".into()),
                    Some("5.0000".into()),
                    None,
                ],
                vec![
                    Some("2".into()),
                    Some("1".into()),
                    Some("dup-b".into()),
                    Some("5.0000".into()),
                    None,
                ],
                // NULL key component on Left
                vec![
                    Some("1".into()),
                    None,
                    Some("null-key".into()),
                    Some("0.0000".into()),
                    None,
                ],
            ],
        };

        let right = CompareDataset {
            label: "mysql.orders".into(),
            columns: right_cols,
            truncated: false,
            rows: vec![
                // Key (1, 10): "" on `note`, "10.50" on `AMOUNT`
                vec![
                    Some("ignored-r".into()),
                    Some("10".into()),
                    Some("1".into()),
                    Some("10.50".into()),
                    Some("".into()),
                ],
                // Key (1, 2): identical
                vec![
                    None,
                    Some("2".into()),
                    Some("1".into()),
                    Some("9223372036854775807.0001".into()),
                    Some("same".into()),
                ],
                // Key (1, 4): only in right
                vec![
                    None,
                    Some("4".into()),
                    Some("1".into()),
                    Some("9.99".into()),
                    Some("only-r".into()),
                ],
                // Key (2, 1): single on Right, duplicated on Left -> excluded counterpart row!
                vec![
                    None,
                    Some("1".into()),
                    Some("2".into()),
                    Some("5.0000".into()),
                    Some("dup-a".into()),
                ],
            ],
        };

        let report = compare_datasets(left, right, &[], true).unwrap();
        assert_eq!(report.key_columns, vec!["tenant_id", "order_no"]);
        assert_eq!(report.unaligned_left_columns, vec!["left_extra"]);
        assert_eq!(report.unaligned_right_columns, vec!["right_extra"]);
        assert_eq!(report.identical_count, 1);
        assert_eq!(report.different_count, 1);
        assert_eq!(report.only_left_count, 1);
        assert_eq!(report.only_right_count, 1);

        // Anomaly accounting: every row on both sides is accounted for
        assert_eq!(report.left_anomalies.null_key_count, 1);
        assert_eq!(report.left_anomalies.duplicate_key_rows, 2);
        assert_eq!(report.left_anomalies.duplicate_distinct_keys, 1);
        assert_eq!(
            report.left_anomalies.duplicate_key_samples[0],
            DuplicateKeySample {
                key_values: vec!["2".into(), "1".into()],
                occurrences: 2,
                counterpart_occurrences: 1,
            }
        );
        assert_eq!(report.right_anomalies.excluded_counterpart_rows, 1);
        assert_eq!(
            report.left_total_rows,
            report.identical_count
                + report.different_count
                + report.only_left_count
                + report.left_anomalies.null_key_count
                + report.left_anomalies.duplicate_key_rows
                + report.left_anomalies.excluded_counterpart_rows
        );
        assert_eq!(
            report.right_total_rows,
            report.identical_count
                + report.different_count
                + report.only_right_count
                + report.right_anomalies.null_key_count
                + report.right_anomalies.duplicate_key_rows
                + report.right_anomalies.excluded_counterpart_rows
        );

        // Numeric key ordering: (1, 3) OnlyLeft -> (1, 4) OnlyRight -> (1, 10) Different
        assert_eq!(report.diffs.len(), 3);
        assert_eq!(report.diffs[0].kind, RowDiffKind::OnlyLeft);
        assert_eq!(report.diffs[0].key_values, vec!["1", "3"]);
        assert_eq!(report.diffs[1].kind, RowDiffKind::OnlyRight);
        assert_eq!(report.diffs[1].key_values, vec!["1", "4"]);
        assert_eq!(report.diffs[2].kind, RowDiffKind::Different);
        assert_eq!(report.diffs[2].key_values, vec!["1", "10"]);
        assert_eq!(
            report.diffs[2].cells,
            vec![
                CellDiff {
                    column: "note".into(),
                    left: None,
                    right: Some("".into()),
                    type_mismatch: false,
                },
                CellDiff {
                    column: "amount".into(),
                    left: Some("10.5000".into()),
                    right: Some("10.50".into()),
                    type_mismatch: true,
                },
            ]
        );
    }

    #[test]
    fn enforces_non_empty_key_and_side_row_or_byte_caps() {
        let cols = vec![
            col("a", ColumnKind::Text, "text", false),
            col("b", ColumnKind::Text, "text", false),
        ];
        let left = CompareDataset {
            label: "L".into(),
            columns: cols.clone(),
            rows: vec![vec![Some("1".into()), Some("x".into())]],
            truncated: false,
        };
        let right = CompareDataset {
            label: "R".into(),
            columns: cols.clone(),
            rows: vec![vec![Some("1".into()), Some("x".into())]],
            truncated: false,
        };
        assert_eq!(
            compare_datasets(left, right, &[], false).unwrap_err().code,
            ErrorCode::InvalidInput
        );

        let mut row_capped = CompareDatasetBuilder::with_limits(
            "Left dataset",
            "L".into(),
            cols.clone(),
            false,
            2,
            1024 * 1024,
        );
        row_capped
            .append_rows(vec![
                vec![Some("1".into()), Some("a".into())],
                vec![Some("2".into()), Some("b".into())],
            ])
            .unwrap();
        assert_eq!(
            row_capped
                .append_rows(vec![vec![Some("3".into()), Some("c".into())]])
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );

        let mut byte_capped =
            CompareDatasetBuilder::with_limits("Right dataset", "R".into(), cols, false, 100, 256);
        assert_eq!(
            byte_capped
                .append_rows(vec![vec![Some("1".into()), Some("x".repeat(512))]])
                .unwrap_err()
                .code,
            ErrorCode::ResultLimitReached
        );
    }
}
