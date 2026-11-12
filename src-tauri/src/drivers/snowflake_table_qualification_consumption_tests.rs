use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

const EXECUTION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_EXECUTION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RUN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const OTHER_RUN: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    directory
}
fn physical(directory: &tempfile::TempDir) -> PathBuf {
    directory.path().canonicalize().unwrap()
}
fn marker(directory: &Path, kind: Marker, hash: &str) -> PathBuf {
    directory.join(kind.name(hash))
}
fn assert_spent(directory: &Path) {
    let reopened = ConsumptionStore::open(directory).unwrap();
    assert!(reopened.consume(EXECUTION, RUN).is_err());
    assert!(reopened.consume(OTHER_EXECUTION, RUN).is_err());
}

#[test]
fn consumption_success_binds_both_hashes_and_drop_never_releases_markers() {
    let directory = directory();
    let path = physical(&directory);
    let store = ConsumptionStore::open(&path).unwrap();
    let token = store.consume(EXECUTION, RUN).unwrap();
    assert_eq!(token.execution_sha256(), EXECUTION);
    assert_eq!(token.run_key_sha256(), RUN);
    let run = marker(&path, Marker::Run, RUN);
    let plan = marker(&path, Marker::Plan, EXECUTION);
    let bytes = fs::read(&run).unwrap();
    assert_eq!(bytes, fs::read(&plan).unwrap());
    assert_eq!(bytes.last(), Some(&b'\n'));
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert_eq!(value["format"], FORMAT);
    assert_eq!(value["executionSha256"], EXECUTION);
    assert_eq!(value["runKeySha256"], RUN);
    for file in [&run, &plan] {
        let metadata = fs::symlink_metadata(file).unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.uid(), owner());
        assert_eq!(metadata.nlink(), 1);
    }
    // A caller can lose the capability before launch; there is no refund path.
    drop(token);
    drop(store);
    assert_spent(&path);
    let store = ConsumptionStore::open(&path).unwrap();
    assert!(store.consume(EXECUTION, OTHER_RUN).is_err());
    // Run is always spent first, including a previously consumed plan collision.
    assert!(marker(&path, Marker::Run, OTHER_RUN).exists());
    assert!(store.consume(OTHER_EXECUTION, OTHER_RUN).is_err());
    assert_eq!(fs::read(run).unwrap(), bytes);
    assert_eq!(fs::read(plan).unwrap(), bytes);
}

#[test]
fn consumption_rejects_invalid_hashes_before_creating_any_markers() {
    for bad in [
        "".to_string(),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        format!("{RUN}\n"),
    ] {
        let directory = directory();
        let path = physical(&directory);
        let store = ConsumptionStore::open(&path).unwrap();
        assert!(store.consume(&bad, RUN).is_err());
        assert!(store.consume(EXECUTION, &bad).is_err());
        assert_eq!(fs::read_dir(path).unwrap().count(), 0);
    }
}

#[test]
fn consumption_output_parent_must_be_physically_separate_and_remain_safe() {
    let base = directory();
    let base_path = physical(&base);
    let markers = base_path.join("markers");
    let output = base_path.join("output");
    let below_markers = markers.join("output");
    for path in [&markers, &output] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fs::create_dir(&below_markers).unwrap();
    fs::set_permissions(&below_markers, fs::Permissions::from_mode(0o700)).unwrap();
    let store = ConsumptionStore::open(&markers).unwrap();
    let _consumed = store.consume(EXECUTION, RUN).unwrap();
    let run_bytes = fs::read(marker(&markers, Marker::Run, RUN)).unwrap();
    for rejected in [&markers, &base_path, &below_markers] {
        assert!(
            store.require_separate_from(rejected).is_err(),
            "{rejected:?}"
        );
    }
    store.require_separate_from(&output).unwrap();
    let link = base_path.join("output-link");
    symlink(&output, &link).unwrap();
    assert!(store.require_separate_from(&link).is_err());
    assert!(store.require_separate_from(Path::new("relative")).is_err());
    fs::set_permissions(&output, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(store.require_separate_from(&output).is_err());
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&markers, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(store.require_separate_from(&output).is_err());
    fs::set_permissions(&markers, fs::Permissions::from_mode(0o700)).unwrap();
    store.require_separate_from(&output).unwrap();
    assert_eq!(
        fs::read(marker(&markers, Marker::Run, RUN)).unwrap(),
        run_bytes
    );
    assert_eq!(
        fs::read(marker(&markers, Marker::Plan, EXECUTION)).unwrap(),
        run_bytes
    );
    assert!(store.consume(EXECUTION, RUN).is_err());
}

#[test]
fn consumption_output_separation_rejects_replaced_store_identity() {
    let base = directory();
    let base_path = physical(&base);
    let markers = base_path.join("markers");
    let output = base_path.join("output");
    for path in [&markers, &output] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let store = ConsumptionStore::open(&markers).unwrap();
    let _consumed = store.consume(EXECUTION, RUN).unwrap();
    let held = base_path.join("held-markers");
    fs::rename(&markers, &held).unwrap();
    fs::create_dir(&markers).unwrap();
    fs::set_permissions(&markers, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(store.require_separate_from(&output).is_err());
    assert_eq!(fs::read_dir(&held).unwrap().count(), 2);
    assert_eq!(fs::read_dir(&markers).unwrap().count(), 0);
}

#[test]
fn consumption_simultaneous_threads_issue_exactly_one_capability() {
    let directory = directory();
    let store = Arc::new(ConsumptionStore::open(&physical(&directory)).unwrap());
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.consume(EXECUTION, RUN).is_ok()
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|granted| *granted)
            .count(),
        1
    );
    assert_spent(&physical(&directory));
}

