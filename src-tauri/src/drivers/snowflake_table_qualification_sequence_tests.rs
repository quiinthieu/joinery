use super::*;
use crate::{
    domain::{ErrorCode, ExplicitTransactionAction, SnowflakeDdlKind, SnowflakeDmlKind},
    drivers::snowflake_process::{
        BoundaryAcknowledgement, DdlAcknowledgement, DmlAcknowledgement, Retirement,
        TransactionAction,
    },
};

const GENERATION: &str = "synthetic-generation";

fn profile() -> Value {
    json!({"account":"synthetic","host":"synthetic.snowflakecomputing.com","username":"LOGIN","role":"ROLE","warehouse":"WH","database":DATABASE,"schema":SCHEMA})
}
fn statement() -> StatementIdentity {
    StatementIdentity {
        sdk_request_id: "00000000-0000-4000-8000-000000000001".into(),
        query_id: Some("00000000-0000-4000-8000-000000000002".into()),
    }
}
fn report() -> Report {
    Report {
        ok: true,
        certainty: Certainty::ServerTerminal,
        phase: ProcessPhase::Sent,
        result: Some(json!({"ok":true})),
        error_code: None,
        retirement: None,
        statements: vec![],
        use_acknowledged: None,
        boundary_acknowledged: None,
        dml_acknowledged: None,
        statement_acknowledged: None,
        ddl_acknowledged: None,
    }
}

struct Frame {
    generation: String,
    logical: String,
    ticket: u64,
    operation: String,
    impact: Impact,
    target: Option<(String, Option<u64>)>,
    value: Value,
    bytes: Vec<u8>,
}
impl Frame {
    fn new(wire: u64, ticket: u64, operation: &str, impact: Impact, payload: Value) -> Self {
        let value = json!({"v":1,"generation":GENERATION,"id":wire.to_string(),"op":operation,"payload":payload});
        let bytes = serde_json::to_vec(&value).unwrap();
        Self {
            generation: GENERATION.into(),
            logical: operation.into(),
            ticket,
            operation: operation.into(),
            impact,
            target: None,
            value,
            bytes,
        }
    }
    fn request(&self) -> PhysicalRequest<'_> {
        PhysicalRequest {
            generation: &self.generation,
            operation_id: &self.logical,
            ticket: self.ticket,
            operation: &self.operation,
            impact: self.impact,
            target: self
                .target
                .as_ref()
                .map(|(id, ticket)| (id.as_str(), *ticket)),
            payload: &self.value,
            wire_payload: &self.bytes,
        }
    }
}

struct Harness {
    seq: Sequencer,
    wire: u64,
    ticket: u64,
}
impl Harness {
    fn new() -> Self {
        Self::new_from_ticket(0)
    }
    fn new_from_ticket(ticket: u64) -> Self {
        let mut h = Self {
            seq: Sequencer::new(profile()).unwrap(),
            wire: 0,
            ticket,
        };
        let frame = h.frame("connect", Impact::Control, profile());
        let admission = h.seq.reserve(&frame.request()).unwrap();
        assert_eq!(admission.ticket, ticket);
        assert!(h.finish(&admission, &report()).error().is_none());
        h
    }
    fn frame(&mut self, op: &str, impact: Impact, payload: Value) -> Frame {
        self.wire += 1;
        let ticket = self.ticket;
        self.ticket += 1;
        Frame::new(self.wire, ticket, op, impact, payload)
    }
    fn sql_frame(&mut self) -> Frame {
        let entry = self.seq.next_entry().unwrap();
        let expected = json!({"user":"SERVER_USER","session":"SERVER_SESSION"});
        let (impact, payload) = match entry.kind() {
            SqlKind::BootstrapContext => (Impact::Read, json!({})),
            SqlKind::Read(_) | SqlKind::Readback(_) => {
                let r = entry.retention().unwrap();
                (
                    Impact::Read,
                    json!({"sql":entry.sql(),"binds":[],"classification":"read","credits":1,"maxRows":r.max_rows,"maxBytes":r.max_bytes}),
                )
            }
            SqlKind::Ddl(kind) => (
                Impact::Mutating,
                json!({"kind":kind.as_str(),"sql":entry.sql(),"expected":expected}),
            ),
            SqlKind::Dml(kind) => (
                Impact::Mutating,
                json!({"kind":kind,"sql":entry.sql(),"binds":[],"expected":expected}),
            ),
            SqlKind::Transaction(action) => {
                let action = wire_action(action);
                (
                    action.impact(),
                    json!({"action":action.as_str(),"expected":expected}),
                )
            }
        };
        let op = entry.kind().operation();
        self.frame(op, impact, payload)
    }
    fn finish(&mut self, a: &Admission, r: &Report) -> TerminalRecord {
        self.seq.terminal(PhysicalTerminal {
            generation: &a.generation,
            operation_id: &a.logical_id,
            ticket: a.ticket,
            wire_id: &a.wire_id.to_string(),
            operation: &a.operation,
            impact: a.impact,
            report: r,
        })
    }
    fn sql_success(entry: &SqlEntry) -> Report {
        let mut r = report();
        r.phase = ProcessPhase::Acknowledged;
        r.certainty = Certainty::StatementSucceeded;
        r.statements = vec![statement()];
        r.result = Some(match entry.kind() {
            SqlKind::BootstrapContext => {
                json!({"user":"SERVER_USER","role":"ROLE","database":DATABASE,"schema":SCHEMA,"warehouse":"WH","session":"SERVER_SESSION","sdkRequestId":statement().sdk_request_id,"queryId":statement().query_id})
            }
            SqlKind::Read(_) | SqlKind::Readback(_) => json!({"rowsSeen":0}),
            SqlKind::Ddl(kind) => {
                r.ddl_acknowledged = Some(DdlAcknowledgement {
                    kind,
                    statement: statement(),
                });
                json!({"kind":kind.as_str()})
            }
            SqlKind::Dml(kind) => {
                r.dml_acknowledged = Some(DmlAcknowledgement {
                    kind,
                    statement: statement(),
                });
                json!({"kind":kind})
            }
            SqlKind::Transaction(action) => {
                let action = wire_action(action);
                r.boundary_acknowledged = Some(BoundaryAcknowledgement {
                    action,
                    statement: statement(),
                });
                json!({"action":action.as_str()})
            }
        });
        r
    }
    fn step(&mut self) {
        let entry = self.seq.next_entry().unwrap().clone();
        let frame = self.sql_frame();
        let a = self.seq.reserve(&frame.request()).unwrap();
        if usize::from(entry.ordinal()) == INDEPENDENT_PREFIX {
            self.seq.freeze_census(&[SCHEMA.into()]).unwrap();
        }
        assert!(
            self.finish(&a, &Self::sql_success(&entry))
                .error()
                .is_none()
        );
    }
    fn walk_to(&mut self, predicate: impl Fn(&SqlEntry) -> bool) {
        loop {
            if self.seq.phase_ready().is_ok() {
                self.seq.advance_phase().unwrap();
            } else if self.seq.next_entry().is_some_and(&predicate) {
                break;
            } else {
                self.step();
            }
        }
    }
    fn control(&mut self, owner: &Admission, op: &str, discard: bool) -> Frame {
        let payload = if op == "cancel" {
            json!({"target":owner.wire_id.to_string()})
        } else {
            json!({"target":owner.wire_id.to_string(),"batches":if discard {0}else{1},"discard":discard})
        };
        let mut frame = self.frame(op, Impact::Control, payload);
        frame.logical = format!("control-{}", frame.ticket);
        frame.target = Some((owner.logical_id.clone(), Some(owner.ticket)));
        frame
    }
}
fn wire_action(action: ExplicitTransactionAction) -> TransactionAction {
    match action {
        ExplicitTransactionAction::Begin => TransactionAction::Begin,
        ExplicitTransactionAction::Commit => TransactionAction::Commit,
        ExplicitTransactionAction::Rollback => TransactionAction::Rollback,
    }
}
fn rejected() -> Report {
    let mut r = report();
    r.ok = false;
    r.phase = ProcessPhase::Dispatching;
    r.result = None;
    r.error_code = Some("database".into());
    r
}

