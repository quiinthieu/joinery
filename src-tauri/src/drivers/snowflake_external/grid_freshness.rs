//! Lane-owned observations only. A bracket is neither remote object identity nor
//! an edit/transaction capability, and never repairs drift or replays a read.
#![cfg_attr(
    not(all(test, feature = "test-harness")),
    allow(
        dead_code,
        reason = "Freshness bracket precedes public UI staging and IPC wiring"
    )
)]
use super::*;
use crate::drivers::snowflake_edit_snapshot::NativeEditBinding;

#[derive(Clone, Copy)]
pub(super) struct GridReadOwner<'a> {
    pub token: &'a str,
    pub transaction_id: &'a str,
}

// An abandoned bracket cannot release the caller's lane while leaving a private
// owner read active. This also covers cancellation between individual awaits.
struct FreshnessLease<'a> {
    driver: &'a SnowflakeExternal,
    completed: bool,
}
impl Drop for FreshnessLease<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.driver.start_shutdown();
        }
    }
}

impl SnowflakeExternal {
    /// Caller owns `lane` and validates the private local binding before/after.
    /// Ordinary queries remain excluded while a helper transaction owner exists.
    pub(super) async fn verify_grid_freshness(
        &self,
        binding: &NativeEditBinding,
        owner: Option<GridReadOwner<'_>>,
    ) -> AppResult<()> {
        if self.is_closed() {
            return Err(closed_error().with_completion_invalidated());
        }
        if binding.session_id() != self.session_id
            || !binding
                .helper_generation()
                .same_helper(&self.process.helper_generation())
            || owner.is_some_and(|owner| !valid_owner(owner))
        {
            return Err(stale_grid().with_completion_invalidated());
        }
        let mut lease = FreshnessLease {
            driver: self,
            completed: false,
        };
        let result = tokio::time::timeout(Duration::from_secs(65), async {
            self.grid_context_sample(binding.context(), owner).await?;
            let relation = metadata::exact_relation_read(binding.context(), binding.relation())?;
            let before = self
                .grid_read(
                    binding.context(),
                    Some(binding.relation()),
                    owner,
                    "exactRelation",
                    &relation,
                )
                .await?;
            browsing::require_relation(binding.context(), binding.relation(), &before)?;
            let [columns, describe, resolved] = metadata::inspection_reads(binding.relation())?;
            let columns = self
                .grid_read(
                    binding.context(),
                    Some(binding.relation()),
                    owner,
                    "columns",
                    &columns,
                )
                .await?;
            let describe = self
                .grid_read(
                    binding.context(),
                    Some(binding.relation()),
                    owner,
                    "describe",
                    &describe,
                )
                .await?;
            let resolved = self
                .grid_read(
                    binding.context(),
                    Some(binding.relation()),
                    owner,
                    "zeroRowResolution",
                    &resolved,
                )
                .await?;
            let observed =
                browsing::parse_inspection(binding.relation(), &columns, &describe, &resolved)?;
            if observed.as_slice() != binding.columns() {
                return Err(stale_grid());
            }
            let after = self
                .grid_read(
                    binding.context(),
                    Some(binding.relation()),
                    owner,
                    "exactRelation",
                    &relation,
                )
                .await?;
            browsing::require_relation(binding.context(), binding.relation(), &after)?;
            self.grid_context_sample(binding.context(), owner).await?;
            if self.is_closed() {
                return Err(closed_error());
            }
            Ok(())
        })
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                self.start_shutdown();
                Err(AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Snowflake edit metadata deadline expired; reconnect before continuing",
                ))
            }
        };
        if result.as_ref().is_err_and(|error| {
            matches!(
                error.code,
                ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
            )
        }) {
            self.start_shutdown();
        }
        lease.completed = true;
        result.map_err(AppError::with_completion_invalidated)
    }

    /// Caller pins the exact private owner/helper and owns the session lane.
    /// Safety rollback deliberately does not depend on these observations.
    pub(super) async fn verify_grid_owner_context(
        &self,
        expected: &LiveContext,
        owner: GridReadOwner<'_>,
    ) -> AppResult<()> {
        if self.is_closed() {
            return Err(closed_error().with_completion_invalidated());
        }
        if !valid_owner(owner) {
            return Err(stale_grid().with_completion_invalidated());
        }
        let mut lease = FreshnessLease {
            driver: self,
            completed: false,
        };
        let result = tokio::time::timeout(Duration::from_secs(65), async {
            self.grid_context_sample(expected, Some(owner)).await?;
            self.grid_context_sample(expected, Some(owner)).await?;
            if self.is_closed() {
                return Err(closed_error());
            }
            Ok(())
        })
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                self.start_shutdown();
                Err(closed_error())
            }
        };
        if result.as_ref().is_err_and(|error| {
            matches!(
                error.code,
                ErrorCode::ConnectionClosed | ErrorCode::OutcomeUnknown
            )
        }) {
            self.start_shutdown();
        }
        lease.completed = true;
        result.map_err(AppError::with_completion_invalidated)
    }

    async fn grid_context_sample(
        &self,
        expected: &LiveContext,
        owner: Option<GridReadOwner<'_>>,
    ) -> AppResult<()> {
        let [context, policy] = metadata::context_reads();
        let context = self
            .grid_read(expected, None, owner, "context", &context)
            .await?;
        let policy = self
            .grid_read(expected, None, owner, "quotedIdentifierPolicy", &policy)
            .await?;
        let observed = metadata::parse_context(&context, &policy)?;
        self.require_session_identity(&observed)?;
        if observed != *expected {
            return Err(stale_grid());
        }
        let [_, _, _, _, transaction] = metadata::sample_reads();
        let transaction = self
            .grid_read(expected, None, owner, "currentTransaction", &transaction)
            .await?;
        if session::parse_transaction_id(&transaction)?.as_deref()
            != owner.map(|owner| owner.transaction_id)
        {
            return Err(stale_grid());
        }
        Ok(())
    }

    async fn grid_read(
        &self,
        context: &LiveContext,
        relation: Option<&BrowseRelation>,
        owner: Option<GridReadOwner<'_>>,
        kind: &str,
        spec: &metadata::ReadSpec,
    ) -> AppResult<MetadataResult> {
        let Some(owner) = owner else {
            return self.read_spec(spec).await;
        };
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        let target = if matches!(
            kind,
            "context" | "quotedIdentifierPolicy" | "currentTransaction"
        ) {
            Value::Null
        } else {
            let relation = relation.ok_or_else(protocol_error)?;
            json!({"database":relation.database,"schema":relation.table.schema,"name":relation.table.name,"tableType":relation.table_type})
        };
        let mut operation = self.process.request("grid-metadata", "mutationRead", json!({
            "owner":owner.token,"expected":{"user":context.principal,"session":context.session_id,
                "role":context.role,"database":context.database,"schema":context.schema,"warehouse":context.warehouse},
            "kind":kind,"target":target,"credits":1
        }), Impact::Read).map_err(control_error)?;
        let token = operation.token();
        *self.active.lock() = Some(Active {
            token: token.clone(),
            terminal: operation.terminal_receiver(),
        });
        let _active = ActiveLease(&self.active);
        let mut sink = MetadataSink::new(
            usize::try_from(spec.max_rows()).map_err(|_| metadata_limit())?,
            usize::try_from(spec.max_bytes()).map_err(|_| metadata_limit())?,
        );
        let mut described = false;
        let mut columns_open = true;
        loop {
            let columns = operation.columns.borrow().clone();
            if !described && let Some(columns) = columns {
                if let Err(error) =
                    sink.describe(columns.into_iter().enumerate().map(column_info).collect())
                {
                    self.start_shutdown();
                    return Err(error);
                }
                described = true;
            }
            tokio::select! {
                biased;
                changed = operation.columns.changed(), if !described && columns_open => { columns_open = changed.is_ok(); }
                batch = operation.rows.recv() => {
                    let Some(mut batch) = batch else { break };
                    if !described { self.start_shutdown(); return Err(protocol_error()); }
                    let rows = std::mem::take(&mut batch.rows);
                    drop(batch);
                    if let Err(error) = sink.rows(rows) { self.start_shutdown(); return Err(error); }
                    if operation.report().is_none() {
                        match self.process.credit(&token, 1) {
                            Ok(mut credit) => {
                                let report = credit.wait_bounded(Duration::from_secs(3)).await;
                                if operation.report().is_none() { check_report(&report)?; }
                            }
                            Err(_) if operation.report().is_some() => {},
                            Err(super::super::snowflake_process::INACTIVE_OPERATION)
                                if operation.wait_if_finalizing().await.is_some() => {},
                            Err(_) => { self.start_shutdown(); return Err(closed_error()); }
                        }
                    }
                }
            }
        }
        let report = operation.wait().await;
        check_report(&report)?;
        if !described {
            self.start_shutdown();
            return Err(protocol_error());
        }
        let result = report.result.as_ref().ok_or_else(protocol_error)?;
        let count = result["rowsSeen"].as_u64().ok_or_else(protocol_error)?;
        if result["truncated"].as_bool() == Some(true) {
            sink.limit("Snowflake owner metadata retention limit reached");
        }
        sink.complete(count)?;
        #[cfg(all(unix, feature = "test-harness"))]
        self.check_metadata_observer()?;
        Ok(sink.data)
    }
}

fn stale_grid() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "Snowflake edit context, transaction, relation or columns changed; refresh before preparing edits",
    )
}

fn valid_owner(owner: GridReadOwner<'_>) -> bool {
    uuid::Uuid::parse_str(owner.token).is_ok_and(|id| {
        id.to_string() == owner.token
            && matches!(id.get_version_num(), 1..=5)
            && id.get_variant() == uuid::Variant::RFC4122
    }) && owner
        .transaction_id
        .parse::<i64>()
        .is_ok_and(|id| id.to_string() == owner.transaction_id)
}
