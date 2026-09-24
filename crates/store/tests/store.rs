//! Behaviour tests for the Vault store, through its public interface only.

use std::fs;
use std::path::{Path, PathBuf};

use store::{
    AUTO_SAFETY_KEEP, CloudProvider, REPLACED_SAFETY_KEEP_SECS, SAFETY_DIR, SETTINGS_FILE,
    SafetyKind, StoreError, VAULT_FILE, VaultDir, cloud_location, write_backup,
};

const DAY: i64 = 24 * 60 * 60;
const T0: i64 = 1_700_000_000;

fn accept(_: &[u8]) -> bool {
    true
}

fn reject(_: &[u8]) -> bool {
    false
}

fn data_dir() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Vault.noindex");
    (tmp, root)
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn temp_files(dir: &Path) -> Vec<String> {
    entries(dir)
        .into_iter()
        .filter(|n| n.starts_with(".tmp-"))
        .collect()
}

// ---------------------------------------------------------------- open & lock

#[test]
fn open_creates_the_data_folder_with_no_vault() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    assert!(root.is_dir());
    assert_eq!(dir.root(), root.as_path());
    assert!(!dir.has_vault());
    assert!(matches!(dir.read_vault(), Err(StoreError::NoVault)));
}

#[cfg(unix)]
#[test]
fn data_folder_is_private_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);

    dir.save_vault(b"fake-vault-v1", &accept, T0).unwrap();
    dir.save_vault(b"fake-vault-v2", &accept, T0 + 1).unwrap();
    dir.write_settings(b"{}").unwrap();
    for file in [root.join(VAULT_FILE), root.join(SETTINGS_FILE)] {
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{}", file.display());
    }
    let safety = root.join(SAFETY_DIR);
    for name in entries(&safety) {
        let mode = fs::metadata(safety.join(&name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{name}");
    }
}

#[test]
fn a_second_open_of_the_same_folder_reports_already_open() {
    let (_tmp, root) = data_dir();
    let first = VaultDir::open(&root).unwrap();
    assert!(matches!(
        VaultDir::open(&root),
        Err(StoreError::AlreadyOpen)
    ));
    drop(first);
    assert!(VaultDir::open(&root).is_ok());
}

#[test]
fn leftover_temp_files_are_removed_at_open_and_never_read_as_the_vault() {
    let (_tmp, root) = data_dir();
    {
        let dir = VaultDir::open(&root).unwrap();
        dir.save_vault(b"fake-vault-good", &accept, T0).unwrap();
    }
    fs::write(root.join(".tmp-deadbeef"), b"partial write").unwrap();
    fs::create_dir_all(root.join(SAFETY_DIR)).unwrap();
    fs::write(root.join(SAFETY_DIR).join(".tmp-cafebabe"), b"partial").unwrap();

    let dir = VaultDir::open(&root).unwrap();
    assert!(temp_files(&root).is_empty());
    assert!(temp_files(&root.join(SAFETY_DIR)).is_empty());
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-good");
}

// ---------------------------------------------------------------- saving

#[test]
fn saved_vault_reads_back() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-v1", &accept, T0).unwrap();
    assert!(dir.has_vault());
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-v1");
    assert!(temp_files(&root).is_empty());
}

#[test]
fn verify_is_called_on_the_bytes_read_back_from_disk() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    let seen = std::cell::RefCell::new(Vec::new());
    let verify = |b: &[u8]| {
        seen.borrow_mut().push(b.to_vec());
        true
    };
    dir.save_vault(b"fake-vault-v1", &verify, T0).unwrap();
    assert_eq!(seen.borrow().as_slice(), &[b"fake-vault-v1".to_vec()]);
}

#[test]
fn rejected_save_leaves_the_old_vault_and_no_temp_file() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-v1", &accept, T0).unwrap();
    let copies_before = dir.safety_copies().unwrap();

    let err = dir
        .save_vault(b"fake-vault-bad", &reject, T0 + 1)
        .unwrap_err();
    assert!(matches!(err, StoreError::VerifyFailed));
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-v1");
    assert!(temp_files(&root).is_empty());
    assert_eq!(dir.safety_copies().unwrap(), copies_before);
}

#[test]
fn rejected_first_save_leaves_no_vault() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    assert!(matches!(
        dir.save_vault(b"fake-vault-bad", &reject, T0),
        Err(StoreError::VerifyFailed)
    ));
    assert!(!dir.has_vault());
    assert!(temp_files(&root).is_empty());
}

// ---------------------------------------------------------------- Safety Copies

