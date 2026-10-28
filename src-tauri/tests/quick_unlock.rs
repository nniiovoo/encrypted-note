//! Quick Unlock through public Rust interfaces and a fake Keychain payload. No OS storage/UI.

use encrypted_note_app::core::native::{NativeAction, NativeCredentials, NativeResult};
use encrypted_note_app::core::{
    ApiError, AppCore, Clipboard, Clock, Config, CredentialInput, NoteInput,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use vault_format::{KdfParams, Limits, QuickUnlockKey};
use zeroize::Zeroizing;

const PASSWORD: &str = "fake-lantern-quartz-mosaic-velvet-orbit";
const NEW_PASSWORD: &str = "fake-harbor-pebble-cinder-meadow-tundra";
const WRONG: &str = "fake-wrong-password-only";
const WEEK_MS: u64 = 7 * 24 * 60 * 60_000;

fn text(value: &str) -> Zeroizing<String> {
    Zeroizing::new(value.to_owned())
}

struct FakeClock {
    ms: Arc<AtomicU64>,
    continuous: Arc<AtomicBool>,
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }
    fn unix_secs(&self) -> i64 {
        1_790_000_000 + (self.now_ms() / 1_000) as i64
    }
    fn continuous_ms(&self) -> Option<u64> {
        self.continuous
            .load(Ordering::SeqCst)
            .then(|| self.now_ms())
    }
}
struct FakeClipboard;
impl Clipboard for FakeClipboard {
    fn write(&mut self, _: &str) -> Result<u64, ApiError> {
        Ok(1)
    }
    fn clear_if(&mut self, _: u64) -> bool {
        true
    }
}
struct App {
    core: AppCore,
    temp: tempfile::TempDir,
    ms: Arc<AtomicU64>,
    continuous: Arc<AtomicBool>,
}
fn start(root: &Path, ms: &Arc<AtomicU64>, continuous: &Arc<AtomicBool>) -> AppCore {
    let mut core = AppCore::new(Config {
        data_dir: root.join("Vault.noindex"),
        home_dir: root.to_owned(),
        limits: Limits::TESTING,
        clock: Box::new(FakeClock {
            ms: ms.clone(),
            continuous: continuous.clone(),
        }),
        clipboard: Box::new(FakeClipboard),
        activity_anywhere_supported: false,
    })
    .unwrap();
    core.set_quick_unlock_availability(true, None);
    core
}
impl App {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let ms = Arc::new(AtomicU64::new(1_000));
        let continuous = Arc::new(AtomicBool::new(true));
        let mut core = start(temp.path(), &ms, &continuous);
        // The platform runtime consumes startup cleanup before accepting enrollment.
        let _ = core.take_quick_unlock_cleanup();
        let key = core
            .create_vault(text(PASSWORD), KdfParams::TESTING)
            .unwrap()
            .recovery_key
            .unwrap();
        core.confirm_recovery_kit(&key[key.len() - 9..]).unwrap();
        Self {
            core,
            temp,
            ms,
            continuous,
        }
    }
    fn restart(self) -> Self {
        let Self {
            core,
            temp,
            ms,
            continuous,
        } = self;
        drop(core);
        Self {
            core: start(temp.path(), &ms, &continuous),
            temp,
            ms,
            continuous,
        }
    }
    fn native(
        &mut self,
        action: NativeAction,
        credentials: NativeCredentials,
    ) -> Result<NativeResult, ApiError> {
        let operation = self.core.begin_native(action)?;
        let job = self.core.prepare_native(operation.id, credentials)?;
        self.core.finish_native(job.run())
    }
    fn enroll(&mut self) -> Zeroizing<Vec<u8>> {
        let NativeResult::QuickUnlockEnrollment(enrollment) = self
            .native(
                NativeAction::EnrollQuickUnlock,
                NativeCredentials::Password(text(PASSWORD)),
            )
            .unwrap()
        else {
            panic!("expected enrollment");
        };
        let payload = enrollment
            .key
            .with_keychain_payload(|bytes| Zeroizing::new(bytes.to_vec()));
        self.core
            .finish_quick_unlock_enrollment(enrollment.verify(item(&payload)))
            .unwrap();
        payload
    }
    fn vault(&self) -> PathBuf {
        self.temp.path().join("Vault.noindex/vault.enote")
    }
    fn settings(&self) -> serde_json::Value {
        serde_json::from_slice(
            &std::fs::read(self.temp.path().join("Vault.noindex/settings.json")).unwrap(),
        )
        .unwrap()
    }
    fn unlock_with(&mut self, payload: &[u8]) -> Result<(), ApiError> {
        let job = self.core.prepare_quick_unlock()?;
        self.core.finish_quick_unlock(job.open(item(payload)))
    }
    fn native_unlock(&mut self) {
        self.native(
            NativeAction::Unlock { recovery: false },
            NativeCredentials::Password(text(PASSWORD)),
        )
        .unwrap();
    }
}

