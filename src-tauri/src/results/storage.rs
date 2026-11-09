//! Anonymous, process-local result backing. Files have no recoverable workspace path.
use super::*;
use arrow_ipc::reader::StreamReader;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

pub(super) const INDEX_BYTES: usize = 2 * std::mem::size_of::<BatchEntry>();
pub(super) const NORMAL_BATCH_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_STORED_BYTES: usize = RESULT_BUDGET + FRAME_BYTES;

pub(super) struct BatchEntry {
    pub start: usize,
    pub rows: usize,
    pub data: BatchData,
}
pub(super) enum BatchData {
    Memory(RecordBatch),
    Disk { offset: u64, bytes: usize },
}
pub(super) struct Spool {
    pub file: File,
    pub bytes: u64,
    pub failed: bool,
    #[cfg(test)]
    pub inject_partial_write_failure: bool,
}
impl Spool {
    pub fn new() -> AppResult<Self> {
        let file = tempfile::tempfile().map_err(|_| io_error())?;
        Ok(Self {
            file,
            bytes: 0,
            failed: false,
            #[cfg(test)]
            inject_partial_write_failure: false,
        })
    }
    pub fn append(&mut self, bytes: &[u8]) -> AppResult<u64> {
        if self.failed {
            return Err(io_error());
        }
        let offset = self.bytes;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|_| io_error())?;
        // Reserve the entire possible tail before a write that may fail part-way.
        // If both truncation and stat later fail, quota still covers all bytes.
        self.bytes = offset + bytes.len() as u64;
        #[cfg(test)]
        if self.inject_partial_write_failure {
            // A real partial file write, followed by simulated filesystem faults
            // for the remaining write, truncate and stat paths.
            self.file
                .write_all(&bytes[..bytes.len().min(16)])
                .map_err(|_| io_error())?;
            return Err(io_error());
        }
        self.file.write_all(bytes).map_err(|_| io_error())?;
        Ok(offset)
    }
    pub fn rollback(&mut self, offset: u64) -> AppResult<()> {
        #[cfg(test)]
        if self.inject_partial_write_failure {
            self.failed = true;
            return Err(io_error());
        }
        if self.file.set_len(offset).is_err() {
            self.failed = true;
            // Keep physical tail accounted until release, never let it exceed reserved quota.
            self.bytes = self.file.metadata().map_err(|_| io_error())?.len();
            return Err(io_error());
        }
        self.bytes = offset;
        Ok(())
    }
    pub fn read(&self, offset: u64, bytes: usize) -> AppResult<RecordBatch> {
        if bytes > MAX_STORED_BYTES || offset.saturating_add(bytes as u64) > self.bytes {
            return Err(io_error());
        }
        let mut file = self.file.try_clone().map_err(|_| io_error())?;
        file.seek(SeekFrom::Start(offset)).map_err(|_| io_error())?;
        let mut encoded = vec![0; bytes];
        file.read_exact(&mut encoded).map_err(|_| io_error())?;
        validate_stream(&encoded)?;
        let mut reader =
            StreamReader::try_new(Cursor::new(encoded), None).map_err(|_| io_error())?;
        let batch = reader
            .next()
            .ok_or_else(io_error)?
            .map_err(|_| io_error())?;
        if reader.next().is_some() {
            return Err(io_error());
        }
        Ok(batch)
    }
}
pub(super) fn load(result: &StoredResult, entry: &BatchEntry) -> AppResult<RecordBatch> {
    let batch = match &entry.data {
        BatchData::Memory(batch) => batch.clone(),
        BatchData::Disk { offset, bytes } => result
            .spool
            .as_ref()
            .ok_or_else(io_error)?
            .read(*offset, *bytes)?,
    };
    if batch.num_rows() != entry.rows
        || batch.num_columns() != result.info.columns.len()
        || batch
            .columns()
            .iter()
            .any(|a| a.data_type() != &DataType::Utf8)
    {
        return Err(io_error());
    }
    Ok(batch)
}
struct BoundedBatch(Vec<u8>);
impl Write for BoundedBatch {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_STORED_BYTES {
            return Err(std::io::Error::other("result batch limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(batch: &RecordBatch) -> AppResult<Vec<u8>> {
    let mut output = BoundedBatch(Vec::new());
    {
        let mut writer =
            StreamWriter::try_new(&mut output, batch.schema().as_ref()).map_err(|_| io_error())?;
        writer.write(batch).map_err(|_| io_error())?;
        writer.finish().map_err(|_| io_error())?;
    }
    Ok(output.0)
}
// Arrow's reader allocates a body from its declared length. Validate every frame
// against the bounded encoded slice before permitting that allocation.
fn validate_stream(bytes: &[u8]) -> AppResult<()> {
    let mut offset = 0usize;
    let mut messages = 0;
    loop {
        let prefix = bytes.get(offset..offset + 8).ok_or_else(io_error)?;
        if prefix[..4] != [255; 4] {
            return Err(io_error());
        }
        let length = u32::from_le_bytes(prefix[4..8].try_into().map_err(|_| io_error())?) as usize;
        offset += 8;
        if length == 0 {
            return if offset == bytes.len() && messages == 2 {
                Ok(())
            } else {
                Err(io_error())
            };
        }
        let end = offset.checked_add(length).ok_or_else(io_error)?;
        let message = arrow_ipc::root_as_message(bytes.get(offset..end).ok_or_else(io_error)?)
            .map_err(|_| io_error())?;
        match (messages, message.header_type()) {
            (0, arrow_ipc::MessageHeader::Schema) => {
                let schema = message.header_as_schema().ok_or_else(io_error)?;
                let fields = schema.fields().ok_or_else(io_error)?;
                if fields.len() > 1000
                    || fields.iter().any(|field| {
                        field.type_type() != arrow_ipc::Type::Utf8
                            || field.dictionary().is_some()
                            || field
                                .children()
                                .is_some_and(|children| !children.is_empty())
                    })
                {
                    return Err(io_error());
                }
            }
            (1, arrow_ipc::MessageHeader::RecordBatch) => {
                let batch = message.header_as_record_batch().ok_or_else(io_error)?;
                if batch.length() < 0
                    || batch.length() > BATCH_ROWS as i64
                    || batch.compression().is_some()
                    || batch.nodes().is_none_or(|nodes| nodes.len() > 1000)
                    || batch.buffers().is_none_or(|buffers| buffers.len() > 3000)
                {
                    return Err(io_error());
                }
            }
            _ => return Err(io_error()),
        }
        let body = usize::try_from(message.bodyLength()).map_err(|_| io_error())?;
        offset = end.checked_add(body).ok_or_else(io_error)?;
        if offset > bytes.len() || body > MAX_STORED_BYTES {
            return Err(io_error());
        }
        messages += 1;
        if messages > 2 {
            return Err(io_error());
        }
    }
}
pub(super) fn io_error() -> AppError {
    AppError::internal(
        "Temporary result storage could not be accessed. Re-run the query to refresh the result.",
    )
}