#[test]
fn each_save_keeps_the_previous_version_as_an_automatic_safety_copy() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-v1", &accept, T0).unwrap();
    dir.save_vault(b"fake-vault-v2", &accept, T0 + 10).unwrap();
    dir.save_vault(b"fake-vault-v3", &accept, T0 + 20).unwrap();

    let copies = dir.safety_copies().unwrap();
    assert_eq!(copies.len(), 2);
    assert!(copies.iter().all(|c| c.kind == SafetyKind::Automatic));
    // Newest first.
    assert_eq!(copies[0].created_at, T0 + 20);
    assert_eq!(copies[1].created_at, T0 + 10);
    assert_eq!(
        dir.read_safety_copy(&copies[0].id).unwrap(),
        b"fake-vault-v2"
    );
    assert_eq!(
        dir.read_safety_copy(&copies[1].id).unwrap(),
        b"fake-vault-v1"
    );
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-v3");
}

#[test]
fn only_the_newest_automatic_safety_copies_are_kept() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    let saves = AUTO_SAFETY_KEEP as i64 + 5;
    for i in 0..=saves {
        dir.save_vault(format!("fake-vault-v{i}").as_bytes(), &accept, T0 + i)
            .unwrap();
    }
    let copies = dir.safety_copies().unwrap();
    assert_eq!(copies.len(), AUTO_SAFETY_KEEP);
    assert_eq!(copies[0].created_at, T0 + saves);
    assert_eq!(
        dir.read_safety_copy(&copies[0].id).unwrap(),
        format!("fake-vault-v{}", saves - 1).as_bytes()
    );
    let oldest = copies.last().unwrap();
    assert_eq!(oldest.created_at, T0 + saves - AUTO_SAFETY_KEEP as i64 + 1);
    assert_eq!(entries(&root.join(SAFETY_DIR)).len(), AUTO_SAFETY_KEEP);
}

#[test]
fn replacing_keeps_the_current_vault_as_a_replaced_safety_copy() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-mine", &accept, T0).unwrap();
    dir.replace_vault(b"fake-vault-backup", &accept, T0 + 5)
        .unwrap();

    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-backup");
    let copies = dir.safety_copies().unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].kind, SafetyKind::Replaced);
    assert_eq!(copies[0].created_at, T0 + 5);
    assert_eq!(
        dir.read_safety_copy(&copies[0].id).unwrap(),
        b"fake-vault-mine"
    );
}

#[test]
fn replacing_when_there_is_no_vault_installs_it_without_a_safety_copy() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.replace_vault(b"fake-vault-backup", &accept, T0)
        .unwrap();
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-backup");
    assert!(dir.safety_copies().unwrap().is_empty());
}

#[test]
fn rejected_replace_leaves_the_vault_untouched() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-mine", &accept, T0).unwrap();
    assert!(matches!(
        dir.replace_vault(b"fake-vault-damaged", &reject, T0 + 1),
        Err(StoreError::VerifyFailed)
    ));
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-mine");
    assert!(dir.safety_copies().unwrap().is_empty());
    assert!(temp_files(&root).is_empty());
}

#[test]
fn replaced_safety_copies_survive_many_saves_within_30_days() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-mine", &accept, T0).unwrap();
    dir.replace_vault(b"fake-vault-backup", &accept, T0)
        .unwrap();
    for i in 1..=(AUTO_SAFETY_KEEP as i64 + 3) {
        dir.save_vault(format!("fake-vault-v{i}").as_bytes(), &accept, T0 + i)
            .unwrap();
    }
    let at_29_days = T0 + REPLACED_SAFETY_KEEP_SECS - DAY;
    dir.save_vault(b"fake-vault-later", &accept, at_29_days)
        .unwrap();

    let copies = dir.safety_copies().unwrap();
    let replaced: Vec<_> = copies
        .iter()
        .filter(|c| c.kind == SafetyKind::Replaced)
        .collect();
    assert_eq!(replaced.len(), 1);
    assert_eq!(
        dir.read_safety_copy(&replaced[0].id).unwrap(),
        b"fake-vault-mine"
    );
    let autos = copies
        .iter()
        .filter(|c| c.kind == SafetyKind::Automatic)
        .count();
    assert_eq!(autos, AUTO_SAFETY_KEEP);
}