#[cfg(target_os = "macos")]
#[test]
fn startup_reaps_unregistered_blob_without_rewriting_disabled_settings() {
    let app = App::new();
    let path = app.temp.path().join("Vault.noindex/quick-unlock.sealed");
    std::fs::write(&path, b"obviously-fake-crash-before-enrollment-commit").unwrap();
    let settings = app.temp.path().join("Vault.noindex/settings.json");
    let before = std::fs::read(&settings).ok();
    let mut restarted = app.restart();
    assert!(!restarted.core.app_status().quick_unlock.enabled);
    assert!(restarted.core.take_quick_unlock_cleanup());
    assert!(!restarted.core.take_quick_unlock_cleanup());
    assert_eq!(std::fs::read(&settings).ok(), before);
    // Core queues exact-file cleanup; it never performs platform storage work itself.
    assert!(path.is_file());
}
fn item(payload: &[u8]) -> QuickUnlockKey {
    QuickUnlockKey::from_keychain_payload(Zeroizing::new(payload.to_vec())).unwrap()
}

#[test]
fn off_by_default_and_only_verified_protected_enrollment_enables_it() {
    let mut app = App::new();
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(
        app.native(
            NativeAction::EnrollQuickUnlock,
            NativeCredentials::Password(text(WRONG))
        )
        .is_err()
    );
    assert!(!app.core.app_status().quick_unlock.enabled);
    let payload = app.enroll();
    assert_eq!(payload.len(), vault_format::QUICK_UNLOCK_PAYLOAD_LEN);
    assert!(app.core.app_status().quick_unlock.enabled);
    let settings = app.settings();
    assert_eq!(
        settings["quick_unlock_binding"].as_array().unwrap().len(),
        32
    );
    assert!(!settings.to_string().contains(PASSWORD));
    assert!(!settings.to_string().contains("ENOTE-QUICK"));
    app.core.lock();
    assert!(app.core.app_status().quick_unlock.can_unlock);
    app.unlock_with(&payload).unwrap();
    assert_eq!(app.core.app_status().phase, "unlocked");
}

#[test]
fn pending_biometry_blocks_disclosure_without_spending_budget() {
    for finish in [false, true] {
        let mut app = App::new();
        let ids: Vec<_> = (0..9)
            .map(|i| {
                let input: NoteInput = serde_json::from_value(serde_json::json!({
                    "kind": "api_key", "title": format!("Fake pending-biometry key {i}"),
                    "visible": {"env_var": format!("FAKE_KEY_{i}")},
                    "hidden": {"key": format!("fake-pending-key-{i}")}
                }))
                .unwrap();
                app.core.create_note(input, None).unwrap()
            })
            .collect();
        let NativeResult::QuickUnlockEnrollment(enrollment) = app
            .native(
                NativeAction::EnrollQuickUnlock,
                NativeCredentials::Password(text(PASSWORD)),
            )
            .unwrap()
        else {
            panic!("expected enrollment");
        };
        for id in &ids {
            assert_eq!(
                app.core.show_field(id, "key").err().map(|e| e.code),
                Some("invalid_input")
            );
            assert_eq!(
                app.core.copy_field(id, "key", None).err().map(|e| e.code),
                Some("invalid_input")
            );
            assert_eq!(
                app.core.copy_field_line(id, "key", 0).err().map(|e| e.code),
                Some("invalid_input")
            );
            assert_eq!(
                app.core.copy_field_env(id).err().map(|e| e.code),
                Some("invalid_input")
            );
        }
        assert!(app.core.quick_unlock_pending());
        let status = app.core.app_status();
        assert_eq!(status.copies, 0);
        assert_eq!(status.clipboard_clear_in_ms, None);
        assert_eq!(status.reveal, None);
        let payload = enrollment
            .key
            .with_keychain_payload(|bytes| Zeroizing::new(bytes.to_vec()));
        let completion = enrollment.verify(item(&payload));
        if finish {
            app.core.finish_quick_unlock_enrollment(completion).unwrap();
            assert!(app.core.app_status().quick_unlock.enabled);
        } else {
            app.core.cancel_all_native();
            assert!(app.core.finish_quick_unlock_enrollment(completion).is_err());
            assert!(!app.core.app_status().quick_unlock.enabled);
        }
        assert!(!app.core.quick_unlock_pending());
        // All eight real admissions remain available after the refused requests.
        for id in &ids[..8] {
            app.core.copy_field(id, "key", None).unwrap();
        }
        assert_eq!(app.core.app_status().copies, 8);
        assert_eq!(
            app.core.copy_field(&ids[8], "key", None).unwrap_err().code,
            "too_many"
        );
    }
}

