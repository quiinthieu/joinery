use super::*;
use serde_json::json;

struct Frame {
    value: Value,
    bytes: Vec<u8>,
    operation: &'static str,
    impact: Impact,
}

impl Frame {
    fn new(operation: &'static str, payload: Value) -> Self {
        let value = json!({"v":1,"generation":"synthetic-generation","id":"9",
            "op":operation,"payload":payload});
        let bytes = serde_json::to_vec(&value).unwrap();
        Self {
            value,
            bytes,
            operation,
            impact: Impact::Control,
        }
    }
    fn request(&self) -> PhysicalRequest<'_> {
        PhysicalRequest {
            generation: "synthetic-generation",
            operation_id: "control-1",
            ticket: 0,
            operation: self.operation,
            impact: self.impact,
            target: if self.operation == "close" {
                None
            } else {
                Some(("query-owner", Some(0)))
            },
            payload: &self.value,
            wire_payload: &self.bytes,
        }
    }
    fn change(&mut self, change: impl FnOnce(&mut Value)) {
        change(&mut self.value);
        self.bytes = serde_json::to_vec(&self.value).unwrap();
    }
}

fn target(operation: &str) -> ControlTarget<'_> {
    ControlTarget {
        logical_id: "query-owner",
        ticket: 0,
        wire_id: 3,
        operation,
    }
}

fn credit(discard: bool) -> Frame {
    Frame::new(
        "credit",
        json!({"target":"3","batches":if discard { 0 } else { 1 },"discard":discard}),
    )
}

#[test]
fn exact_credit_and_discard_accept_zero_tickets_and_separate_wire_ids() {
    for (discard, kind) in [(false, ControlKind::Credit), (true, ControlKind::Discard)] {
        let frame = credit(discard);
        assert_eq!(frame.request().ticket, 0);
        assert_eq!(
            validate_control(&frame.request(), Some(target("query")), false).unwrap(),
            kind
        );
    }
}

#[test]
fn credit_rejects_extra_missing_numeric_boolean_and_batch_drift() {
    for discard in [false, true] {
        for (key, value) in [
            ("extra", json!(true)),
            ("batches", json!(2)),
            ("batches", json!(-1)),
            ("batches", json!("1")),
            ("batches", json!(1.0)),
            ("discard", json!("false")),
            ("discard", json!(!discard)),
        ] {
            let mut frame = credit(discard);
            frame.change(|value_| value_["payload"][key] = value);
            assert!(
                validate_control(&frame.request(), Some(target("query")), false).is_err(),
                "{key}"
            );
        }
        for key in ["target", "batches", "discard"] {
            let mut frame = credit(discard);
            frame.change(|value| {
                value["payload"].as_object_mut().unwrap().remove(key);
            });
            assert!(validate_control(&frame.request(), Some(target("query")), false).is_err());
        }
        let mut frame = credit(discard);
        frame.change(|value| value["payload"]["batches"] = json!(if discard { 1 } else { 0 }));
        assert!(validate_control(&frame.request(), Some(target("query")), false).is_err());
    }
}

#[test]
fn target_must_match_logical_ticket_and_canonical_resolved_wire_id() {
    let frame = credit(false);
    assert!(validate_control(&frame.request(), None, false).is_err());
    for owned in [
        ControlTarget {
            logical_id: "other-owner",
            ..target("query")
        },
        ControlTarget {
            ticket: 1,
            ..target("query")
        },
        ControlTarget {
            wire_id: 4,
            ..target("query")
        },
        ControlTarget {
            wire_id: 0,
            ..target("query")
        },
        ControlTarget {
            logical_id: "",
            ..target("query")
        },
    ] {
        assert!(validate_control(&frame.request(), Some(owned), false).is_err());
    }
    let mut request = frame.request();
    request.target = None;
    assert!(validate_control(&request, Some(target("query")), false).is_err());
    request.target = Some(("query-owner", None));
    assert!(validate_control(&request, Some(target("query")), false).is_err());
    for value in [
        json!("03"),
        json!("+3"),
        json!(3),
        json!("query-owner"),
        json!(null),
    ] {
        let mut frame = credit(false);
        frame.change(|payload| payload["payload"]["target"] = value);
        assert!(validate_control(&frame.request(), Some(target("query")), false).is_err());
    }
}

#[test]
fn credit_and_discard_only_target_query() {
    for operation in [
        "connect",
        "context",
        "switchSchema",
        "transaction",
        "dml",
        "ddl",
        "credit",
        "cancel",
        "close",
        "unknown",
    ] {
        for discard in [false, true] {
            let frame = credit(discard);
            assert!(validate_control(&frame.request(), Some(target(operation)), false).is_err());
        }
    }
}

#[test]
fn cancel_uses_production_closed_target_set_and_exact_payload() {
    let frame = Frame::new("cancel", json!({"target":"3"}));
    for operation in [
        "query",
        "connect",
        "switchSchema",
        "transaction",
        "dml",
        "ddl",
    ] {
        assert_eq!(
            validate_control(&frame.request(), Some(target(operation)), false).unwrap(),
            ControlKind::Cancel
        );
    }
    for operation in ["context", "credit", "cancel", "close", "unknown", ""] {
        assert!(validate_control(&frame.request(), Some(target(operation)), false).is_err());
    }
    for payload in [
        json!({}),
        json!({"target":"3","batches":0}),
        json!({"target":"4"}),
        json!({"target":3}),
    ] {
        let frame = Frame::new("cancel", payload);
        assert!(validate_control(&frame.request(), Some(target("query")), false).is_err());
    }
}

#[test]
fn close_requires_explicit_permission_no_targets_and_empty_payload() {
    let frame = Frame::new("close", json!({}));
    assert_eq!(
        validate_control(&frame.request(), None, true).unwrap(),
        ControlKind::Close
    );
    assert!(validate_control(&frame.request(), None, false).is_err());
    assert!(validate_control(&frame.request(), Some(target("query")), true).is_err());
    let mut request = frame.request();
    request.target = Some(("query-owner", Some(0)));
    assert!(validate_control(&request, None, true).is_err());
    let frame = Frame::new("close", json!({"target":"3"}));
    assert!(validate_control(&frame.request(), None, true).is_err());
}

#[test]
fn every_control_requires_control_impact_and_the_canonical_envelope() {
    for mut frame in [
        credit(false),
        credit(true),
        Frame::new("cancel", json!({"target":"3"})),
        Frame::new("close", json!({})),
    ] {
        let owned = if frame.operation == "close" {
            None
        } else {
            Some(target("query"))
        };
        for impact in [
            Impact::Read,
            Impact::Mutating,
            Impact::Begin,
            Impact::Commit,
            Impact::Rollback,
            Impact::ContextChange,
        ] {
            frame.impact = impact;
            assert!(validate_control(&frame.request(), owned, true).is_err());
        }
        frame.impact = Impact::Control;
        frame.bytes.push(b' ');
        assert!(validate_control(&frame.request(), owned, true).is_err());
        frame.change(|value| value["generation"] = json!("other-generation"));
        assert!(validate_control(&frame.request(), owned, true).is_err());
    }
}

#[test]
fn sql_and_invented_control_operations_are_rejected() {
    for operation in [
        "query",
        "context",
        "transaction",
        "dml",
        "ddl",
        "connect",
        "discard",
        "unknown",
    ] {
        let frame = Frame::new(operation, json!({"target":"3"}));
        assert!(validate_control(&frame.request(), Some(target("query")), true).is_err());
    }
}