#[test]
fn replaced_safety_copies_older_than_30_days_are_deleted() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-mine", &accept, T0).unwrap();
    dir.replace_vault(b"fake-vault-backup", &accept, T0)
        .unwrap();

    dir.save_vault(b"fake-vault-v2", &accept, T0 + REPLACED_SAFETY_KEEP_SECS)
        .unwrap();
    assert!(
        dir.safety_copies()
            .unwrap()
            .iter()
            .any(|c| c.kind == SafetyKind::Replaced),
        "exactly 30 days old is still kept"
    );

    dir.save_vault(
        b"fake-vault-v3",
        &accept,
        T0 + REPLACED_SAFETY_KEEP_SECS + DAY,
    )
    .unwrap();
    assert!(
        !dir.safety_copies()
            .unwrap()
            .iter()
            .any(|c| c.kind == SafetyKind::Replaced)
    );
}

#[test]
fn safety_copies_ignore_unrelated_files() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-v1", &accept, T0).unwrap();
    dir.save_vault(b"fake-vault-v2", &accept, T0 + 1).unwrap();
    let safety = root.join(SAFETY_DIR);
    fs::write(safety.join(".DS_Store"), b"x").unwrap();
    fs::write(safety.join("auto-notanumber-0.enote"), b"x").unwrap();
    fs::write(safety.join("notes.txt"), b"x").unwrap();
    let copies = dir.safety_copies().unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(
        dir.read_safety_copy(&copies[0].id).unwrap(),
        b"fake-vault-v1"
    );
}

#[test]
fn unknown_or_malicious_safety_copy_ids_are_not_found() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-v1", &accept, T0).unwrap();
    for id in [
        "auto-1-0.enote",
        "../vault.enote",
        "../../etc/passwd",
        "vault.enote",
        "",
        "auto-1-0.enote/../../vault.enote",
    ] {
        assert!(
            matches!(dir.read_safety_copy(id), Err(StoreError::NotFound)),
            "{id:?}"
        );
    }
}

// ---------------------------------------------------------------- settings

#[test]
fn settings_are_absent_until_written_and_then_read_back() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    assert_eq!(dir.read_settings().unwrap(), None);
    dir.write_settings(br#"{"auto_lock_minutes":5}"#).unwrap();
    assert_eq!(
        dir.read_settings().unwrap().unwrap(),
        br#"{"auto_lock_minutes":5}"#
    );
    dir.write_settings(br#"{"auto_lock_minutes":1}"#).unwrap();
    assert_eq!(
        dir.read_settings().unwrap().unwrap(),
        br#"{"auto_lock_minutes":1}"#
    );
    assert!(temp_files(&root).is_empty());
}

// ---------------------------------------------------------------- Backups

#[test]
fn backup_is_written_and_verified_from_what_was_read_back() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("my-backup.enote");
    let seen = std::cell::RefCell::new(Vec::new());
    let verify = |b: &[u8]| {
        seen.borrow_mut().push(b.to_vec());
        true
    };
    write_backup(&dest, b"fake-vault-backup", &verify).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"fake-vault-backup");
    assert_eq!(seen.borrow().as_slice(), &[b"fake-vault-backup".to_vec()]);
    assert!(temp_files(tmp.path()).is_empty());
}

#[test]
fn backup_overwrites_an_existing_file_only_after_verification() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("my-backup.enote");
    fs::write(&dest, b"fake-vault-older-backup").unwrap();

    assert!(matches!(
        write_backup(&dest, b"fake-vault-new", &reject),
        Err(StoreError::VerifyFailed)
    ));
    assert_eq!(fs::read(&dest).unwrap(), b"fake-vault-older-backup");
    assert!(temp_files(tmp.path()).is_empty());

    write_backup(&dest, b"fake-vault-new", &accept).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"fake-vault-new");
}

#[test]
fn backup_into_a_missing_folder_is_an_io_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("no-such-folder").join("my-backup.enote");
    assert!(matches!(
        write_backup(&dest, b"fake-vault", &accept),
        Err(StoreError::Io(_))
    ));
}

#[test]
fn a_backup_can_be_installed_as_the_vault() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("usb-backup.enote");
    let root = tmp.path().join("Vault.noindex");
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-vault-here", &accept, T0).unwrap();
    write_backup(&backup, &dir.read_vault().unwrap(), &accept).unwrap();
    dir.save_vault(b"fake-vault-changed", &accept, T0 + 1)
        .unwrap();

    let bytes = fs::read(&backup).unwrap();
    dir.replace_vault(&bytes, &accept, T0 + 2).unwrap();
    assert_eq!(dir.read_vault().unwrap(), b"fake-vault-here");
}

