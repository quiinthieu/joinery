use super::*;
use crate::{
    domain::TableRef,
    drivers::snowflake_metadata::{ResolvedSchema, SecondaryRoles},
};

fn plan() -> Plan {
    Plan::compile_run("0123456789ABCDEF0123456789ABCDEF").unwrap()
}

fn context() -> LiveContext {
    LiveContext {
        principal: "FIXTURE".into(),
        role: Some("QUALIFICATION".into()),
        database: Some(DATABASE.into()),
        schema: Some(SCHEMA.into()),
        warehouse: Some("FIXTURE_WH".into()),
        session_id: "opaque-session".into(),
        search_path: ["AUX", SCHEMA]
            .into_iter()
            .map(|schema| ResolvedSchema {
                raw: format!("{DATABASE}.{schema}"),
                database: Some(DATABASE.into()),
                schema: Some(schema.into()),
            })
            .collect(),
        secondary_roles: SecondaryRoles {
            roles: "QUALIFICATION".into(),
            value: "ALL".into(),
        },
        quoted_identifiers_ignore_case: false,
    }
}

fn names() -> Vec<String> {
    vec!["AUX".into(), SCHEMA.into()]
}

fn relation(plan: &Plan) -> BrowseRelation {
    BrowseRelation {
        database: DATABASE.into(),
        table: TableRef {
            schema: SCHEMA.into(),
            name: plan.table().into(),
        },
        table_type: "BASE TABLE".into(),
        traits: RelationTraits {
            transient: Some(false),
            temporary: Some(false),
            iceberg: Some(false),
            dynamic: Some(false),
            hybrid: Some(false),
            immutable: Some(false),
        },
        availability: RelationAvailability::Available,
        created: Some("created-marker".into()),
        last_ddl: Some("create-ddl-marker".into()),
    }
}

fn schemas(
    lifecycle: &mut Lifecycle,
    plan: &Plan,
    number: usize,
    site: MetadataSite,
    context: &LiveContext,
    wire_names: &[String],
    parsed_names: &[String],
) -> AppResult<()> {
    lifecycle.observe_schemas(
        &plan.phases()[number - 1],
        SchemaObservation {
            site,
            context,
            wire_names,
            parsed_names,
        },
    )
}

fn relations(
    lifecycle: &mut Lifecycle,
    plan: &Plan,
    number: usize,
    site: MetadataSite,
    context: &LiveContext,
    schema: Option<&str>,
    relations: &[BrowseRelation],
) -> AppResult<()> {
    lifecycle.observe_relations(
        &plan.phases()[number - 1],
        RelationObservation {
            site,
            context,
            schema,
            relations,
        },
    )
}

fn frozen(plan: &Plan) -> Lifecycle {
    let mut lifecycle = Lifecycle::new(plan);
    schemas(
        &mut lifecycle,
        plan,
        2,
        MetadataSite::CreateTargetCheck,
        &context(),
        &names(),
        &names(),
    )
    .unwrap();
    lifecycle
}

fn created(plan: &Plan) -> Lifecycle {
    let mut lifecycle = frozen(plan);
    relations(
        &mut lifecycle,
        plan,
        4,
        MetadataSite::RelationCatalogBatch,
        &context(),
        Some(SCHEMA),
        &[relation(plan)],
    )
    .unwrap();
    lifecycle
}