#[test]
fn restart_requires_native_current_master_even_if_registration_survives() {
    let mut app = App::new();
    let payload = app.enroll();
    let mut app = app.restart();
    assert!(app.core.app_status().quick_unlock.enabled);
    assert!(!app.core.app_status().quick_unlock.can_unlock);
    assert!(app.unlock_with(&payload).is_err());
    app.core
        .unlock(CredentialInput::MasterPassword(text(PASSWORD)))
        .unwrap();
    app.core.lock();
    assert!(
        app.unlock_with(&payload).is_err(),
        "legacy credential path cannot authorize biometry"
    );
    app.native_unlock();
    app.core.lock();
    app.unlock_with(&payload).unwrap();
}

#[test]
fn biometry_never_extends_the_seven_day_master_deadline() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.lock();
    app.ms.store(1_000 + WEEK_MS - 1, Ordering::SeqCst);
    app.unlock_with(&payload).unwrap();
    app.core.lock();
    app.ms.store(1_000 + WEEK_MS, Ordering::SeqCst);
    assert!(app.unlock_with(&payload).is_err());
    app.native_unlock();
    app.core.lock();
    app.unlock_with(&payload).unwrap();
}

#[test]
fn ordinary_save_keeps_binding_and_opens_the_current_document() {
    let mut app = App::new();
    let payload = app.enroll();
    let input: NoteInput = serde_json::from_value(serde_json::json!({
        "kind": "text", "title": "fake later note", "visible": {}, "hidden": {"body": "fake body"}
    }))
    .unwrap();
    app.core.create_note(input, None).unwrap();
    app.core.lock();
    app.unlock_with(&payload).unwrap();
    let filter = serde_json::from_value(serde_json::json!({"type":"all"})).unwrap();
    assert_eq!(
        app.core
            .list_notes(filter, "fake later note")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn cancelled_old_callback_does_not_open_or_cancel_a_newer_request() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.lock();
    let old = app
        .core
        .prepare_quick_unlock()
        .unwrap()
        .open(item(&payload));
    app.core.os_event(session::OsEvent::Sleep);
    let new = app
        .core
        .prepare_quick_unlock()
        .unwrap()
        .open(item(&payload));
    assert!(app.core.finish_quick_unlock(old).is_err());
    assert!(app.core.quick_unlock_pending());
    app.core.finish_quick_unlock(new).unwrap();
}

#[test]
fn current_disk_must_still_equal_the_snapshot_after_biometry() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.lock();
    let completion = app
        .core
        .prepare_quick_unlock()
        .unwrap()
        .open(item(&payload));
    let other = App::new();
    std::fs::write(app.vault(), std::fs::read(other.vault()).unwrap()).unwrap();
    assert!(app.core.finish_quick_unlock(completion).is_err());
    assert_eq!(app.core.app_status().phase, "locked");
}

