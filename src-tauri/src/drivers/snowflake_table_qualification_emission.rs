//! Offline bounded Plan output. Success proves exact local bytes and fsyncs,
//! never account authority. Failure preserves partial output without adoption,
//! overwrite or cleanup. Same-user hostile rewriting and dishonest storage are
//! outside these filesystem identity checks.

use super::{
    MAX_PLAN_SNAPSHOT_BYTES, Plan,
    consumption::{DirectoryProof, EmissionFile},
};
use crate::domain::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_ENVELOPE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct EmissionReceipt {
    /// Exclusive plan-RUN directory containing both fixed output filenames.
    pub(super) path: PathBuf,
    pub(super) envelope_sha256: String,
    pub(super) envelope_bytes: u64,
}

pub(super) struct Emission;

#[cfg(test)]
type Hook<'a> = Box<dyn FnOnce(&DirectoryProof) -> AppResult<()> + 'a>;

impl Emission {
    /// Caller validates the exact envelope and its separation from output before
    /// invoking this offline store. Envelope bytes are opaque and never parsed.
    pub(super) fn persist(
        output_parent: &Path,
        plan: &Plan,
        envelope_bytes: &[u8],
    ) -> AppResult<EmissionReceipt> {
        Self::persist_inner(
            output_parent,
            plan,
            envelope_bytes,
            #[cfg(test)]
            None,
        )
    }

    fn persist_inner(
        output_parent: &Path,
        plan: &Plan,
        envelope_bytes: &[u8],
        #[cfg(test)] hook: Option<Hook<'_>>,
    ) -> AppResult<EmissionReceipt> {
        if envelope_bytes.is_empty() || envelope_bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(invalid());
        }
        let plan_bytes = plan.canonical_json()?;
        if plan_bytes.is_empty() || plan_bytes.len() > MAX_PLAN_SNAPSHOT_BYTES {
            return Err(invalid());
        }
        let plan_sha256 = hash(&plan_bytes);
        // Recompile the closed plan, rejecting malformed RUNs and corrupted
        // in-memory definitions before any filesystem mutation.
        Plan::restore_snapshot(&plan_bytes, &plan_sha256)?;
        let envelope_sha256 = hash(envelope_bytes);
        let envelope_len = u64::try_from(envelope_bytes.len()).map_err(|_| invalid())?;

        let parent = DirectoryProof::open(output_parent)?;
        let output = parent.reserve_emission_directory(plan.run())?;
        let envelope = output.write_emission_file(EmissionFile::Envelope, envelope_bytes)?;
        #[cfg(test)]
        if let Some(hook) = hook {
            hook(&output)?;
        }
        let snapshot = output.write_emission_file(EmissionFile::Plan, &plan_bytes)?;
        output.sync_directory()?;
        parent.sync_directory()?;
        // Read both named files again after all writes and directory syncs.
        output.validate_emission_file(EmissionFile::Envelope, &envelope, envelope_bytes)?;
        output.validate_emission_file(EmissionFile::Plan, &snapshot, &plan_bytes)?;
        parent.validate_directory()?;
        output.validate_directory()?;
        Ok(EmissionReceipt {
            path: output.path().to_path_buf(),
            envelope_sha256,
            envelope_bytes: envelope_len,
        })
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn invalid() -> AppError {
    AppError::invalid("Offline table qualification Plan output is invalid")
}

#[cfg(test)]
#[path = "snowflake_table_qualification_emission_tests.rs"]
mod tests;