#[test]
fn lifecycle_census_freezes_only_first_create_preflight_and_never_partial_bad_evidence() {
    let plan = plan();
    for (phase, site) in [
        (1, MetadataSite::CreateTargetCheck),
        (3, MetadataSite::CreateTargetCheck),
        (4, MetadataSite::SessionContextOptions),
        (6, MetadataSite::RelationCatalogBatch),
        (27, MetadataSite::SessionContextOptions),
        (2, MetadataSite::SchemaSwitch),
        (2, MetadataSite::CompletionCatalog),
        (2, MetadataSite::RelationCatalogSnapshot),
    ] {
        let mut lifecycle = Lifecycle::new(&plan);
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                site,
                &context(),
                &names(),
                &names()
            )
            .is_err()
        );
        assert!(lifecycle.census().is_none());
        assert!(lifecycle.context().is_none());
    }
    let cases = [
        vec![],
        vec!["AUX".into()],
        vec![SCHEMA.into(), "AUX".into()],
        vec![SCHEMA.into(), SCHEMA.into()],
        (0..32)
            .map(|i| format!("A{i:02}"))
            .chain([SCHEMA.into()])
            .collect(),
    ];
    for names in cases {
        let mut lifecycle = Lifecycle::new(&plan);
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                2,
                MetadataSite::CreateTargetCheck,
                &context(),
                &names,
                &names
            )
            .is_err()
        );
        assert!(lifecycle.census().is_none());
        assert!(lifecycle.context().is_none());
    }
    let mut lifecycle = Lifecycle::new(&plan);
    let mut reversed = names();
    reversed.reverse();
    assert!(
        schemas(
            &mut lifecycle,
            &plan,
            2,
            MetadataSite::CreateTargetCheck,
            &context(),
            &reversed,
            &names()
        )
        .is_err()
    );
    assert!(lifecycle.census().is_none());
    let maximum: Vec<String> = (0..31)
        .map(|i| format!("A{i:02}"))
        .chain([SCHEMA.into()])
        .collect();
    schemas(
        &mut lifecycle,
        &plan,
        2,
        MetadataSite::CreateTargetCheck,
        &context(),
        &maximum,
        &maximum,
    )
    .unwrap();
    assert_eq!(lifecycle.census().unwrap().names(), maximum);
    assert_eq!(lifecycle.context(), Some(&context()));
}

#[test]
fn lifecycle_census_wire_and_parser_order_are_identical_at_every_allowed_site() {
    let plan = plan();
    let mut lifecycle = frozen(&plan);
    for (phase, site) in [
        (3, MetadataSite::CreateTargetCheck),
        (4, MetadataSite::SessionContextOptions),
        (4, MetadataSite::RelationCatalogBatch),
        (6, MetadataSite::RelationCatalogBatch),
        (10, MetadataSite::SessionContextOptions),
        (27, MetadataSite::SessionContextOptions),
        (31, MetadataSite::SessionContextOptions),
        (32, MetadataSite::RelationCatalogBatch),
    ] {
        schemas(
            &mut lifecycle,
            &plan,
            phase,
            site,
            &context(),
            &names(),
            &names(),
        )
        .unwrap();
        let changed = vec!["ANOTHER".into(), SCHEMA.into()];
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                site,
                &context(),
                &changed,
                &changed
            )
            .is_err()
        );
        let mut reversed = names();
        reversed.reverse();
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                site,
                &context(),
                &reversed,
                &names()
            )
            .is_err()
        );
    }
    for (phase, site) in [
        (6, MetadataSite::SessionContextOptions),
        (27, MetadataSite::RelationCatalogBatch),
        (4, MetadataSite::SchemaSwitch),
        (4, MetadataSite::CompletionCatalog),
    ] {
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                site,
                &context(),
                &names(),
                &names()
            )
            .is_err()
        );
    }
}

#[test]
fn lifecycle_entire_context_is_frozen_including_nulls_raw_roles_and_ordered_paths() {
    let plan = plan();
    let mut lifecycle = frozen(&plan);
    let mutations: [fn(&mut LiveContext); 12] = [
        |c| c.principal = "OTHER".into(),
        |c| c.role = None,
        |c| c.warehouse = None,
        |c| c.session_id = "replacement".into(),
        |c| c.database = Some("OTHER".into()),
        |c| c.schema = Some("OTHER".into()),
        |c| c.quoted_identifiers_ignore_case = true,
        |c| c.search_path.reverse(),
        |c| c.search_path[0].raw.push(' '),
        |c| c.search_path[0].schema = None,
        |c| c.secondary_roles.roles = "OTHER".into(),
        |c| c.secondary_roles.value = "NONE".into(),
    ];
    for mutate in mutations {
        let mut changed = context();
        mutate(&mut changed);
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                4,
                MetadataSite::SessionContextOptions,
                &changed,
                &names(),
                &names()
            )
            .is_err()
        );
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                4,
                MetadataSite::RelationCatalogBatch,
                &changed,
                Some(SCHEMA),
                &[relation(&plan)]
            )
            .is_err()
        );
    }
    assert_eq!(lifecycle.context(), Some(&context()));
    for mutate in [
        (|c: &mut LiveContext| c.database = None) as fn(&mut LiveContext),
        |c| c.schema = None,
        |c| c.quoted_identifiers_ignore_case = true,
        |c| c.principal.clear(),
        |c| c.session_id.clear(),
    ] {
        let mut lifecycle = Lifecycle::new(&plan);
        let mut changed = context();
        mutate(&mut changed);
        assert!(
            schemas(
                &mut lifecycle,
                &plan,
                2,
                MetadataSite::CreateTargetCheck,
                &changed,
                &names(),
                &names()
            )
            .is_err()
        );
        assert!(lifecycle.context().is_none());
    }
}