#[test]
fn wrong_item_invalidates_registration_without_resetting_password_failures() {
    let mut app = App::new();
    let _ = app.enroll();
    let mut other = App::new();
    let wrong_item = other.enroll();
    app.core.lock();
    assert!(app.unlock_with(&wrong_item).is_err());
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(app.core.take_quick_unlock_cleanup());
    assert_eq!(app.core.app_status().phase, "locked");
}

#[test]
fn biometric_session_preserves_the_password_attempt_count_and_delay() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.lock();
    for _ in 0..4 {
        app.ms.fetch_add(60_000, Ordering::SeqCst);
        assert!(
            app.core
                .unlock(CredentialInput::MasterPassword(text(WRONG)))
                .is_err()
        );
    }
    let before = app.core.app_status();
    assert_eq!(before.failed_attempts, 4);
    app.unlock_with(&payload).unwrap();
    let after = app.core.app_status();
    assert_eq!(after.failed_attempts, 4);
    assert_eq!(after.retry_in_ms, before.retry_in_ms);
}

#[test]
fn disable_persists_before_cleanup_and_surviving_items_are_unusable() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.disable_quick_unlock().unwrap();
    assert!(app.settings()["quick_unlock_binding"].is_null());
    assert!(app.core.take_quick_unlock_cleanup());
    app.core.quick_unlock_cleanup_failed();
    assert!(app.core.app_status().quick_unlock.cleanup_failed);
    assert!(app.core.app_status().quick_unlock.reason.is_none());
    let mut app = app.restart();
    assert!(!app.core.app_status().quick_unlock.enabled);
    app.native_unlock();
    app.core.lock();
    assert!(app.unlock_with(&payload).is_err());
}

#[test]
fn cleanup_retry_stays_disabled_until_removal_success_clears_the_warning() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.disable_quick_unlock().unwrap();
    assert!(app.core.take_quick_unlock_cleanup());
    app.core.quick_unlock_cleanup_failed();
    assert!(app.core.app_status().quick_unlock.cleanup_failed);

    // Retrying uses the same disable path and remains a cleanup request, never enrollment.
    app.core.disable_quick_unlock().unwrap();
    assert!(app.settings()["quick_unlock_binding"].is_null());
    assert!(app.core.take_quick_unlock_cleanup());
    assert!(!app.core.take_quick_unlock_cleanup());
    let pending = app.core.app_status().quick_unlock;
    assert!(pending.cleanup_failed);
    assert!(!pending.enabled);
    assert!(!pending.can_unlock);

    app.core.quick_unlock_cleanup_finished(true);
    let cleaned = app.core.app_status().quick_unlock;
    assert!(!cleaned.cleanup_failed);
    assert!(cleaned.reason.is_none());
    assert!(!cleaned.enabled);
    assert!(!cleaned.can_unlock);
    app.core.lock();
    assert!(app.unlock_with(&payload).is_err());
}

#[test]
fn cleanup_retry_preserves_independent_unavailable_hardware_reason() {
    const REASON: &str = "Touch ID is unavailable on this fictional test computer.";
    let mut app = App::new();
    let _ = app.enroll();
    app.core.set_quick_unlock_availability(false, Some(REASON));
    app.core.disable_quick_unlock().unwrap();
    assert!(app.core.take_quick_unlock_cleanup());
    app.core.quick_unlock_cleanup_failed();
    let failed = app.core.app_status().quick_unlock;
    assert!(failed.cleanup_failed);
    assert_eq!(failed.reason, Some(REASON));
    assert!(!failed.available);

    // Hardware availability is not needed to persist disabled policy and requeue removal.
    app.core.disable_quick_unlock().unwrap();
    assert!(app.core.take_quick_unlock_cleanup());
    assert!(app.core.app_status().quick_unlock.cleanup_failed);
    app.core.quick_unlock_cleanup_finished(true);
    let cleaned = app.core.app_status().quick_unlock;
    assert!(!cleaned.cleanup_failed);
    assert_eq!(cleaned.reason, Some(REASON));
    assert!(!cleaned.available);
    assert!(!cleaned.enabled);
    assert!(!cleaned.can_unlock);
}

