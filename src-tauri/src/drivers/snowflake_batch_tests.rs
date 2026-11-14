use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;

fn payload(count: usize, mode: Mode, owner: Option<Uuid>) -> Value {
    json!({"mode":mode,"owner":owner,"rollbackPrior":owner.is_some(),
        "expected":{"user":"TEST","session":"123","role":"ROLE","database":"DB","schema":"PUBLIC","warehouse":"WH"},
        "units":(0..count).map(|_|json!({"kind":"update","sql":"UPDATE synthetic SET value = ? WHERE id = ?","binds":["private-value","1"]})).collect::<Vec<_>>()})
}
fn evidence(count: usize) -> BatchEvidence {
    BatchEvidence::new(
        BatchRequest::parse("mutationBatch", &payload(count, Mode::Automatic, None))
            .unwrap()
            .unwrap(),
    )
}
fn frame(extra: Value) -> Value {
    let mut event = json!({"v":1,"generation":"generation","id":"operation"});
    event
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    event
}
fn phase(step: usize, role: Role, unit: Option<usize>, phase: StepPhase) -> Value {
    frame(json!({"type":"batchStep","step":step,"role":role,"unit":unit,"phase":phase}))
}
fn identity(step: usize, query: bool) -> Value {
    let sdk = Uuid::from_u128(0x12340000000040008000000000000000 + step as u128);
    let mut event = frame(json!({"type":"batchStatement","step":step,"sdkRequestId":sdk}));
    if query {
        event["queryId"] = json!(Uuid::from_u128(
            0x56780000000040008000000000000000 + step as u128
        ));
    }
    event
}
fn complete(
    evidence: &mut BatchEvidence,
    role: Role,
    unit: Option<usize>,
    counts: Option<(&str, Option<&str>)>,
) {
    let step = evidence.steps.len() + 1;
    assert!(evidence.apply_event(&phase(step, role, unit, StepPhase::Dispatching)));
    assert!(evidence.apply_event(&identity(step, false)));
    assert!(evidence.apply_event(&phase(step, role, unit, StepPhase::Acknowledged)));
    assert!(evidence.apply_event(&identity(step, true)));
    if let Some((affected, multi)) = counts {
        assert!(
            evidence.apply_event(&frame(json!({"type":"batchCount","step":step,"unit":unit,
            "affectedRows":affected,"multiJoinedRows":multi})))
        );
    }
    assert!(evidence.apply_event(&phase(step, role, unit, StepPhase::Drained)));
}
fn preflight(evidence: &mut BatchEvidence) {
    for role in [
        Role::Context,
        Role::Settings,
        Role::Begin,
        Role::Transaction,
    ] {
        complete(evidence, role, None, None);
    }
}
fn summary(outcome: Outcome, failure: Option<Failure>) -> Value {
    json!({"outcome":outcome,"owner":null,"transactionId":"1234567890123456789","failure":failure,"priorBatches":0})
}

fn largest_summary(request: &BatchRequest, outcome: Outcome, failure: Option<Failure>) -> Value {
    let owner = match request {
        BatchRequest::Batch {
            mode: Mode::Automatic,
            ..
        } => None,
        _ => Some(
            request
                .owner()
                .unwrap_or_else(|| Uuid::from_u128(0x99990000000040008000000000000000)),
        ),
    };
    json!({"outcome":outcome,"owner":owner,"transactionId":i64::MIN.to_string(),
        "failure":failure,"priorBatches":MAX_MANUAL_BATCHES})
}

fn assert_capture_fits_request(evidence: &BatchEvidence) -> Vec<u8> {
    let bound = BatchCapture::maximum_encoded_len(&evidence.request).unwrap();
    let base64_bound = BatchCapture::maximum_base64_len(&evidence.request).unwrap();
    let bytes = BatchCapture::encode(evidence).unwrap();
    assert!(bytes.len() <= bound, "{} exceeds {bound}", bytes.len());
    assert!(STANDARD.encode(&bytes).len() <= base64_bound);
    assert_eq!(base64_bound, bound.div_ceil(3) * 4);
    assert_eq!(BatchCapture::decode(&bytes).unwrap(), *evidence);
    bytes
}

