//! Native preview originals and linear preparation, without dispatch authority.
//!
//! Callers must share one store globally, bracket observations against drift,
//! own the session lane, and revalidate the physical helper before dispatch.
//! Names and DDL timestamps do not establish immutable object identity. Manual
//! continuation freshness remains unsupported by the ordinary metadata lane.
//! Limits below are provisional logical bounds, not RSS or workflow acceptance.
use super::snowflake_browsing::BrowseRelation;
use super::snowflake_edit_codec::InspectedMutationTarget;
use super::snowflake_metadata::{CONTEXT_MAX_BYTES, LiveContext};
use super::snowflake_mutations::{
    CellValue, CompiledBatch, MAX_BATCH_BYTES, MAX_CELL_BYTES, MAX_COLUMNS, MAX_ROWS, Mutation,
};
use super::snowflake_process::HelperGeneration;
use crate::domain::{AppError, AppResult, ErrorCode, TextRow};
use crate::relation_inspection::ColumnInspection;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
const MAX_BINDING_BYTES: usize = 2 * 1024 * 1024;
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);
const PLAN_TTL: Duration = Duration::from_secs(60);

fn stale() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake edit snapshot is stale, unavailable or already used; refresh the preview",
    )
}
fn invalid() -> AppError {
    AppError::invalid("Snowflake edit snapshot inputs are invalid")
}
fn limit() -> AppError {
    AppError::new(
        ErrorCode::ResultLimitReached,
        "Native Snowflake edit preparation capacity is full; release previews or plans before retrying",
    )
}
fn canonical_id(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
struct Counter {
    bytes: usize,
    maximum: usize,
}
impl Write for Counter {
    fn write(&mut self, value: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(value.len())
            .filter(|n| *n <= self.maximum)
            .ok_or_else(|| io::Error::other("bounded metadata limit"))?;
        Ok(value.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encoded_charge(value: &impl Serialize, maximum: usize) -> AppResult<usize> {
    let mut counter = Counter { bytes: 0, maximum };
    serde_json::to_writer(&mut counter, value).map_err(|_| limit())?;
    Ok(counter.bytes)
}
fn add(total: &mut usize, amount: usize, maximum: usize) -> AppResult<()> {
    *total = total
        .checked_add(amount)
        .filter(|n| *n <= maximum)
        .ok_or_else(limit)?;
    Ok(())
}

/// Construct only from native retained observations after the driver's checks.
/// Shape validation here neither performs remote reads nor proves freshness.
pub(crate) struct NativeEditBinding {
    session_id: String,
    helper: HelperGeneration,
    context: LiveContext,
    browse_revision: String,
    target_id: String,
    relation: BrowseRelation,
    inspection_revision: String,
    columns: Vec<ColumnInspection>,
    codec: InspectedMutationTarget,
    charge: usize,
}
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Native binding accessors precede typed batch dispatch integration"
    )
)]
impl NativeEditBinding {
    #[allow(
        clippy::too_many_arguments,
        reason = "All native observation components must be pinned together"
    )]
    pub(crate) fn new(
        session_id: &str,
        helper: &HelperGeneration,
        context: &LiveContext,
        browse_revision: &str,
        target_id: &str,
        relation: &BrowseRelation,
        inspection_revision: &str,
        columns: &[ColumnInspection],
    ) -> AppResult<Self> {
        if session_id.is_empty()
            || session_id.len() > 1024
            || session_id.contains('\0')
            || ![browse_revision, target_id, inspection_revision]
                .into_iter()
                .all(canonical_id)
            || columns.is_empty()
            || columns.len() > MAX_COLUMNS
            || context.quoted_identifiers_ignore_case
            || context.database.as_deref() != Some(relation.database.as_str())
            || context.principal.is_empty()
            || context.session_id.is_empty()
            || context.search_path.len() > 1000
        {
            return Err(invalid());
        }
        // Count borrowed metadata before cloning any producer-controlled string.
        let mut charge = encoded_charge(context, CONTEXT_MAX_BYTES)?;
        add(
            &mut charge,
            encoded_charge(relation, MAX_BINDING_BYTES)?,
            MAX_BINDING_BYTES,
        )?;
        add(
            &mut charge,
            encoded_charge(&columns, MAX_BINDING_BYTES)?,
            MAX_BINDING_BYTES,
        )?;
        // Owned struct/Vec fields, duplicated compiler identifiers/types and keys.
        add(
            &mut charge,
            2048 + session_id.len() + columns.len() * 4096,
            MAX_BINDING_BYTES,
        )?;
        let codec = InspectedMutationTarget::from_inspection(relation, columns)?;
        Ok(Self {
            session_id: session_id.into(),
            helper: helper.clone(),
            context: context.clone(),
            browse_revision: browse_revision.into(),
            target_id: target_id.into(),
            relation: relation.clone(),
            inspection_revision: inspection_revision.into(),
            columns: columns.to_vec(),
            codec,
            charge,
        })
    }
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }
    pub(crate) fn helper_generation(&self) -> &HelperGeneration {
        &self.helper
    }
    pub(crate) fn context(&self) -> &LiveContext {
        &self.context
    }
    pub(crate) fn browse_revision(&self) -> &str {
        &self.browse_revision
    }
    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }
    pub(crate) fn relation(&self) -> &BrowseRelation {
        &self.relation
    }
    pub(crate) fn inspection_revision(&self) -> &str {
        &self.inspection_revision
    }
    pub(crate) fn columns(&self) -> &[ColumnInspection] {
        &self.columns
    }
    fn matches(&self, other: &Self) -> bool {
        self.session_id == other.session_id
            && self.helper.same_helper(&other.helper)
            && self.context == other.context
            && self.browse_revision == other.browse_revision
            && self.target_id == other.target_id
            && self.relation == other.relation
            && self.inspection_revision == other.inspection_revision
            && self.columns == other.columns
    }
}

