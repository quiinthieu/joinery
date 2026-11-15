use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    sync::{Arc, Barrier},
};

const RUN: &str = "1234567890ABCDEF1234567890ABCDEF";
const ENVELOPE: &[u8] = b"{\"offline\":true}\n";

fn directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

fn physical(directory: &tempfile::TempDir) -> PathBuf {
    directory.path().canonicalize().unwrap()
}

fn plan() -> Plan {
    Plan::compile_run(RUN).unwrap()
}

fn output(parent: &Path) -> PathBuf {
    parent.join(format!("plan-{RUN}"))
}

#[test]
fn emission_persists_exact_bounded_bytes_hash_and_private_modes() {
    let directory = directory();
    let parent = physical(&directory);
    let plan = plan();
    let receipt = Emission::persist(&parent, &plan, ENVELOPE).unwrap();
    assert_eq!(receipt.path, output(&parent));
    assert_eq!(receipt.envelope_bytes, ENVELOPE.len() as u64);
    assert_eq!(receipt.envelope_sha256, hash(ENVELOPE));
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&receipt.path).unwrap().count(), 2);
    assert_eq!(
        fs::read(receipt.path.join("execution-envelope.json")).unwrap(),
        ENVELOPE
    );
    let snapshot = fs::read(receipt.path.join("plan.json")).unwrap();
    assert_eq!(snapshot, plan.canonical_json().unwrap());
    plan.verify_snapshot(&snapshot, &hash(&snapshot)).unwrap();
    let metadata = fs::symlink_metadata(&receipt.path).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o700);
    assert_eq!(metadata.uid(), fs::metadata(&parent).unwrap().uid());
    for name in ["execution-envelope.json", "plan.json"] {
        let metadata = fs::symlink_metadata(receipt.path.join(name)).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.uid(), fs::metadata(&parent).unwrap().uid());
        assert_eq!(metadata.nlink(), 1);
    }
}

#[test]
fn emission_rerun_never_adopts_or_overwrites_even_an_empty_directory() {
    for existing_complete in [false, true] {
        let directory = directory();
        let parent = physical(&directory);
        if existing_complete {
            Emission::persist(&parent, &plan(), ENVELOPE).unwrap();
        } else {
            fs::create_dir(output(&parent)).unwrap();
            fs::set_permissions(output(&parent), fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(Emission::persist(&parent, &plan(), b"replacement").is_err());
        if existing_complete {
            assert_eq!(
                fs::read(output(&parent).join("execution-envelope.json")).unwrap(),
                ENVELOPE
            );
        } else {
            assert_eq!(fs::read_dir(output(&parent)).unwrap().count(), 0);
        }
    }
}

#[test]
fn emission_rejects_invalid_run_plan_and_envelope_bounds_before_writing() {
    let directory = directory();
    let parent = physical(&directory);
    for run in [
        "".to_string(),
        "0".repeat(32),
        "a".repeat(32),
        "G".repeat(32),
        "A".repeat(33),
        "../escape".to_string(),
    ] {
        let mut changed = plan();
        changed.run = run;
        assert!(Emission::persist(&parent, &changed, ENVELOPE).is_err());
    }
    let mut oversized = plan();
    oversized.table = "X".repeat(MAX_PLAN_SNAPSHOT_BYTES + 1);
    assert!(Emission::persist(&parent, &oversized, ENVELOPE).is_err());
    let mut changed = plan();
    changed.table.push('X');
    assert!(Emission::persist(&parent, &changed, ENVELOPE).is_err());
    for bytes in [Vec::new(), vec![b'x'; MAX_ENVELOPE_BYTES + 1]] {
        assert!(Emission::persist(&parent, &plan(), &bytes).is_err());
    }
    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
}

#[test]
fn emission_accepts_exact_envelope_limit_without_changing_bytes() {
    let directory = directory();
    let parent = physical(&directory);
    let envelope = vec![b'x'; MAX_ENVELOPE_BYTES];
    let receipt = Emission::persist(&parent, &plan(), &envelope).unwrap();
    assert_eq!(receipt.envelope_bytes, MAX_ENVELOPE_BYTES as u64);
    assert_eq!(receipt.envelope_sha256, hash(&envelope));
    assert_eq!(
        fs::read(receipt.path.join("execution-envelope.json")).unwrap(),
        envelope
    );
}

#[test]
fn emission_rejects_symlink_parent_component_and_nonprivate_parent() {
    let directory = directory();
    let base = physical(&directory);
    let parent = base.join("private");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let alias = base.join("alias");
    symlink(&parent, &alias).unwrap();
    assert!(Emission::persist(&alias, &plan(), ENVELOPE).is_err());
    let child = parent.join("child");
    fs::create_dir(&child).unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Emission::persist(&alias.join("child"), &plan(), ENVELOPE).is_err());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Emission::persist(&parent, &plan(), ENVELOPE).is_err());
    assert!(Emission::persist(Path::new("relative"), &plan(), ENVELOPE).is_err());
    assert!(Emission::persist(&base.join("private/../private"), &plan(), ENVELOPE).is_err());
    assert!(!output(&parent).exists());
    assert_eq!(fs::read_dir(&child).unwrap().count(), 0);
}

