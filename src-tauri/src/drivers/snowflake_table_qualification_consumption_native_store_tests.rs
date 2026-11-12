use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    process::{Child, Command, Stdio},
};

const PARTS: [&str; 5] = [
    "Library",
    "Application Support",
    "Joinery",
    "private-qualification",
    "table-lifecycle-consumption-v1",
];
const EXECUTION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const RUN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

// Native lookup is deliberately never invoked by these tests. Only temporary,
// synthetic homes reach the private bootstrap helper, including in children.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let home = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join("synthetic-home");
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    (temporary, home)
}

fn namespace(home: &Path) -> PathBuf {
    PARTS
        .iter()
        .fold(home.to_path_buf(), |path, name| path.join(name))
}

fn create_components(home: &Path, count: usize) -> PathBuf {
    let mut path = home.to_path_buf();
    for name in PARTS.iter().take(count) {
        path.push(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}

#[test]
fn native_store_bootstraps_fixed_private_namespace_below_safe_shared_directories() {
    let (_temporary, home) = fixture();
    create_components(&home, 2);
    for path in [
        &home,
        &home.join(PARTS[0]),
        &home.join(PARTS[0]).join(PARTS[1]),
    ] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let store = ConsumptionStore::open_native_home(&home, owner()).unwrap();
    assert_eq!(store.proof.path(), namespace(&home));
    let mut path = home.clone();
    for (index, name) in PARTS.iter().enumerate() {
        path.push(name);
        assert_eq!(
            fs::metadata(&path).unwrap().mode() & 0o7777,
            if index < 2 { 0o755 } else { 0o700 }
        );
    }
    assert!(ConsumptionStore::open(&home).is_err()); // Existing private-leaf policy remains 0700.
    let token = store.consume(EXECUTION, RUN).unwrap();
    token.revalidate().unwrap();
    drop(token);
    drop(store);
    assert!(
        ConsumptionStore::open_native_home(&home, owner())
            .unwrap()
            .consume(EXECUTION, RUN)
            .is_err()
    );
}

#[test]
fn native_store_rejects_unsafe_modes_without_repair_or_fresh_namespace() {
    for index in 0..PARTS.len() {
        let (_temporary, home) = fixture();
        let path = create_components(&home, index + 1);
        let mode = if index < 2 { 0o777 } else { 0o755 };
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            ConsumptionStore::open_native_home(&home, owner()).is_err(),
            "{path:?}"
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, mode);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    }
    let (_temporary, home) = fixture();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o1777)).unwrap();
    assert!(ConsumptionStore::open_native_home(&home, owner()).is_err());
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
}

