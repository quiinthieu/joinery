//! Retained preview originals. Preparation compiles intent; it cannot dispatch.
use super::*;
use crate::drivers::snowflake_edit_snapshot::{
    NativeEditBinding, PreparedSnowflakeEdits, SnapshotMutation, SnapshotStore,
};
use std::sync::OnceLock;

pub(super) fn shared_store() -> Arc<SnapshotStore> {
    static STORE: OnceLock<Arc<SnapshotStore>> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(SnapshotStore::new())).clone()
}

impl SnowflakeExternal {
    pub(super) fn revoke_edit_snapshots(&self) {
        self.edit_snapshots
            .revoke_helper_session(&self.session_id, &self.process.helper_generation());
    }

    pub(super) fn edit_profile_enabled(&self) -> bool {
        self.profile.safe_mode != SafeMode::ReadOnly
            && self.profile.snowflake.as_ref().is_some_and(|config| {
                config.authentication == SnowflakeAuthentication::ExternalBrowser
                    && config.reviewed_dml
            })
    }

    // Advance before the first await: a failed or abandoned replacement cannot
    // let an older in-flight preview republish an edit token later.
    pub(super) fn begin_edit_preview(&self, request: &PreviewRelationRequest) -> AppResult<String> {
        let mut state = self.browsing.lock();
        let slot = inspection_slot(&mut state, request)?;
        let revision = new_id();
        slot.preview_revision = Some(revision.clone());
        self.edit_snapshots.revoke_target(
            &self.session_id,
            &self.process.helper_generation(),
            &request.target_id,
        );
        Ok(revision)
    }

    // Caller owns the lane. All registry interactions under a browsing guard
    // follow browsing -> store; the store never calls back into this driver.
    pub(super) fn retain_edit_preview(
        &self,
        request: &PreviewRelationRequest,
        preview_revision: &str,
        mut preview: RelationPreview,
    ) -> AppResult<RelationPreview> {
        preview.edit_snapshot_id = None;
        let mut state = self.browsing.lock();
        if self.is_closed() {
            return Err(closed_error());
        }
        if inspection_slot(&mut state, request)?
            .preview_revision
            .as_deref()
            != Some(preview_revision)
        {
            return Err(stale_browse());
        }
        if !self.edit_profile_enabled() {
            return Ok(preview);
        }
        let retained = (|| {
            let binding = self.edit_binding(&state, request)?;
            // One private original store plus the bounded response copy. The
            // response is not also retained in ResultStore.
            let rows = std::mem::take(&mut preview.rows);
            preview.rows = rows.clone();
            self.edit_snapshots.retain(binding, rows)
        })();
        match retained {
            Ok(id) => preview.edit_snapshot_id = Some(id),
            // Browsing stays available when this closed codec or its bounded
            // native capacity cannot admit an edit snapshot.
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::Unsupported | ErrorCode::ResultLimitReached
                ) => {}
            Err(error) => return Err(error),
        }
        if serde_json::to_vec(&preview)
            .map_err(|_| protocol_error())?
            .len()
            > browsing::PREVIEW_MAX_BYTES
        {
            self.edit_snapshots.revoke_target(
                &self.session_id,
                &self.process.helper_generation(),
                &request.target_id,
            );
            return Err(metadata_limit());
        }
        Ok(preview)
    }

    pub(super) fn edit_binding(
        &self,
        state: &BrowseState,
        request: &PreviewRelationRequest,
    ) -> AppResult<NativeEditBinding> {
        if state.revision != request.revision {
            return Err(stale_browse());
        }
        let snapshot = state.snapshot.as_ref().ok_or_else(stale_browse)?;
        let relation = snapshot
            .targets
            .get(&request.target_id)
            .ok_or_else(stale_browse)?;
        let columns = snapshot
            .inspections
            .get(&request.target_id)
            .filter(|slot| slot.revision == request.inspection_revision)
            .and_then(|slot| slot.columns.as_deref())
            .ok_or_else(stale_browse)?;
        NativeEditBinding::with_slice(
            &self.session_id,
            &self.process.helper_generation(),
            &snapshot.context,
            &state.revision,
            &request.target_id,
            relation,
            &request.inspection_revision,
            columns,
            browsing::PreviewSlice::from_request(request),
        )
    }

    /// Native preparation only. Dispatch validates fresh remote context/columns
    /// and reserves durable outcome capacity before any SQL.
    pub(crate) async fn prepare_relation_edits(
        &self,
        request: &PreviewRelationRequest,
        snapshot_id: &str,
        mutations: &[SnapshotMutation],
    ) -> AppResult<PreparedSnowflakeEdits> {
        let _lane = tokio::time::timeout(Duration::from_millis(1500), self.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The Snowflake session is busy"))?;
        self.grid.require_ordinary()?;
        self.require_unrestricted()?;
        if !self.edit_profile_enabled() {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake generated edits require an explicitly enabled writable external-browser profile",
            ));
        }
        let state = self.browsing.lock();
        if self.is_closed() {
            return Err(closed_error());
        }
        let binding = self.edit_binding(&state, request)?;
        self.edit_snapshots
            .prepare(&self.session_id, snapshot_id, &binding, mutations)
    }

    pub(crate) async fn cancel_relation_edits(
        &self,
        edits: PreparedSnowflakeEdits,
    ) -> AppResult<String> {
        let _lane = tokio::time::timeout(Duration::from_millis(1500), self.lane.lock())
            .await
            .map_err(|_| AppError::new(ErrorCode::Conflict, "The Snowflake session is busy"))?;
        self.grid.require_ordinary()?;
        self.require_unrestricted()?;
        if !self.edit_profile_enabled() {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake generated edits require an explicitly enabled writable external-browser profile",
            ));
        }
        let state = self.browsing.lock();
        if self.is_closed() {
            return Err(closed_error());
        }
        let current = self.current_grid_binding_locked(edits.binding(), &state)?;
        self.edit_snapshots.cancel_prepared(edits, current)
    }
}

fn inspection_slot<'a>(
    state: &'a mut BrowseState,
    request: &PreviewRelationRequest,
) -> AppResult<&'a mut InspectionSlot> {
    if state.revision != request.revision {
        return Err(stale_browse());
    }
    state
        .snapshot
        .as_mut()
        .and_then(|snapshot| snapshot.inspections.get_mut(&request.target_id))
        .filter(|slot| slot.revision == request.inspection_revision && slot.columns.is_some())
        .ok_or_else(stale_browse)
}