/// Intent only: originals, schema, SQL and physical identity are never supplied.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Native intent variants precede the application batch preparation caller"
    )
)]
pub(crate) enum SnapshotMutation {
    Insert {
        values: Vec<(usize, CellValue)>,
    },
    Update {
        row_index: usize,
        changes: Vec<(usize, CellValue)>,
    },
    Delete {
        row_index: usize,
    },
}
#[derive(Default)]
struct Usage {
    bytes: usize,
    count: usize,
}
struct BudgetLease {
    usage: Arc<Mutex<Usage>>,
    bytes: usize,
}
impl BudgetLease {
    fn claim(usage: &Arc<Mutex<Usage>>, bytes: usize) -> AppResult<Self> {
        let mut state = usage.lock();
        if state.count >= MAX_ENTRIES || state.bytes.saturating_add(bytes) > MAX_BYTES {
            return Err(limit());
        }
        state.count += 1;
        state.bytes += bytes;
        Ok(Self {
            usage: usage.clone(),
            bytes,
        })
    }
    fn resize(&mut self, bytes: usize) -> AppResult<()> {
        let mut state = self.usage.lock();
        let next = state
            .bytes
            .checked_sub(self.bytes)
            .and_then(|n| n.checked_add(bytes))
            .filter(|n| *n <= MAX_BYTES)
            .ok_or_else(limit)?;
        state.bytes = next;
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for BudgetLease {
    fn drop(&mut self) {
        let mut state = self.usage.lock();
        state.bytes -= self.bytes;
        state.count -= 1;
    }
}
struct Authority {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "Prepared snapshot identity precedes typed batch dispatch integration"
        )
    )]
    id: String,
    binding: NativeEditBinding,
    revoked: AtomicBool,
    expires: Mutex<Instant>,
    budget: Mutex<BudgetLease>,
}
impl Authority {
    fn valid(&self, now: Instant) -> bool {
        !self.revoked.load(Ordering::Acquire) && now < *self.expires.lock()
    }
}
struct Snapshot {
    authority: Arc<Authority>,
    rows: Vec<TextRow>,
}
#[derive(Default)]
struct Registry {
    snapshots: HashMap<String, Snapshot>,
    // Weak records let revocation reach prepared plans without retaining them.
    authorities: HashMap<String, Weak<Authority>>,
}
impl Registry {
    fn prune(&mut self, now: Instant) {
        self.authorities.retain(|_, record| {
            if let Some(authority) = record.upgrade() {
                if !authority.valid(now) {
                    authority.revoked.store(true, Ordering::Release);
                }
                true
            } else {
                false
            }
        });
        self.snapshots
            .retain(|_, snapshot| snapshot.authority.valid(now));
        self.authorities
            .retain(|_, record| record.strong_count() != 0);
    }
    fn revoke(&mut self, mut matches: impl FnMut(&NativeEditBinding) -> bool) {
        for authority in self.authorities.values().filter_map(Weak::upgrade) {
            if matches(&authority.binding) {
                authority.revoked.store(true, Ordering::Release);
            }
        }
        self.prune(Instant::now());
    }
}