// The parent launches only this exact Rust test in the current test binary.
// It has no account/profile/SDK setup, shell wrapper, or production entry point.
#[test]
fn consumption_cross_process_child() {
    let Some(path) = std::env::var_os("JOINERY_CONSUMPTION_CHILD_DIRECTORY") else {
        return;
    };
    let mut start = [0];
    std::io::stdin().read_exact(&mut start).unwrap();
    assert_eq!(start, [1]);
    let store = ConsumptionStore::open(Path::new(&path)).unwrap();
    match store.consume(EXECUTION, RUN) {
        Ok(token) => {
            assert_eq!(token.execution_sha256(), EXECUTION);
            assert_eq!(token.run_key_sha256(), RUN);
            println!("CONSUMPTION_CHILD_GRANTED");
        }
        Err(_) => println!("CONSUMPTION_CHILD_REJECTED"),
    }
}

#[test]
fn consumption_cross_process_exclusive_creation_issues_exactly_one_capability() {
    let directory = directory();
    let path = physical(&directory);
    let scope = module_path!().split_once("::").unwrap().1;
    let exact = format!("{scope}::consumption_cross_process_child");
    let mut children: Vec<_> = (0..2)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &exact, "--nocapture"])
                .env("JOINERY_CONSUMPTION_CHILD_DIRECTORY", &path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        child.stdin.take().unwrap().write_all(&[1]).unwrap();
    }
    let outputs: Vec<_> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    let mut granted = 0;
    let mut rejected = 0;
    for output in outputs {
        assert!(
            output.status.success(),
            "child stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        granted += stdout.matches("CONSUMPTION_CHILD_GRANTED").count();
        rejected += stdout.matches("CONSUMPTION_CHILD_REJECTED").count();
    }
    assert_eq!((granted, rejected), (1, 1));
    assert_spent(&path);
}

#[test]
fn consumption_partial_write_and_each_sync_failure_leave_unreusable_markers() {
    for kind in [Marker::Run, Marker::Plan] {
        for stage in [Stage::PartialWrite, Stage::FileSync, Stage::DirectorySync] {
            let directory = directory();
            let path = physical(&directory);
            let store = ConsumptionStore::open(&path).unwrap();
            *store.fault.lock().unwrap() = Some(Fault {
                marker: kind,
                stage,
            });
            assert!(store.consume(EXECUTION, RUN).is_err());
            let run = marker(&path, Marker::Run, RUN);
            assert!(run.exists());
            let failed = if kind == Marker::Run {
                run.clone()
            } else {
                marker(&path, Marker::Plan, EXECUTION)
            };
            let bytes = fs::read(failed).unwrap();
            if matches!(stage, Stage::PartialWrite) {
                assert_eq!(bytes.len(), 8);
            } else {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["format"],
                    FORMAT
                );
            }
            if kind == Marker::Run {
                assert!(!marker(&path, Marker::Plan, EXECUTION).exists());
            }
            assert_spent(&path);
            // The first marker survives reopening and attempted different bindings.
            assert!(!fs::read(run).unwrap().is_empty());
        }
    }
}