#[test]
fn sequence_completes_only_by_explicit_internal_phase_advancement() {
    let mut h = Harness::new();
    let mut phases = 0;
    while h.seq.phase().is_some() {
        while h.seq.phase_ready().is_err() {
            h.step();
        }
        phases += 1;
        h.seq.advance_phase().unwrap();
    }
    assert_eq!(phases, 32);
    assert_eq!(h.seq.index, 393);
    assert!(h.seq.next_entry().is_none());
    assert!(h.seq.advance_phase().is_err());
    let frame = h.frame("close", Impact::Control, json!({}));
    let a = h.seq.reserve(&frame.request()).unwrap();
    assert!(h.finish(&a, &report()).error().is_none());
    let frame = h.frame("close", Impact::Control, json!({}));
    assert!(h.seq.reserve(&frame.request()).is_err());
}

#[test]
fn sequence_with_restored_plan_retains_the_exact_closed_definition_and_ledger() {
    let original = Plan::generate().unwrap();
    let snapshot = original.canonical_json().unwrap();
    let hash = original.sha256().unwrap();
    let restored = Plan::restore_snapshot(&snapshot, &hash).unwrap();
    let mut h = Harness {
        seq: Sequencer::with_plan(restored, profile()).unwrap(),
        wire: 0,
        ticket: 0,
    };
    assert_eq!(h.seq.plan().canonical_json().unwrap(), snapshot);
    assert_eq!(h.seq.plan().run(), original.run());
    assert_eq!(h.seq.plan().table(), original.table());
    assert_eq!(h.seq.ledger().plan_sha256(), hash);
    assert_eq!(
        h.seq.ledger().canonical_json().unwrap(),
        original
            .expand_physical_ledger(&SchemaCensus::freeze(&[SCHEMA.into()]).unwrap())
            .unwrap()
            .canonical_json()
            .unwrap()
    );
    let connect = h.frame("connect", Impact::Control, profile());
    let admission = h.seq.reserve(&connect.request()).unwrap();
    assert!(h.finish(&admission, &report()).error().is_none());
    h.walk_to(|entry| entry.kind() == SqlKind::Ddl(SnowflakeDdlKind::CreateTable));
    assert_eq!(
        h.seq.next_entry().unwrap().sql(),
        original
            .statement(super::super::StatementId::Create)
            .wire_sql()
    );
    assert_eq!(h.seq.ledger().plan_sha256(), hash);
    assert_eq!(h.seq.plan().canonical_json().unwrap(), snapshot);
}

#[test]
fn sequence_with_plan_preserves_existing_frozen_profile_validation() {
    let mut invalid_profiles = vec![Value::Null, json!({})];
    for (field, value) in [
        ("account", json!("")),
        ("host", json!("bad\u{0}host")),
        ("username", json!("x".repeat(1021))),
        ("role", Value::Null),
        ("warehouse", json!(1)),
        ("database", json!("OTHER")),
        ("schema", json!("OTHER")),
        ("extra", json!("unexpected")),
    ] {
        let mut changed = profile();
        changed[field] = value;
        invalid_profiles.push(changed);
    }
    let mut missing = profile();
    missing.as_object_mut().unwrap().remove("username");
    invalid_profiles.push(missing);
    for profile in invalid_profiles {
        assert!(Sequencer::new(profile.clone()).is_err());
        assert!(Sequencer::with_plan(Plan::generate().unwrap(), profile).is_err());
    }
}

#[test]
fn sequence_requires_connect_once_before_sql_and_exact_frozen_profile() {
    let mut h = Harness {
        seq: Sequencer::new(profile()).unwrap(),
        wire: 0,
        ticket: 0,
    };
    let frame = h.sql_frame();
    assert!(h.seq.reserve(&frame.request()).is_err());
    let mut h = Harness::new();
    let frame = h.frame("connect", Impact::Control, profile());
    assert!(h.seq.reserve(&frame.request()).is_err());
    let mut h = Harness {
        seq: Sequencer::new(profile()).unwrap(),
        wire: 0,
        ticket: 0,
    };
    let mut other = profile();
    other["warehouse"] = json!("OTHER");
    let frame = h.frame("connect", Impact::Control, other);
    assert!(h.seq.reserve(&frame.request()).is_err());
}