#[test]
fn batch_request_rejects_unbounded_or_ambiguous_payload_before_dispatch() {
    for count in [0, MAX_UNITS + 1] {
        assert!(
            BatchRequest::parse("mutationBatch", &payload(count, Mode::Automatic, None)).is_err()
        );
    }
    let valid = payload(1, Mode::Automatic, None);
    for (field, value) in [
        ("mode", json!("unknown")),
        ("owner", json!(Uuid::new_v4())),
        ("rollbackPrior", json!(true)),
        ("extra", json!(null)),
    ] {
        let mut malformed = valid.clone();
        malformed[field] = value;
        assert!(BatchRequest::parse("mutationBatch", &malformed).is_err());
    }
    for unit in [
        json!({"kind":"merge","sql":"x","binds":[]}),
        json!({"kind":"insert","sql":" ","binds":[]}),
        json!({"kind":"insert","sql":"x","binds":[1]}),
        json!({"kind":"insert","sql":"x","binds":[{}]}),
        json!({"kind":"insert","sql":"x","binds":vec![Value::Null;2001]}),
        json!({"kind":"insert","sql":"x".repeat(786_433),"binds":[]}),
    ] {
        let mut malformed = valid.clone();
        malformed["units"] = json!([unit]);
        assert!(BatchRequest::parse("mutationBatch", &malformed).is_err());
    }
    let mut max_wide = valid.clone();
    max_wide["units"] = json!([{
        "kind": "update",
        "sql": "x".repeat(786_432),
        "binds": vec![Value::Null; 2000]
    }]);
    assert!(BatchRequest::parse("mutationBatch", &max_wide).is_ok());
    assert!(BatchRequest::parse("query", &valid).unwrap().is_none());
    let owner = Uuid::new_v4();
    assert!(BatchRequest::parse("mutationBatch", &payload(1, Mode::Manual, Some(owner))).is_ok());
    let finish = json!({"owner":owner,"action":"rollback","expected":valid["expected"]});
    assert!(matches!(
        BatchRequest::parse("mutationFinish", &finish).unwrap(),
        Some(BatchRequest::Finish { .. })
    ));
}

#[test]
fn batch_frames_validate_roles_associations_order_and_query_upgrades_atomically() {
    let mut evidence = evidence(2);
    let pristine = evidence.clone();
    assert!(!evidence.apply_event(&phase(1, Role::Unit, Some(0), StepPhase::Dispatching)));
    assert_eq!(evidence, pristine);
    preflight(&mut evidence);
    let step = evidence.steps.len() + 1;
    assert!(evidence.apply_event(&phase(step, Role::Unit, Some(0), StepPhase::Dispatching)));
    let before = evidence.clone();
    assert!(!evidence.apply_event(&phase(step, Role::Unit, Some(0), StepPhase::Drained)));
    assert_eq!(evidence, before);
    assert!(evidence.apply_event(&identity(step, false)));
    assert!(!evidence.apply_event(&identity(step, true))); // upgrade only after ACK
    assert!(evidence.apply_event(&phase(step, Role::Unit, Some(0), StepPhase::Acknowledged)));
    assert!(evidence.apply_event(&identity(step, true)));
    let before = evidence.clone();
    for invalid in [
        frame(
            json!({"type":"batchCount","step":step,"unit":1,"affectedRows":"1","multiJoinedRows":"0"}),
        ),
        frame(
            json!({"type":"batchCount","step":step,"unit":0,"affectedRows":1,"multiJoinedRows":"0"}),
        ),
        frame(
            json!({"type":"batchCount","step":step,"unit":0,"affectedRows":"01","multiJoinedRows":"0"}),
        ),
        frame(
            json!({"type":"batchCount","step":step,"unit":0,"affectedRows":"1","multiJoinedRows":"1e0"}),
        ),
        phase(step, Role::Unit, Some(0), StepPhase::Drained),
        identity(step, true),
    ] {
        assert!(!evidence.apply_event(&invalid));
        assert_eq!(evidence, before);
    }
    assert!(evidence.apply_event(&frame(
        json!({"type":"batchCount","step":step,"unit":0,"affectedRows":"1","multiJoinedRows":"0"})
    )));
    assert!(evidence.apply_event(&phase(step, Role::Unit, Some(0), StepPhase::Drained)));
    assert!(!evidence.apply_event(&identity(step, true)));
    let next = step + 1;
    assert!(evidence.apply_event(&phase(next, Role::Unit, Some(1), StepPhase::Dispatching)));
    let mut duplicate = identity(next, false);
    duplicate["sdkRequestId"] = identity(step, false)["sdkRequestId"].clone();
    assert!(!evidence.apply_event(&duplicate));
}