#[test]
fn lifecycle_rejects_collisions_unfrozen_relations_wrong_sites_and_out_of_census_rows() {
    let plan = plan();
    let target = relation(&plan);
    let mut lifecycle = Lifecycle::new(&plan);
    assert!(
        relations(
            &mut lifecycle,
            &plan,
            2,
            MetadataSite::CreateTargetCheck,
            &context(),
            Some(SCHEMA),
            &[]
        )
        .is_err()
    );
    let mut lifecycle = frozen(&plan);
    for phase in [2, 3] {
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::CreateTargetCheck,
                &context(),
                Some(SCHEMA),
                std::slice::from_ref(&target)
            )
            .is_err()
        );
        relations(
            &mut lifecycle,
            &plan,
            phase,
            MetadataSite::CreateTargetCheck,
            &context(),
            Some(SCHEMA),
            &[],
        )
        .unwrap();
    }
    for (phase, site, schema) in [
        (4, MetadataSite::CreateTargetCheck, Some(SCHEMA)),
        (4, MetadataSite::ExactRelationCheck, Some(SCHEMA)),
        (7, MetadataSite::RelationCatalogBatch, Some(SCHEMA)),
        (23, MetadataSite::ExactRelationCheck, Some(SCHEMA)),
        (4, MetadataSite::RelationCatalogBatch, None),
        (4, MetadataSite::RelationCatalogSnapshot, Some(SCHEMA)),
        (4, MetadataSite::RelationCatalogBatch, Some("UNLISTED")),
    ] {
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                phase,
                site,
                &context(),
                schema,
                std::slice::from_ref(&target)
            )
            .is_err()
        );
    }
    assert!(
        relations(
            &mut lifecycle,
            &plan,
            4,
            MetadataSite::RelationCatalogSnapshot,
            &context(),
            None,
            &[target.clone(), target.clone()]
        )
        .is_err()
    );
    assert!(lifecycle.target.is_none());
    for mutate in [
        (|r: &mut BrowseRelation| r.database = "OTHER".into()) as fn(&mut BrowseRelation),
        |r| r.table.schema = "UNLISTED".into(),
        |r| r.table.schema = "AUX".into(),
        |r| r.table.name = "OTHER".into(),
    ] {
        let mut changed = target.clone();
        mutate(&mut changed);
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                4,
                MetadataSite::RelationCatalogBatch,
                &context(),
                Some(SCHEMA),
                &[changed]
            )
            .is_err()
        );
        assert!(lifecycle.target.is_none());
    }
    // An unrelated schema's empty batch cannot assert target presence or pin it.
    relations(
        &mut lifecycle,
        &plan,
        4,
        MetadataSite::RelationCatalogBatch,
        &context(),
        Some("AUX"),
        &[],
    )
    .unwrap();
    assert!(lifecycle.target.is_none());
    assert!(
        relations(
            &mut lifecycle,
            &plan,
            6,
            MetadataSite::RelationCatalogBatch,
            &context(),
            Some(SCHEMA),
            &[target]
        )
        .is_err()
    );
}