#[test]
fn sequence_rejects_wire_ticket_and_generation_replay_but_allows_logical_reuse() {
    for which in ["wire", "ticket", "generation"] {
        let mut h = Harness::new();
        let mut frame = h.sql_frame();
        match which {
            "wire" => frame.value["id"] = json!("1"),
            "ticket" => frame.ticket = 0,
            _ => {
                frame.generation = "other".into();
                frame.value["generation"] = json!("other");
            }
        }
        frame.bytes = serde_json::to_vec(&frame.value).unwrap();
        assert!(h.seq.reserve(&frame.request()).is_err());
    }
    let mut h = Harness::new();
    h.step();
    h.step();
    h.seq.advance_phase().unwrap();
    // Every metadata read legitimately reuses the query logical handle.
    for _ in 0..5 {
        h.step();
    }
    assert!(h.seq.check().is_ok());
    // Priority queue ordering can put a newer-created ticket on the wire first.
    // Freshness is global uniqueness, not ticket monotonicity.
    let mut h = Harness::new();
    let mut frame = h.sql_frame();
    frame.ticket = 100;
    let a = h.seq.reserve(&frame.request()).unwrap();
    let r = Harness::sql_success(match a.kind() {
        AdmissionKind::Sql(entry) => entry,
        _ => unreachable!(),
    });
    assert!(h.finish(&a, &r).error().is_none());
    let frame = h.sql_frame();
    assert!(frame.ticket < 100);
    assert!(h.seq.reserve(&frame.request()).is_ok());
}

#[test]
fn sequence_placeholder_never_authorizes_post_schema_sql_without_real_census() {
    let mut h = Harness::new();
    h.walk_to(|e| usize::from(e.ordinal()) == INDEPENDENT_PREFIX);
    let frame = h.sql_frame();
    let a = h.seq.reserve(&frame.request()).unwrap();
    assert!(!h.seq.census_frozen());
    assert!(
        h.finish(
            &a,
            &Harness::sql_success(match a.kind() {
                AdmissionKind::Sql(e) => e,
                _ => unreachable!(),
            })
        )
        .error()
        .is_none()
    );
    let frame = h.sql_frame();
    assert!(h.seq.reserve(&frame.request()).is_err());
}

#[test]
fn sequence_freezes_ordered_real_census_after_reservation_preserving_entire_prefix() {
    let mut h = Harness::new();
    h.walk_to(|e| usize::from(e.ordinal()) == INDEPENDENT_PREFIX);
    let before = serde_json::to_value(&h.seq.ledger().entries()[..INDEPENDENT_PREFIX]).unwrap();
    let frame = h.sql_frame();
    let a = h.seq.reserve(&frame.request()).unwrap();
    let census = vec!["A".into(), SCHEMA.into()];
    h.seq.freeze_census(&census).unwrap();
    assert_eq!(h.seq.ledger().entries().len(), 400);
    assert!(h.seq.census_frozen());
    assert_eq!(
        serde_json::to_value(&h.seq.ledger().entries()[..INDEPENDENT_PREFIX]).unwrap(),
        before
    );
    h.seq.freeze_census(&census).unwrap();
    assert!(
        h.finish(
            &a,
            &Harness::sql_success(match a.kind() {
                AdmissionKind::Sql(e) => e,
                _ => unreachable!(),
            })
        )
        .error()
        .is_none()
    );
    let drift = vec!["AA".into(), SCHEMA.into()];
    assert!(h.seq.freeze_census(&drift).is_err());
}

#[test]
fn sequence_rejects_early_or_invalid_census_and_caps_schema_count() {
    let mut h = Harness::new();
    assert!(h.seq.freeze_census(&[SCHEMA.into()]).is_err());
    for census in [
        vec!["OTHER".into()],
        vec![SCHEMA.into(), "A".into()],
        vec![SCHEMA.into(), SCHEMA.into()],
        (0..32)
            .map(|i| format!("A{i:02}"))
            .chain(std::iter::once(SCHEMA.into()))
            .collect(),
    ] {
        let mut h = Harness::new();
        h.walk_to(|e| usize::from(e.ordinal()) == INDEPENDENT_PREFIX);
        let frame = h.sql_frame();
        h.seq.reserve(&frame.request()).unwrap();
        assert!(h.seq.freeze_census(&census).is_err());
    }
}

#[test]
fn sequence_controls_have_separate_terminals_and_do_not_complete_sql() {
    let mut h = Harness::new();
    h.step();
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    let frame = h.control(&sql, "credit", false);
    let credit = h.seq.reserve(&frame.request()).unwrap();
    assert!(h.finish(&credit, &report()).error().is_none());
    assert!(h.seq.active.is_some());
    assert_eq!(h.seq.next_entry().unwrap().ordinal(), 2);
    assert!(h.seq.phase_ready().is_err());
    let frame = h.control(&sql, "credit", false);
    let credit = h.seq.reserve(&frame.request()).unwrap();
    let r = Harness::sql_success(match sql.kind() {
        AdmissionKind::Sql(e) => e,
        _ => unreachable!(),
    });
    assert!(h.finish(&sql, &r).error().is_none());
    assert!(h.seq.phase_ready().is_err());
    assert!(h.finish(&credit, &report()).error().is_none());
    h.seq.phase_ready().unwrap();
    h.seq.advance_phase().unwrap();
}

#[test]
fn sequence_phase_and_active_control_barriers_reject_future_sql() {
    let mut h = Harness::new();
    h.step();
    h.step();
    let frame = h.sql_frame();
    assert!(h.seq.reserve(&frame.request()).is_err());
    let mut h = Harness::new();
    h.step();
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    let frame = h.control(&sql, "credit", false);
    let _credit = h.seq.reserve(&frame.request()).unwrap();
    let r = Harness::sql_success(match sql.kind() {
        AdmissionKind::Sql(e) => e,
        _ => unreachable!(),
    });
    h.finish(&sql, &r);
    let frame = h.sql_frame();
    assert!(h.seq.reserve(&frame.request()).is_err());
}

