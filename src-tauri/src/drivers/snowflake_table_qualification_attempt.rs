//! Persistent, blocking output ownership for one fixed qualification attempt.
//! Reservation precedes permanent consumption. No failure or Drop removes,
//! repairs, reopens or resumes any output; partial reservations remain spent.
//! Physical-path/descriptor checks do not defeat hostile same-user races or
//! storage that falsely reports successful fsync.

use super::{
    Plan,
    consumption::{AttemptDirectory, DirectoryProof},
    journal::Journal,
};
use crate::domain::{AppError, AppResult};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

const FORMAT: &str = "joinery-snowflake-table-qualification-terminal-v1";
const MAX_TERMINAL_BYTES: usize = 1024 * 1024;
const MAX_RESULT_DEPTH: usize = 64;
const MAX_RESULT_NODES: usize = 65_536;

fn invalid() -> AppError {
    AppError::invalid("Private qualification attempt input or limit is invalid")
}
fn unavailable() -> AppError {
    AppError::internal("Private qualification attempt storage failed or was already used")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TerminalReceipt {
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Terminal<'a> {
    format: &'static str,
    plan_sha256: &'a str,
    execution_sha256: &'a str,
    run: &'a str,
    result: &'a Value,
}

/// Held directories and files, without deletion authority or a reopen route.
pub(super) struct AttemptStorage {
    parent: DirectoryProof,
    directory: DirectoryProof,
    workspace: DirectoryProof,
    state: DirectoryProof,
    terminal: File,
    terminal_bytes: Vec<u8>,
    terminal_attempted: bool,
    terminal_failed: bool,
    journal: Option<Journal>,
    journal_attempted: bool,
    plan_sha256: String,
    execution_sha256: String,
    run: String,
    #[cfg(test)]
    fault: Option<Fault>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum Fault {
    PartialWrite,
    FileSync,
    DirectorySync,
}

impl AttemptStorage {
    /// The parent must already exist at a strict absolute physical 0700 path.
    /// No marker is consumed here. Failure leaves every created name in place.
    pub(super) fn reserve(
        output_parent: &Path,
        plan: &Plan,
        execution_sha256: &str,
    ) -> AppResult<Self> {
        if execution_sha256.len() != 64
            || !execution_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        let plan_sha256 = plan.sha256()?;
        let parent = DirectoryProof::open(output_parent)?;
        let directory = parent.reserve_attempt_directory(AttemptDirectory::Run(plan.run()))?;
        let workspace = directory.reserve_attempt_directory(AttemptDirectory::Workspace)?;
        let state = directory.reserve_attempt_directory(AttemptDirectory::HelperState)?;
        let terminal = directory.reserve_terminal()?;
        let journal = Journal::create(directory.path(), &plan_sha256)?;
        let storage = Self {
            parent,
            directory,
            workspace,
            state,
            terminal,
            terminal_bytes: Vec::new(),
            terminal_attempted: false,
            terminal_failed: false,
            journal: Some(journal),
            journal_attempted: false,
            plan_sha256,
            execution_sha256: execution_sha256.into(),
            run: plan.run().into(),
            #[cfg(test)]
            fault: None,
        };
        storage.validate()?;
        Ok(storage)
    }

    pub(super) fn path(&self) -> &Path {
        self.directory.path()
    }

    pub(super) fn workspace(&self) -> &Path {
        self.workspace.path()
    }

    pub(super) fn state(&self) -> &Path {
        self.state.path()
    }

    /// Journal/Policy owns its file checks after take_journal transfers it.
    /// This owner continues checking the full physical directory paths and the
    /// exact reserved/persisted terminal file, including after journal transfer.
    pub(super) fn validate(&self) -> AppResult<()> {
        if self.terminal_failed {
            return Err(unavailable());
        }
        self.parent.validate_directory()?;
        self.directory.validate_directory()?;
        self.workspace.validate_directory()?;
        self.state.validate_directory()?;
        self.directory
            .validate_terminal(&self.terminal, &self.terminal_bytes)?;
        if let Some(journal) = &self.journal {
            journal.validate_reserved(&self.plan_sha256)?;
        }
        Ok(())
    }

    pub(super) fn take_journal(&mut self) -> AppResult<Journal> {
        if self.journal_attempted {
            return Err(unavailable());
        }
        self.journal_attempted = true;
        self.validate()?;
        self.journal.take().ok_or_else(unavailable)
    }

    /// Exactly one attempt, including encoding or persistence failure. No
    /// retry, truncate, replacement or release path exists after this call.
    pub(super) fn persist_terminal(&mut self, value: &Value) -> AppResult<TerminalReceipt> {
        if self.terminal_attempted {
            return Err(unavailable());
        }
        self.terminal_attempted = true;
        let result = self.write_terminal(value);
        if result.is_err() {
            self.terminal_failed = true;
        }
        result
    }

    fn write_terminal(&mut self, value: &Value) -> AppResult<TerminalReceipt> {
        self.validate()?;
        validate_result(value)?;
        let envelope = Terminal {
            format: FORMAT,
            plan_sha256: &self.plan_sha256,
            execution_sha256: &self.execution_sha256,
            run: &self.run,
            result: value,
        };
        let mut output = BoundedBytes(Vec::new());
        serde_json::to_writer(&mut output, &envelope).map_err(|_| invalid())?;
        output.write_all(b"\n").map_err(|_| invalid())?;
        let bytes = output.0;
        #[cfg(test)]
        if let Some(fault) = self.fault.take() {
            match fault {
                Fault::PartialWrite => self.terminal.write_all(&bytes[..8]),
                Fault::FileSync | Fault::DirectorySync => self.terminal.write_all(&bytes),
            }
            .map_err(|_| unavailable())?;
            if matches!(fault, Fault::DirectorySync) {
                self.terminal.sync_all().map_err(|_| unavailable())?;
            }
            return Err(unavailable());
        }
        self.terminal.write_all(&bytes).map_err(|_| unavailable())?;
        self.terminal.sync_all().map_err(|_| unavailable())?;
        self.directory.sync_directory()?;
        self.directory.validate_terminal(&self.terminal, &bytes)?;
        self.terminal_bytes = bytes;
        self.validate()?;
        Ok(TerminalReceipt {
            sha256: format!("{:x}", Sha256::digest(&self.terminal_bytes)),
            bytes: self.terminal_bytes.len() as u64,
        })
    }
}

struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_TERMINAL_BYTES {
            return Err(io::Error::other(
                "qualification terminal exceeds byte limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn validate_result(value: &Value) -> AppResult<()> {
    let mut pending = vec![(value, 0)];
    let mut nodes = 0;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        if depth > MAX_RESULT_DEPTH || nodes > MAX_RESULT_NODES {
            return Err(invalid());
        }
        match value {
            Value::Array(values) => {
                if values.len() > MAX_RESULT_NODES.saturating_sub(nodes + pending.len()) {
                    return Err(invalid());
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values.len() > MAX_RESULT_NODES.saturating_sub(nodes + pending.len()) {
                    return Err(invalid());
                }
                pending.extend(values.values().map(|value| (value, depth + 1)));
            }
            _ => {}
        }
        if pending.len() > MAX_RESULT_NODES {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "snowflake_table_qualification_attempt_tests.rs"]
mod tests;