#[test]
fn lifecycle_plain_target_requires_complete_traits_markers_and_first_phase_four_observation() {
    let plan = plan();
    let mutations: [fn(&mut BrowseRelation); 14] = [
        |r| r.table_type = "VIEW".into(),
        |r| r.availability = RelationAvailability::Ambiguous,
        |r| r.traits.transient = None,
        |r| r.traits.temporary = Some(true),
        |r| r.traits.temporary = None,
        |r| r.traits.iceberg = Some(true),
        |r| r.traits.dynamic = None,
        |r| r.traits.hybrid = Some(true),
        |r| r.traits.immutable = None,
        |r| r.created = None,
        |r| r.created = Some(String::new()),
        |r| r.last_ddl = None,
        |r| r.last_ddl = Some(" ".into()),
        |r| r.last_ddl = Some("X".repeat(129)),
    ];
    for mutate in mutations {
        let mut lifecycle = frozen(&plan);
        let mut target = relation(&plan);
        mutate(&mut target);
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                4,
                MetadataSite::RelationCatalogSnapshot,
                &context(),
                None,
                &[target]
            )
            .is_err()
        );
        assert!(lifecycle.target.is_none());
    }
    for transient in [false, true] {
        let mut lifecycle = frozen(&plan);
        let mut target = relation(&plan);
        target.traits.transient = Some(transient);
        relations(
            &mut lifecycle,
            &plan,
            4,
            MetadataSite::RelationCatalogSnapshot,
            &context(),
            None,
            std::slice::from_ref(&target),
        )
        .unwrap();
        for phase in [6, 11, 28] {
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::RelationCatalogSnapshot,
                &context(),
                None,
                std::slice::from_ref(&target),
            )
            .unwrap();
        }
        let mut changed = target.clone();
        changed.traits.transient = Some(!transient);
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                28,
                MetadataSite::RelationCatalogSnapshot,
                &context(),
                None,
                &[changed]
            )
            .is_err()
        );
    }
}

#[test]
fn lifecycle_created_and_ddl_markers_cannot_be_replaced_or_repin_outside_post_add() {
    let plan = plan();
    let mut lifecycle = created(&plan);
    for phase in [6, 11, 28] {
        let mut replacement = relation(&plan);
        replacement.created = Some("replacement-created".into());
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::RelationCatalogSnapshot,
                &context(),
                None,
                &[replacement]
            )
            .is_err()
        );
        let mut ddl_drift = relation(&plan);
        ddl_drift.last_ddl = Some("unexpected-ddl".into());
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::RelationCatalogSnapshot,
                &context(),
                None,
                &[ddl_drift]
            )
            .is_err()
        );
    }
    for phase in [7, 8, 9, 12, 29, 30] {
        let mut drift = relation(&plan);
        drift.last_ddl = Some("unexpected-ddl".into());
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::ExactRelationCheck,
                &context(),
                Some(SCHEMA),
                &[drift]
            )
            .is_err()
        );
    }
    for phase in [6, 28] {
        assert!(
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::RelationCatalogSnapshot,
                &context(),
                None,
                &[]
            )
            .is_err()
        );
    }
    assert!(
        relations(
            &mut lifecycle,
            &plan,
            7,
            MetadataSite::ExactRelationCheck,
            &context(),
            Some(SCHEMA),
            &[]
        )
        .is_err()
    );
    let mut extra = relation(&plan);
    extra.table.name = "UNRELATED".into();
    assert!(
        relations(
            &mut lifecycle,
            &plan,
            7,
            MetadataSite::ExactRelationCheck,
            &context(),
            Some(SCHEMA),
            &[relation(&plan), extra]
        )
        .is_err()
    );
}