#[test]
fn sequence_controls_bind_exact_live_target_and_cancel_discard_once() {
    for (op, discard) in [("cancel", false), ("credit", true)] {
        let mut h = Harness::new();
        h.step();
        let frame = h.sql_frame();
        let sql = h.seq.reserve(&frame.request()).unwrap();
        let frame = h.control(&sql, op, discard);
        let control = h.seq.reserve(&frame.request()).unwrap();
        h.finish(&control, &report());
        let frame = h.control(&sql, op, discard);
        assert!(h.seq.reserve(&frame.request()).is_err());
    }
    let mut h = Harness::new();
    h.step();
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    let mut frame = h.control(&sql, "credit", false);
    frame.target = Some((sql.logical_id.clone(), Some(sql.ticket + 1)));
    assert!(h.seq.reserve(&frame.request()).is_err());
}

#[test]
fn sequence_bad_terminal_key_retains_original_report_and_cannot_clear_owner() {
    for which in [
        "generation",
        "logical",
        "ticket",
        "wire",
        "operation",
        "impact",
    ] {
        let mut h = Harness::new();
        let frame = h.sql_frame();
        let sql = h.seq.reserve(&frame.request()).unwrap();
        let r = Harness::sql_success(match sql.kind() {
            AdmissionKind::Sql(e) => e,
            _ => unreachable!(),
        });
        let wire = sql.wire_id.to_string();
        let mut terminal = PhysicalTerminal {
            generation: &sql.generation,
            operation_id: &sql.logical_id,
            ticket: sql.ticket,
            wire_id: &wire,
            operation: &sql.operation,
            impact: sql.impact,
            report: &r,
        };
        match which {
            "generation" => terminal.generation = "other",
            "logical" => terminal.operation_id = "other",
            "ticket" => terminal.ticket += 1,
            "wire" => terminal.wire_id = "9999",
            "operation" => terminal.operation = "query",
            _ => terminal.impact = Impact::Control,
        }
        let record = h.seq.terminal(terminal);
        assert!(record.admission().is_none());
        assert!(record.error().is_some());
        assert_eq!(record.report().result, r.result);
        assert!(h.seq.active.is_some());
        assert_eq!(
            record.record()["report"]["statements"][0]["sdkRequestId"],
            statement().sdk_request_id
        );
        let matched = h.finish(&sql, &r);
        assert!(matched.admission().is_some());
        assert!(matched.error().is_some());
        assert!(h.seq.active.is_none());
        let frame = h.frame("close", Impact::Control, json!({}));
        assert!(h.seq.reserve(&frame.request()).is_ok());
    }
}

#[test]
fn sequence_unmatched_terminal_preserves_every_original_report_field() {
    let mut seq = Sequencer::new(profile()).unwrap();
    let mut r = rejected();
    r.certainty = Certainty::OutcomeUnknown;
    r.result = Some(json!({"original":"retained"}));
    r.retirement = Some(Retirement::Protocol);
    r.statements = vec![statement()];
    r.use_acknowledged = Some(statement());
    r.boundary_acknowledged = Some(BoundaryAcknowledgement {
        action: TransactionAction::Commit,
        statement: statement(),
    });
    r.dml_acknowledged = Some(DmlAcknowledgement {
        kind: SnowflakeDmlKind::Update,
        statement: statement(),
    });
    r.ddl_acknowledged = Some(DdlAcknowledgement {
        kind: SnowflakeDdlKind::AddColumn,
        statement: statement(),
    });
    let record = seq.terminal(PhysicalTerminal {
        generation: GENERATION,
        operation_id: "unmatched",
        ticket: 0,
        wire_id: "1",
        operation: "ddl",
        impact: Impact::Mutating,
        report: &r,
    });
    assert!(record.admission().is_none());
    assert!(record.error().is_some());
    assert_eq!(record.report().result, r.result);
    assert_eq!(record.report().use_acknowledged, r.use_acknowledged);
    assert_eq!(
        record.report().boundary_acknowledged,
        r.boundary_acknowledged
    );
    assert_eq!(record.report().dml_acknowledged, r.dml_acknowledged);
    assert_eq!(record.report().ddl_acknowledged, r.ddl_acknowledged);
    let value = record.record();
    let value = &value["report"];
    assert_eq!(value.as_object().unwrap().len(), 11);
    assert_eq!(value["ok"], false);
    assert_eq!(value["certainty"], "OutcomeUnknown");
    assert_eq!(value["phase"], "Dispatching");
    assert_eq!(value["errorCode"], "database");
    assert_eq!(value["retirement"], "Protocol");
    assert_eq!(value["boundaryAcknowledged"]["action"], "commit");
    assert_eq!(value["dmlAcknowledged"]["kind"], "update");
    assert_eq!(value["ddlAcknowledged"]["kind"], "addColumn");
}

#[test]
fn sequence_bootstrap_identity_requires_six_strings_and_frozen_context() {
    for field in ["user", "session", "role", "database", "schema", "warehouse"] {
        let mut h = Harness::new();
        let frame = h.sql_frame();
        let sql = h.seq.reserve(&frame.request()).unwrap();
        let mut r = Harness::sql_success(match sql.kind() {
            AdmissionKind::Sql(e) => e,
            _ => unreachable!(),
        });
        r.result.as_mut().unwrap()[field] = if field == "user" || field == "session" {
            json!("")
        } else {
            json!("OTHER")
        };
        assert!(h.finish(&sql, &r).error().is_some());
        assert!(h.seq.identity().is_none());
    }
    let mut h = Harness::new();
    h.step();
    let identity = h.seq.identity().unwrap();
    assert_eq!(identity.user(), "SERVER_USER");
    assert_eq!(identity.session(), "SERVER_SESSION");
}

fn observed_context() -> crate::drivers::snowflake_metadata::LiveContext {
    let mut context = super::super::template_context();
    context.principal = "SERVER_USER".into();
    context.session_id = "SERVER_SESSION".into();
    context.role = Some("ROLE".into());
    context.warehouse = Some("WH".into());
    context
}

#[test]
fn sequence_context_requires_bootstrap_identity_and_preserves_first_failure() {
    let mut h = Harness::new();
    let context = observed_context();
    assert!(h.seq.validate_context(&context).is_err());
    h.step();
    // The server principal is deliberately different from the login username.
    h.seq.validate_context(&context).unwrap();
    h.seq.fail(AppError::new(
        ErrorCode::PolicyBlocked,
        "first context failure",
    ));
    let error = h.seq.validate_context(&context).unwrap_err();
    assert_eq!(error.code, ErrorCode::PolicyBlocked);
    assert_eq!(error.message, "first context failure");
}