#[test]
fn known_disposition_requires_unit_counts_and_exact_post_boundary_observation() {
    let mut evidence = evidence(1);
    assert!(!evidence.accept_terminal(&summary(Outcome::Committed, None)));
    preflight(&mut evidence);
    complete(&mut evidence, Role::Unit, Some(0), Some(("1", Some("0"))));
    complete(&mut evidence, Role::Verify, None, None);
    complete(&mut evidence, Role::Commit, None, None);
    complete(&mut evidence, Role::Idle, None, None);
    assert!(!evidence.accept_terminal(&summary(Outcome::Committed, None)));
    complete(&mut evidence, Role::Disposition, None, None);
    assert!(evidence.accept_terminal(&summary(Outcome::Committed, None)));
    assert!(!evidence.accept_terminal(&summary(Outcome::Committed, None)));
    let bytes = BatchCapture::encode(&evidence).unwrap();
    assert_eq!(BatchCapture::decode(&bytes).unwrap(), evidence);
    for end in 0..bytes.len() {
        assert!(BatchCapture::decode(&bytes[..end]).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(BatchCapture::decode(&trailing).is_err());
}

#[test]
fn cardinality_failure_stops_later_units_and_preserves_exact_counts() {
    let mut evidence = evidence(2);
    preflight(&mut evidence);
    let huge = "99999999999999999999999999999999999999";
    complete(&mut evidence, Role::Unit, Some(0), Some((huge, Some("0"))));
    assert!(!evidence.apply_event(&phase(6, Role::Unit, Some(1), StepPhase::Dispatching)));
    assert!(!evidence.apply_event(&phase(6, Role::Verify, None, StepPhase::Dispatching)));
    for role in [Role::Rollback, Role::Idle, Role::Disposition] {
        complete(&mut evidence, role, None, None);
    }
    assert!(evidence.accept_terminal(&summary(Outcome::RolledBack, Some(Failure::Cardinality))));
    assert_eq!(
        BatchCapture::decode(&BatchCapture::encode(&evidence).unwrap()).unwrap(),
        evidence
    );
}

#[test]
fn definite_sql_rejection_can_verify_rollback_without_inventing_unit_ack() {
    let mut evidence = evidence(1);
    preflight(&mut evidence);
    assert!(evidence.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Dispatching)));
    assert!(evidence.apply_event(&identity(5, false)));
    for role in [Role::Rollback, Role::Idle, Role::Disposition] {
        complete(&mut evidence, role, None, None);
    }
    assert!(!evidence.accept_terminal(&summary(Outcome::RolledBack, Some(Failure::Cardinality))));
    assert!(evidence.accept_terminal(&summary(Outcome::RolledBack, Some(Failure::Database))));
    assert_eq!(evidence.steps[4].phase, StepPhase::Dispatching);
    assert!(evidence.steps[4].count.is_none());
    assert_eq!(
        BatchCapture::decode(&BatchCapture::encode(&evidence).unwrap()).unwrap(),
        evidence
    );
}

