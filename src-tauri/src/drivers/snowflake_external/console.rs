//! Private retained-session authority. This is not a public approval/IPC boundary.
use super::super::snowflake_process::{Operation, ReviewedAcknowledgement, ReviewedContext};
use super::*;
use crate::safety::snowflake_script::CompiledScript;
use tokio::sync::MutexGuard;

/// The execution owner must record original acknowledgement evidence before any
/// result publication. The adapter does not supply a default/no-op recorder or
/// claim that implementing this trait itself provides durability.
#[async_trait]
pub(crate) trait ReviewedConsoleAckRecorder: Send {
    async fn record_ack(&mut self, acknowledgement: &ReviewedAcknowledgement) -> AppResult<()>;
}

/// The execution owner serializes final policy checks and durable intent with
/// actual supervisor admission. `dispatch` is single use and performs no await.
/// Lock ordering is owner configuration gate, then driver console ownership.
pub(crate) trait ReviewedConsoleAdmission: Send {
    fn admit(&mut self, dispatch: &mut dyn FnMut() -> AppResult<Operation>)
    -> AppResult<Operation>;
}

struct AckRecording {
    attempted: bool,
    recorded: bool,
    error: Option<AppError>,
}
impl AckRecording {
    async fn observe(
        &mut self,
        acknowledgement: Option<ReviewedAcknowledgement>,
        recorder: &mut dyn ReviewedConsoleAckRecorder,
    ) -> bool {
        if self.attempted {
            return false;
        }
        let Some(acknowledgement) = acknowledgement else {
            return false;
        };
        self.attempted = true;
        match tokio::time::timeout(
            Duration::from_secs(65),
            recorder.record_ack(&acknowledgement),
        )
        .await
        {
            Ok(Ok(())) => self.recorded = true,
            Ok(Err(error)) => self.error = Some(error),
            Err(_) => {
                self.error = Some(AppError::new(
                    ErrorCode::Conflict,
                    "Reviewed acknowledgement recording deadline expired",
                ))
            }
        }
        true
    }
}

/// The sole owner of a permanently console-owned physical session. Neither its
/// nonce nor construction is exposed. Dropping this owner closes, never hands
/// the connection back to a probe-bearing adapter path.
#[allow(
    dead_code,
    reason = "Private foundation; the retained execution owner is not wired yet"
)]
pub(crate) struct ReviewedConsole {
    driver: Arc<SnowflakeExternal>,
    nonce: String,
    context: Mutex<Option<ReviewedContext>>,
    timeout_seconds: u32,
}

#[allow(
    dead_code,
    reason = "Private foundation; the retained execution owner is not wired yet"
)]
pub(crate) struct ReviewedConsoleRun<'a> {
    console: &'a ReviewedConsole,
    script: &'a CompiledScript,
    _lane: MutexGuard<'a, ()>,
    cancellation: CancellationToken,
    next: usize,
    terminal: bool,
    stopped: bool,
    in_flight: bool,
}

/// `attempted` means the supervisor accepted this operation, not proof of remote
/// submission. Original ACK/terminal evidence is independent of sink delivery.
#[allow(
    dead_code,
    reason = "Private foundation; the retained execution owner is not wired yet"
)]
pub(crate) struct ReviewedConsoleUnitOutcome {
    pub unit_index: usize,
    pub attempted: bool,
    pub report: Option<Report>,
    pub delivery_error: Option<AppError>,
    pub recording_error: Option<AppError>,
    pub acknowledgement_recorded: bool,
    /// Rows in full batches accepted with Ok(true); a sink returning false or
    /// error owns its exact partial retention count. Never an affected-row count.
    pub retained_rows: u64,
    pub truncated: bool,
    pub cancellation_observed: bool,
}