#[cfg(unix)]
#[test]
fn a_failed_backup_leaves_the_old_backup_untouched_even_when_the_folder_cannot_be_flushed() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    // Write-only folder: files can be created and renamed, but the folder itself cannot be
    // opened for the directory flush.
    let drop_box = tmp.path().join("write-only-folder");
    fs::create_dir(&drop_box).unwrap();
    let dest = drop_box.join("my-backup.enote");
    fs::write(&dest, b"fake-old-backup").unwrap();
    fs::set_permissions(&drop_box, fs::Permissions::from_mode(0o300)).unwrap();
    let result = write_backup(&dest, b"fake-new-backup", &accept);
    fs::set_permissions(&drop_box, fs::Permissions::from_mode(0o700)).unwrap();

    let on_disk = fs::read(&dest).unwrap();
    match result {
        Ok(()) => assert_eq!(on_disk, b"fake-new-backup"),
        Err(_) => assert_eq!(
            on_disk, b"fake-old-backup",
            "failure reported but old Backup was replaced"
        ),
    }
    assert!(temp_files(&drop_box).is_empty());
}

#[test]
fn safety_copies_of_both_kinds_in_the_same_second_list_newest_first() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-v0", &accept, T0).unwrap();
    dir.save_vault(b"fake-v1", &accept, T0).unwrap(); // Automatic copy of v0
    dir.replace_vault(b"fake-v2", &accept, T0).unwrap(); // Replaced copy of v1 (newer)
    dir.save_vault(b"fake-v3", &accept, T0).unwrap(); // Automatic copy of v2 (newest)

    let contents: Vec<Vec<u8>> = dir
        .safety_copies()
        .unwrap()
        .iter()
        .map(|c| dir.read_safety_copy(&c.id).unwrap())
        .collect();
    assert_eq!(
        contents,
        vec![
            b"fake-v2".to_vec(),
            b"fake-v1".to_vec(),
            b"fake-v0".to_vec()
        ]
    );
}

#[test]
fn a_clock_going_back_never_loses_the_version_just_replaced() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    for i in 0..=AUTO_SAFETY_KEEP as i64 {
        dir.save_vault(format!("fake-v{i}").as_bytes(), &accept, T0 + i)
            .unwrap();
    }
    let previous = dir.read_vault().unwrap();
    dir.save_vault(b"fake-after-clock-change", &accept, T0 - 3600)
        .unwrap();

    let copies = dir.safety_copies().unwrap();
    assert_eq!(
        dir.read_safety_copy(&copies[0].id).unwrap(),
        previous,
        "{copies:?}"
    );
    assert_eq!(
        copies
            .iter()
            .filter(|c| c.kind == SafetyKind::Automatic)
            .count(),
        AUTO_SAFETY_KEEP
    );

    // And the next save after that still keeps the one just before it.
    dir.save_vault(b"fake-later", &accept, T0 - 3599).unwrap();
    let copies = dir.safety_copies().unwrap();
    assert_eq!(
        dir.read_safety_copy(&copies[0].id).unwrap(),
        b"fake-after-clock-change"
    );
    assert_eq!(dir.read_safety_copy(&copies[1].id).unwrap(), previous);
}

#[test]
fn a_replaced_copy_made_after_the_clock_went_back_is_not_expired() {
    let (_tmp, root) = data_dir();
    let dir = VaultDir::open(&root).unwrap();
    dir.save_vault(b"fake-v0", &accept, T0).unwrap();
    dir.save_vault(b"fake-v1", &accept, T0 + 1).unwrap();
    dir.replace_vault(b"fake-v2", &accept, T0 - 40 * DAY)
        .unwrap();
    let copies = dir.safety_copies().unwrap();
    assert_eq!(copies[0].kind, SafetyKind::Replaced);
    assert_eq!(dir.read_safety_copy(&copies[0].id).unwrap(), b"fake-v1");
}

// ---------------------------------------------------------------- cloud folders

fn home() -> PathBuf {
    PathBuf::from("/Users/tester")
}

fn at(parts: &[&str]) -> PathBuf {
    parts.iter().fold(home(), |p, part| p.join(part))
}

#[test]
fn icloud_drive_folders_are_cloud() {
    assert_eq!(
        cloud_location(
            &at(&[
                "Library",
                "Mobile Documents",
                "com~apple~CloudDocs",
                "b.enote"
            ]),
            &home(),
            true
        ),
        Some(CloudProvider::ICloudDrive)
    );
    assert_eq!(
        cloud_location(
            &at(&["Library", "CloudStorage", "iCloud Drive"]),
            &home(),
            true
        ),
        Some(CloudProvider::ICloudDrive)
    );
    assert_eq!(
        cloud_location(
            &at(&["Library", "CloudStorage", "SomeOtherProvider", "x"]),
            &home(),
            true
        ),
        Some(CloudProvider::ICloudDrive)
    );
}