#[test]
fn callback_ack_without_request_id_is_retained_as_an_incomplete_prefix() {
    let mut evidence = evidence(2);
    preflight(&mut evidence);
    assert!(evidence.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Dispatching)));
    assert!(evidence.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Acknowledged)));
    let original = evidence.clone();
    for invalid in [
        phase(5, Role::Unit, Some(0), StepPhase::Drained),
        phase(6, Role::Unit, Some(1), StepPhase::Dispatching),
        phase(6, Role::Rollback, None, StepPhase::Dispatching),
        frame(
            json!({"type":"batchCount","step":5,"unit":0,"affectedRows":"1","multiJoinedRows":null}),
        ),
        frame(json!({"type":"batchCountUnavailable","step":5,"unit":0})),
    ] {
        assert!(!evidence.apply_event(&invalid));
        assert_eq!(evidence, original);
    }
    assert_eq!(
        BatchCapture::decode(&BatchCapture::encode(&evidence).unwrap()).unwrap(),
        original
    );
    assert!(evidence.accept_terminal(&summary(Outcome::Unknown, Some(Failure::Observer))));
    assert_eq!(
        BatchCapture::decode(&BatchCapture::encode(&evidence).unwrap()).unwrap(),
        evidence
    );
    assert!(evidence.steps[4].statement.is_none());
    let mut attached = original;
    assert!(attached.apply_event(&identity(5, false)));
    assert!(attached.apply_event(&identity(5, true)));
    assert!(attached.apply_event(&frame(
        json!({"type":"batchCount","step":5,"unit":0,"affectedRows":"1","multiJoinedRows":null})
    )));
    assert!(attached.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Drained)));
}

#[test]
fn unavailable_drained_count_allows_only_verified_rollback_and_preserves_absence() {
    let mut evidence = evidence(2);
    preflight(&mut evidence);
    assert!(evidence.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Dispatching)));
    assert!(evidence.apply_event(&identity(5, false)));
    assert!(evidence.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Acknowledged)));
    assert!(evidence.apply_event(&frame(
        json!({"type":"batchCountUnavailable","step":5,"unit":0})
    )));
    assert!(evidence.apply_event(&phase(5, Role::Unit, Some(0), StepPhase::Drained)));
    assert!(!evidence.apply_event(&phase(6, Role::Unit, Some(1), StepPhase::Dispatching)));
    assert!(!evidence.apply_event(&phase(6, Role::Verify, None, StepPhase::Dispatching)));
    for role in [Role::Rollback, Role::Idle, Role::Disposition] {
        complete(&mut evidence, role, None, None);
    }
    assert!(!evidence.accept_terminal(&summary(Outcome::RolledBack, Some(Failure::Cardinality))));
    assert!(evidence.accept_terminal(&summary(
        Outcome::RolledBack,
        Some(Failure::SummaryUnavailable)
    )));
    assert_capture_fits_request(&evidence);
}

#[test]
fn optional_update_multi_count_preserves_absence_and_present_nonzero_conflicts() {
    for multi in [None, Some("0"), Some("1")] {
        let mut evidence = evidence(1);
        preflight(&mut evidence);
        complete(&mut evidence, Role::Unit, Some(0), Some(("1", multi)));
        if multi == Some("1") {
            assert!(!evidence.apply_event(&phase(6, Role::Verify, None, StepPhase::Dispatching)));
            for role in [Role::Rollback, Role::Idle, Role::Disposition] {
                complete(&mut evidence, role, None, None);
            }
            assert!(
                evidence.accept_terminal(&summary(Outcome::RolledBack, Some(Failure::Cardinality)))
            );
        } else {
            for role in [Role::Verify, Role::Commit, Role::Idle, Role::Disposition] {
                complete(&mut evidence, role, None, None);
            }
            for invalid in [
                json!({"outcome":"committed"}),
                json!({"outcome":"committed","owner":null,"transactionId":"123","failure":null,"priorBatches":1.5}),
                json!({"outcome":"committed","owner":null,"transactionId":"0123","failure":null,"priorBatches":0}),
                json!({"outcome":"committed","owner":null,"transactionId":"123","failure":null,"priorBatches":65537}),
            ] {
                assert!(!evidence.accept_terminal(&invalid));
                assert!(evidence.summary.is_none());
            }
            assert!(evidence.accept_terminal(&summary(Outcome::Committed, None)));
        }
        assert_eq!(
            evidence.steps[4]
                .count
                .as_ref()
                .unwrap()
                .multi_joined_rows
                .as_deref(),
            multi
        );
        assert_eq!(
            BatchCapture::decode(&BatchCapture::encode(&evidence).unwrap()).unwrap(),
            evidence
        );
    }
}