impl SnowflakeExternal {
    pub(super) fn require_ordinary_mode(&self) -> AppResult<()> {
        if self.console_mode.lock().is_some() {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "This Snowflake session belongs to a reviewed console; automatic and generated SQL is disabled",
            ));
        }
        self.grid.require_ordinary()
    }

    /// Admission does not execute SQL. Bootstrap identity/version SQL has already
    /// happened in `open`; there is no reconnect or new baseline here.
    #[allow(
        dead_code,
        reason = "Private foundation; no application route is exposed yet"
    )]
    pub(crate) async fn claim_reviewed_console(self: &Arc<Self>) -> AppResult<ReviewedConsole> {
        let _lane = self.lane.lock().await;
        self.require_unrestricted()?;
        if self.profile.safe_mode == SafeMode::ReadOnly
            || !self.profile.snowflake.as_ref().is_some_and(|config| {
                config.authentication == SnowflakeAuthentication::ExternalBrowser
                    && config.reviewed_console
            })
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "A reviewed Snowflake console requires an explicitly enabled writable profile",
            ));
        }
        let timeout_seconds = self
            .profile
            .snowflake
            .as_ref()
            .ok_or_else(closed_error)?
            .effective_console_timeout_seconds()?;
        self.invalidate_relation_catalog();
        self.invalidate_session_options();
        self.revoke_transaction_grant();
        let nonce = new_id();
        *self.console_mode.lock() = Some(nonce.clone());
        Ok(ReviewedConsole {
            driver: self.clone(),
            nonce,
            context: Mutex::new(None),
            timeout_seconds,
        })
    }
}

#[allow(
    dead_code,
    reason = "Private foundation; the retained execution owner is not wired yet"
)]
impl ReviewedConsole {
    pub(crate) fn timeout_seconds(&self) -> u32 {
        self.timeout_seconds
    }

    fn check(&self) -> AppResult<()> {
        if self.driver.is_closed() {
            return Err(closed_error());
        }
        #[cfg(all(unix, feature = "test-harness"))]
        self.driver.check_metadata_observer()?;
        if self.driver.console_mode.lock().as_deref() != Some(self.nonce.as_str()) {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Reviewed console ownership changed",
            ));
        }
        if self
            .driver
            .transaction_authority
            .lock()
            .recovery
            .as_ref()
            .is_some_and(|r| r.restricted)
        {
            return Err(AppError::new(
                ErrorCode::PolicyBlocked,
                "Snowflake rollback-only recovery blocks reviewed console execution",
            ));
        }
        Ok(())
    }

    /// Response-derived context only. Null fields stay unknown; startup context
    /// is never substituted and no transaction/autocommit authority is inferred.
    pub(crate) fn passive_context(&self) -> Option<ReviewedContext> {
        self.context.lock().clone()
    }

    pub(crate) async fn begin_run<'a>(
        &'a mut self,
        script: &'a CompiledScript,
        cancellation: CancellationToken,
    ) -> AppResult<ReviewedConsoleRun<'a>> {
        script.verify(
            &script
                .units()
                .iter()
                .map(|u| u.bound().clone())
                .collect::<Vec<_>>(),
        )?;
        self.check()?;
        let lane = self.driver.lane.lock().await;
        self.check()?;
        Ok(ReviewedConsoleRun {
            console: self,
            script,
            _lane: lane,
            cancellation,
            next: 0,
            terminal: false,
            stopped: false,
            in_flight: false,
        })
    }
}

impl Drop for ReviewedConsole {
    fn drop(&mut self) {
        self.driver.start_shutdown();
    }
}

