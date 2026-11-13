use super::*;
use serde_json::json;

pub(super) fn payload(kind: &str) -> Value {
    let target = if matches!(
        kind,
        "context" | "quotedIdentifierPolicy" | "currentTransaction"
    ) {
        Value::Null
    } else {
        json!({"database":"DB","schema":"PUBLIC","name":"T","tableType":"BASE TABLE"})
    };
    json!({"owner":"11111111-1111-4111-8111-111111111111",
        "expected":{"user":"TEST","role":null,"database":"DB","schema":"PUBLIC","warehouse":"WH","session":"123"},
        "kind":kind,"target":target,"credits":1})
}

#[test]
fn owner_metadata_shape_is_closed_before_queue_admission() {
    assert!(ReadState::parse(&payload("context")).is_ok());
    for (path, value) in [
        ("kind", json!("query")),
        ("owner", json!("not-an-owner")),
        ("owner", json!("00000000-0000-0000-0000-000000000000")),
        ("credits", json!(0)),
        ("credits", json!(33)),
        ("credits", json!(1.5)),
        ("target", json!({})),
    ] {
        let mut request = payload("context");
        request[path] = value;
        assert!(ReadState::parse(&request).is_err(), "{path}");
    }
    for extra in ["sql", "binds", "maxRows", "maxBytes"] {
        let mut request = payload("columns");
        request[extra] = json!("untrusted");
        assert!(ReadState::parse(&request).is_err());
    }
    for nullable in ["role", "database", "schema", "warehouse"] {
        let mut request = payload("context");
        request["expected"]
            .as_object_mut()
            .unwrap()
            .remove(nullable);
        assert!(ReadState::parse(&request).is_err());
    }
}

#[test]
fn targeted_reads_bind_database_and_component_names_without_expressions() {
    for kind in ["exactRelation", "columns", "describe", "zeroRowResolution"] {
        let mut request = payload(kind);
        assert!(ReadState::parse(&request).is_ok());
        request["target"]["name"] = json!("quote\"; DROP TABLE X; --雪");
        // Components are data; only the helper's factory can turn them into SQL.
        assert!(ReadState::parse(&request).is_ok());
        request["target"]["schema"] = json!("雪".repeat(256));
        assert!(ReadState::parse(&request).is_err());
        request["target"]["schema"] = json!("PUBLIC");
        request["target"]["database"] = json!("OTHER");
        assert!(ReadState::parse(&request).is_err());
        request["target"]["database"] = json!("DB");
        request["target"]["tableType"] = json!("VIEW");
        assert!(ReadState::parse(&request).is_err());
    }
}

#[test]
fn owner_metadata_counts_and_terminal_cannot_invent_complete_rows() {
    let mut read = ReadState::parse(&payload("context")).unwrap();
    assert!(!read.batch(&json!([["too early"]])));
    assert!(read.columns(&json!([{"name":"PRINCIPAL","type":"TEXT"}])));
    assert!(!read.columns(&json!([])));
    assert!(read.batch(&json!([["TEST"]])));
    assert!(!read.batch(&json!([["extra"]])));
    assert!(read.terminal(&json!({"rowsSeen":1,"rowsRetained":1,"truncated":false})));
    for result in [
        json!({"rowsSeen":2,"rowsRetained":1,"truncated":false}),
        json!({"rowsSeen":0,"rowsRetained":1,"truncated":true}),
        json!({"rowsSeen":1,"rowsRetained":0,"truncated":false}),
        json!({"rowsSeen":1,"rowsRetained":1}),
        json!({"rowsSeen":1,"rowsRetained":1,"truncated":false,"sql":"hidden"}),
    ] {
        assert!(!read.terminal(&result));
    }
    // Honest truncation is transport evidence only, never full freshness.
    assert!(read.terminal(&json!({"rowsSeen":2,"rowsRetained":1,"truncated":true})));
}

#[test]
fn zero_row_and_column_byte_bounds_cannot_be_bypassed() {
    let mut read = ReadState::parse(&payload("zeroRowResolution")).unwrap();
    assert!(read.columns(&json!([{"name":"N","type":"FIXED"}])));
    assert!(!read.batch(&json!([["1"]])));
    assert!(read.terminal(&json!({"rowsSeen":0,"rowsRetained":0,"truncated":false})));
    for columns in [
        json!([{"name":"","type":"TEXT"}]),
        json!([{"name":"a","type":"TEXT","extra":1}]),
        json!([{"name":"a","type":"X".repeat(129)}]),
        json!([{"name":"a".repeat(4097),"type":"TEXT"}]),
        json!(
            (0..1001)
                .map(|_| json!({"name":"a","type":"TEXT"}))
                .collect::<Vec<_>>()
        ),
    ] {
        assert!(
            !ReadState::parse(&payload("columns"))
                .unwrap()
                .columns(&columns)
        );
    }
    let mut bytes = ReadState::parse(&payload("context")).unwrap();
    assert!(bytes.columns(&json!([{"name":"N","type":"TEXT"}])));
    assert!(!bytes.batch(&json!([["x".repeat(65536)]])));
}

#[test]
fn every_kind_uses_fixed_limits_and_overflow_sentinels() {
    for (kind, rows, bytes) in [
        ("context", 1, 65536),
        ("quotedIdentifierPolicy", 1, 65536),
        ("currentTransaction", 1, 65536),
        ("exactRelation", 3, 2097152),
        ("columns", 1001, 1048576),
        ("describe", 1001, 1048576),
        ("zeroRowResolution", 0, 1048576),
    ] {
        let read = ReadState::parse(&payload(kind)).unwrap();
        assert_eq!((read.max_rows, read.max_bytes), (rows, bytes));
    }
}