#[test]
fn maximum_batch_capture_keeps_every_uuid_pair_and_exact_failed_counts_below_32k_base64() {
    let mut evidence = evidence(MAX_UNITS);
    assert_eq!(
        BatchCapture::maximum_encoded_len(&evidence.request).unwrap(),
        21_819
    );
    assert_eq!(
        BatchCapture::maximum_base64_len(&evidence.request).unwrap(),
        29_092
    );
    preflight(&mut evidence);
    let huge = "99999999999999999999999999999999999999";
    for unit in 0..MAX_UNITS {
        complete(
            &mut evidence,
            Role::Unit,
            Some(unit),
            Some(if unit == MAX_UNITS - 1 {
                (huge, Some(huge))
            } else {
                ("1", Some("0"))
            }),
        );
    }
    for role in [Role::Rollback, Role::Idle, Role::Disposition] {
        complete(&mut evidence, role, None, None);
    }
    assert!(evidence.accept_terminal(&summary(Outcome::RolledBack, Some(Failure::Cardinality))));
    let bytes = assert_capture_fits_request(&evidence);
    let encoded = STANDARD.encode(&bytes);
    // This is the evidence-only bound; storage must size the complete envelope.
    assert!(
        encoded.len() <= 30 * 1024,
        "{} binary / {} base64 bytes",
        bytes.len(),
        encoded.len()
    );
    assert_eq!(BatchCapture::decode(&bytes).unwrap(), evidence);
    for step in &evidence.steps {
        let statement = step.statement.as_ref().unwrap();
        for identity in [
            &statement.sdk_request_id,
            statement.query_id.as_ref().unwrap(),
        ] {
            let id = Uuid::parse_str(identity).unwrap();
            assert!(bytes.windows(16).any(|window| window == id.as_bytes()));
        }
    }
    assert!(!encoded.contains("private-value"));
}

#[test]
fn capture_size_bound_rejects_malformed_public_descriptors_before_dispatch() {
    let owner = Uuid::from_u128(0x99990000000040008000000000000000);
    for request in [
        BatchRequest::Batch {
            mode: Mode::Automatic,
            owner: None,
            rollback_prior: false,
            units: vec![],
        },
        BatchRequest::Batch {
            mode: Mode::Automatic,
            owner: None,
            rollback_prior: false,
            units: vec![UnitKind::Update; MAX_UNITS + 1],
        },
        BatchRequest::Batch {
            mode: Mode::Automatic,
            owner: Some(owner),
            rollback_prior: false,
            units: vec![UnitKind::Update],
        },
        BatchRequest::Batch {
            mode: Mode::Automatic,
            owner: None,
            rollback_prior: true,
            units: vec![UnitKind::Update],
        },
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: Some(owner),
            rollback_prior: false,
            units: vec![UnitKind::Update],
        },
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: None,
            rollback_prior: true,
            units: vec![UnitKind::Update],
        },
        BatchRequest::Batch {
            mode: Mode::Manual,
            owner: Some(Uuid::nil()),
            rollback_prior: true,
            units: vec![UnitKind::Update],
        },
        BatchRequest::Finish {
            owner: Uuid::nil(),
            action: FinishAction::Rollback,
        },
    ] {
        assert!(BatchCapture::maximum_encoded_len(&request).is_err());
        assert!(BatchCapture::maximum_base64_len(&request).is_err());
        assert!(BatchCapture::encode(&BatchEvidence::new(request)).is_err());
    }
}