/// Share this store across sessions. Count and bytes include outstanding plans.
#[derive(Default)]
pub(crate) struct SnapshotStore {
    registry: Mutex<Registry>,
    usage: Arc<Mutex<Usage>>,
}
impl SnapshotStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn retain(
        &self,
        binding: NativeEditBinding,
        rows: Vec<TextRow>,
    ) -> AppResult<String> {
        let row_bytes = rows_charge(&rows, rows.capacity(), binding.columns.len())?;
        let bytes = binding.charge.checked_add(row_bytes).ok_or_else(limit)?;
        let mut registry = self.registry.lock();
        registry.prune(Instant::now());
        // A newly retained preview replaces the prior preview/plan for this
        // exact native target. Failed remote previews must revoke before await.
        registry.revoke(|old| {
            old.session_id == binding.session_id
                && old.helper.same_helper(&binding.helper)
                && old.target_id == binding.target_id
        });
        let scratch = rows
            .iter()
            .map(|row| row.len() * 96 + row.iter().flatten().map(String::len).sum::<usize>())
            .max()
            .unwrap_or(0);
        let mut budget =
            BudgetLease::claim(&self.usage, bytes.checked_add(scratch).ok_or_else(limit)?)?;
        // Validate every original against exact codecs, including nullable and
        // generated columns. Scratch is reserved before these bounded clones.
        for row in &rows {
            binding.codec.original(row)?;
        }
        budget.resize(bytes)?;
        let id = uuid::Uuid::new_v4().to_string();
        let authority = Arc::new(Authority {
            id: id.clone(),
            binding,
            revoked: AtomicBool::new(false),
            expires: Mutex::new(Instant::now() + SNAPSHOT_TTL),
            budget: Mutex::new(budget),
        });
        registry
            .authorities
            .insert(id.clone(), Arc::downgrade(&authority));
        registry
            .snapshots
            .insert(id.clone(), Snapshot { authority, rows });
        Ok(id)
    }
    pub(crate) fn prepare(
        &self,
        session_id: &str,
        snapshot_id: &str,
        current: &NativeEditBinding,
        mutations: &[SnapshotMutation],
    ) -> AppResult<PreparedSnowflakeEdits> {
        if !canonical_id(snapshot_id) || session_id != current.session_id {
            return Err(stale());
        }
        let snapshot = {
            let mut registry = self.registry.lock();
            registry.prune(Instant::now());
            let snapshot = registry.snapshots.get(snapshot_id).ok_or_else(stale)?;
            if snapshot.authority.binding.session_id != session_id
                || !snapshot.authority.binding.matches(current)
            {
                return Err(stale());
            }
            let now = Instant::now();
            if !snapshot.authority.valid(now) {
                return Err(stale());
            }
            *snapshot.authority.expires.lock() = now + PLAN_TTL;
            registry.snapshots.remove(snapshot_id).ok_or_else(stale)?
        };
        // From here the token is consumed even if validation or compilation
        // fails. A failed preparation never recreates native authority.
        let authority = snapshot.authority;
        let result = (|| {
            let clone_bytes =
                preflight_mutations(&snapshot.rows, authority.binding.columns.len(), mutations)?;
            {
                let mut budget = authority.budget.lock();
                let peak = budget
                    .bytes
                    .checked_add(MAX_BATCH_BYTES)
                    .and_then(|n| n.checked_add(clone_bytes))
                    .ok_or_else(limit)?;
                budget.resize(peak)?;
            }
            let mut native = Vec::with_capacity(mutations.len());
            for mutation in mutations {
                native.push(match mutation {
                    SnapshotMutation::Insert { values } => Mutation::Insert {
                        values: values.clone(),
                    },
                    SnapshotMutation::Update { row_index, changes } => Mutation::Update {
                        original: authority
                            .binding
                            .codec
                            .original(&snapshot.rows[*row_index])?,
                        changes: changes.clone(),
                    },
                    SnapshotMutation::Delete { row_index } => Mutation::Delete {
                        original: authority
                            .binding
                            .codec
                            .original(&snapshot.rows[*row_index])?,
                    },
                });
            }
            let compiled = authority.binding.codec.compile(&native)?;
            let retained = authority
                .binding
                .charge
                .checked_add(compiled.accounted_bytes())
                .ok_or_else(limit)?;
            drop(native);
            drop(snapshot.rows);
            authority.budget.lock().resize(retained)?;
            if !authority.valid(Instant::now()) {
                return Err(stale());
            }
            Ok(PreparedSnowflakeEdits {
                authority: authority.clone(),
                compiled,
            })
        })();
        if result.is_err() {
            authority.revoked.store(true, Ordering::Release);
        }
        result
    }
    pub(crate) fn revoke_session(&self, session_id: &str) {
        self.registry
            .lock()
            .revoke(|binding| binding.session_id == session_id);
    }
    pub(crate) fn revoke_target(
        &self,
        session_id: &str,
        helper: &HelperGeneration,
        target_id: &str,
    ) {
        self.registry.lock().revoke(|binding| {
            binding.session_id == session_id
                && binding.helper.same_helper(helper)
                && binding.target_id == target_id
        });
    }
    #[cfg(test)]
    pub(crate) fn clear(&self) {
        self.registry.lock().revoke(|_| true);
    }
    #[cfg(test)]
    pub(crate) fn usage(&self) -> (usize, usize) {
        self.registry.lock().prune(Instant::now());
        let usage = self.usage.lock();
        (usage.bytes, usage.count)
    }
}

