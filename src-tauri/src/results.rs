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
    pub fn usage(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.bytes, state.results.len())
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