#[test]
fn cloud_storage_folders_report_the_specific_provider() {
    let cases = [
        ("OneDrive-Personal", CloudProvider::OneDrive),
        ("OneDrive-SharedLibraries-Contoso", CloudProvider::OneDrive),
        ("Dropbox", CloudProvider::Dropbox),
        ("Dropbox-Personal", CloudProvider::Dropbox),
        ("GoogleDrive-tester@example.com", CloudProvider::GoogleDrive),
    ];
    for (folder, provider) in cases {
        assert_eq!(
            cloud_location(
                &at(&["Library", "CloudStorage", folder, "b.enote"]),
                &home(),
                true
            ),
            Some(provider),
            "{folder}"
        );
    }
}

#[test]
fn provider_folders_are_matched_anywhere_case_insensitively() {
    let cases: [(&[&str], CloudProvider); 7] = [
        (&["OneDrive", "b.enote"], CloudProvider::OneDrive),
        (&["onedrive"], CloudProvider::OneDrive),
        (
            &["OneDrive - Contoso Ltd", "Backups"],
            CloudProvider::OneDrive,
        ),
        (&["Dropbox", "b.enote"], CloudProvider::Dropbox),
        (&["DROPBOX"], CloudProvider::Dropbox),
        (&["Google Drive", "b.enote"], CloudProvider::GoogleDrive),
        (&["My Drive"], CloudProvider::GoogleDrive),
    ];
    for (parts, provider) in cases {
        assert_eq!(
            cloud_location(&at(parts), &home(), false),
            Some(provider),
            "{parts:?}"
        );
    }
    assert_eq!(
        cloud_location(
            Path::new("/Volumes/External/Dropbox/b.enote"),
            &home(),
            false
        ),
        Some(CloudProvider::Dropbox)
    );
}

#[test]
fn mac_desktop_and_documents_may_be_synced() {
    for folder in ["Desktop", "Documents", "documents"] {
        assert_eq!(
            cloud_location(&at(&[folder, "b.enote"]), &home(), true),
            Some(CloudProvider::MacDesktopOrDocuments),
            "{folder}"
        );
    }
    assert_eq!(
        cloud_location(&at(&["Documents"]), &home(), true),
        Some(CloudProvider::MacDesktopOrDocuments)
    );
}

#[test]
fn desktop_and_documents_are_not_flagged_off_mac() {
    assert_eq!(
        cloud_location(&at(&["Desktop", "b.enote"]), &home(), false),
        None
    );
    assert_eq!(
        cloud_location(&at(&["Documents", "b.enote"]), &home(), false),
        None
    );
}

#[test]
fn a_specific_provider_wins_over_the_mac_documents_rule() {
    assert_eq!(
        cloud_location(&at(&["Documents", "Dropbox", "b.enote"]), &home(), true),
        Some(CloudProvider::Dropbox)
    );
}

#[test]
fn ordinary_local_folders_are_not_cloud() {
    let local = [
        at(&["Backups", "b.enote"]),
        at(&["Downloads"]),
        at(&["Library", "Application Support", "x"]),
        PathBuf::from("/Volumes/USB STICK/b.enote"),
        PathBuf::from("/tmp/b.enote"),
        at(&["OneDriveNotReally"]),
        at(&["MyDropboxStuff"]),
        // A folder named Documents that is not the home Documents folder.
        PathBuf::from("/Volumes/USB STICK/Documents/b.enote"),
    ];
    for path in local {
        assert_eq!(
            cloud_location(&path, &home(), true),
            None,
            "{}",
            path.display()
        );
    }
}

#[test]
fn parent_folder_steps_are_resolved_before_matching() {
    assert_eq!(
        cloud_location(
            &at(&[
                "tmp",
                "..",
                "Library",
                "CloudStorage",
                "OneDrive-Personal",
                "x"
            ]),
            &home(),
            true
        ),
        Some(CloudProvider::OneDrive)
    );
    assert_eq!(
        cloud_location(&at(&["x", "..", "Documents", "b.enote"]), &home(), true),
        Some(CloudProvider::MacDesktopOrDocuments)
    );
    assert_eq!(
        cloud_location(
            &at(&[
                "Library",
                "Mobile Documents",
                "..",
                "..",
                "Backups",
                "b.enote"
            ]),
            &home(),
            true
        ),
        None
    );
    assert_eq!(
        cloud_location(&at(&["Dropbox", "..", "Backups", "b.enote"]), &home(), true),
        None
    );
    // `..` above the root stays at the root.
    assert_eq!(
        cloud_location(
            Path::new("/../Users/tester/Documents/b.enote"),
            &home(),
            true
        ),
        Some(CloudProvider::MacDesktopOrDocuments)
    );
}