#[test]
fn rotation_password_change_and_recovery_start_disable_even_when_cancelled() {
    for action in [
        NativeAction::ChangePassword { rotate: true },
        NativeAction::ChangePassword { rotate: false },
    ] {
        let mut app = App::new();
        let payload = app.enroll();
        let operation = app.core.begin_native(action).unwrap();
        app.core.cancel_native(operation.id);
        assert!(!app.core.app_status().quick_unlock.enabled);
        assert!(app.settings()["quick_unlock_binding"].is_null());
        app.core.lock();
        assert!(app.unlock_with(&payload).is_err());
    }
    let mut app = App::new();
    let _ = app.enroll();
    app.core.lock();
    let operation = app
        .core
        .begin_native(NativeAction::Unlock { recovery: true })
        .unwrap();
    app.core.cancel_native(operation.id);
    assert!(!app.core.app_status().quick_unlock.enabled);
}

#[test]
fn a_failed_password_change_leaves_quick_unlock_off_and_the_vault_unchanged() {
    let mut app = App::new();
    let _ = app.enroll();
    let before = std::fs::read(app.vault()).unwrap();
    assert!(
        app.native(
            NativeAction::ChangePassword { rotate: false },
            NativeCredentials::PasswordChange {
                current: text(WRONG),
                password: text(NEW_PASSWORD),
                confirmation: text(NEW_PASSWORD)
            }
        )
        .is_err()
    );
    assert_eq!(before, std::fs::read(app.vault()).unwrap());
    assert!(!app.core.app_status().quick_unlock.enabled);
}

#[test]
fn foreign_or_copy_inspection_never_authorizes_quick_unlock() {
    let mut app = App::new();
    let payload = app.enroll();
    let backup = app.temp.path().join("fake-copy.enote");
    std::fs::copy(app.vault(), &backup).unwrap();
    let mut app = app.restart();
    app.core.set_backup_to_open(backup).unwrap();
    app.native(
        NativeAction::InspectBackup { recovery: false },
        NativeCredentials::Password(text(PASSWORD)),
    )
    .unwrap();
    app.core.discard_inspection();
    assert!(app.unlock_with(&payload).is_err());
}

#[test]
fn missing_or_backwards_clocks_and_operation_expiry_fail_closed() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.lock();
    app.continuous.store(false, Ordering::SeqCst);
    assert!(app.unlock_with(&payload).is_err());
    app.continuous.store(true, Ordering::SeqCst);
    assert!(app.unlock_with(&payload).is_err());
    app.native_unlock();
    app.core.lock();
    let completion = app
        .core
        .prepare_quick_unlock()
        .unwrap()
        .open(item(&payload));
    app.ms.fetch_add(2 * 60_000, Ordering::SeqCst);
    app.core.tick();
    assert!(!app.core.quick_unlock_pending());
    assert!(app.core.finish_quick_unlock(completion).is_err());
    app.ms.store(0, Ordering::SeqCst);
    assert!(app.unlock_with(&payload).is_err());
}

#[test]
fn uncommitted_or_late_enrollment_never_sets_the_enabled_flag() {
    let mut app = App::new();
    let NativeResult::QuickUnlockEnrollment(enrollment) = app
        .native(
            NativeAction::EnrollQuickUnlock,
            NativeCredentials::Password(text(PASSWORD)),
        )
        .unwrap()
    else {
        panic!("expected enrollment");
    };
    let payload = enrollment
        .key
        .with_keychain_payload(|bytes| Zeroizing::new(bytes.to_vec()));
    let completion = enrollment.verify(item(&payload));
    app.core.lock();
    assert!(app.core.finish_quick_unlock_enrollment(completion).is_err());
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(app.settings()["quick_unlock_binding"].is_null());
}

#[test]
fn installing_even_an_identical_backup_requires_explicit_reenrollment() {
    let mut app = App::new();
    let payload = app.enroll();
    let backup = app.temp.path().join("fake-install-copy.enote");
    std::fs::copy(app.vault(), &backup).unwrap();
    app.core.set_backup_to_open(backup).unwrap();
    app.native(
        NativeAction::InspectBackup { recovery: false },
        NativeCredentials::Password(text(PASSWORD)),
    )
    .unwrap();
    app.core.replace_with_backup().unwrap();
    assert!(!app.core.app_status().quick_unlock.enabled);
    app.core.lock();
    assert!(app.unlock_with(&payload).is_err());
}