#[test]
fn sequence_context_rejects_every_identity_and_frozen_context_drift() {
    let mut h = Harness::new();
    h.step();
    for field in [
        "principal",
        "session",
        "role",
        "warehouse",
        "database",
        "schema",
    ] {
        let mut context = observed_context();
        match field {
            "principal" => context.principal = "OTHER".into(),
            "session" => context.session_id = "OTHER".into(),
            "role" => context.role = Some("OTHER".into()),
            "warehouse" => context.warehouse = Some("OTHER".into()),
            "database" => context.database = Some("OTHER".into()),
            _ => context.schema = Some("OTHER".into()),
        }
        assert!(h.seq.validate_context(&context).is_err(), "{field}");
    }
    for field in ["role", "warehouse", "database", "schema"] {
        let mut context = observed_context();
        match field {
            "role" => context.role = None,
            "warehouse" => context.warehouse = None,
            "database" => context.database = None,
            _ => context.schema = None,
        }
        assert!(h.seq.validate_context(&context).is_err(), "{field}");
    }
}

#[test]
fn sequence_definitive_primary_rejections_allow_only_frozen_same_phase_suffix() {
    for (kind, count) in [("ddl", 5), ("readback", 10), ("dml", 0), ("transaction", 0)] {
        let mut h = Harness::new();
        h.walk_to(|e| {
            matches!(
                (kind, e.kind()),
                ("ddl", SqlKind::Ddl(_))
                    | ("readback", SqlKind::Readback(_))
                    | ("dml", SqlKind::Dml(_))
                    | ("transaction", SqlKind::Transaction(_))
            )
        });
        let frame = h.sql_frame();
        let sql = h.seq.reserve(&frame.request()).unwrap();
        assert!(h.finish(&sql, &rejected()).error().is_none());
        assert!(h.seq.check().is_ok());
        for _ in 0..count {
            h.step();
        }
        assert!(h.seq.phase_ready().is_err());
        assert!(h.seq.advance_phase().is_err());
        let frame = h.sql_frame();
        assert!(h.seq.reserve(&frame.request()).is_err());
    }
}

#[test]
fn sequence_failed_suffix_ambiguous_or_acknowledged_rejection_stops_without_losing_ack() {
    for reason in ["suffix", "retirement", "ack", "policy", "wrong-success-ack"] {
        let mut h = Harness::new();
        h.walk_to(|e| matches!(e.kind(), SqlKind::Ddl(_)));
        let frame = h.sql_frame();
        let mut sql = h.seq.reserve(&frame.request()).unwrap();
        let mut r = rejected();
        match reason {
            "suffix" => {
                assert!(h.finish(&sql, &r).error().is_none());
                let frame = h.sql_frame();
                sql = h.seq.reserve(&frame.request()).unwrap();
            }
            "retirement" => r.retirement = Some(Retirement::Eof),
            "ack" => {
                r.ddl_acknowledged = Some(DdlAcknowledgement {
                    kind: SnowflakeDdlKind::CreateTable,
                    statement: statement(),
                });
                r.statements = vec![statement()];
                r.certainty = Certainty::StatementSucceeded;
            }
            "policy" => h
                .seq
                .fail(AppError::new(ErrorCode::PolicyBlocked, "first failure")),
            _ => {
                r = Harness::sql_success(match sql.kind() {
                    AdmissionKind::Sql(e) => e,
                    _ => unreachable!(),
                });
                r.ddl_acknowledged.as_mut().unwrap().kind = SnowflakeDdlKind::DropTable;
            }
        }
        let record = h.finish(&sql, &r);
        assert!(record.error().is_some());
        assert!(h.seq.check().is_err());
        assert_eq!(record.report().ddl_acknowledged, r.ddl_acknowledged);
        assert_eq!(
            record.record()["report"]["retirement"],
            json!(r.retirement.map(|v| format!("{v:?}")))
        );
    }
}

#[test]
fn sequence_request_capacity_is_bounded_and_reservation_precedes_journal() {
    let mut h = Harness::new();
    h.step();
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    assert!(h.seq.pending.contains_key(&sql.wire_id));
    assert!(h.seq.tickets.contains(&sql.ticket));
    h.seq.fail(AppError::new(
        ErrorCode::Internal,
        "synthetic intent sync failure",
    ));
    assert!(h.seq.check().is_err());
    assert!(h.seq.pending.contains_key(&sql.wire_id));
    let mut h = Harness::new();
    h.seq.tickets = (100_000..100_000 + MAX_REQUESTS as u64).collect();
    let frame = h.sql_frame();
    assert!(h.seq.reserve(&frame.request()).is_err());
}

struct LocalFixture {
    generation: String,
    logical: String,
    ticket: u64,
    wire: Option<String>,
    candidate: Option<String>,
    operation: String,
    impact: Impact,
    target: Option<(String, Option<u64>)>,
    payload: Value,
    origin: LocalTerminalOrigin,
}

impl LocalFixture {
    fn inactive(frame: &Frame, admission: Option<&Admission>) -> Self {
        let mut payload = frame.value["payload"].clone();
        if let Some((logical, _)) = &frame.target {
            payload["target"] = json!(logical);
        }
        Self {
            generation: frame.generation.clone(),
            logical: frame.logical.clone(),
            ticket: frame.ticket,
            wire: None,
            candidate: admission.map(|admission| admission.wire_id.to_string()),
            operation: frame.operation.clone(),
            impact: frame.impact,
            target: frame.target.clone(),
            payload,
            origin: LocalTerminalOrigin::InactiveControl {
                admitted: admission.is_some(),
            },
        }
    }
    fn terminal<'a>(&'a self, report: &'a Report) -> LocalTerminal<'a> {
        LocalTerminal {
            generation: &self.generation,
            operation_id: &self.logical,
            ticket: self.ticket,
            wire_id: self.wire.as_deref(),
            candidate_wire_id: self.candidate.as_deref(),
            operation: &self.operation,
            impact: self.impact,
            target: self
                .target
                .as_ref()
                .map(|(logical, ticket)| (logical.as_str(), *ticket)),
            payload: &self.payload,
            origin: self.origin,
            report,
        }
    }
}