#[test]
fn consumption_rejects_nonphysical_or_nonprivate_roots_without_chmod() {
    assert!(ConsumptionStore::open(Path::new(".")).is_err());
    let directory = directory();
    let path = physical(&directory);
    for mode in [0o755, 0o770, 0o1700, 0o500] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(ConsumptionStore::open(&path).is_err());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, mode);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let inner = path.join("inner");
    fs::create_dir(&inner).unwrap();
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o700)).unwrap();
    let alias = path.join("alias");
    symlink(&inner, &alias).unwrap();
    assert!(ConsumptionStore::open(&alias).is_err());
    let nested = inner.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(ConsumptionStore::open(&alias.join("nested")).is_err());
    let ordinary = path.join("ordinary");
    fs::write(&ordinary, b"sentinel").unwrap();
    assert!(ConsumptionStore::open(&ordinary).is_err());
    assert_eq!(fs::read(ordinary).unwrap(), b"sentinel");
}

#[test]
fn consumption_preexisting_marker_symlink_hardlink_and_special_files_are_never_opened_or_replaced()
{
    for kind in [Marker::Run, Marker::Plan] {
        for shape in 0..5 {
            let directory = directory();
            let path = physical(&directory);
            let name = marker(
                &path,
                kind,
                if kind == Marker::Run { RUN } else { EXECUTION },
            );
            let sentinel = path.join("sentinel");
            fs::write(&sentinel, b"sentinel").unwrap();
            match shape {
                0 => fs::write(&name, b"existing").unwrap(),
                1 => symlink(&sentinel, &name).unwrap(),
                2 => fs::hard_link(&sentinel, &name).unwrap(),
                3 => fs::create_dir(&name).unwrap(),
                _ => {
                    let c = CString::new(name.as_os_str().as_bytes()).unwrap();
                    // SAFETY: c is a live NUL-terminated pathname; mkfifo owns no fd.
                    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
                }
            }
            let before = fs::symlink_metadata(&name).unwrap();
            let store = ConsumptionStore::open(&path).unwrap();
            assert!(store.consume(EXECUTION, RUN).is_err());
            assert_eq!(
                identity(&fs::symlink_metadata(&name).unwrap()),
                identity(&before)
            );
            assert_eq!(fs::read(sentinel).unwrap(), b"sentinel");
            if kind == Marker::Plan {
                assert_spent(&path);
            }
        }
    }
}

#[test]
fn consumption_detects_marker_replacement_permission_change_and_new_hardlink_before_grant() {
    for shape in 0..3 {
        let directory = directory();
        let path = physical(&directory);
        let store = ConsumptionStore::open(&path).unwrap();
        *store.hook.lock().unwrap() = Some(Box::new(move |kind, directory| {
            assert!(kind == Marker::Run);
            let run = marker(directory, Marker::Run, RUN);
            match shape {
                0 => {
                    let original = directory.join("original");
                    fs::rename(&run, &original).unwrap();
                    fs::copy(original, run).unwrap();
                }
                1 => fs::set_permissions(run, fs::Permissions::from_mode(0o640)).unwrap(),
                _ => fs::hard_link(run, directory.join("extra-link")).unwrap(),
            }
        }));
        assert!(store.consume(EXECUTION, RUN).is_err());
        assert_spent(&path);
        assert!(!marker(&path, Marker::Plan, EXECUTION).exists());
    }
}

#[test]
fn consumption_detects_root_replacement_or_permissions_changed_after_open_and_during_write() {
    for during_write in [false, true] {
        let parent = directory();
        let path = physical(&parent).join("private");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let store = ConsumptionStore::open(&path).unwrap();
        let displaced = path.with_file_name("displaced");
        if during_write {
            *store.hook.lock().unwrap() = Some(Box::new(move |_, path| {
                fs::rename(path, path.with_file_name("displaced")).unwrap();
                fs::create_dir(path).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }));
        } else {
            fs::rename(&path, &displaced).unwrap();
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(store.consume(EXECUTION, RUN).is_err());
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        if during_write {
            assert!(marker(&displaced, Marker::Run, RUN).exists());
        }
    }
    let directory = directory();
    let path = physical(&directory);
    let store = ConsumptionStore::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(store.consume(EXECUTION, RUN).is_err());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
}

#[test]
fn consumption_rejects_raw_path_aliases_limits_and_permissive_ancestors() {
    use std::os::unix::ffi::OsStringExt;
    let directory = directory();
    let path = physical(&directory);
    let raw = path.as_os_str().as_bytes();
    let mut nul = raw.to_vec();
    nul.extend(b"\0suffix");
    let aliases = [
        PathBuf::from(format!("{}/", path.display())),
        PathBuf::from(format!("{}/.", path.display())),
        PathBuf::from(format!(
            "{}/../{}",
            path.display(),
            path.file_name().unwrap().to_str().unwrap()
        )),
        PathBuf::from(format!("//{}", path.display())),
        PathBuf::from(format!("{}/./unused", path.display())),
        PathBuf::from(format!("{}//unused", path.display())),
        PathBuf::from(std::ffi::OsString::from_vec(nul)),
        PathBuf::from(format!("/{}", "x".repeat(MAX_PATH_BYTES))),
        PathBuf::from(format!("/{}", vec!["x"; MAX_COMPONENTS + 1].join("/"))),
    ];
    for alias in aliases {
        assert!(ConsumptionStore::open(&alias).is_err());
    }
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    let child = path.join("private");
    fs::create_dir(&child).unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
    for mode in [0o777, 0o770, 0o2700, 0o4700] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(ConsumptionStore::open(&child).is_err());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, mode);
        assert_eq!(fs::read_dir(&child).unwrap().count(), 0);
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let store = ConsumptionStore::open(&child).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o770)).unwrap();
    assert!(store.consume(EXECUTION, RUN).is_err());
    assert_eq!(fs::read_dir(&child).unwrap().count(), 0);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    if owner() == 0 {
        let name = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: only an owned temporary directory is changed; root can restore it.
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 12345, !0) }, 0);
        assert!(ConsumptionStore::open(&child).is_err());
        // SAFETY: restore the same owned temporary directory before cleanup.
        assert_eq!(unsafe { libc::chown(name.as_ptr(), 0, !0) }, 0);
    }
}