#[test]
fn capture_size_bound_covers_500_manual_new_and_continued_units_and_both_finishes() {
    let owner = Uuid::from_u128(0x99990000000040008000000000000000);
    let huge = "99999999999999999999999999999999999999";
    for prior_owner in [None, Some(owner)] {
        let request = BatchRequest::parse(
            "mutationBatch",
            &payload(MAX_UNITS, Mode::Manual, prior_owner),
        )
        .unwrap()
        .unwrap();
        let binary_bound = BatchCapture::maximum_encoded_len(&request).unwrap();
        assert_eq!(
            binary_bound,
            if prior_owner.is_some() {
                21_851
            } else {
                21_835
            }
        );
        assert!(BatchCapture::maximum_base64_len(&request).unwrap() < 30 * 1024);
        let mut evidence = BatchEvidence::new(request);
        if prior_owner.is_some() {
            for role in [Role::Context, Role::Settings] {
                complete(&mut evidence, role, None, None);
            }
        } else {
            preflight(&mut evidence);
        }
        for unit in 0..MAX_UNITS {
            complete(
                &mut evidence,
                Role::Unit,
                Some(unit),
                Some(if unit == MAX_UNITS - 1 {
                    (huge, Some(huge))
                } else {
                    ("1", Some("0"))
                }),
            );
        }
        for role in [Role::Rollback, Role::Idle, Role::Disposition] {
            complete(&mut evidence, role, None, None);
        }
        assert!(evidence.accept_terminal(&largest_summary(
            &evidence.request,
            Outcome::RolledBack,
            Some(Failure::Cardinality)
        )));
        let original = assert_capture_fits_request(&evidence);
        let last = evidence
            .steps
            .iter()
            .rfind(|step| step.role == Role::Unit)
            .unwrap();
        assert_eq!(last.count.as_ref().unwrap().affected_rows, huge);
        assert_eq!(
            last.count.as_ref().unwrap().multi_joined_rows.as_deref(),
            Some(huge)
        );
        // Two large unit results cannot fit the admitted prefix contract:
        // dispatch of the second unit requires the first's exact successful count.
        evidence
            .steps
            .iter_mut()
            .find(|step| step.role == Role::Unit)
            .unwrap()
            .count
            .as_mut()
            .unwrap()
            .affected_rows = huge.into();
        assert!(BatchCapture::encode(&evidence).is_err());
        assert_eq!(
            BatchCapture::decode(&original).unwrap().steps.len(),
            MAX_UNITS + if prior_owner.is_some() { 5 } else { 7 }
        );
    }
    for action in [FinishAction::Commit, FinishAction::Rollback] {
        let request = BatchRequest::Finish { owner, action };
        assert_eq!(BatchCapture::maximum_encoded_len(&request).unwrap(), 438);
        assert_eq!(BatchCapture::maximum_base64_len(&request).unwrap(), 584);
        let mut evidence = BatchEvidence::new(request);
        let roles: &[Role] = if action == FinishAction::Commit {
            &[
                Role::Context,
                Role::Settings,
                Role::Commit,
                Role::Idle,
                Role::Disposition,
            ]
        } else {
            &[Role::Rollback, Role::Idle, Role::Disposition]
        };
        for &role in roles {
            complete(&mut evidence, role, None, None);
        }
        let outcome = if action == FinishAction::Commit {
            Outcome::Committed
        } else {
            Outcome::RolledBack
        };
        assert!(evidence.accept_terminal(&largest_summary(&evidence.request, outcome, None)));
        assert_capture_fits_request(&evidence);
    }
}