/// A consumed native snapshot, still requiring lane ownership and fresh checks.
/// Retain this value while using its compiled SQL/binds; it owns their budget.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Opaque prepared plans precede typed batch dispatch integration"
    )
)]
pub(crate) struct PreparedSnowflakeEdits {
    authority: Arc<Authority>,
    compiled: CompiledBatch,
}
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "Opaque prepared plans precede typed batch dispatch integration"
    )
)]
impl PreparedSnowflakeEdits {
    pub(crate) fn binding(&self) -> &NativeEditBinding {
        &self.authority.binding
    }
    pub(crate) fn compiled(&self) -> &CompiledBatch {
        &self.compiled
    }
    pub(crate) fn snapshot_id(&self) -> &str {
        &self.authority.id
    }
    /// Reserve the logical dispatch peak before allocating a JSON payload.
    /// Compilation already charges 96 bytes per binding and twice its text,
    /// plus SQL/identifier bytes. Three additional copies conservatively cover
    /// temporary JSON construction and the retained transport payload. Two
    /// bounded frames cover encoding/queue handoff; the fixed margin covers
    /// envelope/map keys. This is shared-budget accounting, not an RSS bound.
    pub(crate) fn reserve_dispatch(&mut self) -> AppResult<()> {
        let bytes = self
            .compiled
            .accounted_bytes()
            .checked_mul(4)
            .and_then(|n| n.checked_add(self.authority.binding.charge))
            .and_then(|n| n.checked_add(2 * super::snowflake_process::MAX_FRAME))
            .and_then(|n| n.checked_add(64 * 1024))
            .ok_or_else(limit)?;
        self.authority.budget.lock().resize(bytes)
    }
    pub(crate) fn validate_current(&self, current: &NativeEditBinding) -> AppResult<()> {
        if self.authority.valid(Instant::now()) && self.authority.binding.matches(current) {
            Ok(())
        } else {
            Err(stale())
        }
    }
}

