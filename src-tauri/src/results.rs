use crate::domain::*;
use arrow_array::builder::StringBuilder;
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{DataType, Field, Schema};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
mod storage;
use storage::{BatchData, BatchEntry, INDEX_BYTES, Spool};

pub const RESULT_BUDGET: usize = 64 * 1024 * 1024;
pub const ROW_LIMIT: usize = 1_000_000;
pub const DISK_BUDGET: usize = 8 * 1024 * 1024 * 1024;
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
    batches: Vec<BatchEntry>,
    schema: Arc<Schema>,
    bytes: usize,
    identity_bytes: usize,
    disk_bytes: usize,
    disk_enabled: bool,
    spool: Option<Spool>,
    provenance: Option<TableSchema>,
    edit_identity: Option<crate::drivers::EditIdentity>,
}

#[derive(Default)]
struct State {
    results: HashMap<String, StoredResult>,
    bytes: usize,
    disk_bytes: usize,
}

pub struct ResultStore {
    state: Mutex<State>,
    budget: usize,
    row_limit: usize,
    disk_budget: usize,
    temporary_disk_enabled: AtomicBool,
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
            disk_budget: DISK_BUDGET,
            temporary_disk_enabled: AtomicBool::new(false),
        }
    }
    pub fn set_temporary_disk_enabled(&self, enabled: bool) {
        self.temporary_disk_enabled.store(enabled, Ordering::SeqCst);
    }
    pub fn temporary_disk_enabled(&self) -> bool {
        self.temporary_disk_enabled.load(Ordering::SeqCst)
    }
    pub fn disk_usage(&self) -> usize {
        self.state.lock().disk_bytes
    }

    pub fn create(
        &self,
        run_id: &str,
        statement_index: usize,
        columns: Vec<ColumnInfo>,
        provenance: Option<TableSchema>,
    ) -> AppResult<ResultInfo> {
        if run_id
            .len()
            .saturating_add(columns_charge(&columns))
            .saturating_add(provenance.as_ref().map_or(0, schema_charge))
            > self.budget
        {
            return Err(metadata_limit());
        }
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
        if run_id.len() > self.budget
            || columns_charge(&columns).saturating_add(provenance.as_ref().map_or(0, schema_charge))
                > self.budget
        {
            return Err(metadata_limit());
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
        let metadata = metadata_charge(&info, provenance.as_ref());
        if state.bytes.saturating_add(metadata) > self.budget {
            return Err(metadata_limit());
        }
        let schema = Arc::new(Schema::new(
            info.columns
                .iter()
                .map(|c| Field::new(&c.id, DataType::Utf8, true))
                .collect::<Vec<_>>(),
        ));
        state.bytes += metadata;
        state.results.insert(
            info.id.clone(),
            StoredResult {
                info: info.clone(),
                batches: Vec::new(),
                schema,
                bytes: metadata,
                identity_bytes: 0,
                disk_bytes: 0,
                disk_enabled: self.temporary_disk_enabled(),
                spool: None,
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
        let State {
            results,
            bytes: global_bytes,
            disk_bytes: global_disk,
        } = &mut *state;
        let result = results.get_mut(id).ok_or_else(not_found)?;
        let columns = &result.info.columns;
        if rows.iter().any(|row| row.len() != columns.len()) {
            return Err(AppError::internal(
                "Result column count changed unexpectedly",
            ));
        }
        if result.info.retained_rows.saturating_add(rows.len()) > self.row_limit {
            return Ok(false);
        }
        // Reject oversized individual rows before allocating Arrow buffers. Input
        // ownership belongs to the caller; encoding never doubles an unbounded row.
        let row_bytes: Vec<usize> = rows
            .iter()
            .map(|row| {
                row.iter().fold(0usize, |n, v| {
                    n.saturating_add(v.as_ref().map_or(0, String::len))
                        .saturating_add(8)
                })
            })
            .collect();
        if row_bytes.iter().any(|&n| n > RESULT_BUDGET) {
            return Ok(false);
        }
        let schema = result.schema.clone();
        let original_disk = result.spool.as_ref().map_or(0, |s| s.bytes);
        let mut memory_added = 0usize;
        let mut disk_added = 0usize;
        let mut staged = Vec::new();
        // Leave bounded global headroom for spill indexes instead of filling RAM
        // with payloads and then being unable to index the next disk batch.
        let memory_limit = self.budget.saturating_sub(if result.disk_enabled {
            (self.budget / 8).min(FRAME_BYTES)
        } else {
            0
        });
        let attempt = (|| -> AppResult<bool> {
            let mut first = 0;
            while first < rows.len() {
                let mut last = first;
                let mut estimated = 0usize;
                while last < rows.len() && last - first < BATCH_ROWS {
                    let next = estimated.saturating_add(row_bytes[last]);
                    if last > first && next > storage::NORMAL_BATCH_BYTES {
                        break;
                    }
                    estimated = next;
                    last += 1;
                }
                if global_bytes
                    .saturating_add(memory_added)
                    .saturating_add(INDEX_BYTES)
                    > self.budget
                {
                    return Ok(false);
                }
                let slice = &rows[first..last];
                let arrays: Vec<ArrayRef> = (0..columns.len())
                    .map(|i| {
                        let capacity = slice
                            .iter()
                            .map(|row| row[i].as_ref().map_or(0, String::len))
                            .sum();
                        let mut builder = StringBuilder::with_capacity(slice.len(), capacity);
                        for row in slice {
                            builder.append_option(row[i].as_deref());
                        }
                        Arc::new(builder.finish()) as ArrayRef
                    })
                    .collect();
                let options =
                    arrow_array::RecordBatchOptions::new().with_row_count(Some(slice.len()));
                let batch = RecordBatch::try_new_with_options(schema.clone(), arrays, &options)
                    .map_err(|_| AppError::internal("Could not encode result batch"))?;
                let retained = batch.get_array_memory_size().saturating_add(
                    batch.num_columns()
                        * (std::mem::size_of::<ArrayRef>() + 2 * std::mem::size_of::<usize>()),
                );
                let data = if global_bytes
                    .saturating_add(memory_added)
                    .saturating_add(retained)
                    .saturating_add(INDEX_BYTES)
                    <= memory_limit
                {
                    memory_added += retained + INDEX_BYTES;
                    BatchData::Memory(batch)
                } else {
                    if !result.disk_enabled {
                        return Ok(false);
                    }
                    let encoded = storage::encode(&batch)?;
                    if global_disk
                        .saturating_add(disk_added)
                        .saturating_add(encoded.len())
                        > self.disk_budget
                    {
                        return Ok(false);
                    }
                    let spool = match &mut result.spool {
                        Some(spool) => spool,
                        None => result.spool.insert(Spool::new()?),
                    };
                    let offset = spool.append(&encoded)?;
                    disk_added += encoded.len();
                    memory_added += INDEX_BYTES;
                    BatchData::Disk {
                        offset,
                        bytes: encoded.len(),
                    }
                };
                staged.push(BatchEntry {
                    start: result.info.retained_rows + first,
                    rows: slice.len(),
                    data,
                });
                first = last;
            }
            Ok(true)
        })();
        if !matches!(attempt, Ok(true)) {
            if let Some(spool) = &mut result.spool
                && let Err(error) = spool.rollback(original_disk)
            {
                // A filesystem failure may prevent tail removal. Account for
                // those actual bytes until release and disallow further writes.
                let tail = spool.bytes.saturating_sub(original_disk) as usize;
                *global_disk += tail;
                result.disk_bytes += tail;
                return Err(error);
            }
            return attempt;
        }
        *global_bytes += memory_added;
        *global_disk += disk_added;
        result.bytes += memory_added;
        result.disk_bytes += disk_added;
        result.info.retained_rows += rows.len();
        result.batches.extend(staged);
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
        let mut state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        let next = identity.as_ref().map_or(0, identity_charge);
        let total = state
            .bytes
            .saturating_sub(result.identity_bytes)
            .saturating_add(next);
        if total > self.budget {
            return Err(metadata_limit());
        }
        state.bytes = total;
        let result = state.results.get_mut(id).ok_or_else(not_found)?;
        result.bytes = result
            .bytes
            .saturating_sub(result.identity_bytes)
            .saturating_add(next);
        result.identity_bytes = next;
        result.edit_identity = identity;
        Ok(())
    }

    pub fn row(&self, id: &str, index: usize) -> AppResult<TextRow> {
        let state = self.state.lock();
        let result = state.results.get(id).ok_or_else(not_found)?;
        if let Some(entry) = find_batch(result, index) {
            let batch = storage::load(result, entry)?;
            return text_row(&batch, index - entry.start);
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
        let mut estimated = 4096usize
            + result
                .info
                .columns
                .iter()
                .map(|c| 128 + c.id.len())
                .sum::<usize>();
        if estimated > FRAME_BYTES {
            return Err(frame_limit_error());
        }
        let first = result
            .batches
            .partition_point(|b| b.start + b.rows <= start);
        for entry in result.batches[first..].iter().take_while(|b| b.start < end) {
            let batch = storage::load(result, entry)?;
            let from = start.saturating_sub(entry.start);
            let length = end.min(entry.start + entry.rows) - entry.start - from;
            estimated = estimated.saturating_add(batch.num_columns() * (128 + (length + 1) * 5));
            for column in batch.columns() {
                let values = column
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| AppError::internal("Invalid result column"))?;
                estimated = estimated.saturating_add(
                    (values.value_offsets()[from + length] - values.value_offsets()[from]) as usize,
                );
            }
            if estimated > FRAME_BYTES {
                return Err(frame_limit_error());
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
            for entry in result.batches[first..].iter().take_while(|b| b.start < end) {
                let batch = storage::load(result, entry)?;
                let from = start.saturating_sub(entry.start);
                let length = end.min(entry.start + entry.rows) - entry.start - from;
                let slice = batch.slice(from, length);
                let slice = RecordBatch::try_new(schema.clone(), slice.columns().to_vec())
                    .map_err(|_| AppError::internal("Invalid result slice"))?;
                writer.write(&slice).map_err(frame_limit)?;
            }
            writer.finish().map_err(frame_limit)?;
        }
        Ok(output.0)
    }

    pub fn release(&self, id: &str) {
        let mut state = self.state.lock();
        if let Some(result) = state.results.remove(id) {
            state.bytes = state.bytes.saturating_sub(result.bytes);
            state.disk_bytes = state.disk_bytes.saturating_sub(result.disk_bytes);
        }
        // Refunding entry reservations must also release the map's old buckets.
        state.results.shrink_to_fit();
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
        let Some(entry) = result.batches.get(batch_index) else {
            return Ok(None);
        };
        if column_indices
            .iter()
            .any(|&i| i >= result.info.columns.len())
        {
            return Err(AppError::invalid("Invalid result column index"));
        }
        let batch = storage::load(result, entry)?;
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

    pub fn retained_ids(&self) -> Vec<String> {
        self.state.lock().results.keys().cloned().collect()
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

fn metadata_limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Result metadata exceeds the retained memory budget",
    )
}
fn strings_charge(values: &Vec<String>) -> usize {
    values.iter().fold(
        values
            .capacity()
            .saturating_mul(std::mem::size_of::<String>()),
        |n, v| n.saturating_add(v.capacity()),
    )
}
fn table_charge(table: &TableRef) -> usize {
    table
        .schema
        .capacity()
        .saturating_add(table.name.capacity())
}
fn columns_charge(columns: &Vec<ColumnInfo>) -> usize {
    columns.iter().fold(
        columns
            .capacity()
            .saturating_mul(std::mem::size_of::<ColumnInfo>()),
        |n, c| {
            n.saturating_add(c.id.capacity())
                .saturating_add(c.name.capacity())
                .saturating_add(c.database_type.capacity())
        },
    )
}
fn schema_charge(schema: &TableSchema) -> usize {
    let mut size = table_charge(&schema.table)
        .saturating_add(columns_charge(&schema.columns))
        .saturating_add(strings_charge(&schema.primary_key));
    size = size.saturating_add(
        schema
            .foreign_keys
            .capacity()
            .saturating_mul(std::mem::size_of::<ForeignKey>()),
    );
    for key in &schema.foreign_keys {
        size = size
            .saturating_add(key.name.capacity())
            .saturating_add(table_charge(&key.target))
            .saturating_add(strings_charge(&key.columns))
            .saturating_add(strings_charge(&key.target_columns));
    }
    size = size.saturating_add(
        schema
            .inbound_foreign_keys
            .capacity()
            .saturating_mul(std::mem::size_of::<InboundForeignKey>()),
    );
    for key in &schema.inbound_foreign_keys {
        size = size
            .saturating_add(key.name.capacity())
            .saturating_add(table_charge(&key.source))
            .saturating_add(strings_charge(&key.columns))
            .saturating_add(strings_charge(&key.target_columns));
    }
    size
}
fn metadata_charge(info: &ResultInfo, provenance: Option<&TableSchema>) -> usize {
    // The first entry can allocate four hash buckets, and growth/shrink may
    // leave spare capacity. Charge that conservative per-entry upper bound,
    // including owned map keys/control bytes and allocator bookkeeping.
    let mut size =
        4 * (std::mem::size_of::<StoredResult>() + std::mem::size_of::<String>() + 1) + 512;
    size = size
        .saturating_add(info.id.capacity() * 2)
        .saturating_add(info.run_id.capacity())
        .saturating_add(columns_charge(&info.columns));
    size = size
        .saturating_add(info.table.as_ref().map_or(0, table_charge))
        .saturating_add(provenance.map_or(0, schema_charge));
    size = size.saturating_add(match &info.editability {
        Some(ResultEditability::Editable { table, primary_key }) => {
            table_charge(table).saturating_add(strings_charge(primary_key))
        }
        Some(ResultEditability::ReadOnly { reason }) => reason.capacity(),
        None => 0,
    });
    // One shared Arrow schema per result; batches clone its Arc, not its fields.
    size.saturating_add(std::mem::size_of::<Schema>())
        .saturating_add(info.columns.iter().fold(0usize, |n, c| {
            n.saturating_add(std::mem::size_of::<Field>() + 64)
                .saturating_add(c.id.len())
        }))
}
fn identity_charge(identity: &crate::drivers::EditIdentity) -> usize {
    match identity {
        crate::drivers::EditIdentity::Postgres { attributes, .. } => attributes.iter().fold(
            attributes
                .capacity()
                .saturating_mul(std::mem::size_of::<crate::drivers::edit_target::PgAttribute>()),
            |n, a| n.saturating_add(a.name.capacity()),
        ),
        crate::drivers::EditIdentity::Mysql { guard } => table_charge(&guard.identity().table),
    }
}

fn find_batch(result: &StoredResult, index: usize) -> Option<&BatchEntry> {
    let at = result
        .batches
        .partition_point(|b| b.start + b.rows <= index);
    result
        .batches
        .get(at)
        .filter(|b| index >= b.start && index < b.start + b.rows)
}
fn text_row(batch: &RecordBatch, index: usize) -> AppResult<TextRow> {
    batch
        .columns()
        .iter()
        .map(|array| {
            let array = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| AppError::internal("Invalid result column encoding"))?;
            Ok(if array.is_null(index) {
                None
            } else {
                Some(array.value(index).to_string())
            })
        })
        .collect()
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
            let store = ResultStore::new(8192, 1);
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
                store.usage().0 <= 8192,
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
        let store = ResultStore::new(8192, 1);
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
        let store = ResultStore::new(8192, 2);
        let a = store.create("a", 0, columns(), None).unwrap();
        let before = store.usage().0;
        assert!(
            !store
                .append(&a.id, vec![vec![Some("x".repeat(16_384))]])
                .unwrap()
        );
        assert_eq!(store.usage().0, before);
        assert!(
            store
                .append(&a.id, vec![vec![None], vec![Some("a".into())]])
                .unwrap()
        );
        assert!(!store.append(&a.id, vec![vec![None]]).unwrap());
        assert_eq!(store.info(&a.id).unwrap().retained_rows, 2);
    }

    #[test]
    fn disk_spill_random_ranges_and_export_preserve_exact_values() {
        let store = ResultStore::new(8192, 10_000);
        store.set_temporary_disk_enabled(true);
        let mut cols = columns();
        let mut second = cols[0].clone();
        second.id = "c1".into();
        cols.push(second);
        let info = store.create("disk", 0, cols, None).unwrap();
        let rows: Vec<TextRow> = (0..1701)
            .map(|i| {
                vec![
                    if i % 7 == 0 {
                        None
                    } else {
                        Some(format!("{i}:雪😀\n\"{},{}", "a".repeat(40), i))
                    },
                    Some(if i % 11 == 0 {
                        "".into()
                    } else {
                        "9223372036854775807".into()
                    }),
                ]
            })
            .collect();
        assert!(store.append(&info.id, rows.clone()).unwrap());
        assert!(store.disk_usage() > 0);
        for index in [1700, 0, 512, 511, 1025, 37] {
            assert_eq!(store.row(&info.id, index).unwrap(), rows[index]);
        }
        let encoded = store.range(&info.id, 500, 40).unwrap();
        let mut reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(encoded), None).unwrap();
        let mut actual = Vec::new();
        for batch in &mut reader {
            let batch = batch.unwrap();
            for i in 0..batch.num_rows() {
                actual.push(text_row(&batch, i).unwrap());
            }
        }
        assert_eq!(actual, rows[500..540]);
        let mut exported = Vec::new();
        for index in 0..store.batch_count(&info.id).unwrap() {
            exported.extend(store.batch_rows(&info.id, index, &[0, 1]).unwrap().unwrap());
        }
        assert_eq!(exported, rows);
        assert!(store.batch_rows(&info.id, 0, &[2]).is_err());
        store.release(&info.id);
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(store.disk_usage(), 0);
    }

    #[test]
    fn disk_opt_in_is_captured_at_create_and_existing_spools_survive_toggle() {
        let store = ResultStore::new(16_384, 20);
        assert!(!store.temporary_disk_enabled());
        let before = store.create("before", 0, columns(), None).unwrap();
        store.set_temporary_disk_enabled(true);
        let enabled = store.create("enabled", 0, columns(), None).unwrap();
        store.set_temporary_disk_enabled(false);
        let after = store.create("after", 0, columns(), None).unwrap();
        let rows = vec![vec![Some("x".repeat(9000))]];
        assert!(!store.append(&before.id, rows.clone()).unwrap());
        assert!(!store.append(&after.id, rows.clone()).unwrap());
        assert!(store.append(&enabled.id, rows.clone()).unwrap());
        assert!(store.append(&enabled.id, rows.clone()).unwrap());
        assert_eq!(store.row(&enabled.id, 1).unwrap(), rows[0]);
        assert!(store.disk_usage() > 0);
        store.release(&enabled.id);
        assert_eq!(store.disk_usage(), 0);
    }

    #[test]
    fn disk_quota_is_global_and_partial_append_rolls_back_tail_and_reservations() {
        let mut store = ResultStore::new(8192, 10_000);
        store.set_temporary_disk_enabled(true);
        let a = store.create("a", 0, columns(), None).unwrap();
        let b = store.create("b", 0, columns(), None).unwrap();
        let seed = vec![vec![Some("seed".repeat(3000))]];
        assert!(store.append(&a.id, seed.clone()).unwrap());
        let used = store.disk_usage();
        store.disk_budget = used;
        assert!(!store.append(&b.id, seed.clone()).unwrap());
        assert_eq!(store.disk_usage(), used);
        store.disk_budget = used + 60_000;
        let ram = store.usage().0;
        assert!(
            !store
                .append(
                    &a.id,
                    (0..600).map(|_| vec![Some("x".repeat(100))]).collect()
                )
                .unwrap()
        );
        assert_eq!(store.info(&a.id).unwrap().retained_rows, 1);
        assert_eq!(store.row(&a.id, 0).unwrap(), seed[0]);
        assert_eq!(store.disk_usage(), used);
        assert_eq!(store.usage().0, ram);
        {
            let state = store.state.lock();
            assert_eq!(
                state.results[&a.id]
                    .spool
                    .as_ref()
                    .unwrap()
                    .file
                    .metadata()
                    .unwrap()
                    .len(),
                used as u64
            );
        }
        store.release(&a.id);
        assert_eq!(store.disk_usage(), 0);
        assert!(store.append(&b.id, seed).unwrap());
    }

    #[test]
    fn disk_read_and_write_errors_are_sanitized_and_keep_existing_prefix() {
        let store = ResultStore::new(8192, 100);
        store.set_temporary_disk_enabled(true);
        let a = store.create("a", 0, columns(), None).unwrap();
        assert!(
            store
                .append(&a.id, vec![vec![Some("prefix".into())]])
                .unwrap()
        );
        let ram = store.usage().0;
        let readonly = tempfile::NamedTempFile::new().unwrap();
        let readonly_path = readonly.path().to_string_lossy().into_owned();
        let readonly_name = readonly
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        {
            let mut state = store.state.lock();
            state.results.get_mut(&a.id).unwrap().spool = Some({
                let mut spool = Spool::new().unwrap();
                spool.file = std::fs::File::open(readonly.path()).unwrap();
                spool
            });
        }
        let error = store
            .append(&a.id, vec![vec![Some("x".repeat(9000))]])
            .unwrap_err();
        assert!(!error.message.contains(&readonly_path));
        assert!(!error.message.contains(&readonly_name));
        assert_eq!(store.disk_usage(), 0);
        assert_eq!(store.usage().0, ram);
        assert_eq!(store.row(&a.id, 0).unwrap(), vec![Some("prefix".into())]);
        store.release(&a.id);
        let b = store.create("b", 0, columns(), None).unwrap();
        assert!(
            store
                .append(&b.id, vec![vec![Some("x".repeat(9000))]])
                .unwrap()
        );
        {
            let state = store.state.lock();
            state.results[&b.id]
                .spool
                .as_ref()
                .unwrap()
                .file
                .set_len(0)
                .unwrap();
        }
        assert!(store.row(&b.id, 0).is_err());
        assert!(store.range(&b.id, 0, 1).is_err());
        assert!(store.batch_rows(&b.id, 0, &[0]).is_err());
        store.release(&b.id);
        assert_eq!(store.disk_usage(), 0);
        assert_eq!(store.usage(), (0, 0));
    }

    #[cfg(unix)]
    #[test]
    fn anonymous_spool_is_private_unlinked_and_closed_on_drop() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let store = ResultStore::new(8192, 10);
        store.set_temporary_disk_enabled(true);
        let result = store.create("private", 0, columns(), None).unwrap();
        assert!(
            store
                .append(&result.id, vec![vec![Some("x".repeat(9000))]])
                .unwrap()
        );
        let (fd, ino, dev) = {
            let state = store.state.lock();
            let file = &state.results[&result.id].spool.as_ref().unwrap().file;
            let meta = file.metadata().unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            assert_eq!(meta.nlink(), 0);
            (file.as_raw_fd(), meta.ino(), meta.dev())
        };
        drop(store);
        if let Ok(meta) = std::fs::metadata(format!("/dev/fd/{fd}")) {
            assert_ne!((meta.dev(), meta.ino()), (dev, ino));
        }
    }

    #[cfg(unix)]
    #[test]
    fn restrict_spool_permissions_clamps_permissive_o_tmpfile_modes_to_0600() {
        use std::os::unix::fs::PermissionsExt;
        let file = tempfile::tempfile().unwrap();
        file.set_permissions(std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o644);
        storage::restrict_spool_permissions(&file).unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn anonymous_spool_spills_reads_and_releases_cleanly() {
        let store = ResultStore::new(8192, 10);
        store.set_temporary_disk_enabled(true);
        let result = store.create("private", 0, columns(), None).unwrap();
        let payload = "x".repeat(9000);
        assert!(
            store
                .append(&result.id, vec![vec![Some(payload.clone())]])
                .unwrap()
        );
        store.finish(&result.id, false).unwrap();
        assert!(store.disk_usage() > 0);
        assert_eq!(
            store.row(&result.id, 0).unwrap(),
            vec![Some(payload.clone())]
        );
        let frame = store.range(&result.id, 0, 1).unwrap();
        let mut reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(frame), None).unwrap();
        let batch = reader.next().unwrap().unwrap();
        assert!(reader.next().is_none());
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values.value(0), payload);
        store.release(&result.id);
        assert_eq!(store.disk_usage(), 0);
    }
    #[test]
    fn large_single_rows_remain_retainable_and_frame_limits_are_separate() {
        let store = ResultStore::new(8192, 10);
        store.set_temporary_disk_enabled(true);
        let info = store.create("large", 0, columns(), None).unwrap();
        let value = "雪".repeat(3_000_000);
        assert!(
            store
                .append(&info.id, vec![vec![Some(value.clone())]])
                .unwrap()
        );
        assert_eq!(store.row(&info.id, 0).unwrap(), vec![Some(value)]);
        assert_eq!(
            store.range(&info.id, 0, 1).unwrap_err().code,
            ErrorCode::ResultLimitReached
        );
        let before = store.disk_usage();
        assert!(
            !store
                .append(&info.id, vec![vec![Some("x".repeat(RESULT_BUDGET + 1))]])
                .unwrap()
        );
        assert_eq!(store.disk_usage(), before);
        assert_eq!(store.info(&info.id).unwrap().retained_rows, 1);
    }

    #[test]
    fn spill_preserves_exact_millionth_row_without_retained_ram_growth() {
        let store = ResultStore::new(8 * 1024 * 1024, ROW_LIMIT);
        store.set_temporary_disk_enabled(true);
        let result = store.create("million", 0, columns(), None).unwrap();
        for first in (0..ROW_LIMIT).step_by(BATCH_ROWS) {
            let last = (first + BATCH_ROWS).min(ROW_LIMIT);
            assert!(
                store
                    .append(
                        &result.id,
                        (first..last)
                            .map(|i| vec![Some(format!("{i:07}:{}", "x".repeat(32)))])
                            .collect()
                    )
                    .unwrap()
            );
            assert!(store.usage().0 <= 8 * 1024 * 1024);
        }
        assert_eq!(store.info(&result.id).unwrap().retained_rows, 1_000_000);
        assert_eq!(
            store.row(&result.id, 999_999).unwrap(),
            vec![Some(format!("0999999:{}", "x".repeat(32)))]
        );
        assert!(store.disk_usage() > 0);
        assert!(!store.append(&result.id, vec![vec![None]]).unwrap());
        let frame = store.range(&result.id, 999_990, 512).unwrap();
        let mut reader =
            arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(frame), None).unwrap();
        assert_eq!(reader.next().unwrap().unwrap().num_rows(), 10);
        store.release(&result.id);
        assert_eq!(store.disk_usage(), 0);
        assert_eq!(store.usage(), (0, 0));
    }
    #[test]
    fn empty_results_and_all_owned_metadata_share_ram_quota() {
        let store = ResultStore::new(12_000, 100);
        let mut ids = Vec::new();
        loop {
            match store.create("metadata", 0, columns(), None) {
                Ok(info) => ids.push(info.id),
                Err(error) => {
                    assert_eq!(error.code, ErrorCode::ResultLimitReached);
                    break;
                }
            }
        }
        assert!(!ids.is_empty());
        assert!(ids.len() < 100);
        assert!(store.usage().0 <= 12_000);
        let released = ids.pop().unwrap();
        store.release(&released);
        ids.push(store.create("replacement", 0, columns(), None).unwrap().id);
        for field in ["id", "name", "type", "run"] {
            let empty = ResultStore::new(12_000, 100);
            let mut cols = columns();
            let giant = "x".repeat(12_001);
            match field {
                "id" => cols[0].id = giant.clone(),
                "name" => cols[0].name = giant.clone(),
                "type" => cols[0].database_type = giant.clone(),
                _ => {}
            }
            let before = empty.usage();
            assert!(
                empty
                    .create(if field == "run" { &giant } else { "r" }, 0, cols, None)
                    .is_err()
            );
            assert_eq!(empty.usage(), before);
        }
        for id in ids {
            store.release(&id);
        }
        assert_eq!(store.usage(), (0, 0));
        assert_eq!(store.state.lock().results.capacity(), 0);
        assert!(store.create("new admission", 0, columns(), None).is_ok());
    }
    #[test]
    fn unknown_failed_tail_remains_reserved_and_original_prefix_is_usable() {
        let mut store = ResultStore::new(8192, 20);
        store.set_temporary_disk_enabled(true);
        let a = store.create("a", 0, columns(), None).unwrap();
        let row = vec![Some("x".repeat(12_000))];
        assert!(store.append(&a.id, vec![row.clone()]).unwrap());
        let original = store.disk_usage();
        {
            let mut state = store.state.lock();
            state
                .results
                .get_mut(&a.id)
                .unwrap()
                .spool
                .as_mut()
                .unwrap()
                .inject_partial_write_failure = true;
        }
        assert!(store.append(&a.id, vec![row.clone()]).is_err());
        assert_eq!(store.info(&a.id).unwrap().retained_rows, 1);
        assert_eq!(store.row(&a.id, 0).unwrap(), row);
        assert!(store.disk_usage() > original);
        {
            let state = store.state.lock();
            let spool = state.results[&a.id].spool.as_ref().unwrap();
            assert!(spool.failed);
            assert_eq!(spool.file.metadata().unwrap().len(), original as u64 + 16);
            assert!(spool.bytes > spool.file.metadata().unwrap().len());
        }
        store.disk_budget = store.disk_usage();
        let b = store.create("b", 0, columns(), None).unwrap();
        assert!(!store.append(&b.id, vec![row.clone()]).unwrap());
        store.release(&a.id);
        assert_eq!(store.disk_usage(), 0);
        assert!(store.append(&b.id, vec![row]).unwrap());
        store.release(&b.id);
        assert_eq!(store.disk_usage(), 0);
        assert_eq!(store.usage(), (0, 0));
    }
}