#[test]
fn capture_size_bound_covers_index_varints_and_exact_large_insert_delete_counts() {
    let mut previous = 0;
    let mut bounds = Vec::new();
    for count in [1, 117, 118, 127, 128, MAX_UNITS] {
        for kind in [UnitKind::Insert, UnitKind::Delete] {
            let request = BatchRequest::Batch {
                mode: Mode::Automatic,
                owner: None,
                rollback_prior: false,
                units: vec![kind; count],
            };
            let bound = BatchCapture::maximum_encoded_len(&request).unwrap();
            assert!(bound >= previous);
            previous = bound;
            if kind == UnitKind::Insert {
                bounds.push(bound);
            }
            let mut evidence = BatchEvidence::new(request);
            preflight(&mut evidence);
            for unit in 0..count {
                complete(
                    &mut evidence,
                    Role::Unit,
                    Some(unit),
                    Some((
                        if unit == count - 1 {
                            "99999999999999999999999999999999999999"
                        } else {
                            "1"
                        },
                        None,
                    )),
                );
            }
            let terminal =
                largest_summary(&evidence.request, Outcome::Unknown, Some(Failure::Observer));
            assert!(evidence.accept_terminal(&terminal));
            assert_capture_fits_request(&evidence);
        }
    }
    // The 118th unit crosses the conservative step-count varint boundary;
    // the 128th crosses both descriptor count and unit-index boundaries.
    assert_eq!(bounds[2] - bounds[1], 42);
    assert_eq!(bounds[4] - bounds[3], 43);
}

#[test]
fn capture_size_bound_covers_every_observed_prefix_including_ack_without_identity() {
    let mut evidence = evidence(2);
    assert_capture_fits_request(&evidence);
    for (role, unit) in [
        (Role::Context, None),
        (Role::Settings, None),
        (Role::Begin, None),
        (Role::Transaction, None),
        (Role::Unit, Some(0)),
        (Role::Unit, Some(1)),
        (Role::Verify, None),
        (Role::Commit, None),
        (Role::Idle, None),
        (Role::Disposition, None),
    ] {
        let step = evidence.steps.len() + 1;
        let mut events = vec![
            phase(step, role, unit, StepPhase::Dispatching),
            phase(step, role, unit, StepPhase::Acknowledged),
            identity(step, false),
            identity(step, true),
        ];
        if role == Role::Unit {
            events.push(frame(json!({"type":"batchCount","step":step,"unit":unit,
                "affectedRows":"1","multiJoinedRows":"0"})));
        }
        events.push(phase(step, role, unit, StepPhase::Drained));
        for event in events {
            assert!(evidence.apply_event(&event));
            assert_capture_fits_request(&evidence);
            let mut unknown = evidence.clone();
            assert!(unknown.accept_terminal(&largest_summary(
                &unknown.request,
                Outcome::Unknown,
                Some(Failure::Transport)
            )));
            assert_capture_fits_request(&unknown);
        }
    }
}

#[test]
fn manual_pending_and_separate_finish_keep_stable_owner_without_overwriting_original() {
    let owner = Uuid::new_v4();
    let mut pending = BatchEvidence::new(
        BatchRequest::parse("mutationBatch", &payload(1, Mode::Manual, None))
            .unwrap()
            .unwrap(),
    );
    preflight(&mut pending);
    complete(&mut pending, Role::Unit, Some(0), Some(("1", Some("0"))));
    complete(&mut pending, Role::Verify, None, None);
    let pending_summary = json!({"outcome":"pending","owner":owner,"transactionId":"-123","failure":null,"priorBatches":0});
    assert!(pending.accept_terminal(&pending_summary));
    let original = assert_capture_fits_request(&pending);
    let mut finish = BatchEvidence::new(BatchRequest::Finish {
        owner,
        action: FinishAction::Rollback,
    });
    for role in [Role::Rollback, Role::Idle, Role::Disposition] {
        complete(&mut finish, role, None, None);
    }
    let finished = json!({"outcome":"rolledBack","owner":owner,"transactionId":"-123","failure":null,"priorBatches":MAX_MANUAL_BATCHES});
    assert!(finish.accept_terminal(&finished));
    assert_eq!(BatchCapture::encode(&pending).unwrap(), original);
    assert_eq!(BatchCapture::decode(&original).unwrap(), pending);
    assert_capture_fits_request(&finish);
}
