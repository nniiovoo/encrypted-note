//! Show in Finder or File Explorer: when the core refuses it and what it points at. Public Rust
//! interfaces and fake values only; nothing here opens a file manager.

use encrypted_note_app::core::native::{NativeAction, NativeCredentials, NativeResult};
use encrypted_note_app::core::{ApiError, AppCore, Clipboard, Clock, Config, NoteInput};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use vault_format::{KdfParams, Limits};
use zeroize::Zeroizing;

const PASSWORD: &str = "fake-lantern-quartz-mosaic-velvet-orbit";
const HOUR_MS: u64 = 60 * 60_000;

fn text(value: &str) -> Zeroizing<String> {
    Zeroizing::new(value.to_owned())
}

struct FakeClock {
    ms: Arc<AtomicU64>,
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }
    fn unix_secs(&self) -> i64 {
        1_790_000_000 + (self.now_ms() / 1_000) as i64
    }
    fn continuous_ms(&self) -> Option<u64> {
        Some(self.now_ms())
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
    _temp: tempfile::TempDir,
    ms: Arc<AtomicU64>,
}
impl App {
    /// A new fake Vault, Unlocked.
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let ms = Arc::new(AtomicU64::new(1_000));
        let mut core = AppCore::new(Config {
            data_dir: temp.path().join("Vault.noindex"),
            home_dir: temp.path().to_owned(),
            limits: Limits::TESTING,
            clock: Box::new(FakeClock { ms: ms.clone() }),
            clipboard: Box::new(FakeClipboard),
            activity_anywhere_supported: false,
        })
        .unwrap();
        core.set_quick_unlock_availability(true, None);
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
            _temp: temp,
            ms,
        }
    }
    fn root(&self) -> PathBuf {
        PathBuf::from(self.core.vault_location())
    }
    fn refused_with(&mut self) -> &'static str {
        self.core.vault_reveal_target().unwrap_err().code
    }
    fn assert_target_is_inside_the_vault_folder(&mut self) {
        let target = self.core.vault_reveal_target().unwrap();
        assert!(target.starts_with(self.root()), "{}", target.display());
    }
}

#[test]
fn the_target_is_inside_the_vault_folder() {
    let mut app = App::new();
    let target = app.core.vault_reveal_target().unwrap();
    #[cfg(target_os = "macos")]
    assert_eq!(target, app.root().join("vault.enote"));
    #[cfg(not(target_os = "macos"))]
    assert_eq!(target, app.root());
}

#[cfg(target_os = "macos")]
#[test]
fn a_link_in_place_of_the_vault_file_is_never_selected() {
    let mut app = App::new();
    let vault = app.root().join("vault.enote");
    let elsewhere = app.root().with_file_name("fake-elsewhere");
    std::fs::rename(&vault, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &vault).unwrap();
    assert_eq!(app.core.vault_reveal_target().unwrap(), app.root());
}

#[test]
fn refused_when_locked() {
    let mut app = App::new();
    app.core.lock();
    assert_eq!(app.refused_with(), "locked");
}

#[test]
fn refused_when_the_auto_lock_time_has_passed() {
    let mut app = App::new();
    app.ms.fetch_add(24 * HOUR_MS, Ordering::SeqCst);
    assert_eq!(app.refused_with(), "locked");
}

#[test]
fn refused_while_the_sharing_guard_is_up() {
    let mut app = App::new();
    app.core.guard_update(&[guard::RunningApp {
        bundle_id: Some("us.zoom.xos".into()),
        exe_name: Some("Zoom.exe".into()),
    }]);
    assert_eq!(app.refused_with(), "sharing_guard_up");
    app.core.guard_update(&[]);
    app.assert_target_is_inside_the_vault_folder();
}

#[test]
fn refused_while_a_quick_unlock_is_pending() {
    let mut app = App::new();
    let operation = app
        .core
        .begin_native(NativeAction::EnrollQuickUnlock)
        .unwrap();
    let job = app
        .core
        .prepare_native(operation.id, NativeCredentials::Password(text(PASSWORD)))
        .unwrap();
    let NativeResult::QuickUnlockEnrollment(_enrollment) =
        app.core.finish_native(job.run()).unwrap()
    else {
        panic!("expected enrollment");
    };
    assert!(app.core.quick_unlock_pending());
    assert_eq!(app.refused_with(), "invalid_input");
    app.core.cancel_all_native();
    app.assert_target_is_inside_the_vault_folder();
}

#[test]
fn refused_while_a_native_operation_is_pending() {
    let mut app = App::new();
    let operation = app
        .core
        .begin_native(NativeAction::ChangePassword { rotate: false })
        .unwrap();
    assert_eq!(app.refused_with(), "invalid_input");
    app.core.cancel_native(operation.id);
    app.assert_target_is_inside_the_vault_folder();
}

#[test]
fn refused_while_a_value_is_shown() {
    let mut app = App::new();
    let input: NoteInput = serde_json::from_value(serde_json::json!({
        "kind": "api_key", "title": "Fake reveal key",
        "visible": {"env_var": "FAKE_REVEAL_KEY"},
        "hidden": {"key": "fake-reveal-key-value"}
    }))
    .unwrap();
    let id = app.core.create_note(input, None).unwrap();
    app.core.show_field(&id, "key").unwrap();
    assert_eq!(app.refused_with(), "invalid_input");
    app.core.hide_field();
    app.assert_target_is_inside_the_vault_folder();
}
