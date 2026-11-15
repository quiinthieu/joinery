use super::super::{Plan, SchemaCensus};
use super::*;
use serde_json::json;

struct Frame {
    value: Value,
    bytes: Vec<u8>,
    operation: String,
    impact: Impact,
    ticket: u64,
}

impl Frame {
    fn new(operation: &str, impact: Impact, payload: Value) -> Self {
        let value = json!({"v":1,"generation":"synthetic-generation","id":"9",
            "op":operation,"payload":payload});
        let bytes = serde_json::to_vec(&value).unwrap();
        Self {
            value,
            bytes,
            operation: operation.into(),
            impact,
            ticket: 0,
        }
    }
    fn request(&self) -> PhysicalRequest<'_> {
        PhysicalRequest {
            generation: "synthetic-generation",
            operation_id: "logical-handle",
            ticket: self.ticket,
            operation: &self.operation,
            impact: self.impact,
            target: None,
            payload: &self.value,
            wire_payload: &self.bytes,
        }
    }
    fn change(&mut self, change: impl FnOnce(&mut Value)) {
        change(&mut self.value);
        self.bytes = serde_json::to_vec(&self.value).unwrap();
    }
}

fn identity() -> WireIdentity {
    WireIdentity::new("SYNTHETIC_USER".into(), "123456".into()).unwrap()
}

fn entries() -> Vec<SqlEntry> {
    Plan::generate()
        .unwrap()
        .expand_physical_ledger(&SchemaCensus::freeze(&["BILLIE_TESTING".into()]).unwrap())
        .unwrap()
        .entries()
        .to_vec()
}