#[test]
fn native_store_rejects_each_symlink_or_file_collision_without_touching_target() {
    for (index, name) in PARTS.iter().enumerate() {
        for link in [false, true] {
            let (temporary, home) = fixture();
            let parent = create_components(&home, index);
            let collision = parent.join(name);
            let outside = temporary.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::set_permissions(&outside, fs::Permissions::from_mode(0o700)).unwrap();
            if link {
                symlink(&outside, &collision).unwrap();
            } else {
                fs::write(&collision, b"existing file").unwrap();
            }
            let before = fs::symlink_metadata(&collision).unwrap();
            assert!(ConsumptionStore::open_native_home(&home, owner()).is_err());
            assert_eq!(
                identity(&fs::symlink_metadata(&collision).unwrap()),
                identity(&before)
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
            if !link {
                assert_eq!(fs::read(collision).unwrap(), b"existing file");
            }
        }
    }
    let (temporary, home) = fixture();
    let alias = temporary.path().canonicalize().unwrap().join("home-alias");
    symlink(&home, &alias).unwrap();
    assert!(ConsumptionStore::open_native_home(&alias, owner()).is_err());
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
}

#[test]
fn native_store_rejects_uid_mismatch_and_replaced_held_directory() {
    let (_temporary, home) = fixture();
    assert!(ConsumptionStore::open_native_home(&home, owner().wrapping_add(1)).is_err());
    assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
    let store = ConsumptionStore::open_native_home(&home, owner()).unwrap();
    let path = namespace(&home);
    fs::rename(&path, path.with_extension("displaced")).unwrap();
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(store.consume(EXECUTION, RUN).is_err());
    assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
}

#[test]
fn native_store_keeps_private_ancestor_modes_and_identities_bound_after_open() {
    for replace in [false, true] {
        let (_temporary, home) = fixture();
        let store = ConsumptionStore::open_native_home(&home, owner()).unwrap();
        let token = store.consume(EXECUTION, RUN).unwrap();
        let application = home.join(PARTS[0]).join(PARTS[1]).join(PARTS[2]);
        if replace {
            fs::rename(&application, application.with_extension("displaced")).unwrap();
            fs::create_dir(&application).unwrap();
            fs::set_permissions(&application, fs::Permissions::from_mode(0o700)).unwrap();
        } else {
            fs::set_permissions(&application, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(token.revalidate().is_err());
        assert!(store.consume(&"b".repeat(64), &"d".repeat(64)).is_err());
    }
}

fn child(home: &Path, output: &Path, action: &str) -> Child {
    let scope = module_path!().split_once("::").unwrap().1;
    let exact = format!("{scope}::native_store_cross_process_child");
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &exact, "--ignored", "--nocapture"])
        .env("JOINERY_NATIVE_STORE_TEST_HOME", home)
        .env("JOINERY_NATIVE_STORE_TEST_OUTPUT", output)
        .env("JOINERY_NATIVE_STORE_TEST_ACTION", action)
        // These caller-selected locations cannot select the bootstrap namespace.
        .env("HOME", output)
        .env("XDG_DATA_HOME", output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn release(child: &mut Child) {
    child.stdin.take().unwrap().write_all(&[1]).unwrap();
}

fn finish(child: Child) -> String {
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[ignore = "internal subprocess entry"]
fn native_store_cross_process_child() {
    let Some(home) = std::env::var_os("JOINERY_NATIVE_STORE_TEST_HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    assert_eq!(home.file_name().unwrap(), "synthetic-home");
    let output = PathBuf::from(std::env::var_os("JOINERY_NATIVE_STORE_TEST_OUTPUT").unwrap());
    let action = std::env::var("JOINERY_NATIVE_STORE_TEST_ACTION").unwrap();
    let mut start = [0];
    std::io::stdin().read_exact(&mut start).unwrap();
    assert_eq!(start, [1]);
    let store = ConsumptionStore::open_native_home(&home, owner()).unwrap();
    assert_eq!(store.proof.path(), namespace(&home));
    store.require_separate_from(&output).unwrap();
    match action.as_str() {
        "consume" => match store.consume(EXECUTION, RUN) {
            Ok(token) => {
                token.revalidate().unwrap();
                println!("NATIVE_STORE_GRANTED");
            }
            Err(_) => println!("NATIVE_STORE_REJECTED"),
        },
        "open" => println!("NATIVE_STORE_OPENED"),
        _ => panic!("invalid synthetic child action"),
    }
    assert_eq!(fs::read_dir(output).unwrap().count(), 0);
}

#[test]
fn native_store_concurrent_process_bootstrap_and_consumption_has_one_winner() {
    let (temporary, home) = fixture();
    let mut children: Vec<_> = (0..4)
        .map(|index| {
            let output = temporary
                .path()
                .canonicalize()
                .unwrap()
                .join(format!("output-{index}"));
            fs::create_dir(&output).unwrap();
            fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();
            child(&home, &output, "consume")
        })
        .collect();
    for child in &mut children {
        release(child);
    }
    let output = children.into_iter().map(finish).collect::<String>();
    assert_eq!(output.matches("NATIVE_STORE_GRANTED").count(), 1);
    assert_eq!(output.matches("NATIVE_STORE_REJECTED").count(), 3);

    // Reopening from a fresh process with another output still spends the same
    // fixed markers; changing output/environment cannot reset the attempt.
    let output = temporary
        .path()
        .canonicalize()
        .unwrap()
        .join("fresh-output");
    fs::create_dir(&output).unwrap();
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).unwrap();
    let mut reopened = child(&home, &output, "consume");
    release(&mut reopened);
    assert!(finish(reopened).contains("NATIVE_STORE_REJECTED"));
    assert_eq!(fs::read_dir(namespace(&home)).unwrap().count(), 2);
}