#[test]
fn lifecycle_post_add_allows_one_ddl_marker_observation_even_when_timestamp_is_equal() {
    let plan = plan();
    for after_add in ["create-ddl-marker", "post-add-ddl-marker"] {
        let mut lifecycle = created(&plan);
        let mut target = relation(&plan);
        target.last_ddl = Some(after_add.into());
        relations(
            &mut lifecycle,
            &plan,
            10,
            MetadataSite::RelationCatalogBatch,
            &context(),
            Some("AUX"),
            &[],
        )
        .unwrap();
        assert!(!lifecycle.post_add_observed);
        relations(
            &mut lifecycle,
            &plan,
            10,
            MetadataSite::RelationCatalogBatch,
            &context(),
            Some(SCHEMA),
            std::slice::from_ref(&target),
        )
        .unwrap();
        assert!(lifecycle.post_add_observed);
        relations(
            &mut lifecycle,
            &plan,
            10,
            MetadataSite::RelationCatalogSnapshot,
            &context(),
            None,
            std::slice::from_ref(&target),
        )
        .unwrap();
        let mut changed_again = target.clone();
        changed_again.last_ddl = Some("second-post-add-marker".into());
        for phase in [10, 11, 28] {
            assert!(
                relations(
                    &mut lifecycle,
                    &plan,
                    phase,
                    MetadataSite::RelationCatalogSnapshot,
                    &context(),
                    None,
                    std::slice::from_ref(&changed_again)
                )
                .is_err()
            );
        }
        // A failed second observation cannot replace the accepted post-ADD pin.
        relations(
            &mut lifecycle,
            &plan,
            28,
            MetadataSite::RelationCatalogSnapshot,
            &context(),
            None,
            std::slice::from_ref(&target),
        )
        .unwrap();
    }
}

#[test]
fn lifecycle_full_metadata_route_retains_target_until_drop_and_requires_absence_afterward() {
    let plan = plan();
    let mut lifecycle = frozen(&plan);
    let mut target = relation(&plan);
    for phase in 2..=32 {
        if [2, 3].contains(&phase) {
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::CreateTargetCheck,
                &context(),
                &names(),
                &names(),
            )
            .unwrap();
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::CreateTargetCheck,
                &context(),
                Some(SCHEMA),
                &[],
            )
            .unwrap();
        }
        if [7, 8, 9, 12, 29, 30].contains(&phase) {
            relations(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::ExactRelationCheck,
                &context(),
                Some(SCHEMA),
                std::slice::from_ref(&target),
            )
            .unwrap();
        }
        if phase == 27 {
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::SessionContextOptions,
                &context(),
                &names(),
                &names(),
            )
            .unwrap();
        }
        if ![4, 6, 10, 11, 28, 31, 32].contains(&phase) {
            continue;
        }
        if phase == 10 {
            target.last_ddl = Some("post-add-marker".into());
        }
        if [4, 10, 31].contains(&phase) {
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::SessionContextOptions,
                &context(),
                &names(),
                &names(),
            )
            .unwrap();
        }
        schemas(
            &mut lifecycle,
            &plan,
            phase,
            MetadataSite::RelationCatalogBatch,
            &context(),
            &names(),
            &names(),
        )
        .unwrap();
        let rows = if phase < 31 {
            std::slice::from_ref(&target)
        } else {
            &[]
        };
        relations(
            &mut lifecycle,
            &plan,
            phase,
            MetadataSite::RelationCatalogBatch,
            &context(),
            Some("AUX"),
            &[],
        )
        .unwrap();
        relations(
            &mut lifecycle,
            &plan,
            phase,
            MetadataSite::RelationCatalogBatch,
            &context(),
            Some(SCHEMA),
            rows,
        )
        .unwrap();
        relations(
            &mut lifecycle,
            &plan,
            phase,
            MetadataSite::RelationCatalogSnapshot,
            &context(),
            None,
            rows,
        )
        .unwrap();
        if [4, 10, 31].contains(&phase) {
            schemas(
                &mut lifecycle,
                &plan,
                phase,
                MetadataSite::SessionContextOptions,
                &context(),
                &names(),
                &names(),
            )
            .unwrap();
        }
        if phase >= 31 {
            assert!(
                relations(
                    &mut lifecycle,
                    &plan,
                    phase,
                    MetadataSite::RelationCatalogSnapshot,
                    &context(),
                    None,
                    std::slice::from_ref(&target)
                )
                .is_err()
            );
        }
    }
    assert_eq!(lifecycle.target.as_ref().unwrap().created, "created-marker");
    assert_eq!(
        lifecycle.target.as_ref().unwrap().last_ddl,
        "post-add-marker"
    );
}