fn inactive_report() -> Report {
    let mut report = report();
    report.certainty = Certainty::NotSubmitted;
    report.phase = ProcessPhase::Queued;
    report.result = Some(json!({"inactive":true}));
    report
}

fn local_query(discard: bool, admitted: bool) -> (Harness, Admission, LocalFixture) {
    let mut h = Harness::new();
    h.step(); // Bootstrap context precedes the version query.
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    let frame = h.control(&sql, "credit", discard);
    let control = admitted.then(|| h.seq.reserve(&frame.request()).unwrap());
    let local = LocalFixture::inactive(&frame, control.as_ref());
    let AdmissionKind::Sql(entry) = sql.kind() else {
        unreachable!()
    };
    assert!(
        h.finish(&sql, &Harness::sql_success(entry))
            .error()
            .is_none()
    );
    (h, sql, local)
}

fn completed_query_control(discard: bool) -> (Harness, Admission, Frame) {
    let mut h = Harness::new();
    h.step();
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    // Process already prepared this exact target while it was live, then its
    // awaited policy work lets the helper terminal beat the reservation.
    let control = h.control(&sql, "credit", discard);
    let AdmissionKind::Sql(entry) = sql.kind() else {
        unreachable!()
    };
    assert!(
        h.finish(&sql, &Harness::sql_success(entry))
            .error()
            .is_none()
    );
    (h, sql, control)
}

#[test]
fn sequence_local_completed_before_reservation_is_inactive_only() {
    for discard in [false, true] {
        let (mut h, sql, frame) = completed_query_control(discard);
        let before = (h.seq.index, h.seq.phase_index);
        let admission = h.seq.reserve(&frame.request()).unwrap();
        assert!(matches!(
            admission.kind(),
            AdmissionKind::InactiveControl(_)
        ));
        assert!(h.seq.active.is_none());
        assert!(h.seq.phase_ready().is_err());
        let local = LocalFixture::inactive(&frame, Some(&admission));
        let original = inactive_report();
        let record = h.seq.local_terminal(local.terminal(&original));
        assert!(record.error().is_none());
        assert!(record.record()["wireId"].is_null());
        assert_eq!(
            record.record()["candidateWireId"],
            admission.wire_id.to_string()
        );
        assert_eq!(record.report().certainty, Certainty::NotSubmitted);
        assert_eq!(before, (h.seq.index, h.seq.phase_index));
        assert_eq!(h.seq.last_wire_id, admission.wire_id);
        assert!(h.seq.active.is_none() && h.seq.pending.is_empty());
        h.seq.phase_ready().unwrap();
        assert!(
            h.seq
                .local_terminal(local.terminal(&original))
                .error()
                .is_some()
        );
        assert!(!h.seq.pending.contains_key(&sql.wire_id));
    }
}

#[test]
fn sequence_local_inactive_only_reservation_rejects_helper_terminal() {
    let (mut h, _, frame) = completed_query_control(false);
    let admission = h.seq.reserve(&frame.request()).unwrap();
    let before = (h.seq.index, h.seq.phase_index);
    let forged = report();
    let recorded = h.finish(&admission, &forged);
    assert!(recorded.error().is_some());
    assert!(recorded.report().ok);
    assert_eq!(recorded.report().result, forged.result);
    assert_eq!(before, (h.seq.index, h.seq.phase_index));
    assert!(h.seq.active.is_none() && h.seq.pending.is_empty());
    assert!(h.seq.phase_ready().is_err());
}

#[test]
fn sequence_local_completed_reservation_rejects_wrong_owner_grammar_and_replay() {
    for changed in [
        "generation",
        "logical",
        "ticket",
        "wire",
        "phase",
        "cancel",
        "batches",
        "duplicate",
        "discarded",
    ] {
        let (mut h, sql, mut frame) = completed_query_control(false);
        match changed {
            "generation" => {
                frame.generation = "foreign-generation".into();
                frame.value["generation"] = json!(frame.generation);
            }
            "logical" => frame.target.as_mut().unwrap().0 = "foreign-query".into(),
            "ticket" => frame.target.as_mut().unwrap().1 = Some(sql.ticket + 100),
            "wire" => frame.value["payload"]["target"] = json!((sql.wire_id + 100).to_string()),
            "phase" => h.seq.advance_phase().unwrap(),
            "cancel" => {
                frame.operation = "cancel".into();
                frame.value["op"] = json!("cancel");
                frame.value["payload"] = json!({"target":sql.wire_id.to_string()});
            }
            "batches" => frame.value["payload"]["batches"] = json!(2),
            "duplicate" | "discarded" => {
                let mut first = if changed == "discarded" {
                    h.control(&sql, "credit", true)
                } else {
                    frame
                };
                // The duplicate case uses the original frame again after its
                // local completion; discard uses a fresh later candidate.
                first.bytes = serde_json::to_vec(&first.value).unwrap();
                let admission = h.seq.reserve(&first.request()).unwrap();
                let local = LocalFixture::inactive(&first, Some(&admission));
                assert!(
                    h.seq
                        .local_terminal(local.terminal(&inactive_report()))
                        .error()
                        .is_none()
                );
                frame = if changed == "duplicate" {
                    first
                } else {
                    h.control(&sql, "credit", false)
                };
            }
            _ => unreachable!(),
        }
        frame.bytes = serde_json::to_vec(&frame.value).unwrap();
        let before = (h.seq.index, h.seq.phase_index);
        assert!(h.seq.reserve(&frame.request()).is_err(), "{changed}");
        assert_eq!(before, (h.seq.index, h.seq.phase_index), "{changed}");
    }
}

#[test]
fn sequence_local_admitted_inactive_credit_and_discard_consume_only_their_reservation() {
    for discard in [false, true] {
        let (mut h, sql, local) = local_query(discard, true);
        let index = h.seq.index;
        let phase = h.seq.phase_index;
        let candidate = local.candidate.clone().unwrap();
        assert!(h.seq.phase_ready().is_err());
        let original = inactive_report();
        let record = h.seq.local_terminal(local.terminal(&original));
        assert!(record.error().is_none());
        assert!(matches!(
            record.admission().unwrap().kind(),
            AdmissionKind::Control(_)
        ));
        assert_eq!(record.report().result, original.result);
        let value = record.record();
        assert!(
            value["wireId"].is_null(),
            "candidate was never assigned for writing"
        );
        assert_eq!(value["candidateWireId"], candidate);
        assert_eq!(
            value["origin"],
            json!({"kind":"inactiveControl","admitted":true})
        );
        assert_eq!(value["payload"], local.payload);
        assert_eq!(value["target"], json!(local.target));
        assert_eq!(value["report"].as_object().unwrap().len(), 11);
        assert_eq!(h.seq.index, index);
        assert_eq!(h.seq.phase_index, phase);
        assert_eq!(h.seq.last_wire_id.to_string(), candidate);
        assert!(!h.seq.pending.contains_key(&sql.wire_id));
        assert!(h.seq.pending.is_empty());
        h.seq.phase_ready().unwrap();
    }
}