#[test]
fn emission_retains_partial_files_and_never_follows_a_plan_symlink() {
    for symlink_collision in [false, true] {
        let directory = directory();
        let parent = physical(&directory);
        let outside = parent.join("outside");
        fs::write(&outside, b"untouched").unwrap();
        let target = outside.clone();
        let result = Emission::persist_inner(
            &parent,
            &plan(),
            ENVELOPE,
            Some(Box::new(move |proof| {
                let snapshot = proof.path().join("plan.json");
                if symlink_collision {
                    symlink(&target, snapshot).unwrap();
                } else {
                    fs::write(snapshot, b"partial").unwrap();
                }
                Ok(())
            })),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        assert_eq!(
            fs::read(output(&parent).join("execution-envelope.json")).unwrap(),
            ENVELOPE
        );
        if !symlink_collision {
            assert_eq!(
                fs::read(output(&parent).join("plan.json")).unwrap(),
                b"partial"
            );
        }
        assert!(Emission::persist(&parent, &plan(), ENVELOPE).is_err());
    }
}

#[test]
fn emission_final_readback_rejects_content_mode_link_and_identity_changes() {
    for change in 0..5 {
        let directory = directory();
        let parent = physical(&directory);
        let result = Emission::persist_inner(
            &parent,
            &plan(),
            ENVELOPE,
            Some(Box::new(move |proof| {
                let envelope = proof.path().join("execution-envelope.json");
                match change {
                    0 => fs::write(&envelope, b"changed").unwrap(),
                    1 => fs::set_permissions(&envelope, fs::Permissions::from_mode(0o640)).unwrap(),
                    2 => fs::hard_link(&envelope, proof.path().join("extra-link")).unwrap(),
                    3 => {
                        fs::rename(&envelope, proof.path().join("original-envelope")).unwrap();
                        fs::write(&envelope, ENVELOPE).unwrap();
                        fs::set_permissions(&envelope, fs::Permissions::from_mode(0o600)).unwrap();
                    }
                    4 => fs::set_permissions(proof.path(), fs::Permissions::from_mode(0o755))
                        .unwrap(),
                    _ => unreachable!(),
                }
                Ok(())
            })),
        );
        assert!(result.is_err(), "change {change}");
        assert!(output(&parent).exists());
        assert!(output(&parent).join("execution-envelope.json").exists());
        assert!(Emission::persist(&parent, &plan(), ENVELOPE).is_err());
    }
}

#[test]
fn emission_concurrent_same_run_has_one_success() {
    let directory = directory();
    let parent = physical(&directory);
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let parent = parent.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                Emission::persist(&parent, &plan(), ENVELOPE).is_ok()
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|succeeded| *succeeded)
            .count(),
        1
    );
    assert_eq!(fs::read_dir(output(&parent)).unwrap().count(), 2);
}
