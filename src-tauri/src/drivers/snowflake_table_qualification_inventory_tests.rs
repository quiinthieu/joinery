use super::*;
use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
};

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(directory.path()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _directory: directory,
            path,
        }
    }

    fn file(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        path
    }

    fn directory(&self, relative: impl AsRef<Path>) -> PathBuf {
        let path = self.path.join(relative);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn private_file(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> PathBuf {
        let path = self.file(relative, bytes);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
}

#[test]
fn inventory_complete_sorted_content_includes_root_and_empty_directories() {
    let first = Fixture::new();
    first.directory("empty");
    first.directory("nested");
    first.file("nested/z", b"last");
    first.file("a", b"abc");
    let second = Fixture::new();
    second.file("a", b"abc");
    second.directory("nested");
    second.file("nested/z", b"last");
    second.directory("empty");
    let a = Inventory::capture(&first.path, InventoryKind::Tree).unwrap();
    let b = Inventory::capture(&second.path, InventoryKind::Tree).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.sha256().unwrap(), b.sha256().unwrap());
    assert_eq!(a.entries.len(), 5);
    assert_eq!(a.entries[0].path_hex, "");
    assert_eq!(a.entries[0].kind, InventoryKind::Tree);
    assert_eq!(a.entries[1].path_hex, hex(b"a"));
    assert_eq!(a.entries[1].length, 3);
    assert_eq!(
        a.entries[1].sha256.as_deref(),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    assert_eq!(a.entries[2].path_hex, hex(b"empty"));
    assert_eq!(a.entries[2].length, 0);
    assert!(a.entries[2].sha256.is_none());
    assert!(a.verify(&first.path).is_ok());
    let encoded = serde_json::to_vec(&a).unwrap();
    assert_eq!(
        a.sha256().unwrap(),
        format!("{:x}", Sha256::digest(encoded))
    );
}

#[test]
fn inventory_full_file_hash_matches_standard_million_byte_vector() {
    let fixture = Fixture::new();
    // The standard SHA-256 million-'a' vector spans many inventory reads and
    // compression blocks; keep it independent of the selected digest backend.
    let path = fixture.file("million-a", &vec![b'a'; 1_000_000]);
    let inventory = Inventory::capture(&path, InventoryKind::File).unwrap();
    assert_eq!(inventory.entries.len(), 1);
    assert_eq!(inventory.entries[0].length, 1_000_000);
    assert_eq!(
        inventory.entries[0].sha256.as_deref(),
        Some("cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0")
    );
    inventory.verify(&path).unwrap();
}

#[test]
fn inventory_hashes_full_content_despite_same_length_and_restored_mtime() {
    let fixture = Fixture::new();
    let mut bytes = vec![b'a'; 3 * 64 * 1024 + 17];
    let file = fixture.file("content", &bytes);
    let before = fs::metadata(&file).unwrap();
    let inventory = Inventory::capture(&file, InventoryKind::File).unwrap();
    bytes[64 * 1024 + 7] = b'b';
    let last = bytes.len() - 1;
    bytes[last] = b'c';
    fs::write(&file, &bytes).unwrap();
    File::open(&file)
        .unwrap()
        .set_modified(before.modified().unwrap())
        .unwrap();
    let after = fs::metadata(&file).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert!(inventory.verify(&file).is_err());
    let changed = Inventory::capture(&file, InventoryKind::File).unwrap();
    assert_eq!(
        changed.entries[0].sha256.as_deref(),
        Some(format!("{:x}", Sha256::digest(bytes)).as_str())
    );
}

#[test]
fn inventory_verify_detects_added_removed_renamed_nested_empty_and_modes() {
    for change in [
        "add", "remove", "rename", "nested", "empty", "fileMode", "dirMode",
    ] {
        let fixture = Fixture::new();
        let original = fixture.file("original", b"retained");
        let nested = fixture.directory("nested");
        let inventory = Inventory::capture(&fixture.path, InventoryKind::Tree).unwrap();
        match change {
            "add" => {
                fixture.file("added", b"new");
            }
            "remove" => fs::remove_file(original).unwrap(),
            "rename" => fs::rename(original, fixture.path.join("renamed")).unwrap(),
            "nested" => {
                fixture.file("nested/data", b"new");
            }
            "empty" => {
                fixture.directory("empty");
            }
            "fileMode" => fs::set_permissions(original, fs::Permissions::from_mode(0o444)).unwrap(),
            "dirMode" => fs::set_permissions(nested, fs::Permissions::from_mode(0o700)).unwrap(),
            _ => unreachable!(),
        }
        assert!(inventory.verify(&fixture.path).is_err(), "{change}");
    }
}

#[test]
fn inventory_rejects_symlink_roots_ancestors_and_entries() {
    let fixture = Fixture::new();
    let file = fixture.file("target", b"content");
    let directory = fixture.directory("directory");
    fixture.file("directory/leaf", b"leaf");
    let file_link = fixture.path.join("file-link");
    let dir_link = fixture.path.join("dir-link");
    std::os::unix::fs::symlink(&file, &file_link).unwrap();
    std::os::unix::fs::symlink(&directory, &dir_link).unwrap();
    assert!(Inventory::capture(&file_link, InventoryKind::File).is_err());
    assert!(Inventory::capture(&dir_link, InventoryKind::Tree).is_err());
    assert!(Inventory::capture(&dir_link.join("leaf"), InventoryKind::File).is_err());
    assert!(Inventory::capture(&fixture.path, InventoryKind::Tree).is_err());
}

#[test]
fn inventory_rejects_aliases_and_preserves_non_utf8_names_losslessly() {
    let fixture = Fixture::new();
    let name = vec![0xff, b'a'];
    assert_eq!(hex(&name), "ff61");
    let file = fixture.path.join(OsString::from_vec(name));
    match fs::write(&file, b"bytes") {
        Ok(()) => {
            fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
            let inventory = Inventory::capture(&fixture.path, InventoryKind::Tree).unwrap();
            assert_eq!(inventory.entries[1].path_hex, "ff61");
            assert!(inventory.verify(&fixture.path).is_ok());
        }
        // APFS rejects invalid UTF-8 filename bytes. Keep every alias check
        // below, and test the lossless byte encoding above on that filesystem.
        Err(error) if error.raw_os_error() == Some(libc::EILSEQ) => {
            eprintln!("Filesystem rejects non-UTF8 filename creation (EILSEQ)");
        }
        Err(error) => panic!("Unexpected fixture filename creation failure: {error}"),
    }
    assert!(Inventory::capture(Path::new("relative"), InventoryKind::Tree).is_err());
    assert!(
        Inventory::capture(
            Path::new(&format!("/{}", vec!["component"; MAX_DEPTH + 1].join("/"))),
            InventoryKind::Tree,
        )
        .is_err()
    );
    let display = fixture.path.display();
    for path in [
        format!("{display}/."),
        format!("{display}/../x"),
        format!("{display}//x"),
        format!("{display}/"),
    ] {
        assert!(Inventory::capture(Path::new(&path), InventoryKind::Tree).is_err());
    }
    let valid_file = fixture.file("regular-file", b"content");
    assert!(Inventory::capture(&valid_file, InventoryKind::Tree).is_err());
    assert!(Inventory::capture(&fixture.path, InventoryKind::File).is_err());
}

#[test]
fn inventory_rejects_hardlinks_and_special_files_without_opening_them() {
    let fixture = Fixture::new();
    let file = fixture.file("original", b"content");
    let link = fixture.path.join("hardlink");
    fs::hard_link(&file, &link).unwrap();
    assert!(Inventory::capture(&file, InventoryKind::File).is_err());
    assert!(Inventory::capture(&fixture.path, InventoryKind::Tree).is_err());
    fs::remove_file(link).unwrap();
    let socket = fixture.path.join("socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    assert!(Inventory::capture(&socket, InventoryKind::File).is_err());
    assert!(Inventory::capture(&fixture.path, InventoryKind::Tree).is_err());
    fs::remove_file(socket).unwrap();
    let fifo = fixture.path.join("fifo");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(Inventory::capture(&fifo, InventoryKind::File).is_err());
    assert!(Inventory::capture(&fixture.path, InventoryKind::Tree).is_err());
}

#[test]
fn inventory_permissions_allow_read_execute_but_reject_write_and_special_bits() {
    let fixture = Fixture::new();
    let file = fixture.file("file", b"content");
    for mode in [0o400, 0o444, 0o500, 0o555, 0o600, 0o644, 0o755] {
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            Inventory::capture(&file, InventoryKind::File).is_ok(),
            "{mode:o}"
        );
    }
    for mode in [0o620, 0o602, 0o666, 0o4755, 0o2755, 0o1755] {
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            Inventory::capture(&file, InventoryKind::File).is_err(),
            "{mode:o}"
        );
    }
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    for mode in [0o777, 0o1777, 0o2755] {
        fs::set_permissions(&fixture.path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(Inventory::capture(&fixture.path, InventoryKind::Tree).is_err());
    }
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o700)).unwrap();
    let nested = fixture.directory("nested");
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(Inventory::capture(&nested, InventoryKind::Tree).is_err());
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn inventory_foreign_owner_and_sticky_ancestor_rules_are_closed() {
    let fixture = Fixture::new();
    let file = fixture.file("file", b"content");
    let euid = unsafe { libc::geteuid() };
    let root = Root::open(&file, InventoryKind::File, euid).unwrap();
    let mut foreign = root.stamp;
    foreign.uid = euid.wrapping_add(1);
    assert!(included(foreign, euid).is_err());
    let mut directory = held(&root.parent).unwrap();
    directory.uid = 0;
    directory.mode = libc::S_IFDIR | 0o1777;
    assert!(ancestor(directory, euid).is_ok());
    directory.mode = libc::S_IFDIR | 0o777;
    assert!(ancestor(directory, euid).is_err());
    directory.mode = libc::S_IFDIR | 0o3777;
    assert!(ancestor(directory, euid).is_err());
    directory.uid = euid.wrapping_add(1);
    directory.mode = libc::S_IFDIR | 0o755;
    assert!(ancestor(directory, euid).is_err());
}

#[test]
fn inventory_fixed_limits_count_root_and_bound_depth_entries_and_bytes() {
    assert_eq!(MAX_DEPTH, 64);
    assert_eq!(MAX_ENTRIES, 32_768);
    assert_eq!(MAX_FILE_BYTES, 256 * 1024 * 1024);
    assert_eq!(MAX_TOTAL_BYTES, 512 * 1024 * 1024);
    let fixture = Fixture::new();
    let one = Limits {
        entries: 1,
        ..Limits::default()
    };
    assert!(Inventory::capture_with_limits(&fixture.path, InventoryKind::Tree, one).is_ok());
    let file = fixture.file("file", b"12345678");
    assert!(Inventory::capture_with_limits(&fixture.path, InventoryKind::Tree, one).is_err());
    let exact = Limits {
        file_bytes: 8,
        total_bytes: 8,
        entries: 2,
        ..Limits::default()
    };
    assert!(Inventory::capture_with_limits(&fixture.path, InventoryKind::Tree, exact).is_ok());
    assert!(
        Inventory::capture_with_limits(
            &file,
            InventoryKind::File,
            Limits {
                file_bytes: 7,
                ..exact
            }
        )
        .is_err()
    );
    fixture.file("second", b"x");
    assert!(
        Inventory::capture_with_limits(
            &fixture.path,
            InventoryKind::Tree,
            Limits {
                entries: 3,
                ..exact
            }
        )
        .is_err()
    );
    let deep = Fixture::new();
    let first = deep.directory("first");
    let depth = Limits {
        depth: 1,
        ..Limits::default()
    };
    assert!(Inventory::capture_with_limits(&deep.path, InventoryKind::Tree, depth).is_ok());
    fs::create_dir(first.join("second")).unwrap();
    assert!(Inventory::capture_with_limits(&deep.path, InventoryKind::Tree, depth).is_err());
}

#[test]
fn inventory_final_recheck_detects_content_changes_and_name_replacement() {
    for mutation in ["content", "replace", "add", "remove", "rename"] {
        let fixture = Fixture::new();
        let file = fixture.file("file", b"before");
        let euid = unsafe { libc::geteuid() };
        let root = Root::open(&fixture.path, InventoryKind::Tree, euid).unwrap();
        let mut capture = Capture {
            limits: Limits::default(),
            euid,
            reserved_entries: 1,
            total_bytes: 0,
            directories: HashSet::new(),
            entries: Vec::new(),
            checks: HashMap::new(),
        };
        capture
            .visit(&root.file, root.stamp, Vec::new(), 0)
            .unwrap();
        match mutation {
            "content" => fs::write(&file, b"after!").unwrap(),
            "replace" => {
                fs::remove_file(&file).unwrap();
                fixture.file("file", b"before");
            }
            "add" => {
                fixture.file("added", b"new");
            }
            "remove" => fs::remove_file(&file).unwrap(),
            "rename" => fs::rename(&file, fixture.path.join("renamed")).unwrap(),
            _ => unreachable!(),
        }
        assert!(capture.recheck(&root.file, &[]).is_err(), "{mutation}");
    }
}

#[test]
fn inventory_named_and_ancestor_rechecks_reject_replacement_with_held_fds() {
    let fixture = Fixture::new();
    let directory = fixture.directory("parent");
    let file = fixture.file("parent/file", b"retained");
    let euid = unsafe { libc::geteuid() };
    let root = Root::open(&file, InventoryKind::File, euid).unwrap();
    fs::rename(&directory, fixture.path.join("moved")).unwrap();
    fixture.directory("parent");
    fixture.file("parent/file", b"retained");
    assert!(root.recheck(euid).is_err());
}

#[test]
fn inventory_named_observation_mismatch_is_rejected_before_restored_open() {
    let fixture = Fixture::new();
    fixture.file("file", b"retained");
    let euid = unsafe { libc::geteuid() };
    let root = Root::open(&fixture.path, InventoryKind::Tree, euid).unwrap();
    let mut capture = Capture {
        limits: Limits::default(),
        euid,
        reserved_entries: 1,
        total_bytes: 0,
        directories: HashSet::new(),
        entries: Vec::new(),
        checks: HashMap::new(),
    };
    capture
        .visit(&root.file, root.stamp, Vec::new(), 0)
        .unwrap();
    let actual = named(&root.file, c"file").unwrap();
    assert!(capture.check_named(b"file", actual).is_ok());
    // Model an already-observed temporary permission/identity mismatch while
    // the actual filesystem presented to a subsequent open is the old node.
    let mut observed = actual;
    observed.mode ^= 0o100;
    assert!(included(observed, euid).is_ok());
    assert!(capture.check_named(b"file", observed).is_err());
    observed = actual;
    observed.inode = observed.inode.wrapping_add(1);
    assert!(capture.check_named(b"file", observed).is_err());
    assert!(capture.recheck(&root.file, &[]).is_ok());
}

#[test]
fn inventory_private_file_is_bounded_exact0600_and_nofollow() {
    let fixture = Fixture::new();
    let file = fixture.file("synthetic-profile", br#"{"account":"synthetic"}"#);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let bytes = fs::read(&file).unwrap();
    assert_eq!(read_private_file(&file, bytes.len()).unwrap(), bytes);
    assert!(read_private_file(&file, bytes.len() - 1).is_err());
    assert!(read_private_file(&file, 0).is_err());
    assert!(read_private_file(&file, 64 * 1024 + 1).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_private_file(&file, 16 * 1024).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let link = fixture.path.join("profile-link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(read_private_file(&link, 16 * 1024).is_err());
    let directory = fixture.directory("private");
    let nested = fixture.file("private/profile", b"synthetic");
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = fixture.path.join("ancestor-link");
    std::os::unix::fs::symlink(directory, &alias).unwrap();
    assert!(read_private_file(&alias.join("profile"), 16 * 1024).is_err());
}

#[test]
fn inventory_private_envelope_accepts_exact_16mib_and_rejects_one_more_byte() {
    assert_eq!(MAX_PRIVATE_ENVELOPE_BYTES, 16 * 1024 * 1024);
    let fixture = Fixture::new();
    let bytes = vec![b'e'; MAX_PRIVATE_ENVELOPE_BYTES];
    let file = fixture.private_file("synthetic-envelope", &bytes);
    assert_eq!(read_private_envelope(&file).unwrap(), bytes);
    File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_len((MAX_PRIVATE_ENVELOPE_BYTES + 1) as u64)
        .unwrap();
    assert!(read_private_envelope(&file).is_err());
}

#[test]
fn inventory_private_envelope_does_not_widen_private_file_64kib_limit() {
    let fixture = Fixture::new();
    let bytes = vec![b'e'; 64 * 1024 + 1];
    let file = fixture.private_file("synthetic-envelope", &bytes);
    // This is the original envelope regression: the existing API must continue
    // rejecting evidence above its profile/approval ceiling, even with a larger
    // requested maximum. Only the closed envelope route accepts these bytes.
    assert!(read_private_file(&file, 64 * 1024).is_err());
    assert!(read_private_file(&file, bytes.len()).is_err());
    assert!(read_private_file(&file, MAX_PRIVATE_ENVELOPE_BYTES).is_err());
    assert_eq!(read_private_envelope(&file).unwrap(), bytes);
}

#[test]
fn inventory_private_envelope_requires_exact0600_regular_single_link_file() {
    let fixture = Fixture::new();
    let file = fixture.private_file("synthetic-envelope", b"synthetic");
    for mode in [0o400, 0o644, 0o700, 0o602, 0o620, 0o1600, 0o2600, 0o4600] {
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
        assert!(read_private_envelope(&file).is_err(), "{mode:o}");
    }
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let hardlink = fixture.path.join("hardlink");
    fs::hard_link(&file, &hardlink).unwrap();
    assert!(read_private_envelope(&file).is_err());
    assert!(read_private_envelope(&hardlink).is_err());
    fs::remove_file(hardlink).unwrap();
    assert_eq!(read_private_envelope(&file).unwrap(), b"synthetic");
    assert!(read_private_envelope(&fixture.path).is_err());
    let socket = fixture.path.join("socket");
    let _listener = UnixListener::bind(&socket).unwrap();
    assert!(read_private_envelope(&socket).is_err());
    let fifo = fixture.path.join("fifo");
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(read_private_envelope(&fifo).is_err());
}

#[test]
fn inventory_private_envelope_rejects_symlinks_and_insecure_ancestors() {
    let fixture = Fixture::new();
    let directory = fixture.directory("private");
    let file = fixture.private_file("private/envelope", b"synthetic");
    let link = fixture.path.join("envelope-link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(read_private_envelope(&link).is_err());
    let alias = fixture.path.join("ancestor-link");
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    assert!(read_private_envelope(&alias.join("envelope")).is_err());
    for mode in [0o777, 0o2755] {
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
        assert!(read_private_envelope(&file).is_err(), "{mode:o}");
    }
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(read_private_envelope(&file).unwrap(), b"synthetic");
}

#[test]
fn inventory_private_envelope_reader_detects_growth_truncation_and_replacement() {
    let euid = unsafe { libc::geteuid() };
    for mutation in ["grow", "truncate", "replace", "ancestor", "mode"] {
        let fixture = Fixture::new();
        let directory = fixture.directory("private");
        let file = fixture.private_file("private/envelope", b"synthetic");
        // The held Root is the existing deterministic seam: mutate only after
        // its no-follow open/metadata capture, before the shared read/recheck.
        let root = Root::open(&file, InventoryKind::File, euid).unwrap();
        match mutation {
            "grow" => fs::write(&file, b"synthetic-more").unwrap(),
            "truncate" => fs::write(&file, b"short").unwrap(),
            "replace" => {
                fs::remove_file(&file).unwrap();
                fixture.private_file("private/envelope", b"synthetic");
            }
            "ancestor" => {
                fs::rename(directory, fixture.path.join("moved")).unwrap();
                fixture.directory("private");
                fixture.private_file("private/envelope", b"synthetic");
            }
            "mode" => fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap(),
            _ => unreachable!(),
        }
        assert!(
            read_private_root(root, MAX_PRIVATE_ENVELOPE_BYTES, euid).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn inventory_private_envelope_reader_bounds_growth_beyond_16mib() {
    let fixture = Fixture::new();
    let file = fixture.private_file("synthetic-envelope", b"");
    let writable = File::options().write(true).open(&file).unwrap();
    writable.set_len(MAX_PRIVATE_ENVELOPE_BYTES as u64).unwrap();
    let euid = unsafe { libc::geteuid() };
    let root = Root::open(&file, InventoryKind::File, euid).unwrap();
    writable
        .set_len((MAX_PRIVATE_ENVELOPE_BYTES + 1) as u64)
        .unwrap();
    let error = read_private_root(root, MAX_PRIVATE_ENVELOPE_BYTES, euid).unwrap_err();
    // Exceeding the ceiling must fail in the bounded loop, before the generic
    // exact-length/post-read mutation error could hide an unbounded read.
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::to_value(invalid()).unwrap()
    );
}

#[test]
fn inventory_errors_do_not_contain_paths_or_contents() {
    let fixture = Fixture::new();
    let file = fixture.file("sensitive-filename", b"sensitive-content");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o666)).unwrap();
    let error = Inventory::capture(&file, InventoryKind::File).unwrap_err();
    let encoded = serde_json::to_string(&error).unwrap();
    assert!(!encoded.contains("sensitive-filename"));
    assert!(!encoded.contains("sensitive-content"));
    assert!(!encoded.contains(&fixture.path.display().to_string()));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "Requires the locally built zero-link macOS Snowflake runtime"]
fn inventory_packaged_runtime_complete_content_capture_and_verify() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/release/bundle/macos/Joinery.app/Contents/Resources/snowflake-runtime");
    let started = std::time::Instant::now();
    let inventory = Inventory::capture(&path, InventoryKind::Tree).unwrap();
    let capture_ms = started.elapsed().as_millis();
    assert!(!inventory.entries.is_empty());
    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| { entry.path_hex == hex(b"helper") && entry.kind == InventoryKind::Tree })
    );
    assert!(
        inventory.entries.iter().any(|entry| {
            entry.path_hex == hex(b"bin/node") && entry.kind == InventoryKind::File
        })
    );
    let file_bytes = inventory
        .entries
        .iter()
        .map(|entry| entry.length)
        .sum::<u64>();
    let started = std::time::Instant::now();
    inventory.verify(&path).unwrap();
    let verify_ms = started.elapsed().as_millis();
    eprintln!(
        "inventory entries={} fileBytes={} sha256={} captureMs={} verifyMs={}",
        inventory.entries.len(),
        file_bytes,
        inventory.sha256().unwrap(),
        capture_ms,
        verify_ms,
    );
}