fn rows_charge(rows: &[TextRow], row_capacity: usize, columns: usize) -> AppResult<usize> {
    if rows.len() > MAX_ROWS {
        return Err(limit());
    }
    let mut bytes = row_capacity
        .checked_mul(std::mem::size_of::<TextRow>())
        .ok_or_else(limit)?;
    if bytes > MAX_PREVIEW_BYTES {
        return Err(limit());
    }
    for row in rows {
        if row.len() != columns {
            return Err(invalid());
        }
        add(
            &mut bytes,
            row.capacity()
                .checked_mul(std::mem::size_of::<Option<String>>())
                .ok_or_else(limit)?,
            MAX_PREVIEW_BYTES,
        )?;
        for cell in row.iter().flatten() {
            if cell.len() > MAX_CELL_BYTES {
                return Err(limit());
            }
            add(&mut bytes, cell.capacity(), MAX_PREVIEW_BYTES)?;
        }
    }
    Ok(bytes)
}
fn preflight_mutations(
    rows: &[TextRow],
    columns: usize,
    mutations: &[SnapshotMutation],
) -> AppResult<usize> {
    if mutations.is_empty() {
        return Err(invalid());
    }
    if mutations.len() > MAX_ROWS {
        return Err(limit());
    }
    let mut bytes = 0;
    let mut clone_bytes = mutations.len() * 128;
    let mut originals = HashSet::new();
    for mutation in mutations {
        let (row, assignments) = match mutation {
            SnapshotMutation::Insert { values } => (None, Some(values)),
            SnapshotMutation::Update { row_index, changes } => (Some(*row_index), Some(changes)),
            SnapshotMutation::Delete { row_index } => (Some(*row_index), None),
        };
        if let Some(index) = row {
            if !originals.insert(index) {
                return Err(stale());
            }
            let row = rows.get(index).ok_or_else(invalid)?;
            for cell in row {
                add(&mut bytes, 96, MAX_BATCH_BYTES)?;
                add(&mut clone_bytes, 96, MAX_BATCH_BYTES)?;
                if let Some(value) = cell {
                    add(
                        &mut bytes,
                        value.len().checked_mul(2).ok_or_else(limit)?,
                        MAX_BATCH_BYTES,
                    )?;
                    add(&mut clone_bytes, value.len(), MAX_BATCH_BYTES)?;
                }
            }
        }
        if let Some(assignments) = assignments {
            if assignments.is_empty() || assignments.len() > columns {
                return Err(invalid());
            }
            let mut seen = [false; MAX_COLUMNS];
            for (column, cell) in assignments {
                if *column >= columns || seen[*column] {
                    return Err(invalid());
                }
                seen[*column] = true;
                add(&mut bytes, 96, MAX_BATCH_BYTES)?;
                add(&mut clone_bytes, 96, MAX_BATCH_BYTES)?;
                if let CellValue::Text(value) = cell {
                    if value.len() > MAX_CELL_BYTES {
                        return Err(limit());
                    }
                    add(
                        &mut bytes,
                        value.len().checked_mul(2).ok_or_else(limit)?,
                        MAX_BATCH_BYTES,
                    )?;
                    add(&mut clone_bytes, value.len(), MAX_BATCH_BYTES)?;
                }
            }
        }
    }
    Ok(clone_bytes)
}

#[cfg(test)]
#[path = "snowflake_edit_snapshot_tests.rs"]
mod tests;