#[test]
fn sequence_local_unreserved_inactive_controls_consume_unique_tickets_without_wire_ids() {
    for discard in [false, true] {
        let (mut h, sql, local) = local_query(discard, false);
        assert!(!h.seq.tickets.contains(&local.ticket));
        let before = (h.seq.index, h.seq.phase_index, h.seq.last_wire_id);
        let record = h.seq.local_terminal(local.terminal(&inactive_report()));
        assert!(record.error().is_none());
        assert!(record.admission().is_none());
        assert!(record.record()["wireId"].is_null());
        assert!(record.record()["candidateWireId"].is_null());
        assert!(h.seq.tickets.contains(&local.ticket));
        assert_eq!(before, (h.seq.index, h.seq.phase_index, h.seq.last_wire_id));
        assert_eq!(h.seq.last_wire_id, sql.wire_id);
        h.seq.phase_ready().unwrap();
        assert!(
            h.seq
                .local_terminal(local.terminal(&inactive_report()))
                .error()
                .is_some()
        );
    }
}

#[test]
fn sequence_local_inactive_candidate_cannot_be_reused_but_next_wire_can() {
    for replay in [false, true] {
        let (mut h, _, local) = local_query(false, true);
        let candidate = local.candidate.clone().unwrap();
        assert!(
            h.seq
                .local_terminal(local.terminal(&inactive_report()))
                .error()
                .is_none()
        );
        h.seq.advance_phase().unwrap();
        let mut next = h.sql_frame();
        if replay {
            next.value["id"] = json!(candidate);
            next.bytes = serde_json::to_vec(&next.value).unwrap();
        }
        assert_eq!(h.seq.reserve(&next.request()).is_err(), replay);
    }
}

#[test]
fn sequence_local_inactive_checks_original_payload_identity_provenance_and_current_phase() {
    for changed in [
        "generation",
        "operation",
        "impact",
        "logical",
        "target",
        "targetTicket",
        "missingTicket",
        "missingTarget",
        "wire",
        "candidate",
        "admitted",
        "batches",
        "discard",
        "extraPayload",
        "payloadTarget",
        "oldTicket",
        "futurePhase",
        "capacity",
    ] {
        let (mut h, _, mut local) = local_query(false, false);
        match changed {
            "generation" => local.generation = "other-generation".into(),
            "operation" => local.operation = "cancel".into(),
            "impact" => local.impact = Impact::Read,
            "logical" => local.logical = "invalid\0logical".into(),
            "target" => local.target.as_mut().unwrap().0 = "another-query".into(),
            "targetTicket" => local.target.as_mut().unwrap().1 = Some(u64::MAX),
            "missingTicket" => local.target.as_mut().unwrap().1 = None,
            "missingTarget" => local.target = None,
            "wire" => local.wire = Some("4".into()),
            "candidate" => local.candidate = Some("4".into()),
            "admitted" => local.origin = LocalTerminalOrigin::InactiveControl { admitted: true },
            "batches" => local.payload["batches"] = json!(2),
            "discard" => local.payload["discard"] = json!(true),
            "extraPayload" => local.payload["extra"] = json!(true),
            "payloadTarget" => local.payload["target"] = json!("another-query"),
            "oldTicket" => local.ticket = 0, // The connect operation already consumed zero.
            "futurePhase" => h.seq.advance_phase().unwrap(),
            _ => {
                h.seq.tickets.extend(0..MAX_REQUESTS as u64);
                local.ticket = MAX_REQUESTS as u64;
            }
        }
        let index = h.seq.index;
        let record = h.seq.local_terminal(local.terminal(&inactive_report()));
        assert!(record.error().is_some(), "{changed}");
        assert!(h.seq.check().is_err(), "{changed}");
        assert_eq!(h.seq.index, index, "{changed}");
        assert_eq!(record.record()["payload"], local.payload);
    }
}

#[test]
fn sequence_local_inactive_requires_the_exact_unsubmitted_report_without_ack_or_sdk_evidence() {
    for changed in [
        "ok",
        "certainty",
        "phase",
        "result",
        "extraResult",
        "error",
        "retirement",
        "statements",
        "useAck",
        "boundaryAck",
        "dmlAck",
        "ddlAck",
    ] {
        let (mut h, _, local) = local_query(false, true);
        let mut original = inactive_report();
        match changed {
            "ok" => original.ok = false,
            "certainty" => original.certainty = Certainty::ServerTerminal,
            "phase" => original.phase = ProcessPhase::Sending,
            "result" => original.result = None,
            "extraResult" => original.result.as_mut().unwrap()["extra"] = json!(true),
            "error" => original.error_code = Some("cancelled".into()),
            "retirement" => original.retirement = Some(Retirement::Protocol),
            "statements" => original.statements.push(statement()),
            "useAck" => original.use_acknowledged = Some(statement()),
            "boundaryAck" => {
                original.boundary_acknowledged = Some(BoundaryAcknowledgement {
                    action: TransactionAction::Commit,
                    statement: statement(),
                })
            }
            "dmlAck" => {
                original.dml_acknowledged = Some(DmlAcknowledgement {
                    kind: SnowflakeDmlKind::Update,
                    statement: statement(),
                })
            }
            _ => {
                original.ddl_acknowledged = Some(DdlAcknowledgement {
                    kind: SnowflakeDdlKind::AddColumn,
                    statement: statement(),
                })
            }
        }
        let record = h.seq.local_terminal(local.terminal(&original));
        assert!(record.error().is_some(), "{changed}");
        assert!(
            record.admission().is_some(),
            "{changed}: exact owner retained"
        );
        assert!(
            h.seq.pending.is_empty(),
            "{changed}: owner consumed for cleanup"
        );
        assert_eq!(record.report().statements, original.statements);
        assert_eq!(record.report().use_acknowledged, original.use_acknowledged);
        assert_eq!(
            record.report().boundary_acknowledged,
            original.boundary_acknowledged
        );
        assert_eq!(record.report().dml_acknowledged, original.dml_acknowledged);
        assert_eq!(record.report().ddl_acknowledged, original.ddl_acknowledged);
        assert_eq!(record.record()["report"]["result"], json!(original.result));
    }
}