#[test]
fn consumption_token_retains_proof_after_store_drop_and_rejects_marker_drift() {
    for kind in [Marker::Run, Marker::Plan] {
        for drift in 0..6 {
            let directory = directory();
            let path = physical(&directory);
            let store = ConsumptionStore::open(&path).unwrap();
            let token = store.consume(EXECUTION, RUN).unwrap();
            drop(store);
            token.revalidate().unwrap();
            let file = marker(
                &path,
                kind,
                if kind == Marker::Run { RUN } else { EXECUTION },
            );
            let original = fs::read(&file).unwrap();
            match drift {
                0 => fs::remove_file(&file).unwrap(),
                1 => {
                    let saved = path.join("original");
                    fs::rename(&file, &saved).unwrap();
                    fs::copy(saved, &file).unwrap();
                }
                2 => fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap(),
                3 => fs::hard_link(&file, path.join("extra-link")).unwrap(),
                4 => {
                    let mut changed = original.clone();
                    changed[0] = b'[';
                    fs::write(&file, changed).unwrap();
                }
                _ => fs::write(&file, b"truncated").unwrap(),
            }
            assert!(token.revalidate().is_err());
            // The other original marker remains spent; revalidation never repairs
            // a changed/deleted marker and never removes any surviving marker.
            let other = marker(
                &path,
                if kind == Marker::Run {
                    Marker::Plan
                } else {
                    Marker::Run
                },
                if kind == Marker::Run { EXECUTION } else { RUN },
            );
            assert_eq!(fs::read(other).unwrap(), original);
            assert!(
                ConsumptionStore::open(&path)
                    .unwrap()
                    .consume(EXECUTION, RUN)
                    .is_err()
            );
        }
    }
}

#[test]
fn consumption_token_revalidation_rejects_root_and_ancestor_drift_after_store_drop() {
    for drift in 0..5 {
        let parent = directory();
        let parent_path = physical(&parent);
        let path = parent_path.join("private");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let store = ConsumptionStore::open(&path).unwrap();
        let token = store.consume(EXECUTION, RUN).unwrap();
        drop(store);
        token.revalidate().unwrap();
        let displaced = parent_path.join("displaced");
        match drift {
            0 => {
                fs::rename(&path, &displaced).unwrap();
                fs::create_dir(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            1 => {
                fs::rename(&path, &displaced).unwrap();
                symlink(&displaced, &path).unwrap();
            }
            2 => fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap(),
            3 => fs::set_permissions(&parent_path, fs::Permissions::from_mode(0o770)).unwrap(),
            _ => {
                fs::remove_file(marker(&path, Marker::Run, RUN)).unwrap();
                fs::remove_file(marker(&path, Marker::Plan, EXECUTION)).unwrap();
                fs::remove_dir(&path).unwrap();
            }
        }
        assert!(token.revalidate().is_err());
        if drift < 2 {
            assert!(marker(&displaced, Marker::Run, RUN).exists());
            assert!(marker(&displaced, Marker::Plan, EXECUTION).exists());
        }
        if drift == 3 {
            fs::set_permissions(&parent_path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}
