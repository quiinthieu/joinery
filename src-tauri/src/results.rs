use crate::domain::*;
use arrow_array::builder::StringBuilder;
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{DataType, Field, Schema};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc};

pub const RESULT_BUDGET: usize = 64 * 1024 * 1024;
pub const ROW_LIMIT: usize = 100_000;
pub const BATCH_ROWS: usize = 512;
pub const MAX_RANGE_ROWS: usize = 512;
const FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct BoundedFrame(Vec<u8>);
impl std::io::Write for BoundedFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "Arrow frame budget exceeded",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn frame_limit(_: arrow_schema::ArrowError) -> AppError {
    frame_limit_error()
}
fn frame_limit_error() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "This page contains large values. Request fewer rows.",
    )
}

struct StoredResult {
    info: ResultInfo,
    batches: Vec<RecordBatch>,
    bytes: usize,
    provenance: Option<TableSchema>,
    edit_identity: Option<crate::drivers::EditIdentity>,
}

#[derive(Default)]
struct State {
    results: HashMap<String, StoredResult>,
    bytes: usize,
}

pub struct ResultStore {
    state: Mutex<State>,
    budget: usize,
    row_limit: usize,
}

impl Default for ResultStore {
    fn default() -> Self {
        Self::new(RESULT_BUDGET, ROW_LIMIT)
    }
}
impl ResultStore {
    pub fn new(budget: usize, row_limit: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            budget,
            row_limit,
        }
    }

    pub fn create(
        &self,
        run_id: &str,
        statement_index: usize,
        columns: Vec<ColumnInfo>,
        provenance: Option<TableSchema>,
    ) -> AppResult<ResultInfo> {
        let editability = provenance.as_ref().map(|schema| {
            if schema.editable && !schema.primary_key.is_empty() {
                ResultEditability::Editable {
                    table: schema.table.clone(),
                    primary_key: schema.primary_key.clone(),
                }
            } else {
                ResultEditability::ReadOnly {
                    reason: if schema.primary_key.is_empty() {
                        format!(
                            "Table \"{}.{}\" has no primary key",
                            schema.table.schema, schema.table.name
                        )
                    } else {
                        format!(
                            "Table \"{}.{}\" is read-only",
                            schema.table.schema, schema.table.name
                        )
                    },
                }
            }
        });
        self.create_with_editability(run_id, statement_index, columns, provenance, editability)
    }

    pub fn create_with_editability(
        &self,
        run_id: &str,
        statement_index: usize,
        columns: Vec<ColumnInfo>,
        provenance: Option<TableSchema>,
        editability: Option<ResultEditability>,
    ) -> AppResult<ResultInfo> {
        if columns.len() > 1000 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Results with more than 1,000 columns are not supported",
            ));
        }
        let info = ResultInfo {
            id: new_id(),
            run_id: run_id.into(),
            statement_index,
            columns,
            retained_rows: 0,
            status: RetentionStatus::Streaming,
            table: provenance.as_ref().map(|s| s.table.clone()),
            editability,
        };
        let mut state = self.state.lock();
        if state.results.len() >= 100 {
            return Err(AppError::new(
                ErrorCode::ResultLimitReached,
                "Close a result tab before opening more results",
            ));
        }
        state.results.insert(
            info.id.clone(),
            StoredResult {
                info: info.clone(),
                batches: Vec::new(),
                bytes: 0,
                provenance,
                edit_identity: None,
            },
        );
        Ok(info)
    }

    pub fn append(&self, id: &str, rows: Vec<TextRow>) -> AppResult<bool> {
        if rows.is_empty() {
            return Ok(true);
        }
        let mut state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        let columns = &result.info.columns;
        if rows.iter().any(|row| row.len() != columns.len()) {
            return Err(AppError::internal(
                "Result column count changed unexpectedly",
            ));
        }
        if result.info.retained_rows + rows.len() > self.row_limit {
            return Ok(false);
        }
        let estimated = rows
            .iter()
            .flat_map(|row| row.iter())
            .map(|v| v.as_ref().map_or(0, String::len) + 8)
            .sum::<usize>();
        if state.bytes.saturating_add(estimated) > self.budget {
            return Ok(false);
        }
        let arrays: Vec<ArrayRef> = (0..columns.len())
            .map(|i| {
                let bytes = rows
                    .iter()
                    .map(|row| row[i].as_ref().map_or(0, String::len))
                    .sum();
                let mut builder = StringBuilder::with_capacity(rows.len(), bytes);
                for row in &rows {
                    builder.append_option(row[i].as_deref());
                }
                Arc::new(builder.finish()) as ArrayRef
            })
            .collect();
        let schema = Arc::new(Schema::new(
            columns
                .iter()
                .map(|c| Field::new(&c.id, DataType::Utf8, true))
                .collect::<Vec<_>>(),
        ));
        let batch = RecordBatch::try_new(schema, arrays)
            .map_err(|_| AppError::internal("Could not encode result batch"))?;
        let bytes = batch.get_array_memory_size();
        if state.bytes.saturating_add(bytes) > self.budget {
            return Ok(false);
        }
        state.bytes += bytes;
        let result = state.results.get_mut(id).ok_or_else(not_found)?;
        result.bytes += bytes;
        result.info.retained_rows += batch.num_rows();
        result.batches.push(batch);
        Ok(true)
    }

    pub fn finish(&self, id: &str, truncated: bool) -> AppResult<ResultInfo> {
        let mut state = self.state.lock();
        let result = state.results.get_mut(id).ok_or_else(not_found)?;
        result.info.status = if truncated {
            RetentionStatus::Truncated
        } else {
            RetentionStatus::Complete
        };
        Ok(result.info.clone())
    }

    pub fn info(&self, id: &str) -> AppResult<ResultInfo> {
        self.state
            .lock()
            .results
            .get(id)
            .map(|r| r.info.clone())
            .ok_or_else(not_found)
    }

    pub fn provenance(&self, id: &str) -> AppResult<TableSchema> {
        self.state
            .lock()
            .results
            .get(id)
            .and_then(|r| r.provenance.clone())
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::PolicyBlocked,
                    "Only table browser rows with verified primary keys can be edited",
                )
            })
    }

    pub(crate) fn edit_identity(
        &self,
        id: &str,
    ) -> AppResult<Option<crate::drivers::EditIdentity>> {
        self.state
            .lock()
            .results
            .get(id)
            .map(|r| r.edit_identity.clone())
            .ok_or_else(not_found)
    }

    pub(crate) fn set_edit_identity(
        &self,
        id: &str,
        identity: Option<crate::drivers::EditIdentity>,
    ) -> AppResult<()> {
        self.state
            .lock()
            .results
            .get_mut(id)
            .ok_or_else(not_found)?
            .edit_identity = identity;
        Ok(())
    }

    pub fn row(&self, id: &str, index: usize) -> AppResult<TextRow> {
        let state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        let mut start = 0;
        for batch in &result.batches {
            if index < start + batch.num_rows() {
                return Ok(batch
                    .columns()
                    .iter()
                    .map(|array| {
                        let array = array
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .expect("result arrays are UTF8");
                        if array.is_null(index - start) {
                            None
                        } else {
                            Some(array.value(index - start).to_string())
                        }
                    })
                    .collect());
            }
            start += batch.num_rows();
        }
        Err(AppError::new(
            ErrorCode::NotFound,
            "The original row is no longer retained",
        ))
    }

    pub fn range(&self, id: &str, start: usize, count: usize) -> AppResult<Vec<u8>> {
        if count == 0 || count > MAX_RANGE_ROWS {
            return Err(AppError::invalid(
                "Request between 1 and 512 rows per batch",
            ));
        }
        let state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        if start > result.info.retained_rows {
            return Err(AppError::invalid(
                "The requested result range is not retained",
            ));
        }
        // Arrow creates its encoded body before writing it. Reject wide ranges
        // before encoding, in addition to bounding the destination writer.
        let end = start.saturating_add(count).min(result.info.retained_rows);
        let mut estimated = 4096 + result.info.columns.len() * 128;
        let mut offset = 0;
        for batch in &result.batches {
            let batch_end = offset + batch.num_rows();
            if start < batch_end && end > offset {
                let from = start.saturating_sub(offset);
                let length = (end.min(batch_end) - offset) - from;
                estimated += batch.num_columns() * (128 + (length + 1) * 5);
                for column in batch.columns() {
                    let values = column
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .ok_or_else(|| AppError::internal("Invalid result column"))?;
                    estimated += (values.value_offsets()[from + length]
                        - values.value_offsets()[from]) as usize;
                }
                if estimated > FRAME_BYTES {
                    return Err(frame_limit_error());
                }
            }
            offset = batch_end;
            if offset >= end {
                break;
            }
        }
        let mut metadata = HashMap::new();
        metadata.insert("joinery.protocol".into(), "1".into());
        metadata.insert("joinery.resultId".into(), id.into());
        metadata.insert("joinery.offset".into(), start.to_string());
        let schema = Schema::new_with_metadata(
            result
                .info
                .columns
                .iter()
                .map(|c| Field::new(&c.id, DataType::Utf8, true))
                .collect::<Vec<_>>(),
            metadata,
        );
        let schema = Arc::new(schema);
        let mut output = BoundedFrame::default();
        {
            let mut writer = StreamWriter::try_new(&mut output, &schema).map_err(frame_limit)?;
            let mut offset = 0;
            for batch in &result.batches {
                let batch_end = offset + batch.num_rows();
                if start < batch_end && end > offset {
                    let from = start.saturating_sub(offset);
                    let length = (end.min(batch_end) - offset) - from;
                    let slice = batch.slice(from, length);
                    let slice = RecordBatch::try_new(schema.clone(), slice.columns().to_vec())
                        .map_err(|_| AppError::internal("Invalid result slice"))?;
                    writer.write(&slice).map_err(frame_limit)?;
                }
                offset = batch_end;
                if offset >= end {
                    break;
                }
            }
            writer.finish().map_err(frame_limit)?;
        }
        Ok(output.0)
    }

    pub fn release(&self, id: &str) {
        let mut state = self.state.lock();
        if let Some(result) = state.results.remove(id) {
            state.bytes = state.bytes.saturating_sub(result.bytes);
        }
    }

    pub fn batch_count(&self, id: &str) -> AppResult<usize> {
        let state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        Ok(result.batches.len())
    }

    pub fn batch_rows(
        &self,
        id: &str,
        batch_index: usize,
        column_indices: &[usize],
    ) -> AppResult<Option<Vec<TextRow>>> {
        let state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        let Some(batch) = result.batches.get(batch_index) else {
            return Ok(None);
        };
        let arrays: Vec<&StringArray> = column_indices
            .iter()
            .map(|&col_idx| {
                batch
                    .column(col_idx)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| AppError::internal("Invalid result column encoding"))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let num_rows = batch.num_rows();
        let mut rows = Vec::with_capacity(num_rows);
        for row_idx in 0..num_rows {
            let mut row = Vec::with_capacity(arrays.len());
            for array in &arrays {
                if array.is_null(row_idx) {
                    row.push(None);
                } else {
                    row.push(Some(array.value(row_idx).to_string()));
                }
            }
            rows.push(row);
        }
        Ok(Some(rows))
    }

    pub fn usage(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.bytes, state.results.len())
    }

    /// Keep the query's original bounded snapshot after a successful commit.
    /// Defaults, triggers, coercions, and query membership cannot be reconstructed
    /// from submitted mutations. Re-running SQL is the explicit refresh boundary.
    pub fn apply_committed_changes(
        &self,
        anchor_id: Option<&str>,
        updates: &[ResultChanges],
        deletes: &[ResultDeletes],
        has_inserts: bool,
    ) -> Vec<ResultInfo> {
        let mut affected_ids = Vec::new();
        for id in updates
            .iter()
            .map(|u| u.result_id.as_str())
            .chain(deletes.iter().map(|d| d.result_id.as_str()))
            .chain(anchor_id.filter(|_| has_inserts))
        {
            if !affected_ids.contains(&id) {
                affected_ids.push(id);
            }
        }
        let mut state = self.state.lock();
        let mut updated_infos = Vec::new();
        for id in affected_ids {
            let Some(entry) = state.results.get_mut(id) else {
                continue;
            };
            // Table browser results already refresh through their dedicated path.
            if entry.info.run_id.starts_with("table:")
                || !matches!(
                    entry.info.editability,
                    Some(ResultEditability::Editable { .. })
                )
            {
                continue;
            }
            entry.provenance = None;
            entry.edit_identity = None;
            entry.info.editability = Some(ResultEditability::ReadOnly {
                reason: "Result is a snapshot from before these changes. Re-run query to refresh."
                    .into(),
            });
            for column in &mut entry.info.columns {
                column.editable = false;
                column.insertable = false;
            }
            updated_infos.push(entry.info.clone());
        }
        updated_infos
    }
}

