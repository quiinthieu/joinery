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
            query_id: None,
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

    pub fn apply_committed_changes(
        &self,
        anchor_id: Option<&str>,
        updates: &[ResultChanges],
        deletes: &[ResultDeletes],
        inserted_rows: &[TextRow],
    ) -> AppResult<Vec<ResultInfo>> {
        let mut affected_ids = Vec::new();
        for u in updates {
            if !affected_ids.contains(&u.result_id) {
                affected_ids.push(u.result_id.clone());
            }
        }
        for d in deletes {
            if !affected_ids.contains(&d.result_id) {
                affected_ids.push(d.result_id.clone());
            }
        }
        if !inserted_rows.is_empty()
            && let Some(id) = anchor_id
            && !affected_ids.iter().any(|existing| existing == id)
        {
            affected_ids.push(id.to_owned());
        }

        let mut state = self.state.lock();
        let mut updated_infos = Vec::new();

        for id in affected_ids {
            let Some(entry) = state.results.get(&id) else {
                continue;
            };
            if entry.info.run_id.starts_with("table:")
                || !matches!(
                    entry.info.editability,
                    Some(crate::domain::ResultEditability::Editable { .. })
                )
            {
                continue;
            }
            let col_count = entry.info.columns.len();
            let col_positions: HashMap<String, usize> = entry
                .info
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.id.clone(), idx))
                .collect();

            let mut rows = Vec::with_capacity(entry.info.retained_rows + inserted_rows.len());
            for batch in &entry.batches {
                let arrays: Vec<&StringArray> = (0..col_count)
                    .map(|col_idx| {
                        batch
                            .column(col_idx)
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .ok_or_else(|| AppError::internal("Invalid result column encoding"))
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                for row_idx in 0..batch.num_rows() {
                    let mut row = Vec::with_capacity(col_count);
                    for array in &arrays {
                        if array.is_null(row_idx) {
                            row.push(None);
                        } else {
                            row.push(Some(array.value(row_idx).to_string()));
                        }
                    }
                    rows.push(row);
                }
            }

            for u in updates.iter().filter(|u| u.result_id == id) {
                for change in &u.changes {
                    if let Some(&col_idx) = col_positions.get(&change.column)
                        && let Some(row) = rows.get_mut(change.row)
                        && col_idx < row.len()
                    {
                        row[col_idx] = change.value.clone();
                    }
                }
            }

            let deleted_set: std::collections::HashSet<usize> = deletes
                .iter()
                .filter(|d| d.result_id == id)
                .flat_map(|d| d.rows.iter().copied())
                .collect();
            if !deleted_set.is_empty() {
                let mut idx = 0usize;
                rows.retain(|_| {
                    let keep = !deleted_set.contains(&idx);
                    idx += 1;
                    keep
                });
            }

            if anchor_id == Some(id.as_str()) && !inserted_rows.is_empty() {
                for inserted in inserted_rows {
                    if inserted.len() == col_count {
                        rows.push(inserted.clone());
                    }
                }
            }

            let schema = Arc::new(Schema::new(
                entry
                    .info
                    .columns
                    .iter()
                    .map(|c| Field::new(&c.id, DataType::Utf8, true))
                    .collect::<Vec<_>>(),
            ));
            let mut new_batches = Vec::new();
            let mut new_bytes = 0usize;
            for chunk in rows.chunks(BATCH_ROWS) {
                let arrays: Vec<ArrayRef> = (0..col_count)
                    .map(|i| {
                        let bytes = chunk
                            .iter()
                            .map(|row| row[i].as_ref().map_or(0, String::len))
                            .sum();
                        let mut builder = StringBuilder::with_capacity(chunk.len(), bytes);
                        for row in chunk {
                            builder.append_option(row[i].as_deref());
                        }
                        Arc::new(builder.finish()) as ArrayRef
                    })
                    .collect();
                let batch = RecordBatch::try_new(schema.clone(), arrays)
                    .map_err(|_| AppError::internal("Could not encode updated result batch"))?;
                new_bytes += batch.get_array_memory_size();
                new_batches.push(batch);
            }

            let old_bytes = entry.bytes;
            state.bytes = state
                .bytes
                .saturating_sub(old_bytes)
                .saturating_add(new_bytes);
            if let Some(entry_mut) = state.results.get_mut(&id) {
                entry_mut.bytes = new_bytes;
                entry_mut.batches = new_batches;
                entry_mut.info.retained_rows = rows.len();
                updated_infos.push(entry_mut.info.clone());
            }
        }

        Ok(updated_infos)
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