#[test]
fn sequence_local_admitted_identity_and_candidate_must_match_but_exact_owner_can_always_settle() {
    for changed in [
        "generation",
        "logical",
        "ticket",
        "operation",
        "impact",
        "candidate",
        "missingCandidate",
        "actualWire",
        "target",
        "origin",
    ] {
        let (mut h, _, mut local) = local_query(false, true);
        let key_matches = matches!(
            changed,
            "candidate" | "missingCandidate" | "actualWire" | "target" | "origin"
        );
        match changed {
            "generation" => local.generation = "wrong-generation".into(),
            "logical" => local.logical = "wrong-logical".into(),
            "ticket" => local.ticket += 99,
            "operation" => local.operation = "cancel".into(),
            "impact" => local.impact = Impact::Read,
            "candidate" => local.candidate = Some("999".into()),
            "missingCandidate" => local.candidate = None,
            "actualWire" => local.wire = local.candidate.clone(),
            "target" => local.target.as_mut().unwrap().0 = "other".into(),
            _ => local.origin = LocalTerminalOrigin::InactiveControl { admitted: false },
        }
        let record = h.seq.local_terminal(local.terminal(&inactive_report()));
        assert!(record.error().is_some(), "{changed}");
        assert_eq!(record.admission().is_some(), key_matches, "{changed}");
        assert_eq!(h.seq.pending.is_empty(), key_matches, "{changed}");
    }
}

#[test]
fn sequence_local_cancellation_close_and_retirement_retain_originals_and_never_complete_sql() {
    for origin in [
        LocalTerminalOrigin::QueuedCancellation,
        LocalTerminalOrigin::QueuedClose,
        LocalTerminalOrigin::Retirement,
    ] {
        let mut h = Harness::new();
        let frame = h.sql_frame();
        let sql = h.seq.reserve(&frame.request()).unwrap();
        let mut local = LocalFixture::inactive(&frame, Some(&sql));
        local.origin = origin;
        local.wire = Some(sql.wire_id.to_string());
        let mut original = rejected();
        original.certainty = Certainty::OutcomeUnknown;
        original.retirement = Some(Retirement::Protocol);
        original.ddl_acknowledged = Some(DdlAcknowledgement {
            kind: SnowflakeDdlKind::CreateTable,
            statement: statement(),
        });
        let index = h.seq.index;
        let record = h.seq.local_terminal(local.terminal(&original));
        assert!(record.error().is_some());
        assert!(record.admission().is_some());
        assert!(h.seq.active.is_none());
        assert!(h.seq.pending.is_empty());
        assert_eq!(h.seq.index, index);
        assert_eq!(record.report().ddl_acknowledged, original.ddl_acknowledged);
        let first = h.seq.check().unwrap_err();
        let repeated = h.seq.local_terminal(local.terminal(&inactive_report()));
        assert!(repeated.admission().is_none());
        assert_eq!(repeated.error().unwrap().message, first.message);
        assert!(h.seq.phase_ready().is_err());
    }
}

#[test]
fn sequence_local_unknown_uncompleted_and_failed_queries_cannot_authorize_inactive_controls() {
    for completed in [false, true] {
        let mut h = Harness::new();
        h.step();
        let frame = h.sql_frame();
        let sql = h.seq.reserve(&frame.request()).unwrap();
        let frame = h.control(&sql, "credit", false);
        let credit = h.seq.reserve(&frame.request()).unwrap();
        let local = LocalFixture::inactive(&frame, Some(&credit));
        if completed {
            assert!(h.finish(&sql, &rejected()).error().is_none());
        }
        let record = h.seq.local_terminal(local.terminal(&inactive_report()));
        assert!(record.error().is_some());
        assert!(record.admission().is_some());
        assert_eq!(h.seq.active.is_some(), !completed);
    }
    let mut seq = Sequencer::new(profile()).unwrap();
    let frame = Frame::new(
        1,
        0,
        "credit",
        Impact::Control,
        json!({"target":"unknown","batches":1,"discard":false}),
    );
    let mut local = LocalFixture::inactive(&frame, None);
    local.target = Some(("unknown".into(), Some(42)));
    let record = seq.local_terminal(local.terminal(&inactive_report()));
    assert!(record.admission().is_none());
    assert!(record.error().is_some());
    assert!(record.record()["wireId"].is_null());
}

#[test]
fn sequence_local_discard_is_once_per_completed_target_and_blocks_later_unreserved_credit() {
    for discard_again in [false, true] {
        let (mut h, sql, first) = local_query(true, false);
        assert!(
            h.seq
                .local_terminal(first.terminal(&inactive_report()))
                .error()
                .is_none()
        );
        let frame = h.control(&sql, "credit", discard_again);
        let next = LocalFixture::inactive(&frame, None);
        assert!(
            h.seq
                .local_terminal(next.terminal(&inactive_report()))
                .error()
                .is_some()
        );
    }
}

#[test]
fn sequence_local_unreserved_zero_ticket_is_valid_when_not_used_by_another_operation() {
    let mut h = Harness::new_from_ticket(10);
    h.step();
    let frame = h.sql_frame();
    let sql = h.seq.reserve(&frame.request()).unwrap();
    let mut frame = h.control(&sql, "credit", false);
    frame.ticket = 0;
    let local = LocalFixture::inactive(&frame, None);
    let AdmissionKind::Sql(entry) = sql.kind() else {
        unreachable!()
    };
    h.finish(&sql, &Harness::sql_success(entry));
    assert!(
        h.seq
            .local_terminal(local.terminal(&inactive_report()))
            .error()
            .is_none()
    );
    assert!(h.seq.tickets.contains(&0));
}