#[test]
fn a_settings_write_failure_never_leaves_enrollment_enabled() {
    let mut app = App::new();
    let NativeResult::QuickUnlockEnrollment(enrollment) = app
        .native(
            NativeAction::EnrollQuickUnlock,
            NativeCredentials::Password(text(PASSWORD)),
        )
        .unwrap()
    else {
        panic!("expected enrollment");
    };
    let payload = enrollment
        .key
        .with_keychain_payload(|bytes| Zeroizing::new(bytes.to_vec()));
    let completion = enrollment.verify(item(&payload));
    let settings = app.temp.path().join("Vault.noindex/settings.json");
    if settings.is_file() {
        std::fs::remove_file(&settings).unwrap();
    }
    std::fs::create_dir(&settings).unwrap();
    assert!(app.core.finish_quick_unlock_enrollment(completion).is_err());
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(app.core.take_quick_unlock_cleanup());
}

#[test]
fn enrollment_rejects_a_different_protected_item_and_schedules_cleanup() {
    let mut app = App::new();
    let NativeResult::QuickUnlockEnrollment(enrollment) = app
        .native(
            NativeAction::EnrollQuickUnlock,
            NativeCredentials::Password(text(PASSWORD)),
        )
        .unwrap()
    else {
        panic!("expected enrollment");
    };
    let mut other = App::new();
    let other_payload = other.enroll();
    assert!(
        app.core
            .finish_quick_unlock_enrollment(enrollment.verify(item(&other_payload)))
            .is_err()
    );
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(app.core.take_quick_unlock_cleanup());
    assert_eq!(app.core.app_status().phase, "unlocked");
}

#[test]
fn observing_expiry_while_locked_revokes_authorization_before_a_clock_rollback() {
    let mut app = App::new();
    let payload = app.enroll();
    app.core.lock();
    app.ms.store(1_000 + WEEK_MS, Ordering::SeqCst);
    app.core.tick();
    app.ms.store(1_000 + WEEK_MS - 1, Ordering::SeqCst);
    assert!(app.unlock_with(&payload).is_err());
    assert_eq!(app.core.app_status().phase, "locked");
}

#[test]
fn failed_disable_never_queues_deletion_and_retry_persists_off_before_rotation() {
    let mut app = App::new();
    let _ = app.enroll();
    let _ = app.core.take_quick_unlock_cleanup();
    let settings = app.temp.path().join("Vault.noindex/settings.json");
    let saved = settings.with_extension("saved");
    let previous = std::fs::read(&settings).unwrap();
    std::fs::rename(&settings, &saved).unwrap();
    std::fs::create_dir(&settings).unwrap();
    assert!(app.core.disable_quick_unlock().is_err());
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(!app.core.take_quick_unlock_cleanup());
    assert_eq!(std::fs::read(&saved).unwrap(), previous);
    std::fs::remove_dir(&settings).unwrap();
    std::fs::rename(&saved, &settings).unwrap();
    let operation = app
        .core
        .begin_native(NativeAction::ChangePassword { rotate: true })
        .unwrap();
    app.core.cancel_native(operation.id);
    assert!(app.settings()["quick_unlock_binding"].is_null());
    assert!(app.core.take_quick_unlock_cleanup());
    let app = app.restart();
    assert!(!app.core.app_status().quick_unlock.enabled);
}

#[test]
fn never_enabled_quick_unlock_does_not_add_a_settings_write_requirement_to_account_care() {
    let mut app = App::new();
    let settings = app.temp.path().join("Vault.noindex/settings.json");
    if settings.is_file() {
        std::fs::remove_file(&settings).unwrap();
    }
    std::fs::create_dir(&settings).unwrap();
    let operation = app
        .core
        .begin_native(NativeAction::ChangePassword { rotate: false })
        .unwrap();
    app.core.cancel_native(operation.id);
    assert!(!app.core.app_status().quick_unlock.enabled);
    assert!(!app.core.take_quick_unlock_cleanup());
}