fn not_found() -> AppError {
    AppError::new(
        ErrorCode::NotFound,
        "This result was released or no longer exists",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editable_schema() -> TableSchema {
        let mut id = columns().remove(0);
        id.id = "c0".into();
        id.name = "id".into();
        id.primary_key = true;
        let mut value = columns().remove(0);
        value.id = "c1".into();
        value.editable = true;
        value.insertable = true;
        TableSchema {
            table: TableRef {
                schema: "public".into(),
                name: "items".into(),
            },
            columns: vec![id, value],
            primary_key: vec!["id".into()],
            foreign_keys: Vec::new(),
            inbound_foreign_keys: Vec::new(),
            editable: true,
        }
    }

    #[test]
    fn committed_query_changes_preserve_bounded_snapshot() {
        for mutation in ["insert", "update", "delete"] {
            let store = ResultStore::new(1024, 1);
            let schema = editable_schema();
            let result = store
                .create("query", 0, schema.columns.clone(), Some(schema))
                .unwrap();
            let original = vec![Some("1".into()), Some("original".into())];
            store
                .set_edit_identity(
                    &result.id,
                    Some(crate::drivers::EditIdentity::Postgres {
                        relation_oid: 42,
                        attributes: Vec::new(),
                    }),
                )
                .unwrap();
            assert!(store.append(&result.id, vec![original.clone()]).unwrap());
            let before = store.finish(&result.id, true).unwrap();
            let bytes_before = store.usage().0;
            let updates = if mutation == "update" {
                vec![ResultChanges {
                    result_id: result.id.clone(),
                    changes: vec![CellChange {
                        row: 0,
                        column: "c1".into(),
                        value: Some("x".repeat(2048)),
                    }],
                }]
            } else {
                Vec::new()
            };
            let deletes = if mutation == "delete" {
                vec![ResultDeletes {
                    result_id: result.id.clone(),
                    rows: vec![0],
                }]
            } else {
                Vec::new()
            };
            let updated = store.apply_committed_changes(
                Some(&result.id),
                &updates,
                &deletes,
                mutation == "insert",
            );
            assert!(
                store.usage().0 <= 1024,
                "{mutation} exceeded the global budget"
            );
            assert_eq!(store.usage().0, bytes_before);
            assert_eq!(updated.len(), 1);
            assert_eq!(updated[0].retained_rows, before.retained_rows);
            assert_eq!(updated[0].status, before.status);
            assert_eq!(store.row(&result.id, 0).unwrap(), original);
            assert!(matches!(
                updated[0].editability,
                Some(ResultEditability::ReadOnly { .. })
            ));
            assert!(
                updated[0]
                    .columns
                    .iter()
                    .all(|c| !c.editable && !c.insertable)
            );
            assert_eq!(
                store.provenance(&result.id).unwrap_err().code,
                ErrorCode::PolicyBlocked
            );
            assert!(store.edit_identity(&result.id).unwrap().is_none());
        }
    }

    #[test]
    fn committed_table_pages_and_released_queries_keep_existing_behavior() {
        let store = ResultStore::new(1024, 1);
        let schema = editable_schema();
        let table = store
            .create(
                "table:browse",
                0,
                schema.columns.clone(),
                Some(schema.clone()),
            )
            .unwrap();
        assert!(
            store
                .append(
                    &table.id,
                    vec![vec![Some("1".into()), Some("original".into())]]
                )
                .unwrap()
        );
        let before = store.finish(&table.id, false).unwrap();
        let bytes_before = store.usage().0;
        assert!(
            store
                .apply_committed_changes(Some(&table.id), &[], &[], true)
                .is_empty()
        );
        assert_eq!(store.info(&table.id).unwrap(), before);
        assert_eq!(store.provenance(&table.id).unwrap(), schema);
        assert_eq!(store.usage().0, bytes_before);
        store.release(&table.id);
        assert!(
            store
                .apply_committed_changes(Some(&table.id), &[], &[], true)
                .is_empty()
        );
        assert_eq!(store.usage(), (0, 0));
    }

    fn columns() -> Vec<ColumnInfo> {
        vec![ColumnInfo {
            id: "c0".into(),
            name: "value".into(),
            database_type: "text".into(),
            kind: ColumnKind::Text,
            nullable: true,
            primary_key: false,
            foreign_key: false,
            editable: false,
            insertable: false,
            has_default: false,
        }]
    }
    #[test]
    fn wide_ipc_ranges_are_rejected_before_encoding_and_small_ranges_work() {
        let store = ResultStore::default();
        let result = store.create("wide", 0, columns(), None).unwrap();
        assert!(
            store
                .append(
                    &result.id,
                    (0..8).map(|_| vec![Some("x".repeat(700_000))]).collect()
                )
                .unwrap()
        );
        assert_eq!(
            store.range(&result.id, 0, 8).unwrap_err().code,
            ErrorCode::ResultLimitReached
        );
        let frame = store.range(&result.id, 3, 1).unwrap();
        assert!(frame.len() < FRAME_BYTES);
        let mut reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(frame), None).unwrap();
        assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    }
    #[test]
    fn arrow_round_trip_preserves_null_empty_and_large_integer_text() {
        let store = ResultStore::default();
        let result = store.create("r", 0, columns(), None).unwrap();
        assert!(
            store
                .append(
                    &result.id,
                    vec![
                        vec![None],
                        vec![Some("".into())],
                        vec![Some("9223372036854775807".into())]
                    ]
                )
                .unwrap()
        );
        let data = store.range(&result.id, 0, 3).unwrap();
        let mut reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(data), None).unwrap();
        assert_eq!(reader.schema().metadata()["joinery.resultId"], result.id);
        let batch = reader.next().unwrap().unwrap();
        let a = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert!(a.is_null(0));
        assert_eq!(a.value(1), "");
        assert_eq!(a.value(2), "9223372036854775807");
        store.release(&result.id);
        assert_eq!(store.usage(), (0, 0));
    }
    #[test]
    fn budgets_are_global_and_failed_reservations_do_not_leak() {
        let store = ResultStore::new(1024, 2);
        let a = store.create("a", 0, columns(), None).unwrap();
        assert!(
            !store
                .append(&a.id, vec![vec![Some("x".repeat(4096))]])
                .unwrap()
        );
        assert_eq!(store.usage().0, 0);
        assert!(
            store
                .append(&a.id, vec![vec![None], vec![Some("a".into())]])
                .unwrap()
        );
        assert!(!store.append(&a.id, vec![vec![None]]).unwrap());
        assert_eq!(store.info(&a.id).unwrap().retained_rows, 2);
    }
}