fn frame(entry: &SqlEntry) -> Frame {
    let identity = identity();
    let expected = json!({"user":identity.user(),"session":identity.session()});
    let (impact, payload) = match entry.kind() {
        SqlKind::BootstrapContext => (Impact::Read, json!({})),
        SqlKind::Read(_) | SqlKind::Readback(_) => {
            let retention = entry.retention().unwrap();
            (
                Impact::Read,
                json!({"sql":entry.sql(),"binds":[],"classification":"read",
                "credits":1,"maxRows":retention.max_rows,"maxBytes":retention.max_bytes}),
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
            let action = match action {
                ExplicitTransactionAction::Begin => TransactionAction::Begin,
                ExplicitTransactionAction::Commit => TransactionAction::Commit,
                ExplicitTransactionAction::Rollback => TransactionAction::Rollback,
            };
            (
                action.impact(),
                json!({"action":action.as_str(),"expected":expected}),
            )
        }
    };
    Frame::new(entry.kind().operation(), impact, payload)
}

#[test]
fn every_closed_ledger_entry_matches_its_exact_production_shape() {
    let entries = entries();
    assert_eq!(entries.len(), 393);
    for entry in &entries {
        let frame = frame(entry);
        assert_eq!(validate_envelope(&frame.request()).unwrap(), 9);
        validate_sql(entry, &frame.request(), Some(&identity())).unwrap();
    }
}

#[test]
fn reviewed_console_transport_cannot_substitute_for_any_qualification_entry() {
    let identity = identity();
    for entry in entries() {
        let frame = Frame::new(
            "reviewedSql",
            Impact::Mutating,
            json!({"sql":entry.sql(),"binds":[],
                "expected":{"user":identity.user(),"session":identity.session()},
                "credits":1,"maxRows":1000,"maxBytes":65536}),
        );
        assert!(validate_envelope(&frame.request()).is_ok());
        assert!(validate_sql(&entry, &frame.request(), Some(&identity)).is_err());
    }
}

#[test]
fn envelope_rejects_field_id_generation_operation_and_byte_substitution() {
    let valid = || Frame::new("context", Impact::Read, json!({}));
    for (key, value) in [
        ("v", json!(2)),
        ("v", json!(1.0)),
        ("generation", json!("other")),
        ("op", json!("query")),
        ("id", json!("0")),
        ("id", json!("09")),
        ("id", json!("+9")),
        ("id", json!("18446744073709551616")),
        ("id", json!(9)),
        ("payload", json!([])),
        ("extra", json!(true)),
    ] {
        let mut frame = valid();
        frame.change(|value_| value_[key] = value);
        assert!(validate_envelope(&frame.request()).is_err(), "{key}");
    }
    let mut frame = valid();
    frame.change(|value| {
        value.as_object_mut().unwrap().remove("id");
    });
    assert!(validate_envelope(&frame.request()).is_err());
    let mut frame = valid();
    frame.bytes.push(b' ');
    assert!(validate_envelope(&frame.request()).is_err());
    let mut frame = valid();
    frame.bytes = serde_json::to_vec(&json!({"v":1})).unwrap();
    assert!(validate_envelope(&frame.request()).is_err());
    let frame = valid();
    let mut request = frame.request();
    request.operation_id = "";
    assert!(validate_envelope(&request).is_err());
}

#[test]
fn query_rejects_sql_binds_classification_credit_and_retention_drift() {
    let entry = entries()
        .into_iter()
        .find(|entry| matches!(entry.kind(), SqlKind::Read(_)))
        .unwrap();
    for (key, value) in [
        ("sql", json!(format!("{} ", entry.sql()))),
        ("binds", json!([1])),
        ("binds", json!({})),
        ("classification", json!("write")),
        ("credits", json!(2)),
        ("credits", json!("1")),
        ("maxRows", json!(0)),
        ("maxBytes", json!(0)),
        ("extra", json!(true)),
    ] {
        let mut frame = frame(&entry);
        frame.change(|payload| payload["payload"][key] = value);
        assert!(
            validate_sql(&entry, &frame.request(), None).is_err(),
            "{key}"
        );
    }
    let mut frame = frame(&entry);
    frame.change(|value| {
        value["payload"].as_object_mut().unwrap().remove("binds");
    });
    assert!(validate_sql(&entry, &frame.request(), None).is_err());
}

#[test]
fn all_sql_routes_reject_impact_target_and_operation_drift() {
    for entry in entries() {
        let mut frame = frame(&entry);
        let mut request = frame.request();
        request.target = Some(("another-logical-handle", Some(0)));
        assert!(validate_sql(&entry, &request, Some(&identity())).is_err());
        frame.impact = Impact::Control;
        assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
        frame.operation = "connect".into();
        frame.change(|value| value["op"] = json!("connect"));
        assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
    }
}

#[test]
fn typed_mutations_require_exact_identity_and_closed_expected_fields() {
    for entry in entries()
        .into_iter()
        .filter(|entry| !entry.kind().is_read())
    {
        let valid = frame(&entry);
        assert!(validate_sql(&entry, &valid.request(), None).is_err());
        for (key, value) in [
            ("user", json!("OTHER")),
            ("session", json!("OTHER")),
            ("extra", json!(true)),
        ] {
            let mut frame = frame(&entry);
            frame.change(|payload| payload["payload"]["expected"][key] = value);
            assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
        }
        let mut frame = frame(&entry);
        frame.change(|value| {
            value["payload"]["expected"]
                .as_object_mut()
                .unwrap()
                .remove("session");
        });
        assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
    }
}

#[test]
fn typed_mutations_reject_kind_action_sql_binds_and_extra_keys() {
    for entry in entries()
        .into_iter()
        .filter(|entry| !entry.kind().is_read())
    {
        let key = if matches!(entry.kind(), SqlKind::Transaction(_)) {
            "action"
        } else {
            "kind"
        };
        for (key, value) in [
            (key, json!("invalid")),
            ("sql", json!("SELECT 1")),
            ("extra", json!(true)),
        ] {
            let mut frame = frame(&entry);
            frame.change(|payload| payload["payload"][key] = value);
            assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
        }
        if matches!(entry.kind(), SqlKind::Dml(_)) {
            let mut frame = frame(&entry);
            frame.change(|payload| payload["payload"]["binds"] = json!(["changed"]));
            assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
        }
        let replacement = match entry.kind() {
            SqlKind::Ddl(crate::domain::SnowflakeDdlKind::CreateTable) => "dropTable",
            SqlKind::Ddl(_) => "createTable",
            SqlKind::Dml(crate::domain::SnowflakeDmlKind::Insert) => "delete",
            SqlKind::Dml(_) => "insert",
            SqlKind::Transaction(ExplicitTransactionAction::Begin) => "commit",
            SqlKind::Transaction(_) => "begin",
            _ => unreachable!(),
        };
        let mut frame = frame(&entry);
        frame.change(|payload| payload["payload"][key] = json!(replacement));
        if matches!(entry.kind(), SqlKind::Transaction(_)) {
            // Even a mutually consistent different action/impact must not match
            // the current ledger slot.
            frame.impact = if replacement == "begin" {
                Impact::Begin
            } else {
                Impact::Commit
            };
        }
        assert!(validate_sql(&entry, &frame.request(), Some(&identity())).is_err());
    }
}

#[test]
fn bootstrap_context_accepts_only_the_empty_inner_object() {
    let entry = entries()
        .into_iter()
        .find(|entry| matches!(entry.kind(), SqlKind::BootstrapContext))
        .unwrap();
    let mut frame = frame(&entry);
    validate_sql(&entry, &frame.request(), None).unwrap();
    frame.change(|payload| payload["payload"]["sql"] = json!(entry.sql()));
    assert!(validate_sql(&entry, &frame.request(), None).is_err());
}

#[test]
fn connect_binds_exact_frozen_profile_and_accepts_zero_ticket() {
    let profile = json!({"account":"synthetic","host":"synthetic.snowflakecomputing.com",
        "username":"SYNTHETIC_USER","role":"R","warehouse":"W","database":"SANDBOX","schema":"BILLIE_TESTING"});
    let mut frame = Frame::new("connect", Impact::Control, profile.clone());
    assert_eq!(frame.ticket, 0);
    validate_connect(&frame.request(), &profile).unwrap();
    frame.change(|value| value["payload"]["schema"] = json!("OTHER"));
    assert!(validate_connect(&frame.request(), &profile).is_err());
    frame.change(|value| value["payload"] = profile.clone());
    frame.change(|value| value["payload"]["extra"] = json!(true));
    assert!(validate_connect(&frame.request(), &profile).is_err());
    assert!(validate_connect(&frame.request(), &frame.value["payload"]).is_err());
    let frame = Frame::new("connect", Impact::Read, profile.clone());
    assert!(validate_connect(&frame.request(), &profile).is_err());
    let frame = Frame::new("connect", Impact::Control, profile.clone());
    let mut request = frame.request();
    request.target = Some(("query", None));
    assert!(validate_connect(&request, &profile).is_err());
}

#[test]
fn identity_constructor_enforces_helper_string_boundaries() {
    for (user, session) in [
        ("".into(), "1".into()),
        ("u".into(), "".into()),
        ("u\0".into(), "1".into()),
        ("u".into(), "1\0".into()),
        ("u".repeat(256), "1".into()),
        ("u".into(), "1".repeat(1025)),
    ] {
        assert!(WireIdentity::new(user, session).is_err());
    }
    WireIdentity::new("😀".repeat(255), "1".repeat(1024)).unwrap();
    assert!(WireIdentity::new("😀".repeat(256), "1".into()).is_err());
}
