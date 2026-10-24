use joinery_core::{domain::*, execution::Studio, secrets::MemorySecrets};
use std::sync::Arc;

#[test]
fn groups_inherit_policy_and_reject_cycles_and_downgrades() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let group = studio
        .save_group(ConnectionGroup {
            id: new_id(),
            name: "Analytics".into(),
            parent_id: Some("production".into()),
            environment: Environment::Local,
        })
        .unwrap();
    assert_eq!(group.environment, Environment::Production);
    let child = studio
        .save_group(ConnectionGroup {
            id: new_id(),
            name: "Reporting".into(),
            parent_id: Some(group.id.clone()),
            environment: Environment::Local,
        })
        .unwrap();
    let mut cycle = group.clone();
    cycle.parent_id = Some(child.id.clone());
    assert!(studio.save_group(cycle).is_err());
    let mut downgrade = group.clone();
    downgrade.parent_id = Some("local".into());
    assert_eq!(
        studio.save_group(downgrade).unwrap_err().code,
        ErrorCode::PolicyBlocked
    );
    assert!(studio.delete_group(&group.id).is_err());
    studio.delete_group(&child.id).unwrap();
    studio.delete_group(&group.id).unwrap();
    assert!(studio.delete_group("production").is_err());
}

#[test]
fn group_depth_is_bounded_including_moved_descendants() {
    let root = tempfile::tempdir().unwrap();
    let studio = Studio::with_secrets(root.path(), Arc::new(MemorySecrets::default())).unwrap();
    let mut parent = "local".to_string();
    let mut groups = Vec::new();
    for n in 0..7 {
        let group = studio
            .save_group(ConnectionGroup {
                id: new_id(),
                name: format!("Group {n}"),
                parent_id: Some(parent),
                environment: Environment::Local,
            })
            .unwrap();
        parent = group.id.clone();
        groups.push(group);
    }
    assert!(
        studio
            .save_group(ConnectionGroup {
                id: new_id(),
                name: "Too deep".into(),
                parent_id: Some(parent),
                environment: Environment::Local
            })
            .is_err()
    );
    let mut moved = groups[0].clone();
    moved.parent_id = Some(groups[6].id.clone());
    assert!(studio.save_group(moved).is_err());
}