#[allow(
    dead_code,
    reason = "Private foundation; the retained execution owner is not wired yet"
)]
impl ReviewedConsoleRun<'_> {
    pub(crate) fn next_index(&self) -> usize {
        self.next
    }
    pub(crate) fn planned_units(&self) -> usize {
        self.script.units().len()
    }
    pub(crate) fn is_terminal(&self) -> bool {
        self.terminal
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// An owner may stop after recording a step without silently admitting its
    /// suffix. No automatic COMMIT/ROLLBACK or observer is issued.
    pub(crate) fn stop(&mut self) {
        self.terminal = true;
        self.stopped = true;
    }

    /// Only the contiguous immutable next unit is dispatchable. The caller can
    /// durably record its intent using `next_index` before invoking this method.
    pub(crate) async fn next(
        &mut self,
        sink: &mut dyn QuerySink,
        recorder: &mut dyn ReviewedConsoleAckRecorder,
        admission: &mut dyn ReviewedConsoleAdmission,
    ) -> AppResult<Option<ReviewedConsoleUnitOutcome>> {
        if self.terminal {
            return Ok(None);
        }
        if self.in_flight {
            self.stop();
            self.console.driver.start_shutdown();
            return Err(closed_error());
        }
        let index = self.next;
        if self.cancellation.is_cancelled() {
            self.stop();
            return Ok(Some(ReviewedConsoleUnitOutcome {
                unit_index: index,
                attempted: false,
                report: None,
                delivery_error: Some(AppError::new(
                    ErrorCode::Cancelled,
                    "Reviewed console run cancelled before dispatch",
                )),
                recording_error: None,
                acknowledgement_recorded: false,
                retained_rows: 0,
                truncated: false,
                cancellation_observed: true,
            }));
        }
        if let Err(error) = self.console.check() {
            self.stop();
            return Err(error);
        }
        // Canonical verification remains local and complete at every dispatch;
        // callers cannot substitute a public BoundStatement after review.
        let prepared = (|| {
            self.script.verify(
                &self
                    .script
                    .units()
                    .iter()
                    .map(|u| u.bound().clone())
                    .collect::<Vec<_>>(),
            )?;
            let wire = self.script.units()[index].bound();
            if wire.rewritten_sql.is_empty() || wire.rewritten_sql.len() > SQL_BYTES {
                return Err(AppError::invalid("Invalid reviewed Snowflake unit size"));
            }
            Ok((wire, binds_json(&wire.binds)?))
        })();
        let (wire, binds) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.stop();
                return Err(error);
            }
        };
        // There is no await between the final revocation/cancel check and request.
        if let Err(error) = self.console.check() {
            self.stop();
            return Err(error);
        }
        if self.cancellation.is_cancelled() {
            self.stop();
            return Ok(Some(ReviewedConsoleUnitOutcome {
                unit_index: index,
                attempted: false,
                report: None,
                delivery_error: Some(AppError::new(
                    ErrorCode::Cancelled,
                    "Reviewed console run cancelled before dispatch",
                )),
                recording_error: None,
                acknowledgement_recorded: false,
                retained_rows: 0,
                truncated: false,
                cancellation_observed: true,
            }));
        }
        let driver = self.console.driver.clone();
        let mut dispatched = false;
        let mut admitted_at = None;
        let admitted = admission.admit(&mut || {
            if std::mem::replace(&mut dispatched, true) {
                return Err(AppError::new(ErrorCode::PolicyBlocked, "Console dispatch was already admitted"));
            }
            // start_shutdown/cancel serialize their immediate local revocation
            // under this same lock. The owner's configuration gate is outermost.
            let mode = driver.console_mode.lock();
            if self.cancellation.is_cancelled() {
                Err(AppError::new(ErrorCode::Cancelled, "Reviewed console run cancelled before dispatch"))
            } else if driver.is_closed() || mode.as_deref() != Some(self.console.nonce.as_str()) {
                Err(closed_error())
            } else {
                let began = tokio::time::Instant::now();
                driver.process.request("reviewed-console", "reviewedSql", json!({
                    "sql": wire.rewritten_sql, "binds": binds,
                    "expected": {"user": driver.identity["user"], "session": driver.identity["session"]},
                    "credits": 1, "maxRows": metadata::STANDARD_QUERY_MAX_ROWS,
                    "maxBytes": metadata::STANDARD_QUERY_MAX_BYTES,
                }), Impact::Mutating).inspect(|_| { admitted_at = Some(began); }).map_err(control_error)
            }
        });
        let mut operation = match admitted {
            Ok(operation) => operation,
            Err(error) => {
                self.stop();
                return Err(error);
            }
        };
        self.in_flight = true;
        let token = operation.token();
        *driver.active.lock() = Some(Active {
            token: token.clone(),
            terminal: operation.terminal_receiver(),
        });
        let active = ActiveLease(&driver.active);
        let mut recording = AckRecording {
            attempted: false,
            recorded: false,
            error: None,
        };
        let mut described = false;
        let mut columns_open = true;
        let mut retaining = true;
        let mut retained_rows = 0u64;
        let mut delivery_error = None;
        let mut truncated = false;
        let mut cancelled = false;
        let deadline_at = admitted_at.expect("successful synchronous admission")
            + Duration::from_secs(u64::from(self.console.timeout_seconds));
        // Recording/control/terminal ownership is retained independently. Recheck
        // the absolute admission deadline after those awaits before publication.
        let expired = |error: &mut Option<AppError>| {
            if tokio::time::Instant::now() < deadline_at {
                return false;
            }
            error.get_or_insert_with(|| {
                AppError::new(
                    ErrorCode::ConnectionClosed,
                    "Reviewed console deadline expired",
                )
            });
            driver.start_shutdown();
            true
        };
        let deadline = tokio::time::sleep_until(deadline_at);
        tokio::pin!(deadline);
        let mut boolean_columns = Vec::new();
        loop {
            // ACK is sticky in the supervisor. Check again when columns become
            // visible: the ACK may have arrived between the first read and the
            // column watch read, including a fast terminal publication.
            recording
                .observe(operation.statement_acknowledged(), recorder)
                .await;
            if expired(&mut delivery_error) {
                break;
            }
            let available_columns = operation.columns.borrow().clone();
            if !described && available_columns.is_some() {
                recording
                    .observe(operation.statement_acknowledged(), recorder)
                    .await;
                if expired(&mut delivery_error) {
                    break;
                }
                if !recording.attempted {
                    delivery_error.get_or_insert_with(protocol_error);
                    driver.start_shutdown();
                    break;
                }
            }
            if recording.error.is_some() && retaining {
                retaining = false;
                if let Err(error) = driver.discard(&token, &mut operation).await {
                    delivery_error.get_or_insert(error);
                    driver.start_shutdown();
                }
            }
            if expired(&mut delivery_error) {
                break;
            }
            if !described
                && recording.recorded
                && let Some(columns) = available_columns
            {
                described = true;
                let infos: Vec<ColumnInfo> =
                    columns.into_iter().enumerate().map(column_info).collect();
                boolean_columns = infos
                    .iter()
                    .map(|c| c.kind == ColumnKind::Boolean)
                    .collect();
                if let Err(error) = sink.describe(infos) {
                    delivery_error = Some(error);
                    retaining = false;
                    if let Err(error) = driver.discard(&token, &mut operation).await {
                        delivery_error.get_or_insert(error);
                        driver.start_shutdown();
                    }
                }
            }
            if expired(&mut delivery_error) {
                break;
            }
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled(), if !cancelled => {
                    cancelled = true;
                    retaining = false;
                    // Cancel-control acknowledgement never substitutes for the original report.
                    if let Ok(mut cancel) = driver.process.cancel(&token) {
                        let _ = cancel.wait_bounded(Duration::from_secs(3)).await;
                    }
                }
                _ = &mut deadline => {
                    expired(&mut delivery_error);
                    break;
                }
                changed = operation.columns.changed(), if !described && columns_open => { columns_open = changed.is_ok(); }
                batch = operation.rows.recv() => {
                    if expired(&mut delivery_error) { break; }
                    let Some(mut batch) = batch else { break; };
                    let mut rows = std::mem::take(&mut batch.rows);
                    drop(batch);
                    if !described && recording.error.is_none() {
                        delivery_error.get_or_insert_with(protocol_error); driver.start_shutdown(); break;
                    }
                    if retaining {
                        normalize_wire_booleans(&boolean_columns, &mut rows);
                        let count = rows.len() as u64;
                        match sink.rows(rows) {
                            Ok(true) => retained_rows = retained_rows.saturating_add(count),
                            Ok(false) => { truncated = true; retaining = false; if !expired(&mut delivery_error) { sink.limit("Result retention limit reached"); } },
                            Err(error) => { delivery_error = Some(error); retaining = false; },
                        }
                        if !retaining && let Err(error) = driver.discard(&token, &mut operation).await {
                            delivery_error.get_or_insert(error); driver.start_shutdown();
                        }
                    }
                    if expired(&mut delivery_error) { break; }
                    if retaining && operation.report().is_none() {
                        match driver.process.credit(&token, 1) {
                            Ok(mut credit) => {
                                let report = credit.wait_bounded(Duration::from_secs(3)).await;
                                if operation.report().is_none() && let Err(error) = check_report(&report) {
                                    delivery_error.get_or_insert(error); driver.start_shutdown(); break;
                                }
                            }
                            Err(_) if operation.report().is_some() => {},
                            Err(super::super::snowflake_process::INACTIVE_OPERATION)
                                if operation.wait_if_finalizing().await.is_some() => {},
                            Err(_) => { delivery_error.get_or_insert_with(closed_error); driver.start_shutdown(); break; },
                        }
                    }
                }
            }
        }
        let report = operation.wait_bounded(Duration::from_secs(5)).await;
        // A terminal can win the row-channel close race. Record its retained
        // original ACK even when no result columns were published.
        recording
            .observe(report.statement_acknowledged.clone(), recorder)
            .await;
        expired(&mut delivery_error);
        drop(active);
        self.in_flight = false;
        if let Some(ack) = &report.statement_acknowledged {
            *self.console.context.lock() = Some(ack.context.clone());
        }
        if report.retirement.is_some() || report.certainty == Certainty::OutcomeUnknown {
            driver.start_shutdown();
        }
        if report.ok && !recording.attempted {
            delivery_error.get_or_insert_with(protocol_error);
            driver.start_shutdown();
        }
        if report.ok && recording.recorded {
            if !described {
                delivery_error.get_or_insert_with(protocol_error);
                driver.start_shutdown();
            }
            if report
                .result
                .as_ref()
                .and_then(|r| r.get("truncated"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                truncated = true;
                if !expired(&mut delivery_error) {
                    sink.limit("Snowflake helper result retention limit reached");
                }
            }
            if !expired(&mut delivery_error)
                && delivery_error.is_none()
                && !cancelled
                && let Err(error) = sink.complete(retained_rows)
            {
                delivery_error = Some(error);
            }
        }
        expired(&mut delivery_error);
        cancelled |= self.cancellation.is_cancelled();
        self.next += 1;
        if !report.ok
            || recording.error.is_some()
            || delivery_error.is_some()
            || truncated
            || cancelled
            || driver.is_closed()
        {
            self.stop();
        } else if self.next == self.script.units().len() {
            self.terminal = true;
        }
        Ok(Some(ReviewedConsoleUnitOutcome {
            unit_index: index,
            attempted: true,
            report: Some(report),
            delivery_error,
            recording_error: recording.error,
            acknowledgement_recorded: recording.recorded,
            retained_rows,
            truncated,
            cancellation_observed: cancelled,
        }))
    }
}

impl Drop for ReviewedConsoleRun<'_> {
    fn drop(&mut self) {
        if self.in_flight || !self.terminal {
            self.console.driver.start_shutdown();
        }
    }
}
