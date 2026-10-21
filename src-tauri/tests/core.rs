//! AppCore through its public interface: a temp data folder, tiny Argon2 parameters, a fake clock
//! and a fake clipboard. Every value below is an obviously fake placeholder.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use encrypted_note_app::core::{
    ApiError, AppCore, Clipboard, Clock, Config, CredentialInput, NoteEditInput, NoteInput,
    SettingsInput, ShowMode,
};
use serde_json::{Value, json};
use session::OsEvent;
use vault_format::{KdfParams, Limits};
use zeroize::Zeroizing;

const PASSWORD: &str = "fake-lantern-quartz-mosaic-velvet-orbit";
const NEW_PASSWORD: &str = "fake-harbor-pebble-cinder-meadow-tundra";
const WRONG_PASSWORD: &str = "fake-wrong-guess-for-tests-only-7";
const START_UNIX: i64 = 1_790_000_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

// ---------- fakes ----------

struct FakeClock {
    ms: Arc<AtomicU64>,
    break_vault_on_next_wall_clock_read: Arc<Mutex<Option<PathBuf>>>,
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.ms.load(Ordering::SeqCst)
    }
    fn unix_secs(&self) -> i64 {
        if let Some(vault) = self
            .break_vault_on_next_wall_clock_read
            .lock()
            .unwrap()
            .take()
        {
            std::fs::rename(&vault, vault.with_extension("saved")).unwrap();
            std::fs::create_dir(&vault).unwrap();
        }
        START_UNIX + i64::try_from(self.now_ms() / 1000).unwrap()
    }
}

#[derive(Default)]
struct Board {
    text: Option<String>,
    change_count: u64,
    clear_failures: u32,
    /// How many of the next writes the clipboard refuses.
    write_failures: u32,
}

struct FakeClipboard(Arc<Mutex<Board>>);

impl Clipboard for FakeClipboard {
    fn write(&mut self, text: &str) -> Result<u64, ApiError> {
        let mut board = self.0.lock().unwrap();
        if board.write_failures > 0 {
            // As the real one does: whatever was there is gone, and nothing of ours is left.
            board.write_failures -= 1;
            board.change_count += 1;
            board.text = None;
            return Err(ApiError {
                code: "io",
                message: "The clipboard didn't accept the copy. Try again.".to_owned(),
            });
        }
        board.change_count += 1;
        board.text = Some(text.to_owned());
        Ok(board.change_count)
    }
    fn clear_if(&mut self, change_count: u64) -> bool {
        let mut board = self.0.lock().unwrap();
        if board.clear_failures > 0 {
            board.clear_failures -= 1;
            return false;
        }
        if board.change_count == change_count {
            board.change_count += 1;
            board.text = None;
        }
        true
    }
}

struct App {
    core: AppCore,
    ms: Arc<AtomicU64>,
    board: Arc<Mutex<Board>>,
    break_vault_on_next_wall_clock_read: Arc<Mutex<Option<PathBuf>>>,
    activity_anywhere_supported: bool,
    temp: tempfile::TempDir,
}

impl App {
    /// On a computer where the app can see input outside its own window (a Mac).
    fn new() -> App {
        App::on_a_computer(true)
    }

    /// `activity_anywhere_supported`: whether the OS can say when input was last seen anywhere.
    fn on_a_computer(activity_anywhere_supported: bool) -> App {
        let temp = tempfile::tempdir().unwrap();
        let (ms, board) = (
            Arc::new(AtomicU64::new(1_000)),
            Arc::new(Mutex::new(Board::default())),
        );
        let break_vault_on_next_wall_clock_read = Arc::new(Mutex::new(None));
        let core = start(
            temp.path(),
            &ms,
            &board,
            &break_vault_on_next_wall_clock_read,
            activity_anywhere_supported,
        );
        App {
            core,
            ms,
            board,
            break_vault_on_next_wall_clock_read,
            activity_anywhere_supported,
            temp,
        }
    }

    /// Quit and start again on the same data folder (same clock and clipboard).
    fn restart(self) -> App {
        let App {
            core,
            ms,
            board,
            break_vault_on_next_wall_clock_read,
            activity_anywhere_supported,
            temp,
        } = self;
        drop(core);
        let core = start(
            temp.path(),
            &ms,
            &board,
            &break_vault_on_next_wall_clock_read,
            activity_anywhere_supported,
        );
        App {
            core,
            ms,
            board,
            break_vault_on_next_wall_clock_read,
            activity_anywhere_supported,
            temp,
        }
    }

    fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, Ordering::SeqCst);
    }

    fn data_dir(&self) -> PathBuf {
        self.temp.path().join("Vault.noindex")
    }

    fn break_vault_on_next_wall_clock_read(&self) {
        *self.break_vault_on_next_wall_clock_read.lock().unwrap() =
            Some(self.data_dir().join("vault.enote"));
    }

    fn phase(&self) -> &'static str {
        self.core.app_status().phase
    }

    fn clipboard(&self) -> Option<String> {
        self.board.lock().unwrap().text.clone()
    }

    /// Another app copies something.
    fn someone_else_copies(&self) {
        let mut board = self.board.lock().unwrap();
        board.change_count += 1;
        board.text = Some("something else".into());
    }
}

fn start(
    root: &Path,
    ms: &Arc<AtomicU64>,
    board: &Arc<Mutex<Board>>,
    break_vault_on_next_wall_clock_read: &Arc<Mutex<Option<PathBuf>>>,
    activity_anywhere_supported: bool,
) -> AppCore {
    AppCore::new(Config {
        data_dir: root.join("Vault.noindex"),
        home_dir: root.join("home"),
        limits: Limits::TESTING,
        clock: Box::new(FakeClock {
            ms: ms.clone(),
            break_vault_on_next_wall_clock_read: break_vault_on_next_wall_clock_read.clone(),
        }),
        clipboard: Box::new(FakeClipboard(board.clone())),
        activity_anywhere_supported,
    })
    .unwrap()
}

// ---------- helpers ----------

fn pw(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_owned())
}

fn password(s: &str) -> CredentialInput {
    serde_json::from_value(json!({ "master_password": s })).unwrap()
}

fn recovery(s: &str) -> CredentialInput {
    serde_json::from_value(json!({ "recovery_key": s })).unwrap()
}

fn input(kind: &str, title: &str, visible: Value, hidden: Value) -> NoteInput {
    serde_json::from_value(
        json!({ "kind": kind, "title": title, "visible": visible, "hidden": hidden }),
    )
    .unwrap()
}

fn edit(title: &str, visible: Value, hidden: Value) -> NoteEditInput {
    serde_json::from_value(json!({ "title": title, "visible": visible, "hidden": hidden })).unwrap()
}

fn filter(value: Value) -> notes::Filter {
    serde_json::from_value(value).unwrap()
}

fn all() -> notes::Filter {
    filter(json!({ "type": "all" }))
}

/// The last two groups of a displayed Recovery Key, as the owner would retype them.
fn last_groups(key: &str) -> String {
    key[key.len() - 9..].to_owned()
}

fn code<T>(result: Result<T, ApiError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(e) => {
            assert!(!e.message.is_empty());
            e.code
        }
    }
}

/// A new Vault, confirmed and Unlocked. Returns the Recovery Key.
fn unlocked(app: &mut App) -> String {
    let key = app
        .core
        .create_vault(pw(PASSWORD), KdfParams::TESTING)
        .unwrap()
        .recovery_key
        .unwrap();
    app.core.confirm_recovery_kit(&last_groups(&key)).unwrap();
    assert_eq!(app.phase(), "unlocked");
    key.as_str().to_owned()
}

fn vault_file(app: &App) -> PathBuf {
    app.data_dir().join(store::VAULT_FILE)
}

/// A Backup (one Note) of a Vault made on another computer with [`PASSWORD`]. Keep the returned
/// App alive while the file is used.
fn a_backup() -> (App, PathBuf) {
    a_backup_with(PASSWORD)
}

fn a_backup_with(master_password: &str) -> (App, PathBuf) {
    let mut source = App::new();
    let key = source
        .core
        .create_vault(pw(master_password), KdfParams::TESTING)
        .unwrap()
        .recovery_key
        .unwrap();
    source
        .core
        .confirm_recovery_kit(&last_groups(&key))
        .unwrap();
    source
        .core
        .create_note(
            input("text", "Fake backed up", json!({}), json!({"body": "fake"})),
            None,
        )
        .unwrap();
    let backup = source.temp.path().join("fake-backup.enote");
    source.core.set_backup_destination(backup.clone()).unwrap();
    source.core.write_backup().unwrap();
    (source, backup)
}

fn set_lock_on_app_switch(app: &mut App) {
    app.core.set_settings(lock_rule(5, true)).unwrap();
}

/// What the Settings screen has always sent: the lock rule alone. The newer settings are left
/// out, which keeps them as they are.
fn lock_rule(idle_minutes: u32, lock_on_app_switch: bool) -> SettingsInput {
    SettingsInput {
        idle_minutes,
        lock_on_app_switch,
        clipboard_clear_seconds: None,
        count_activity_anywhere: None,
    }
}

/// Settings as the screens send them: JSON, in which a setting may be left out.
fn settings(value: Value) -> SettingsInput {
    serde_json::from_value(value).unwrap()
}

/// True if the clipboard holds nothing and no clear is waiting for its time.
fn nothing_copied(app: &App) -> bool {
    app.clipboard().is_none() && app.core.app_status().clipboard_clear_in_ms.is_none()
}

// ---------- setup, unlock, lock ----------

#[test]
fn test_passwords_pass_the_policy() {
    for p in [PASSWORD, NEW_PASSWORD] {
        assert!(policy::assess(p).acceptable, "{p}");
    }
}

#[test]
fn create_confirm_lock_unlock() {
    let mut app = App::new();
    assert_eq!(app.phase(), "no_vault");
    let key = app
        .core
        .create_vault(pw(PASSWORD), KdfParams::TESTING)
        .unwrap()
        .recovery_key
        .unwrap();
    assert_eq!(key.len(), 34);
    assert_eq!(app.phase(), "confirm_recovery_kit");
    assert_eq!(
        code(app.core.confirm_recovery_kit("0000 0000")),
        "invalid_input"
    );
    assert_eq!(code(app.core.list_notes(all(), "")), "locked");

    // Case, spaces and dashes don't matter.
    let typed = last_groups(&key).to_lowercase().replace('-', " ");
    app.core.confirm_recovery_kit(&typed).unwrap();
    assert_eq!(app.phase(), "unlocked");
    assert!(vault_file(&app).is_file());
    assert!(app.core.app_status().lock_in_ms.is_some());

    app.core.lock();
    assert_eq!(app.phase(), "locked");
    assert_eq!(app.core.app_status().lock_in_ms, None);
    assert_eq!(code(app.core.list_notes(all(), "")), "locked");
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(app.phase(), "unlocked");

    let mut app = app.restart();
    assert_eq!(app.phase(), "locked");
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(app.core.list_notes(all(), "").unwrap(), vec![]);
}

#[test]
fn nothing_is_on_disk_before_the_recovery_kit_is_confirmed() {
    let mut app = App::new();
    app.core
        .create_vault(pw(PASSWORD), KdfParams::TESTING)
        .unwrap();
    let written: Vec<_> = std::fs::read_dir(app.data_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != ".lock")
        .collect();
    assert_eq!(written, Vec::<String>::new());

    // Locking (or quitting) before confirming throws the new Vault away.
    app.core.os_event(OsEvent::Sleep);
    assert_eq!(app.phase(), "no_vault");
    assert!(!vault_file(&app).exists());
    let app = app.restart();
    assert_eq!(app.phase(), "no_vault");
}

#[test]
fn weak_master_passwords_are_refused() {
    let mut app = App::new();
    assert_eq!(
        code(app.core.create_vault(pw("password123"), KdfParams::TESTING)),
        "weak_password"
    );
    assert_eq!(app.phase(), "no_vault");
    assert!(!app.core.assess_password("password123").acceptable);
    assert!(
        app.core
            .assess_password(&app.core.suggest_passphrase())
            .acceptable
    );
}

#[test]
fn wrong_passwords_back_off_and_never_erase() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core.lock();

    for _ in 0..3 {
        let e = app.core.unlock(password(WRONG_PASSWORD)).unwrap_err();
        assert_eq!(e.code, "wrong_credential");
        assert_eq!(
            e.message,
            "That password didn't unlock this Vault. Check Caps Lock and try again."
        );
    }
    let status = app.core.app_status();
    assert_eq!(
        (
            status.failed_attempts,
            status.show_recovery_hint,
            status.retry_in_ms
        ),
        (3, true, Some(1_000))
    );
    // Even the right password waits.
    assert_eq!(code(app.core.unlock(password(PASSWORD))), "retry_later");

    app.advance(1_000);
    assert_eq!(
        code(app.core.unlock(password(WRONG_PASSWORD))),
        "wrong_credential"
    );
    assert_eq!(app.core.app_status().retry_in_ms, Some(2_000));
    app.advance(2_000);
    app.core.unlock(password(PASSWORD)).unwrap();
    let status = app.core.app_status();
    assert_eq!(
        (
            status.phase,
            status.failed_attempts,
            status.show_recovery_hint
        ),
        ("unlocked", 0, false)
    );
    assert!(vault_file(&app).is_file());
}

#[test]
fn recovery_key_unlock_then_a_new_master_password() {
    let mut app = App::new();
    let key = unlocked(&mut app);
    app.core.lock();

    assert_eq!(
        code(app.core.unlock(recovery("0000-0000"))),
        "invalid_input"
    );
    let typed = key.to_lowercase().replace('-', " ");
    app.core.unlock(recovery(&typed)).unwrap();
    assert_eq!(app.phase(), "needs_new_password");
    assert_eq!(code(app.core.list_notes(all(), "")), "locked");
    assert_eq!(
        code(app.core.set_new_password(pw("short"))),
        "weak_password"
    );
    app.core.set_new_password(pw(NEW_PASSWORD)).unwrap();
    assert_eq!(app.phase(), "unlocked");

    app.core.lock();
    assert_eq!(
        code(app.core.unlock(password(PASSWORD))),
        "wrong_credential"
    );
    app.core.unlock(password(NEW_PASSWORD)).unwrap();
    app.core.lock();
    // The Recovery Key still works.
    app.core.unlock(recovery(&key)).unwrap();
    assert_eq!(app.phase(), "needs_new_password");
    // Locking without setting one keeps the password that was saved.
    app.core.lock();
    app.core.unlock(password(NEW_PASSWORD)).unwrap();
}

// ---------- Notes ----------

#[test]
fn every_kind_can_be_created_viewed_edited_trashed_and_restored() {
    let mut app = App::new();
    unlocked(&mut app);
    let core = &mut app.core;

    let text = core
        .create_note(
            input(
                "text",
                "Fake memo",
                json!({}),
                json!({"body": "fake body one"}),
            ),
            None,
        )
        .unwrap();
    let login = core
        .create_note(
            input(
                "login",
                "Fake exchange",
                json!({"website": "example.test", "username": "fake-user"}),
                json!({"password": "fake-login-pass"}),
            ),
            None,
        )
        .unwrap();
    let api = core
        .create_note(
            input(
                "api_key",
                "Fake RPC",
                json!({"service": "fake-rpc"}),
                json!({"key": "fake-api-key"}),
            ),
            None,
        )
        .unwrap();
    let pk = core
        .create_note(
            input(
                "private_key",
                "Fake wallet key",
                json!({"chain": "evm", "address": "0xfake"}),
                json!({"key": "fake-private-key"}),
            ),
            Some(pw(PASSWORD)),
        )
        .unwrap();
    let words = ["abandon"; 11].join(" ") + " about";
    let seed = core
        .create_note(
            input(
                "seed_phrase",
                "Fake seed",
                json!({"wallet_name": "Fake wallet"}),
                json!({"words": words, "passphrase": "fake-25th"}),
            ),
            Some(pw(PASSWORD)),
        )
        .unwrap();

    let listed = core.list_notes(all(), "").unwrap();
    assert_eq!(listed.len(), 5);
    let view = core.get_note(&login).unwrap();
    assert_eq!(view.visible["username"], "fake-user");
    assert_eq!(view.hidden, vec![("password".to_owned(), true)]);
    assert_eq!(core.get_note(&seed).unwrap().visible["word_count"], "12");
    assert_eq!(
        core.list_notes(filter(json!({"type": "kind", "kind": "api_key"})), "")
            .unwrap()[0]
            .id,
        api
    );
    assert_eq!(core.list_notes(all(), "example.test").unwrap()[0].id, login);

    // Show: ordinary Kinds directly, Wallet Kinds with the password.
    assert_eq!(
        core.show_field(&text, "body").unwrap().as_str(),
        "fake body one"
    );
    assert_eq!(code(core.show_field(&pk, "key")), "invalid_input");
    assert_eq!(
        core.show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
            .unwrap()
            .as_str(),
        "fake-private-key"
    );
    assert_eq!(
        core.show_wallet_field(&seed, "words", pw(PASSWORD), ShowMode::Accessible)
            .unwrap()
            .as_str(),
        words
    );
    assert_eq!(
        core.app_status().reveal.unwrap().expires_in_ms,
        Some(20_000)
    );
    core.hide_field();
    assert_eq!(core.app_status().reveal, None);

    // Edit: visible parts freely, Hidden Fields replace-only ("" clears, omitted keeps).
    core.update_note(
        &login,
        edit(
            "Fake exchange 2",
            json!({"username": "fake-user-2"}),
            json!({}),
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        core.show_field(&login, "password").unwrap().as_str(),
        "fake-login-pass"
    );
    core.update_note(
        &api,
        edit("Fake RPC", json!({}), json!({"key": "fake-api-key-2"})),
        None,
    )
    .unwrap();
    assert_eq!(
        core.show_field(&api, "key").unwrap().as_str(),
        "fake-api-key-2"
    );
    core.update_note(
        &text,
        edit("Fake memo", json!({}), json!({"body": ""})),
        None,
    )
    .unwrap();
    assert_eq!(
        core.get_note(&text).unwrap().hidden,
        vec![("body".to_owned(), false)]
    );
    core.update_note(
        &pk,
        edit(
            "Fake wallet key 2",
            json!({"address": "0xfake2"}),
            json!({}),
        ),
        Some(pw(PASSWORD)),
    )
    .unwrap();
    core.update_note(
        &pk,
        edit(
            "Fake wallet key 2",
            json!({}),
            json!({"key": "fake-private-key-2"}),
        ),
        Some(pw(PASSWORD)),
    )
    .unwrap();
    assert_eq!(
        core.show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
            .unwrap()
            .as_str(),
        "fake-private-key-2"
    );

    // Favorites, Trash.
    core.set_favorite(&api, true).unwrap();
    assert_eq!(core.list_notes(all(), "").unwrap()[0].id, api);
    assert_eq!(
        core.list_notes(filter(json!({"type": "favorites"})), "")
            .unwrap()
            .len(),
        1
    );
    core.trash_note(&text, None).unwrap();
    core.trash_note(&login, None).unwrap();
    assert_eq!(core.list_notes(all(), "").unwrap().len(), 3);
    assert_eq!(
        core.list_notes(filter(json!({"type": "trash"})), "")
            .unwrap()
            .len(),
        2
    );
    core.restore_note(&text).unwrap();
    assert_eq!(code(core.delete_forever(&api, None)), "not_found"); // only from Trash
    core.delete_forever(&login, None).unwrap();
    core.trash_note(&text, None).unwrap();
    assert_eq!(core.empty_trash(None).unwrap(), 1);
    assert_eq!(code(core.get_note(&text)), "not_found");

    // Everything was saved.
    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    let mut ids: Vec<_> = app
        .core
        .list_notes(all(), "")
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(ids[0], api); // the Favorite
    ids.sort();
    let mut expected = vec![api, seed, pk];
    expected.sort();
    assert_eq!(ids, expected);
    assert!(app.core.app_status().backup.unbacked_changes);
}

#[test]
fn a_shown_value_hides_after_30_s_on_another_note_and_on_focus_loss() {
    let mut app = App::new();
    unlocked(&mut app);
    let a = app
        .core
        .create_note(
            input("text", "Fake A", json!({}), json!({"body": "fake a"})),
            None,
        )
        .unwrap();
    let b = app
        .core
        .create_note(
            input("text", "Fake B", json!({}), json!({"body": "fake b"})),
            None,
        )
        .unwrap();

    app.core.show_field(&a, "body").unwrap();
    app.advance(29_999);
    app.core.tick();
    assert!(app.core.app_status().reveal.is_some());
    app.advance(1);
    app.core.tick();
    assert_eq!(app.core.app_status().reveal, None);

    app.core.show_field(&a, "body").unwrap();
    app.core.get_note(&a).unwrap();
    assert!(app.core.app_status().reveal.is_some());
    app.core.get_note(&b).unwrap();
    assert_eq!(app.core.app_status().reveal, None);

    app.core.show_field(&b, "body").unwrap();
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(app.core.app_status().reveal, None);
    assert_eq!(app.phase(), "unlocked");
}

#[test]
fn lists_views_and_status_never_contain_a_hidden_value() {
    let mut app = App::new();
    unlocked(&mut app);
    let secrets = [
        "fake-secret-body-4411",
        "fake-secret-pass-4412",
        "fake-secret-api-4413",
        "fake-secret-pk-4414",
        "fake-secret-pp-4415",
    ];
    let core = &mut app.core;
    let ids = [
        core.create_note(
            input("text", "Fake T", json!({}), json!({"body": secrets[0]})),
            None,
        ),
        core.create_note(
            input(
                "login",
                "Fake L",
                json!({"website": "w.test"}),
                json!({"password": secrets[1]}),
            ),
            None,
        ),
        core.create_note(
            input(
                "api_key",
                "Fake A",
                json!({"service": "s"}),
                json!({"key": secrets[2]}),
            ),
            None,
        ),
        core.create_note(
            input(
                "private_key",
                "Fake P",
                json!({"chain": "other"}),
                json!({"key": secrets[3]}),
            ),
            Some(pw(PASSWORD)),
        ),
        core.create_note(
            input(
                "seed_phrase",
                "Fake S",
                json!({}),
                json!({"words": (["zoo"; 12].join(" ")), "passphrase": secrets[4]}),
            ),
            Some(pw(PASSWORD)),
        ),
    ]
    .map(Result::unwrap);
    core.trash_note(&ids[2], None).unwrap();

    let mut seen = String::new();
    for f in [
        json!({"type": "all"}),
        json!({"type": "favorites"}),
        json!({"type": "trash"}),
        json!({"type": "kind", "kind": "login"}),
    ] {
        for query in ["", "fake", "secret"] {
            seen += &serde_json::to_string(&core.list_notes(filter(f.clone()), query).unwrap())
                .unwrap();
        }
    }
    for id in &ids {
        seen += &serde_json::to_string(&core.get_note(id).unwrap()).unwrap();
    }
    core.copy_field(&ids[1], "password", None).unwrap();
    seen += &serde_json::to_string(&core.app_status()).unwrap();
    assert!(seen.contains("Fake L"));
    for secret in secrets {
        assert!(!seen.contains(secret), "{secret} leaked");
    }
    // Search never matches Hidden Fields.
    assert_eq!(core.list_notes(all(), "fake-secret").unwrap(), vec![]);
    // Nor is anything readable in the file.
    let file = String::from_utf8_lossy(&std::fs::read(vault_file(&app)).unwrap()).into_owned();
    for secret in secrets {
        assert!(!file.contains(secret));
    }
}

#[test]
fn wallet_kinds_need_the_right_master_password_for_every_action() {
    let mut app = App::new();
    unlocked(&mut app);
    let core = &mut app.core;
    let new_pk = || {
        input(
            "private_key",
            "Fake key",
            json!({"chain": "solana"}),
            json!({"key": "fake-sol-key"}),
        )
    };

    assert_eq!(code(core.create_note(new_pk(), None)), "invalid_input");
    assert_eq!(
        code(core.create_note(new_pk(), Some(pw(WRONG_PASSWORD)))),
        "wrong_credential"
    );
    let pk = core.create_note(new_pk(), Some(pw(PASSWORD))).unwrap();

    assert_eq!(
        code(core.show_wallet_field(&pk, "key", pw(WRONG_PASSWORD), ShowMode::Held)),
        "wrong_credential"
    );
    assert_eq!(core.app_status().reveal, None);
    assert_eq!(code(core.copy_field(&pk, "key", None)), "invalid_input");
    assert_eq!(
        code(core.copy_field(&pk, "key", Some(pw(WRONG_PASSWORD)))),
        "wrong_credential"
    );
    assert_eq!(
        code(core.show_wallet_field(&pk, "key", pw(WRONG_PASSWORD), ShowMode::Accessible)),
        "wrong_credential"
    );
    assert_eq!(app.clipboard(), None);
    // Three wrong passwords since the right one (which reset the count): the next try waits.
    assert_eq!(
        code(
            app.core
                .show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
        ),
        "retry_later"
    );
    app.advance(1_000);
    let core = &mut app.core;
    assert_eq!(
        code(core.update_note(
            &pk,
            edit("Fake key", json!({}), json!({"key": "fake-other"})),
            Some(pw(WRONG_PASSWORD))
        )),
        "wrong_credential"
    );
    assert_eq!(
        code(core.update_note(
            &pk,
            edit("Fake key", json!({}), json!({"key": "fake-other"})),
            None
        )),
        "invalid_input"
    );
    assert_eq!(
        code(core.show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)),
        "retry_later"
    );
    app.advance(2_000);

    let core = &mut app.core;
    core.copy_field(&pk, "key", Some(pw(PASSWORD))).unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("fake-sol-key"));
    assert_eq!(
        app.core
            .show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
            .unwrap()
            .as_str(),
        "fake-sol-key"
    );
    // Editing only the visible parts (title, chain, address) needs the password too.
    let rename = || {
        edit(
            "Fake key renamed",
            json!({"address": "fake-addr"}),
            json!({}),
        )
    };
    assert_eq!(
        code(app.core.update_note(&pk, rename(), None)),
        "invalid_input"
    );
    assert_eq!(
        code(
            app.core
                .update_note(&pk, rename(), Some(pw(WRONG_PASSWORD)))
        ),
        "wrong_credential"
    );
    app.core
        .update_note(&pk, rename(), Some(pw(PASSWORD)))
        .unwrap();
    assert_eq!(
        app.core.get_note(&pk).unwrap().visible["address"],
        "fake-addr"
    );
}

#[test]
fn the_right_master_password_for_this_vault_resets_the_wrong_password_count() {
    let mut app = App::new();
    unlocked(&mut app);
    let pk = app
        .core
        .create_note(
            input("private_key", "Fake", json!({}), json!({"key": "fake-key"})),
            Some(pw(PASSWORD)),
        )
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            code(
                app.core
                    .show_wallet_field(&pk, "key", pw(WRONG_PASSWORD), ShowMode::Held)
            ),
            "wrong_credential"
        );
    }
    app.advance(1_000);
    app.core
        .show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
        .unwrap();
    let status = app.core.app_status();
    assert_eq!(
        (
            status.failed_attempts,
            status.show_recovery_hint,
            status.retry_in_ms
        ),
        (0, false, None)
    );

    for _ in 0..3 {
        assert_eq!(
            code(
                app.core
                    .change_password(pw(WRONG_PASSWORD), pw(NEW_PASSWORD), false)
            ),
            "wrong_credential"
        );
    }
    app.advance(1_000);
    app.core
        .change_password(pw(PASSWORD), pw(NEW_PASSWORD), false)
        .unwrap();
    assert_eq!(app.core.app_status().failed_attempts, 0);
    // The lock screen starts clean.
    app.core.lock();
    let status = app.core.app_status();
    assert_eq!((status.failed_attempts, status.retry_in_ms), (0, None));
}

#[test]
fn opening_another_vaults_backup_does_not_reset_the_wrong_password_count() {
    let (_source, backup) = a_backup();
    let mut app = App::new();
    unlocked(&mut app);
    app.core.lock();
    for _ in 0..3 {
        assert_eq!(
            code(app.core.unlock(password(WRONG_PASSWORD))),
            "wrong_credential"
        );
    }
    app.advance(1_000);
    app.core.unlock(password(PASSWORD)).unwrap();
    let (_other, other_backup) = a_backup_with(NEW_PASSWORD);
    app.core.set_backup_to_open(backup).unwrap();
    for _ in 0..3 {
        assert_eq!(
            code(app.core.inspect_backup(password(WRONG_PASSWORD))),
            "wrong_credential"
        );
    }
    app.advance(1_000);
    // A Backup made elsewhere with a password the person at the keyboard knows.
    app.core.set_backup_to_open(other_backup).unwrap();
    app.core.inspect_backup(password(NEW_PASSWORD)).unwrap();
    assert_eq!(app.core.app_status().failed_attempts, 3);
}

// ---------- screen privacy & clipboard ----------

fn zoom() -> Vec<guard::RunningApp> {
    vec![guard::RunningApp {
        bundle_id: Some("us.zoom.xos".into()),
        exe_name: Some("Zoom.exe".into()),
    }]
}

#[test]
fn the_sharing_guard_blocks_show_and_copy_and_hides_what_was_shown() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input("login", "Fake", json!({}), json!({"password": "fake-pass"})),
            None,
        )
        .unwrap();
    app.core.show_field(&id, "password").unwrap();

    app.core.guard_update(&zoom());
    let status = app.core.app_status();
    assert!(status.sharing_guard.up);
    assert_eq!(status.sharing_guard.apps, vec!["Zoom"]);
    assert_eq!(status.reveal, None);
    assert_eq!(
        code(app.core.show_field(&id, "password")),
        "sharing_guard_up"
    );
    assert_eq!(
        code(app.core.copy_field(&id, "password", None)),
        "sharing_guard_up"
    );
    assert_eq!(app.clipboard(), None);

    app.core.dismiss_sharing_guard();
    assert!(!app.core.app_status().sharing_guard.up);
    app.core.show_field(&id, "password").unwrap();

    // Locking forgets the dismissal.
    app.core.guard_update(&zoom());
    app.core.lock();
    assert!(app.core.app_status().sharing_guard.up);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(
        code(app.core.show_field(&id, "password")),
        "sharing_guard_up"
    );
    app.core.guard_update(&[]);
    app.core.show_field(&id, "password").unwrap();
}

#[test]
fn a_copy_is_cleared_after_30_s_only_if_still_ours() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input("api_key", "Fake", json!({}), json!({"key": "fake-key-1"})),
            None,
        )
        .unwrap();

    app.core.copy_field(&id, "key", None).unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("fake-key-1"));
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
    app.advance(29_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-key-1"));
    app.advance(1);
    app.core.tick();
    assert_eq!(app.clipboard(), None);
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, None);

    app.core.copy_field(&id, "key", None).unwrap();
    app.someone_else_copies();
    app.advance(30_000);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("something else"));

    // Locking and quitting clear it too.
    app.core.copy_field(&id, "key", None).unwrap();
    app.core.lock();
    assert_eq!(app.clipboard(), None);
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.copy_field(&id, "key", None).unwrap();
    app.core.os_event(OsEvent::Quit);
    assert_eq!(app.clipboard(), None);
    assert_eq!(app.phase(), "locked");
}

#[test]
fn a_failed_clipboard_clear_is_retried_after_lock() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input("api_key", "Fake", json!({}), json!({"key": "fake-key-2"})),
            None,
        )
        .unwrap();
    app.core.copy_field(&id, "key", None).unwrap();
    app.board.lock().unwrap().clear_failures = 1;

    app.core.lock();
    assert_eq!(app.clipboard().as_deref(), Some("fake-key-2"));
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(250));
    app.advance(250);
    app.core.tick();
    assert_eq!(app.clipboard(), None);
}

#[test]
fn auto_lock_after_the_chosen_idle_time_and_on_os_events() {
    let mut app = App::new();
    unlocked(&mut app);
    assert_eq!(app.core.app_status().lock_in_ms, Some(5 * 60_000));
    app.advance(4 * 60_000);
    app.core.activity();
    app.advance(4 * 60_000);
    app.core.tick();
    assert_eq!(app.phase(), "unlocked");
    app.advance(60_000);
    app.core.tick();
    assert_eq!(app.phase(), "locked");

    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.set_settings(lock_rule(1, true)).unwrap();
    let settings = app.core.app_status().settings;
    assert_eq!(
        (settings.idle_minutes, settings.lock_on_app_switch),
        (1, true)
    );
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(app.phase(), "locked");

    for event in [OsEvent::Sleep, OsEvent::ScreenLocked, OsEvent::UserSwitched] {
        app.core.unlock(password(PASSWORD)).unwrap();
        app.core.os_event(event);
        assert_eq!(app.phase(), "locked", "{event:?}");
    }

    // Settings survive a restart.
    let mut app = app.restart();
    assert_eq!(app.core.app_status().settings.idle_minutes, 1);
    app.core.unlock(password(PASSWORD)).unwrap();
    app.advance(60_000);
    // A command at the deadline sees the lock, even before the next tick.
    assert_eq!(code(app.core.list_notes(all(), "")), "locked");
}

#[test]
fn trash_older_than_30_days_is_purged_at_unlock() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();
    app.core.trash_note(&id, None).unwrap();
    app.core.lock();
    app.advance(30 * DAY_MS);
    // The first unlock that finds it expired only takes note: one clock reading removes nothing.
    app.core.unlock(password(PASSWORD)).unwrap();
    let trash = |app: &mut App| {
        let trash = filter(json!({"type": "trash"}));
        app.core.list_notes(trash, "").unwrap().len()
    };
    assert_eq!(trash(&mut app), 1);
    app.core.lock();
    app.advance(DAY_MS - 1_000);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(trash(&mut app), 1, "not yet a day since it was first seen");
    app.core.lock();
    // An unlock a day or more later that still finds it expired removes it.
    app.advance(1_000);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(
        app.core
            .list_notes(filter(json!({"type": "trash"})), "")
            .unwrap(),
        vec![]
    );
    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(
        app.core
            .list_notes(filter(json!({"type": "trash"})), "")
            .unwrap(),
        vec![]
    );
}

#[test]
fn a_future_clock_does_not_use_an_old_trash_sighting_to_delete_a_newly_overdue_note() {
    let mut app = App::new();
    unlocked(&mut app);
    let old = app
        .core
        .create_note(input("text", "Fake older", json!({}), json!({})), None)
        .unwrap();
    app.core.trash_note(&old, None).unwrap();
    app.core.lock();
    app.advance(29 * DAY_MS);
    app.core.unlock(password(PASSWORD)).unwrap();
    let newer = app
        .core
        .create_note(input("text", "Fake newer", json!({}), json!({})), None)
        .unwrap();
    app.core.trash_note(&newer, None).unwrap();
    app.core.lock();
    app.advance(2 * DAY_MS);

    // Only the older Note is overdue at the first sighting.
    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(app.core.get_note(&old).is_ok());
    assert!(app.core.get_note(&newer).is_ok());
    app.core.lock();
    let correct_time = app.ms.load(Ordering::SeqCst);
    let mut app = app.restart();

    // One unlock with the computer's date mistakenly a year ahead must not delete the newer
    // Note on the strength of the older Note's sighting, including after an app restart.
    app.advance(365 * DAY_MS);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(code(app.core.get_note(&old)), "not_found");
    assert!(app.core.get_note(&newer).is_ok());
    app.core.lock();
    app.ms.store(correct_time + DAY_MS, Ordering::SeqCst);
    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    let trash = app
        .core
        .list_notes(filter(json!({"type": "trash"})), "")
        .unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].id, newer);
}

#[test]
fn trash_added_under_a_backwards_clock_cannot_inherit_an_earlier_sighting() {
    let mut app = App::new();
    unlocked(&mut app);
    let old = app
        .core
        .create_note(input("text", "Fake older", json!({}), json!({})), None)
        .unwrap();
    app.core.trash_note(&old, None).unwrap();
    app.core.lock();
    let correct_time = app.ms.load(Ordering::SeqCst);
    app.advance(365 * DAY_MS);
    app.core.unlock(password(PASSWORD)).unwrap(); // first sighting of the older Note

    // The date is corrected while Unlocked. A new deletion with an earlier timestamp was
    // never in Trash at that first sighting, even though its date can make it look that way.
    app.ms.store(correct_time + DAY_MS, Ordering::SeqCst);
    let newer = app
        .core
        .create_note(input("text", "Fake newer", json!({}), json!({})), None)
        .unwrap();
    app.core.trash_note(&newer, None).unwrap();
    app.core.lock();
    app.ms.store(correct_time + 366 * DAY_MS, Ordering::SeqCst);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(app.core.get_note(&old).is_ok());
    assert!(app.core.get_note(&newer).is_ok());

    // A later unlock that also sees both overdue may remove them.
    app.core.lock();
    app.advance(DAY_MS);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(code(app.core.get_note(&old)), "not_found");
    assert_eq!(code(app.core.get_note(&newer)), "not_found");
}

#[test]
fn failed_trash_purge_keeps_the_note_until_the_vault_can_be_saved() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input("text", "Fake", json!({}), json!({"body": "keep me"})),
            None,
        )
        .unwrap();
    app.core.trash_note(&id, None).unwrap();
    app.core.lock();
    app.advance(30 * DAY_MS);
    app.core.unlock(password(PASSWORD)).unwrap(); // seen expired once
    app.core.lock();
    app.advance(DAY_MS);
    let vault = app.data_dir().join("vault.enote");
    app.break_vault_on_next_wall_clock_read();

    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(app.core.app_status().purge_trash_failed);
    assert_eq!(
        app.core
            .list_notes(filter(json!({"type": "trash"})), "")
            .unwrap()
            .len(),
        1
    );

    std::fs::remove_dir(&vault).unwrap();
    std::fs::rename(vault.with_extension("saved"), &vault).unwrap();
    app.core.lock();
    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(
        app.core
            .list_notes(filter(json!({"type": "trash"})), "")
            .unwrap()
            .is_empty()
    );
}

// ---------- Backups & Safety Copies ----------

#[test]
fn back_up_then_open_the_backup_keeping_a_safety_copy() {
    let mut app = App::new();
    let key = unlocked(&mut app);
    app.core
        .create_note(input("text", "Fake one", json!({}), json!({})), None)
        .unwrap();
    assert_eq!(code(app.core.write_backup()), "invalid_input");

    let backup = app.temp.path().join("usb").join("fake-backup.enote");
    std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
    let picked = app.core.set_backup_destination(backup.clone()).unwrap();
    assert_eq!(picked.cloud, None);
    let written = app.core.write_backup().unwrap();
    assert_eq!(written.note_count, 1);
    let status = app.core.app_status().backup;
    assert_eq!(
        (status.last_backup_at, status.unbacked_changes),
        (Some(app.core.unix_secs()), false)
    );

    // Newer changes here; the Backup is now older.
    app.advance(5_000);
    app.core
        .create_note(input("text", "Fake two", json!({}), json!({})), None)
        .unwrap();
    assert!(app.core.app_status().backup.unbacked_changes);
    app.core.set_backup_to_open(backup.clone()).unwrap();
    assert_eq!(code(app.core.replace_with_backup()), "invalid_input");
    assert_eq!(
        code(app.core.inspect_backup(password(WRONG_PASSWORD))),
        "wrong_credential"
    );
    let preview = app.core.inspect_backup(password(PASSWORD)).unwrap();
    assert_eq!(
        (preview.note_count, preview.older_than_current),
        (1, Some(true))
    );

    app.core.replace_with_backup().unwrap();
    assert_eq!(app.phase(), "unlocked");
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 1);
    let copies = app.core.list_safety_copies().unwrap();
    assert_eq!(copies[0].kind, "replaced");

    // The replaced Vault (two Notes) can be restored from its Safety Copy.
    let preview = app
        .core
        .inspect_safety_copy(&copies[0].id, password(PASSWORD))
        .unwrap();
    assert_eq!(
        (preview.note_count, preview.older_than_current),
        (2, Some(false))
    );
    app.core.restore_safety_copy().unwrap();
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 2);

    // Opening a Backup with the Recovery Key asks for a new Master Password.
    app.core.set_backup_to_open(backup.clone()).unwrap();
    app.core.inspect_backup(recovery(&key)).unwrap();
    app.core.replace_with_backup().unwrap();
    assert_eq!(app.phase(), "needs_new_password");
    app.core.set_new_password(pw(NEW_PASSWORD)).unwrap();
    let mut app = app.restart();
    app.core.unlock(password(NEW_PASSWORD)).unwrap();
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 1);
}

#[test]
fn a_backup_opens_on_a_computer_without_a_vault() {
    let mut source = App::new();
    unlocked(&mut source);
    source
        .core
        .create_note(
            input("text", "Fake", json!({}), json!({"body": "fake"})),
            None,
        )
        .unwrap();
    let backup = source.temp.path().join("fake-backup.enote");
    source.core.set_backup_destination(backup.clone()).unwrap();
    source.core.write_backup().unwrap();

    let mut other = App::new();
    assert_eq!(other.phase(), "no_vault");
    other.core.set_backup_to_open(backup).unwrap();
    let preview = other.core.inspect_backup(password(PASSWORD)).unwrap();
    assert_eq!((preview.note_count, preview.older_than_current), (1, None));
    other.core.replace_with_backup().unwrap();
    assert_eq!(other.phase(), "unlocked");
    assert!(vault_file(&other).is_file());
}

#[test]
fn a_newer_backup_is_not_called_older_and_cloud_folders_are_flagged() {
    let mut app = App::new();
    unlocked(&mut app);
    let backup = app.temp.path().join("fake-backup.enote");
    app.core.set_backup_destination(backup.clone()).unwrap();
    app.core.write_backup().unwrap();
    app.core.set_backup_to_open(backup).unwrap();
    assert_eq!(
        app.core
            .inspect_backup(password(PASSWORD))
            .unwrap()
            .older_than_current,
        Some(false)
    );

    let dropbox = app
        .temp
        .path()
        .join("home")
        .join("Dropbox")
        .join("fake.enote");
    assert_eq!(
        app.core.set_backup_destination(dropbox).unwrap().cloud,
        Some("dropbox")
    );
    if cfg!(target_os = "macos") {
        let documents = app
            .temp
            .path()
            .join("home")
            .join("Documents")
            .join("fake.enote");
        assert_eq!(
            app.core.set_backup_destination(documents).unwrap().cloud,
            Some("mac_desktop_or_documents")
        );
    }
}

#[test]
fn backups_need_an_unlocked_vault_or_no_vault() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core.lock();
    assert_eq!(code(app.core.backup_destination_allowed()), "locked");
    app.core.backup_to_open_allowed().unwrap();
    assert!(app.core.list_safety_copies().is_ok());
    assert_eq!(code(app.core.write_backup()), "locked");
}

#[test]
fn a_locked_vault_can_be_replaced_after_a_backup_is_authenticated() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .create_note(
            input(
                "text",
                "Fake",
                json!({}),
                json!({"body": "fake restore content"}),
            ),
            None,
        )
        .unwrap();
    let backup = app.temp.path().join("fake-restore.enote");
    app.core.set_backup_destination(backup.clone()).unwrap();
    app.core.write_backup().unwrap();
    app.core.lock();

    app.core.set_backup_to_open(backup).unwrap();
    assert_eq!(
        app.core
            .inspect_backup(password(PASSWORD))
            .unwrap()
            .note_count,
        1
    );
    app.core.replace_with_backup().unwrap();
    assert_eq!(app.phase(), "unlocked");
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 1);
}

// ---------- account care ----------

#[test]
fn changing_the_master_password_keeps_the_recovery_key() {
    let mut app = App::new();
    let key = unlocked(&mut app);
    let backup = app.temp.path().join("fake-old-password.enote");
    app.core.set_backup_destination(backup.clone()).unwrap();
    app.core.write_backup().unwrap();
    assert_eq!(
        code(app.core.change_password(pw(PASSWORD), pw("short"), false)),
        "weak_password"
    );
    assert_eq!(
        code(
            app.core
                .change_password(pw(WRONG_PASSWORD), pw(NEW_PASSWORD), false)
        ),
        "wrong_credential"
    );
    assert_eq!(
        app.core
            .change_password(pw(PASSWORD), pw(NEW_PASSWORD), false)
            .unwrap()
            .recovery_key,
        None
    );
    assert_eq!(app.phase(), "unlocked");
    app.core.set_backup_to_open(backup).unwrap();
    assert_eq!(
        app.core
            .inspect_backup(password(PASSWORD))
            .unwrap()
            .older_than_current,
        Some(true)
    );

    let mut app = app.restart();
    assert_eq!(
        code(app.core.unlock(password(PASSWORD))),
        "wrong_credential"
    );
    app.core.unlock(password(NEW_PASSWORD)).unwrap();
    app.core.lock();
    app.core.unlock(recovery(&key)).unwrap();
}

#[test]
fn key_rotation_replaces_the_recovery_key_and_keeps_wallet_fields_readable() {
    let mut app = App::new();
    let old_key = unlocked(&mut app);
    let pk = app
        .core
        .create_note(
            input(
                "private_key",
                "Fake",
                json!({"chain": "evm"}),
                json!({"key": "fake-evm-key"}),
            ),
            Some(pw(PASSWORD)),
        )
        .unwrap();

    let new_key = app
        .core
        .change_password(pw(PASSWORD), pw(NEW_PASSWORD), true)
        .unwrap()
        .recovery_key
        .unwrap();
    assert_ne!(new_key.as_str(), old_key);
    assert_eq!(app.phase(), "confirm_recovery_kit");
    assert_eq!(
        code(app.core.confirm_recovery_kit(&last_groups(&old_key))),
        "invalid_input"
    );
    app.core
        .confirm_recovery_kit(&last_groups(&new_key))
        .unwrap();
    assert_eq!(app.phase(), "unlocked");
    assert_eq!(
        app.core
            .show_wallet_field(&pk, "key", pw(NEW_PASSWORD), ShowMode::Held)
            .unwrap()
            .as_str(),
        "fake-evm-key"
    );

    let mut app = app.restart();
    assert_eq!(
        code(app.core.unlock(recovery(&old_key))),
        "wrong_credential"
    );
    assert_eq!(
        code(app.core.unlock(password(PASSWORD))),
        "wrong_credential"
    );
    app.advance(60_000);
    app.core.unlock(recovery(&new_key)).unwrap();
    app.core.set_new_password(pw(PASSWORD)).unwrap();
    assert_eq!(
        app.core
            .show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
            .unwrap()
            .as_str(),
        "fake-evm-key"
    );
}

#[test]
fn an_unconfirmed_key_rotation_changes_nothing() {
    let mut app = App::new();
    let old_key = unlocked(&mut app);
    app.core
        .change_password(pw(PASSWORD), pw(NEW_PASSWORD), true)
        .unwrap();
    app.core.lock();
    assert_eq!(app.phase(), "locked");
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.lock();
    app.core.unlock(recovery(&old_key)).unwrap();
}

// ---------- API shapes ----------

#[test]
fn status_has_the_shape_api_ts_expects() {
    let app = App::new();
    let status = serde_json::to_value(app.core.app_status()).unwrap();
    let keys: Vec<&str> = status
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = vec![
        "revision",
        "phase",
        "platform",
        "capture_hiding_active",
        "sharing_guard",
        "lock_in_ms",
        "clipboard_clear_in_ms",
        "copies",
        "reveal",
        "failed_attempts",
        "retry_in_ms",
        "show_recovery_hint",
        "backup",
        "purge_trash_failed",
        "save_durability_uncertain",
        "settings_unreadable",
        "settings",
    ];
    expected.sort_unstable();
    let mut keys = keys;
    keys.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(
        status["settings"],
        json!({
            "idle_minutes": 5,
            "lock_on_app_switch": false,
            "seed_explainer_seen": false,
            "clipboard_clear_seconds": 30,
            "count_activity_anywhere": false,
            "count_activity_anywhere_supported": true,
        })
    );
    assert_eq!(
        status["backup"],
        json!({"last_backup_at": null, "reminder_due": false, "unbacked_changes": false})
    );
    assert_eq!(status["sharing_guard"], json!({"up": false, "apps": []}));
    assert_eq!(status["revision"], 0);
    assert_eq!(status["copies"], 0);
    assert_eq!(status["purge_trash_failed"], false);
    assert_eq!(status["settings_unreadable"], false);
}

#[test]
fn the_backup_reminder_is_due_after_7_days_of_unbacked_changes() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();
    app.advance(7 * DAY_MS - 1_000);
    assert!(!app.core.app_status().backup.reminder_due);
    app.advance(1_000);
    assert!(app.core.app_status().backup.reminder_due);
    app.core.mark_seed_explainer_seen().unwrap_err(); // auto-locked long ago
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.mark_seed_explainer_seen().unwrap();
    assert!(app.core.app_status().settings.seed_explainer_seen);
}

// ---------- review fixes ----------

#[test]
fn an_inspected_backup_is_dropped_after_two_minutes_without_an_unlocked_vault() {
    let (_source, backup) = a_backup();
    let mut app = App::new();
    app.core.set_backup_to_open(backup.clone()).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    app.advance(2 * 60_000);
    app.core.tick();
    assert_eq!(code(app.core.replace_with_backup()), "invalid_input");
    assert_eq!(app.phase(), "no_vault");

    // Within that time it can still be installed.
    app.core.set_backup_to_open(backup).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    app.advance(2 * 60_000 - 1);
    app.core.tick();
    app.core.replace_with_backup().unwrap();
    assert_eq!(app.phase(), "unlocked");
}

#[test]
fn closing_a_preview_forgets_the_decrypted_copy_at_once() {
    let (_source, backup) = a_backup();
    let mut app = App::new();
    app.core.set_backup_to_open(backup).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    app.core.discard_inspection();
    assert_eq!(code(app.core.replace_with_backup()), "invalid_input");
    // The picked file is forgotten too: opening it again starts with the file dialog.
    assert_eq!(
        code(app.core.inspect_backup(password(PASSWORD))),
        "invalid_input"
    );
    assert_eq!(app.phase(), "no_vault");
}

#[test]
fn an_inspection_is_dropped_when_the_phase_changes_or_the_vault_changes() {
    let (_source, backup) = a_backup();
    let mut app = App::new();
    app.core.set_backup_to_open(backup.clone()).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    // Creating a Vault throws the inspected Backup (and the picked file) away.
    unlocked(&mut app);
    assert_eq!(code(app.core.replace_with_backup()), "invalid_input");
    assert_eq!(
        code(app.core.inspect_backup(password(PASSWORD))),
        "invalid_input"
    );
    assert_eq!(app.core.list_notes(all(), "").unwrap(), vec![]);

    // An edit after the preview makes the preview stale.
    app.core.set_backup_to_open(backup).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    app.core
        .create_note(input("text", "Fake newer", json!({}), json!({})), None)
        .unwrap();
    assert_eq!(code(app.core.replace_with_backup()), "invalid_input");
    assert_eq!(
        app.core.list_notes(all(), "").unwrap()[0].title,
        "Fake newer"
    );
}

#[test]
fn a_sharing_guard_dismissal_made_while_locked_does_not_carry_into_the_session() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input("login", "Fake", json!({}), json!({"password": "fake-pass"})),
            None,
        )
        .unwrap();
    app.core.lock();
    app.core.guard_update(&zoom());
    app.core.dismiss_sharing_guard();
    assert!(!app.core.app_status().sharing_guard.up);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(app.core.app_status().sharing_guard.up);
    assert_eq!(
        code(app.core.show_field(&id, "password")),
        "sharing_guard_up"
    );

    // A dismissal covers the apps running then; a new one brings the shield back.
    app.core.dismiss_sharing_guard();
    app.core.show_field(&id, "password").unwrap();
    let mut apps = zoom();
    apps.push(guard::RunningApp {
        bundle_id: Some("com.obsproject.obs-studio".into()),
        exe_name: Some("obs64.exe".into()),
    });
    app.core.guard_update(&apps);
    let status = app.core.app_status();
    assert!(status.sharing_guard.up);
    assert_eq!(status.reveal, None);
}

#[test]
fn a_damaged_vault_can_be_restored_from_a_safety_copy_while_locked() {
    let mut app = App::new();
    unlocked(&mut app);
    for title in ["Fake one", "Fake two"] {
        app.core
            .create_note(input("text", title, json!({}), json!({})), None)
            .unwrap();
    }
    app.core.lock();
    std::fs::write(vault_file(&app), b"fake damaged vault").unwrap();

    let mut app = app.restart();
    assert_eq!(app.phase(), "locked");
    assert!(app.core.list_safety_copies().is_ok());
    let e = app.core.unlock(password(PASSWORD)).unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (
            "damaged",
            "This Vault file is damaged. Restore a Safety Copy or Backup."
        )
    );
    // A damaged Vault can be replaced from a separately authenticated Safety Copy or Backup.
    app.core.backup_to_open_allowed().unwrap();
    let copies = app.core.list_safety_copies().unwrap();
    assert_eq!(
        code(
            app.core
                .inspect_safety_copy(&copies[0].id, password(WRONG_PASSWORD))
        ),
        "wrong_credential"
    );
    let preview = app
        .core
        .inspect_safety_copy(&copies[0].id, password(PASSWORD))
        .unwrap();
    assert_eq!((preview.note_count, preview.older_than_current), (1, None));
    app.core.restore_safety_copy().unwrap();
    assert_eq!(app.phase(), "unlocked");
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 1);
    // The damaged file was kept as a Safety Copy too.
    assert_eq!(app.core.list_safety_copies().unwrap()[0].kind, "replaced");

    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 1);
}

#[test]
fn focus_loss_with_lock_on_app_switch_drops_a_pending_inspection_and_an_unconfirmed_rotation() {
    let mut app = App::new();
    let key = unlocked(&mut app);
    set_lock_on_app_switch(&mut app);
    app.core
        .change_password(pw(PASSWORD), pw(NEW_PASSWORD), true)
        .unwrap();
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(app.phase(), "locked");
    app.core.unlock(recovery(&key)).unwrap();
    app.core.set_new_password(pw(PASSWORD)).unwrap();
    app.core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();

    // With the Vault damaged, an inspected Safety Copy is dropped on an app switch too.
    app.core.lock();
    std::fs::write(vault_file(&app), b"fake damaged vault").unwrap();
    assert_eq!(code(app.core.unlock(password(PASSWORD))), "damaged");
    let copies = app.core.list_safety_copies().unwrap();
    app.core
        .inspect_safety_copy(&copies[0].id, password(PASSWORD))
        .unwrap();
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(code(app.core.restore_safety_copy()), "invalid_input");
    assert_eq!(app.phase(), "locked");
}

#[test]
fn a_safety_copy_opened_with_the_recovery_key_asks_for_a_new_password() {
    let mut app = App::new();
    let key = unlocked(&mut app);
    app.core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();
    let copies = app.core.list_safety_copies().unwrap();
    app.core
        .inspect_safety_copy(&copies[0].id, recovery(&key))
        .unwrap();
    app.core.restore_safety_copy().unwrap();
    assert_eq!(app.phase(), "needs_new_password");
    assert_eq!(code(app.core.list_notes(all(), "")), "locked");
    app.core.set_new_password(pw(NEW_PASSWORD)).unwrap();
    assert_eq!(app.core.list_notes(all(), "").unwrap(), vec![]);
    let mut app = app.restart();
    app.core.unlock(password(NEW_PASSWORD)).unwrap();
}

#[test]
fn accessible_reveals_end_after_20_s_and_held_ones_on_hide() {
    let mut app = App::new();
    unlocked(&mut app);
    let pk = app
        .core
        .create_note(
            input("private_key", "Fake", json!({}), json!({"key": "fake-key"})),
            Some(pw(PASSWORD)),
        )
        .unwrap();
    app.core
        .show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Accessible)
        .unwrap();
    app.advance(19_999);
    app.core.tick();
    assert!(app.core.app_status().reveal.is_some());
    app.advance(1);
    app.core.tick();
    assert_eq!(app.core.app_status().reveal, None);

    app.core
        .show_wallet_field(&pk, "key", pw(PASSWORD), ShowMode::Held)
        .unwrap();
    app.advance(60_000);
    app.core.tick();
    assert_eq!(app.core.app_status().reveal.unwrap().expires_in_ms, None);
    app.core.hide_field();
    assert_eq!(app.core.app_status().reveal, None);
}

#[test]
fn changes_that_change_nothing_are_not_saved() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();
    let backup = app.temp.path().join("fake-backup.enote");
    app.core.set_backup_destination(backup).unwrap();
    app.core.write_backup().unwrap();
    let copies = app.core.list_safety_copies().unwrap();

    app.advance(1_000);
    app.core.set_favorite(&id, false).unwrap();
    app.core.restore_note(&id).unwrap();
    assert_eq!(app.core.empty_trash(None).unwrap(), 0);
    assert!(!app.core.app_status().backup.unbacked_changes);
    assert_eq!(app.core.list_safety_copies().unwrap(), copies);
}

#[cfg(unix)]
#[test]
fn a_failed_save_leaves_memory_as_it_was_on_disk() {
    use std::os::unix::fs::PermissionsExt;
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .create_note(input("text", "Fake one", json!({}), json!({})), None)
        .unwrap();
    let dir = app.data_dir();
    let set_mode = |mode| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode));

    set_mode(0o555).unwrap();
    let failed = (
        code(
            app.core
                .create_note(input("text", "Fake two", json!({}), json!({})), None),
        ),
        code(
            app.core
                .change_password(pw(PASSWORD), pw(NEW_PASSWORD), false),
        ),
    );
    set_mode(0o755).unwrap();
    assert_eq!(failed, ("io", "io"));
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 1);

    // The next save keeps the old password.
    app.core
        .create_note(input("text", "Fake three", json!({}), json!({})), None)
        .unwrap();
    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 2);
}

// ---------- security audit fixes (2026-10) ----------

/// One Seed Phrase Note and one Text Note. Returns their ids.
fn a_wallet_and_a_text_note(app: &mut App) -> (String, String) {
    let seed = app
        .core
        .create_note(
            input(
                "seed_phrase",
                "Fake S",
                json!({}),
                json!({"words": (["zoo"; 12].join(" "))}),
            ),
            Some(pw(PASSWORD)),
        )
        .unwrap();
    let text = app
        .core
        .create_note(input("text", "Fake T", json!({}), json!({})), None)
        .unwrap();
    (seed, text)
}

#[test]
fn removing_a_wallet_kind_needs_the_master_password_at_every_step() {
    let mut app = App::new();
    unlocked(&mut app);
    let (seed, text) = a_wallet_and_a_text_note(&mut app);

    // Trash.
    assert_eq!(code(app.core.trash_note(&seed, None)), "invalid_input");
    assert_eq!(
        code(app.core.trash_note(&seed, Some(pw(WRONG_PASSWORD)))),
        "wrong_credential"
    );
    assert!(app.core.get_note(&seed).unwrap().deleted_at.is_none());
    app.core.trash_note(&seed, Some(pw(PASSWORD))).unwrap();
    app.core.trash_note(&text, None).unwrap();

    // Empty Trash, while a Wallet Kind is in it.
    assert_eq!(code(app.core.empty_trash(None)), "invalid_input");
    assert_eq!(
        code(app.core.empty_trash(Some(pw(WRONG_PASSWORD)))),
        "wrong_credential"
    );

    // Delete forever. An ordinary Note still goes without the password.
    assert_eq!(code(app.core.delete_forever(&seed, None)), "invalid_input");
    assert_eq!(
        code(app.core.delete_forever(&seed, Some(pw(WRONG_PASSWORD)))),
        "wrong_credential"
    );
    app.core.delete_forever(&text, None).unwrap();
    assert!(app.core.get_note(&seed).is_ok());
    app.core.delete_forever(&seed, Some(pw(PASSWORD))).unwrap();
    assert_eq!(code(app.core.get_note(&seed)), "not_found");

    // With only ordinary Notes in Trash, emptying it needs no password.
    let other = app
        .core
        .create_note(input("text", "Fake U", json!({}), json!({})), None)
        .unwrap();
    app.core.trash_note(&other, None).unwrap();
    assert_eq!(app.core.empty_trash(None).unwrap(), 1);

    // Restoring from Trash is never destructive, so it needs none either.
    let (seed, _) = a_wallet_and_a_text_note(&mut app);
    app.core.trash_note(&seed, Some(pw(PASSWORD))).unwrap();
    app.core.restore_note(&seed).unwrap();
    assert_eq!(app.core.empty_trash(Some(pw(PASSWORD))).unwrap(), 0);
}

#[test]
fn a_ninth_different_hidden_field_within_a_minute_is_not_shown() {
    let mut app = App::new();
    unlocked(&mut app);
    let ids: Vec<String> = (0..9)
        .map(|i| {
            let note = input(
                "login",
                &format!("Fake {i}"),
                json!({}),
                json!({"password": format!("fake-value-{i}")}),
            );
            app.core.create_note(note, None).unwrap()
        })
        .collect();
    for id in &ids[..8] {
        app.core.show_field(id, "password").unwrap();
        app.advance(1_000);
    }
    let refused = app.core.show_field(&ids[8], "password").unwrap_err();
    assert_eq!(refused.code, "too_many");
    assert!(
        refused.message.contains("52 seconds"),
        "{}",
        refused.message
    );
    // One already shown can be shown again, and so can a new one once the minute has passed.
    app.core.show_field(&ids[0], "password").unwrap();
    app.advance(53_000);
    app.core.activity();
    assert_eq!(
        app.core.show_field(&ids[8], "password").unwrap().as_str(),
        "fake-value-8"
    );
}

#[test]
fn a_backup_destination_is_good_for_one_backup_only() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();
    let dest = app.temp.path().join("fake-once.enote");

    app.core.set_backup_destination(dest.clone()).unwrap();
    let written = app.core.write_backup().unwrap();
    assert!(written.durable);
    assert_eq!(written.note_count, 1);
    let status = app.core.app_status().backup;
    assert!(status.last_backup_at.is_some() && !status.unbacked_changes);
    // Writing again needs a fresh pick: the file just written is not overwritten.
    let first = std::fs::read(&dest).unwrap();
    app.core
        .create_note(input("text", "Fake 2", json!({}), json!({})), None)
        .unwrap();
    assert_eq!(code(app.core.write_backup()), "invalid_input");
    assert_eq!(std::fs::read(&dest).unwrap(), first);

    // A pick the owner backs out of, one left for ten minutes, and one made before a lock are
    // all forgotten.
    let other = app.temp.path().join("fake-declined.enote");
    app.core.set_backup_destination(other.clone()).unwrap();
    app.core.forget_backup_destination();
    assert_eq!(code(app.core.write_backup()), "invalid_input");

    app.core.set_backup_destination(other.clone()).unwrap();
    for _ in 0..10 {
        app.advance(60_000);
        app.core.activity();
    }
    assert_eq!(code(app.core.write_backup()), "invalid_input");

    app.core.set_backup_destination(other.clone()).unwrap();
    app.core.lock();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(code(app.core.write_backup()), "invalid_input");
    assert!(!other.exists());
}

#[test]
fn a_copy_opened_from_the_lock_screen_is_still_compared_with_the_vault_here() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .create_note(input("text", "Fake", json!({}), json!({})), None)
        .unwrap();
    let backup = app.temp.path().join("fake-earlier.enote");
    app.core.set_backup_destination(backup.clone()).unwrap();
    app.core.write_backup().unwrap();
    app.advance(5_000);
    app.core
        .create_note(input("text", "Fake later", json!({}), json!({})), None)
        .unwrap();
    app.core.lock();

    // Same Vault, same password: the older Backup is called older although the app is Locked.
    app.core.set_backup_to_open(backup).unwrap();
    let preview = app.core.inspect_backup(password(PASSWORD)).unwrap();
    assert_eq!(preview.older_than_current, Some(true));
    assert!(preview.replaces_vault);
    app.core.discard_inspection();
    assert_eq!(app.phase(), "locked");
    assert_eq!(app.core.app_status().failed_attempts, 0);

    // A Backup of another Vault with another password can't be compared, and says so: it still
    // replaces a Vault, and the screens must not present it as a first Vault.
    let (_source, foreign) = a_backup_with(NEW_PASSWORD);
    app.core.set_backup_to_open(foreign.clone()).unwrap();
    let preview = app.core.inspect_backup(password(NEW_PASSWORD)).unwrap();
    assert_eq!(preview.older_than_current, None);
    assert!(preview.replaces_vault);
    assert_eq!(app.core.app_status().failed_attempts, 0);

    // On a computer with no Vault there is nothing to replace.
    let mut fresh = App::new();
    fresh.core.set_backup_to_open(foreign).unwrap();
    let preview = fresh.core.inspect_backup(password(NEW_PASSWORD)).unwrap();
    assert_eq!(preview.older_than_current, None);
    assert!(!preview.replaces_vault);
}

#[test]
fn wrong_password_delays_survive_a_restart() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core.lock();
    for _ in 0..5 {
        app.advance(60_000);
        assert_eq!(
            code(app.core.unlock(password(WRONG_PASSWORD))),
            "wrong_credential"
        );
    }
    app.core.tick();
    let mut app = app.restart();
    let status = app.core.app_status();
    assert_eq!(status.failed_attempts, 5);
    assert!(status.show_recovery_hint);
    assert!(status.retry_in_ms.is_some());
    assert_eq!(code(app.core.unlock(password(PASSWORD))), "retry_later");
    app.advance(5_000);
    app.core.unlock(password(PASSWORD)).unwrap();
    // The right password clears the count, on disk too.
    app.core.tick();
    app.core.lock();
    let app = app.restart();
    assert_eq!(app.core.app_status().failed_attempts, 0);
}

#[test]
fn an_unreadable_settings_file_means_the_strictest_lock_rules() {
    let mut app = App::new();
    unlocked(&mut app);
    // Every setting at its most relaxed.
    app.core
        .set_settings(SettingsInput {
            idle_minutes: 30,
            lock_on_app_switch: false,
            clipboard_clear_seconds: Some(60),
            count_activity_anywhere: Some(true),
        })
        .unwrap();
    let relaxed = app.core.app_status().settings;
    assert_eq!(
        (
            relaxed.clipboard_clear_seconds,
            relaxed.count_activity_anywhere
        ),
        (60, true)
    );
    app.core.lock();
    std::fs::write(app.data_dir().join(store::SETTINGS_FILE), b"{ not json").unwrap();

    let app = app.restart();
    let status = app.core.app_status();
    assert!(status.settings_unreadable);
    assert_eq!(status.settings.idle_minutes, 1);
    assert!(status.settings.lock_on_app_switch);
    assert_eq!(status.settings.clipboard_clear_seconds, 30);
    assert!(!status.settings.count_activity_anywhere);

    // No settings file at all is a first run: the ordinary defaults, and nothing to report.
    let fresh = App::new();
    let status = fresh.core.app_status();
    assert!(!status.settings_unreadable);
    assert_eq!(status.settings.idle_minutes, 5);
    assert!(!status.settings.lock_on_app_switch);
    assert_eq!(status.settings.clipboard_clear_seconds, 30);
    assert!(!status.settings.count_activity_anywhere);
}

#[test]
fn a_settings_file_from_before_the_newer_settings_is_read_with_their_defaults() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core.lock();
    // What a build from before these two settings wrote: the lock rule and its own notes.
    let older = json!({
        "idle_minutes": 10,
        "lock_on_app_switch": true,
        "seed_explainer_seen": true,
        "last_backup_at": null,
        "oldest_unbacked_change_at": null,
        "failed_attempts": 0,
        "trash_expiry_seen_at": null,
    });
    let file = app.data_dir().join(store::SETTINGS_FILE);
    std::fs::write(&file, older.to_string()).unwrap();

    // It is not an unreadable file: the owner's lock rule is kept, the new settings start at
    // their defaults.
    let app = app.restart();
    let status = app.core.app_status();
    assert!(!status.settings_unreadable);
    assert_eq!(
        serde_json::to_value(status.settings).unwrap(),
        json!({
            "idle_minutes": 10,
            "lock_on_app_switch": true,
            "seed_explainer_seen": true,
            "clipboard_clear_seconds": 30,
            "count_activity_anywhere": false,
            "count_activity_anywhere_supported": true,
        })
    );

    // A file that can be read but holds a clipboard time that is neither choice: 30, like an
    // idle time that is not one of the choices goes to the nearest one.
    let odd = json!({
        "idle_minutes": 7,
        "lock_on_app_switch": false,
        "clipboard_clear_seconds": 45,
        "count_activity_anywhere": true,
    });
    std::fs::write(&file, odd.to_string()).unwrap();
    let mut app = app.restart();
    let status = app.core.app_status();
    assert!(!status.settings_unreadable);
    assert_eq!(
        (
            status.settings.idle_minutes,
            status.settings.clipboard_clear_seconds,
            status.settings.count_activity_anywhere
        ),
        (5, 30, true)
    );
    app.core.unlock(password(PASSWORD)).unwrap();
    let id = app
        .core
        .create_note(
            input("login", "Fake", json!({}), json!({"password": "fake-pass"})),
            None,
        )
        .unwrap();
    app.core.copy_field(&id, "password", None).unwrap();
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
}

#[test]
fn locking_on_app_switch_leaves_a_copy_long_enough_to_paste_it() {
    let mut app = App::new();
    unlocked(&mut app);
    set_lock_on_app_switch(&mut app);
    let id = app
        .core
        .create_note(
            input(
                "login",
                "Fake",
                json!({}),
                json!({"password": "fake-copy-1"}),
            ),
            None,
        )
        .unwrap();
    app.core.copy_field(&id, "password", None).unwrap();
    app.advance(1_000);
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(app.phase(), "locked");
    assert_eq!(app.clipboard().as_deref(), Some("fake-copy-1"));
    app.advance(28_000);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-copy-1"));
    app.advance(1_000);
    app.core.tick();
    assert_eq!(app.clipboard(), None);
}

#[test]
fn a_vault_from_a_newer_document_schema_is_not_called_damaged() {
    let app = App::new();
    let body = json!({
        "schema_version": notes::SCHEMA_VERSION + 1,
        "changed_at": START_UNIX,
        "change_counter": 1,
        "notes": []
    });
    let created = vault_format::create(
        &secrecy::SecretString::from(PASSWORD),
        KdfParams::TESTING,
        &Limits::TESTING,
        &serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(app.data_dir()).unwrap();
    std::fs::write(vault_file(&app), &created.bytes).unwrap();
    let mut app = app.restart();
    assert_eq!(app.phase(), "locked");
    let refused = app.core.unlock(password(PASSWORD)).unwrap_err();
    assert_eq!(refused.code, "unsupported");
    assert!(
        refused.message.contains("newer version"),
        "{}",
        refused.message
    );
}

#[test]
fn a_file_far_larger_than_any_vault_is_not_opened_as_a_backup() {
    let mut app = App::new();
    let huge = app.temp.path().join("fake-huge.enote");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(store::MAX_FILE_BYTES + 1)
        .unwrap();
    app.core.set_backup_to_open(huge).unwrap();
    let refused = app.core.inspect_backup(password(PASSWORD)).unwrap_err();
    assert_eq!(refused.code, "io");
    assert!(
        refused.message.contains("larger than any Vault"),
        "{}",
        refused.message
    );
}

#[test]
fn a_vault_can_only_be_created_where_there_is_none() {
    let mut app = App::new();
    app.core.create_vault_allowed().unwrap();
    unlocked(&mut app);
    assert_eq!(code(app.core.create_vault_allowed()), "invalid_input");
    app.core.lock();
    assert_eq!(code(app.core.create_vault_allowed()), "invalid_input");
}

// ---------- the optional fields of document schema 2 ----------

#[test]
fn the_fields_added_in_schema_2_are_saved_listed_and_shown_after_unlocking_again() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input(
                "api_key",
                "Fake deploy key",
                json!({
                    "service": "svc.example.test",
                    "key_id": "FAKEKEYID0001",
                    "purpose": "Nightly fake uploads",
                    "url": "https://console.example.test/keys",
                    "expires": "2027-01-31",
                    "env_var": "FAKE_DEPLOY_KEY",
                }),
                // Pasted with a stray space and line ending around it.
                json!({"key": " fake-api-key-0000\n", "secret": "fake-api-secret-0000"}),
            ),
            None,
        )
        .unwrap();
    // Every save is read back and parsed before it replaces the Vault, and unlocking parses it
    // again: a schema 2 document passes both.
    app.core.lock();
    app.core.unlock(password(PASSWORD)).unwrap();

    let rows = app.core.list_notes(all(), "nightly fake_deploy").unwrap();
    assert_eq!(rows.len(), 1);
    let row = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(row["expires"], "2027-01-31");
    assert_eq!(row["has_hidden"], true);
    assert_eq!(row["subtitle"], "svc.example.test");
    let view = serde_json::to_value(app.core.get_note(&id).unwrap()).unwrap();
    assert_eq!(view["visible"]["key_id"], "FAKEKEYID0001");
    assert_eq!(view["visible"]["env_var"], "FAKE_DEPLOY_KEY");
    assert_eq!(view["hidden"], json!([["key", true], ["secret", true]]));
    assert_eq!(
        app.core.show_field(&id, "key").unwrap().as_str(),
        "fake-api-key-0000"
    );
    assert_eq!(
        app.core.show_field(&id, "secret").unwrap().as_str(),
        "fake-api-secret-0000"
    );
}

#[test]
fn a_value_that_breaks_the_rule_for_its_field_is_refused_without_repeating_it() {
    let mut app = App::new();
    unlocked(&mut app);
    // (field, a value that breaks its rule, words the message must have)
    let cases = [
        ("expires", "2027-02-30", "2027-01-31"),
        ("env_var", "FAKE-DEPLOY KEY", "underscores"),
        ("purpose", "fake\npurpose", "one line"),
        ("url", "console.example.test\n/keys", "one line"),
    ];
    for (field, value, hint) in cases {
        let new = input(
            "api_key",
            "Fake key",
            json!({"service": "svc.example.test", field: value}),
            json!({"key": "fake-api-key-0000"}),
        );
        let refused = app.core.create_note(new, None).unwrap_err();
        assert_eq!(refused.code, "invalid_input", "{field}");
        assert!(
            refused.message.contains(hint),
            "{field}: {}",
            refused.message
        );
        assert!(
            !refused.message.contains(value),
            "{field}: {}",
            refused.message
        );
    }
    assert!(app.core.list_notes(all(), "").unwrap().is_empty());
}

#[test]
fn an_edit_saved_without_changing_anything_is_not_saved_as_a_change() {
    let mut app = App::new();
    unlocked(&mut app);
    let visible = json!({"website": "site.example.test", "username": "fake-user"});
    let id = app
        .core
        .create_note(
            input(
                "login",
                "Fake login",
                visible.clone(),
                json!({"password": "fake-login-password-123"}),
            ),
            None,
        )
        .unwrap();
    let backup = app.temp.path().join("fake-backup.enote");
    app.core.set_backup_destination(backup).unwrap();
    app.core.write_backup().unwrap();
    let copies = app.core.list_safety_copies().unwrap();
    let updated_at = app.core.get_note(&id).unwrap().updated_at;

    // What the edit form sends when it is saved with nothing touched.
    app.advance(60_000);
    app.core
        .update_note(&id, edit("Fake login", visible, json!({})), None)
        .unwrap();
    assert!(!app.core.app_status().backup.unbacked_changes);
    assert_eq!(app.core.list_safety_copies().unwrap(), copies);
    assert_eq!(app.core.get_note(&id).unwrap().updated_at, updated_at);

    // A real difference is saved as before.
    app.core
        .update_note(
            &id,
            edit(
                "Fake login",
                json!({"username": "fake-user-two"}),
                json!({}),
            ),
            None,
        )
        .unwrap();
    assert!(app.core.app_status().backup.unbacked_changes);
    assert!(app.core.get_note(&id).unwrap().updated_at > updated_at);
}

#[test]
fn an_api_key_is_created_and_edited_with_the_schema_2_fields() {
    let mut app = App::new();
    unlocked(&mut app);
    let id = app
        .core
        .create_note(
            input(
                "api_key",
                "Fake deploy key",
                json!({
                    "service": "svc.example.test",
                    "key_id": "FAKEKEYID0001",
                    "purpose": "Nightly fake uploads",
                    "url": "https://console.example.test/keys",
                    "expires": "2027-01-31",
                    "env_var": "FAKE_DEPLOY_KEY",
                }),
                json!({"key": "fake-api-key-0000", "secret": "fake-api-secret-0000"}),
            ),
            None,
        )
        .unwrap();

    // An edit: three values changed, one removed, two left out (kept); the secret replaced
    // and the key left out (kept).
    app.advance(5_000);
    app.core
        .update_note(
            &id,
            edit(
                "Fake deploy key",
                json!({
                    "key_id": "FAKEKEYID0002",
                    "expires": "2028-02-29",
                    "env_var": "FAKE_DEPLOY_KEY_2",
                    "purpose": "",
                }),
                json!({"secret": "fake-api-secret-0001"}),
            ),
            None,
        )
        .unwrap();
    let expected = json!({
        "service": "svc.example.test",
        "key_id": "FAKEKEYID0002",
        "url": "https://console.example.test/keys",
        "expires": "2028-02-29",
        "env_var": "FAKE_DEPLOY_KEY_2",
    });
    // Saved and read back, not only held in memory.
    let mut app = app.restart();
    app.core.unlock(password(PASSWORD)).unwrap();
    let view = serde_json::to_value(app.core.get_note(&id).unwrap()).unwrap();
    assert_eq!(view["visible"], expected);
    assert_eq!(view["hidden"], json!([["key", true], ["secret", true]]));
    assert_eq!(
        app.core.show_field(&id, "key").unwrap().as_str(),
        "fake-api-key-0000"
    );
    assert_eq!(
        app.core.show_field(&id, "secret").unwrap().as_str(),
        "fake-api-secret-0001"
    );
    let row =
        serde_json::to_value(&app.core.list_notes(all(), "fakekeyid0002").unwrap()[0]).unwrap();
    assert_eq!(row["expires"], "2028-02-29");
    assert_eq!(row["has_hidden"], true);
    // The edited variable name is the one a NAME=value copy uses.
    app.core.copy_field_env(&id).unwrap();
    assert_eq!(
        app.clipboard().as_deref(),
        Some("FAKE_DEPLOY_KEY_2=fake-api-key-0000")
    );

    // Both Hidden Fields cleared: the row has nothing left to copy.
    app.core
        .update_note(
            &id,
            edit(
                "Fake deploy key",
                json!({}),
                json!({"key": "", "secret": ""}),
            ),
            None,
        )
        .unwrap();
    let view = serde_json::to_value(app.core.get_note(&id).unwrap()).unwrap();
    assert_eq!(view["visible"], expected);
    assert_eq!(view["hidden"], json!([["key", false], ["secret", false]]));
    let row = serde_json::to_value(&app.core.list_notes(all(), "").unwrap()[0]).unwrap();
    assert_eq!(row["has_hidden"], false);
    assert_eq!(code(app.core.copy_field_env(&id)), "not_found");
}

#[test]
fn an_edit_that_breaks_the_rule_for_a_field_names_the_field_and_never_the_value() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);
    let long_purpose = "fake ".repeat(41);
    // (Note, its title, field, a value that breaks the field's rule, what the Editor calls it)
    let cases = [
        (
            &n.api,
            "Fake API key",
            "expires",
            "2027-02-30",
            "expiry date",
        ),
        (
            &n.api,
            "Fake API key",
            "expires",
            "31/01/2027",
            "expiry date",
        ),
        (
            &n.api,
            "Fake API key",
            "env_var",
            "FAKE-DEPLOY KEY",
            "variable name",
        ),
        (&n.api, "Fake API key", "env_var", "9FAKE", "variable name"),
        (
            &n.api,
            "Fake API key",
            "purpose",
            "fake\npurpose",
            "purpose",
        ),
        (
            &n.api,
            "Fake API key",
            "url",
            "console.example.test\n/keys",
            "management URL",
        ),
        (&n.api, "Fake API key", "key_id", "FAKE\tKEYID", "key ID"),
        (
            &n.login,
            "Fake login",
            "purpose",
            long_purpose.as_str(),
            "purpose",
        ),
        (
            &n.seed,
            "Fake seed",
            "address",
            "fake\naddress",
            "public address",
        ),
        (
            &n.seed,
            "Fake seed",
            "derivation_path",
            "m/44'\n/0'",
            "derivation path",
        ),
    ];
    let before: Vec<Value> = [&n.api, &n.login, &n.seed]
        .map(|id| serde_json::to_value(app.core.get_note(id).unwrap()).unwrap())
        .to_vec();
    app.advance(5_000);
    for (id, title, field, value, name) in cases {
        let master_password = (title == "Fake seed").then(|| pw(PASSWORD));
        let refused = app
            .core
            .update_note(
                id,
                edit(title, json!({ field: value }), json!({})),
                master_password,
            )
            .unwrap_err();
        assert_eq!(refused.code, "invalid_input", "{field}");
        assert!(
            refused.message.contains(name),
            "{field}: {}",
            refused.message
        );
        assert!(
            !refused.message.contains(value),
            "{field}: {}",
            refused.message
        );
        // Plain words on one line: nothing of the value's shape got in either.
        assert!(!refused.message.contains(['\n', '\t']), "{field}");
    }
    // Nothing was changed by any of them.
    let after: Vec<Value> = [&n.api, &n.login, &n.seed]
        .map(|id| serde_json::to_value(app.core.get_note(id).unwrap()).unwrap())
        .to_vec();
    assert_eq!(after, before);

    // The same when a new Note is created.
    let new = input(
        "api_key",
        "Fake new key",
        json!({"service": "svc.example.test", "key_id": "FAKE\nKEYID"}),
        json!({"key": "fake-api-key-0000"}),
    );
    let refused = app.core.create_note(new, None).unwrap_err();
    assert_eq!(refused.code, "invalid_input");
    assert!(refused.message.contains("key ID"), "{}", refused.message);
    assert!(!refused.message.contains("KEYID"), "{}", refused.message);
    assert_eq!(app.core.list_notes(all(), "").unwrap().len(), 5);
}

// ---------- key convenience: more ways to copy ----------

/// A Text Note's body of four lines with mixed line endings, the third one empty.
const BODY: &str = "fake first line\r\nfake second line\n\nfake fourth line\r\n";

/// One Note of every Kind. The API Key has a variable name and no management URL.
struct Samples {
    login: String,
    api: String,
    text: String,
    pk: String,
    seed: String,
}

fn sample_notes(app: &mut App) -> Samples {
    let core = &mut app.core;
    Samples {
        login: core
            .create_note(
                input(
                    "login",
                    "Fake login",
                    json!({"website": "site.example.test", "username": "fake-user"}),
                    json!({"password": "fake-login-pass"}),
                ),
                None,
            )
            .unwrap(),
        api: core
            .create_note(
                input(
                    "api_key",
                    "Fake API key",
                    json!({"service": "svc.example.test", "env_var": "FAKE_DEPLOY_KEY"}),
                    json!({"key": "fake-api-key-0000"}),
                ),
                None,
            )
            .unwrap(),
        text: core
            .create_note(
                input("text", "Fake text", json!({}), json!({ "body": BODY })),
                None,
            )
            .unwrap(),
        pk: core
            .create_note(
                input(
                    "private_key",
                    "Fake private key",
                    json!({"chain": "evm", "address": "0xfake-address"}),
                    json!({"key": "fake-private-key"}),
                ),
                Some(pw(PASSWORD)),
            )
            .unwrap(),
        seed: core
            .create_note(
                input(
                    "seed_phrase",
                    "Fake seed",
                    json!({"wallet_name": "Fake wallet"}),
                    json!({"words": (["zoo"; 12].join(" ")), "passphrase": "fake-25th"}),
                ),
                Some(pw(PASSWORD)),
            )
            .unwrap(),
    }
}

/// How many times anything was written to (or cleared from) the clipboard.
fn clipboard_changes(app: &App) -> u64 {
    app.board.lock().unwrap().change_count
}

fn disclosure_budget_notes(app: &mut App) -> Vec<String> {
    (0..9)
        .map(|i| {
            app.core
                .create_note(
                    input(
                        "api_key",
                        &format!("Fake budget {i}"),
                        json!({"service": format!("fake-service-{i}"), "env_var": format!("FAKE_KEY_{i}")}),
                        json!({"key": format!("fake-key-{i}"), "secret": format!("fake-secret-{i}\nfake-second-line-{i}")}),
                    ),
                    None,
                )
                .unwrap()
        })
        .collect()
}

#[test]
fn show_and_all_hidden_copy_paths_share_one_distinct_field_budget() {
    let mut app = App::new();
    unlocked(&mut app);
    let ids = disclosure_budget_notes(&mut app);
    // Two fields of one Note are distinct; different copy formats of one field are not.
    let fields = [
        (&ids[0], "key"),
        (&ids[0], "secret"),
        (&ids[1], "key"),
        (&ids[2], "key"),
        (&ids[3], "key"),
        (&ids[4], "key"),
        (&ids[5], "key"),
        (&ids[6], "key"),
    ];
    for (i, (id, field)) in fields.into_iter().enumerate() {
        match i % 4 {
            0 => {
                app.core.show_field(id, field).unwrap();
                app.core.hide_field();
            }
            1 => app.core.copy_field(id, field, None).unwrap(),
            2 => app.core.copy_field_line(id, field, 0).unwrap(),
            _ => app.core.copy_field_env(id).unwrap(),
        }
        assert_eq!(app.core.app_status().reveal, None);
    }
    // Repeated use works at the limit across every representation of the same field.
    app.core.copy_field(&ids[0], "key", None).unwrap();
    app.core.copy_field_line(&ids[0], "key", 0).unwrap();
    app.core.copy_field_env(&ids[0]).unwrap();
    app.core.show_field(&ids[0], "key").unwrap();
    let shown = app.core.app_status().reveal;
    app.core.copy_field(&ids[0], "secret", None).unwrap();
    app.core.copy_field_line(&ids[0], "secret", 1).unwrap();
    assert_eq!(app.core.app_status().reveal, shown);
    app.core.hide_field();
    // Visible metadata neither uses a slot nor is blocked by the Hidden Field budget.
    for id in &ids {
        app.core.copy_visible_field(id, "service").unwrap();
    }
    let before = clipboard_changes(&app);
    let copies = app.core.app_status().copies;
    assert_eq!(code(app.core.show_field(&ids[7], "key")), "too_many");
    assert_eq!(code(app.core.copy_field(&ids[7], "key", None)), "too_many");
    assert_eq!(
        code(app.core.copy_field_line(&ids[7], "key", 0)),
        "too_many"
    );
    assert_eq!(code(app.core.copy_field_env(&ids[7])), "too_many");
    assert_eq!(clipboard_changes(&app), before);
    assert_eq!(app.core.app_status().copies, copies);
    assert_eq!(app.core.app_status().reveal, None);
    // The exact expiry boundary frees a slot, and copying still starts no reveal.
    app.advance(session::REVEAL_WINDOW_MS - 1);
    assert_eq!(code(app.core.copy_field_env(&ids[7])), "too_many");
    app.advance(1);
    app.core.copy_field_env(&ids[7]).unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("FAKE_KEY_7=fake-key-7"));
    assert_eq!(app.core.app_status().reveal, None);
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
    app.advance(30_000);
    app.core.tick();
    assert!(nothing_copied(&app));
}

#[test]
fn invalid_or_unauthorized_disclosures_do_not_use_the_hidden_field_budget() {
    let mut app = App::new();
    unlocked(&mut app);
    let ids = disclosure_budget_notes(&mut app);
    let n = sample_notes(&mut app);
    for id in &ids {
        assert_eq!(code(app.core.show_field(id, "service")), "invalid_input");
        assert_eq!(
            code(app.core.copy_field(id, "password", None)),
            "invalid_input"
        );
        assert_eq!(
            code(app.core.copy_field_line(id, "key", usize::MAX)),
            "not_found"
        );
    }
    assert_eq!(
        code(app.core.copy_field(&n.api, "secret", None)),
        "not_found"
    );
    assert_eq!(code(app.core.show_field(&n.api, "secret")), "not_found");
    assert_eq!(
        code(app.core.copy_field_line(&n.text, "body", 2)),
        "not_found"
    );
    assert_eq!(
        code(app.core.copy_field(&"0".repeat(32), "key", None)),
        "not_found"
    );
    let unquotable = app
        .core
        .create_note(
            input(
                "api_key",
                "Fake unquotable",
                json!({"env_var": "FAKE_KEY"}),
                json!({"key": "fake'key"}),
            ),
            None,
        )
        .unwrap();
    assert_eq!(code(app.core.copy_field_env(&unquotable)), "invalid_input");
    let unnamed = app
        .core
        .create_note(
            input(
                "api_key",
                "Fake unnamed",
                json!({}),
                json!({"key": "fake-key"}),
            ),
            None,
        )
        .unwrap();
    assert_eq!(code(app.core.copy_field_env(&unnamed)), "invalid_input");
    assert_eq!(
        code(app.core.copy_field(&n.pk, "key", None)),
        "invalid_input"
    );
    assert_eq!(
        code(app.core.copy_field(&n.pk, "key", Some(pw(WRONG_PASSWORD)))),
        "wrong_credential"
    );
    assert_eq!(
        code(
            app.core
                .show_wallet_field(&n.pk, "key", pw(WRONG_PASSWORD), ShowMode::Held)
        ),
        "wrong_credential"
    );
    assert_eq!(
        code(
            app.core
                .show_and_copy_wallet_field(&n.pk, "key", pw(WRONG_PASSWORD))
        ),
        "wrong_credential"
    );
    // Lock and Sharing Guard refusals also leave the allowance untouched.
    app.core.lock();
    assert_eq!(code(app.core.show_field(&ids[0], "key")), "locked");
    assert_eq!(code(app.core.copy_field(&ids[0], "key", None)), "locked");
    assert_eq!(code(app.core.copy_field_line(&ids[0], "key", 0)), "locked");
    assert_eq!(code(app.core.copy_field_env(&ids[0])), "locked");
    app.advance(1_000);
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.guard_update(&zoom());
    assert_eq!(
        code(app.core.show_field(&ids[0], "key")),
        "sharing_guard_up"
    );
    assert_eq!(
        code(app.core.copy_field(&ids[0], "key", None)),
        "sharing_guard_up"
    );
    assert_eq!(
        code(app.core.copy_field_line(&ids[0], "key", 0)),
        "sharing_guard_up"
    );
    assert_eq!(code(app.core.copy_field_env(&ids[0])), "sharing_guard_up");
    app.core.guard_update(&[]);
    assert!(nothing_copied(&app));
    assert_eq!(app.core.app_status().reveal, None);
    // All eight real admissions are still available after the rejected requests.
    for id in &ids[..8] {
        app.core.copy_field(id, "key", None).unwrap();
    }
    assert_eq!(code(app.core.copy_field(&ids[8], "key", None)), "too_many");
    app.core.lock();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(code(app.core.copy_field(&ids[8], "key", None)), "too_many");
}

#[test]
fn admitted_clipboard_failures_use_slots_but_never_show_or_count_a_copy() {
    let mut app = App::new();
    unlocked(&mut app);
    let ids = disclosure_budget_notes(&mut app);
    let n = sample_notes(&mut app);
    // Each path fails twice on the same field: five distinct admissions, not ten.
    app.board.lock().unwrap().write_failures = 10;
    for _ in 0..2 {
        assert_eq!(code(app.core.copy_field(&ids[0], "key", None)), "io");
        assert_eq!(code(app.core.copy_field_line(&ids[1], "key", 0)), "io");
        assert_eq!(code(app.core.copy_field_env(&ids[2])), "io");
        assert_eq!(
            code(app.core.copy_field(&n.pk, "key", Some(pw(PASSWORD)))),
            "io"
        );
        assert_eq!(
            code(
                app.core
                    .show_and_copy_wallet_field(&n.seed, "words", pw(PASSWORD))
            ),
            "io"
        );
        assert_eq!(app.core.app_status().reveal, None);
        assert_eq!(app.core.app_status().copies, 0);
        assert!(nothing_copied(&app));
    }
    for id in &ids[3..6] {
        app.core.show_field(id, "key").unwrap();
        app.core.hide_field();
    }
    let before = clipboard_changes(&app);
    assert_eq!(code(app.core.copy_field_env(&ids[6])), "too_many");
    assert_eq!(clipboard_changes(&app), before);
    // A retry of an admitted field remains allowed, but still needs Wallet authorization.
    assert_eq!(
        code(app.core.copy_field(&n.pk, "key", None)),
        "invalid_input"
    );
    app.core
        .copy_field(&n.pk, "key", Some(pw(PASSWORD)))
        .unwrap();
    assert_eq!(app.core.app_status().copies, 1);
    assert_eq!(app.core.app_status().reveal, None);
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
    assert_eq!(code(app.core.show_field(&ids[6], "key")), "too_many");
}

#[test]
fn a_visible_field_is_copied_and_cleared_like_a_hidden_one() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);

    app.core.copy_visible_field(&n.login, "username").unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("fake-user"));
    assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
    app.advance(29_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-user"));
    app.advance(1);
    app.core.tick();
    assert!(nothing_copied(&app));

    // A Wallet Kind's visible parts are readable while Unlocked, so copying one takes no
    // Master Password.
    app.core.copy_visible_field(&n.pk, "address").unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("0xfake-address"));
    app.core.copy_visible_field(&n.seed, "wallet_name").unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("Fake wallet"));
    app.core.copy_visible_field(&n.api, "service").unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("svc.example.test"));
    // A Note in Trash, as with a Hidden Field.
    app.core.trash_note(&n.login, None).unwrap();
    app.core.copy_visible_field(&n.login, "website").unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("site.example.test"));

    let before = clipboard_changes(&app);
    // A Hidden Field is not copied this way, whatever its Kind.
    for (id, field) in [
        (&n.login, "password"),
        (&n.api, "key"),
        (&n.api, "secret"),
        (&n.text, "body"),
        (&n.pk, "key"),
        (&n.seed, "words"),
        (&n.seed, "passphrase"),
    ] {
        let refused = app.core.copy_visible_field(id, field).unwrap_err();
        assert_eq!(refused.code, "invalid_input", "{field}");
        assert!(
            refused.message.contains("always visible"),
            "{}",
            refused.message
        );
    }
    // Nor the word count, another Kind's field, the title, or a name that is none at all.
    for (id, field) in [
        (&n.seed, "word_count"),
        (&n.login, "service"),
        (&n.text, "website"),
        (&n.login, "title"),
        (&n.login, ""),
    ] {
        assert_eq!(
            code(app.core.copy_visible_field(id, field)),
            "invalid_input",
            "{field:?}"
        );
    }
    // A visible field with nothing in it gets the same answer as an empty Hidden Field.
    let refused = app.core.copy_visible_field(&n.api, "url").unwrap_err();
    assert_eq!(
        (refused.code, refused.message.as_str()),
        ("not_found", "That field is empty.")
    );
    assert_eq!(
        code(app.core.copy_field(&n.api, "secret", None)),
        "not_found"
    );
    let no_such_note = "0".repeat(32);
    assert_eq!(
        code(app.core.copy_visible_field(&no_such_note, "username")),
        "not_found"
    );
    // Refused while the Sharing Guard is up.
    app.core.guard_update(&zoom());
    assert_eq!(
        code(app.core.copy_visible_field(&n.pk, "address")),
        "sharing_guard_up"
    );
    app.core.guard_update(&[]);
    // None of the refusals touched the clipboard.
    assert_eq!(clipboard_changes(&app), before);
    assert_eq!(app.clipboard().as_deref(), Some("site.example.test"));

    // Refused while Locked.
    app.core.lock();
    assert_eq!(
        code(app.core.copy_visible_field(&n.pk, "address")),
        "locked"
    );
    assert!(nothing_copied(&app));
}

#[test]
fn one_line_of_a_hidden_field_is_copied_without_its_line_ending() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);

    // Lines are counted from 0 over every line, the empty one included.
    for (line, expected) in [
        (0, "fake first line"),
        (1, "fake second line"),
        (3, "fake fourth line"),
    ] {
        app.core.copy_field_line(&n.text, "body", line).unwrap();
        assert_eq!(app.clipboard().as_deref(), Some(expected), "line {line}");
        assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
    }
    // Line 0 of a value with one line is the whole value.
    app.core.copy_field_line(&n.login, "password", 0).unwrap();
    assert_eq!(app.clipboard().as_deref(), Some("fake-login-pass"));
    // The lines go to the clipboard and nowhere else.
    let status = serde_json::to_string(&app.core.app_status()).unwrap();
    assert!(!status.contains("fake first line") && !status.contains("fake-login-pass"));
    app.advance(29_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-login-pass"));
    app.advance(1);
    app.core.tick();
    assert!(nothing_copied(&app));

    // An empty line, and the empty piece after the last line ending.
    for line in [2, 4] {
        let refused = app.core.copy_field_line(&n.text, "body", line).unwrap_err();
        assert_eq!(
            (refused.code, refused.message.as_str()),
            ("not_found", "That line is empty."),
            "line {line}"
        );
    }
    // Past the end.
    for line in [5, 6, usize::MAX] {
        let refused = app.core.copy_field_line(&n.text, "body", line).unwrap_err();
        assert_eq!(refused.code, "not_found", "line {line}");
        assert!(
            refused.message.contains("isn't there"),
            "{}",
            refused.message
        );
    }
    assert_eq!(
        code(app.core.copy_field_line(&n.login, "password", 1)),
        "not_found"
    );
    // Wallet Kinds are copied whole and with the Master Password, never by the line.
    for (id, field) in [(&n.pk, "key"), (&n.seed, "words"), (&n.seed, "passphrase")] {
        let refused = app.core.copy_field_line(id, field, 0).unwrap_err();
        assert_eq!(refused.code, "invalid_input", "{field}");
        assert!(
            refused.message.contains("copied whole"),
            "{}",
            refused.message
        );
    }
    // A field the Kind doesn't have, a visible one, and a Hidden Field with no value.
    assert_eq!(
        code(app.core.copy_field_line(&n.text, "password", 0)),
        "invalid_input"
    );
    assert_eq!(
        code(app.core.copy_field_line(&n.login, "username", 0)),
        "invalid_input"
    );
    assert_eq!(
        code(app.core.copy_field_line(&n.api, "secret", 0)),
        "not_found"
    );
    assert!(nothing_copied(&app));

    app.core.guard_update(&zoom());
    assert_eq!(
        code(app.core.copy_field_line(&n.text, "body", 0)),
        "sharing_guard_up"
    );
    app.core.guard_update(&[]);
    assert!(nothing_copied(&app));
    app.core.lock();
    assert_eq!(code(app.core.copy_field_line(&n.text, "body", 0)), "locked");
    assert!(nothing_copied(&app));
}

#[test]
fn an_api_key_is_copied_as_a_name_equals_value_line() {
    let mut app = App::new();
    unlocked(&mut app);
    let api_key = |app: &mut App, visible: Value, hidden: Value| {
        app.core
            .create_note(input("api_key", "Fake API key", visible, hidden), None)
            .unwrap()
    };
    let named = json!({"service": "svc.example.test", "env_var": "FAKE_DEPLOY_KEY"});

    // (the key, the line that is copied)
    let copied = [
        // Only letters, digits and _ @ % + = : , . / - : written as it is.
        ("fake-key_0000", "FAKE_DEPLOY_KEY=fake-key_0000"),
        ("Fake09_@%+=:,./-", "FAKE_DEPLOY_KEY=Fake09_@%+=:,./-"),
        // Anything else on one line: inside single quotes.
        (
            "fake key with spaces",
            "FAKE_DEPLOY_KEY='fake key with spaces'",
        ),
        (
            "fake$HOME;`x`\"y\"\\z*",
            "FAKE_DEPLOY_KEY='fake$HOME;`x`\"y\"\\z*'",
        ),
        ("fake#key", "FAKE_DEPLOY_KEY='fake#key'"),
        ("fake~key!", "FAKE_DEPLOY_KEY='fake~key!'"),
        ("fake\ttab", "FAKE_DEPLOY_KEY='fake\ttab'"),
        ("fak\u{e9}-key", "FAKE_DEPLOY_KEY='fak\u{e9}-key'"),
    ];
    for (key, line) in copied {
        let id = api_key(&mut app, named.clone(), json!({ "key": key }));
        app.core.copy_field_env(&id).unwrap();
        assert_eq!(app.clipboard().as_deref(), Some(line));
        assert_eq!(app.core.app_status().clipboard_clear_in_ms, Some(30_000));
    }
    app.advance(30_000);
    app.core.tick();
    assert!(nothing_copied(&app));

    // The next copies belong to a new disclosure window.
    app.advance(30_000);

    // A key with a single quote or a line break can't be written on one quoted line.
    for key in [
        "fake'key",
        "fake\nkey",
        "fake line one\r\nfake line two\r\n",
        "fake\u{2028}key",
    ] {
        let id = api_key(&mut app, named.clone(), json!({ "key": key }));
        let refused = app.core.copy_field_env(&id).unwrap_err();
        assert_eq!(refused.code, "invalid_input", "{key:?}");
        assert!(
            refused.message.contains("NAME=value"),
            "{}",
            refused.message
        );
        assert!(!refused.message.contains(key), "{}", refused.message);
        assert!(!refused.message.contains("fake"), "{}", refused.message);
        // The key by itself can still be copied.
        app.core.copy_field(&id, "key", None).unwrap();
        assert_eq!(app.clipboard().as_deref(), Some(key));
        app.core.lock();
        app.core.unlock(password(PASSWORD)).unwrap();
        assert!(nothing_copied(&app));
    }

    // No variable name, no key, another Kind, no such Note.
    let unnamed = api_key(
        &mut app,
        json!({"service": "svc.example.test"}),
        json!({"key": "fake-key_0000"}),
    );
    let refused = app.core.copy_field_env(&unnamed).unwrap_err();
    assert_eq!(refused.code, "invalid_input");
    assert!(
        refused.message.contains("variable name"),
        "{}",
        refused.message
    );
    let no_key = api_key(&mut app, named.clone(), json!({"secret": "fake-secret"}));
    let refused = app.core.copy_field_env(&no_key).unwrap_err();
    assert_eq!(
        (refused.code, refused.message.as_str()),
        ("not_found", "That field is empty.")
    );
    let login = app
        .core
        .create_note(
            input("login", "Fake", json!({}), json!({"password": "fake-pass"})),
            None,
        )
        .unwrap();
    assert_eq!(code(app.core.copy_field_env(&login)), "invalid_input");
    assert_eq!(code(app.core.copy_field_env(&"0".repeat(32))), "not_found");
    assert!(nothing_copied(&app));

    let id = api_key(&mut app, named, json!({"key": "fake-key_0000"}));
    app.core.guard_update(&zoom());
    assert_eq!(code(app.core.copy_field_env(&id)), "sharing_guard_up");
    app.core.guard_update(&[]);
    assert!(nothing_copied(&app));
    app.core.lock();
    assert_eq!(code(app.core.copy_field_env(&id)), "locked");
    assert!(nothing_copied(&app));
}

#[test]
fn text_is_added_to_the_end_of_a_text_note_without_showing_it() {
    let mut app = App::new();
    unlocked(&mut app);
    let text = app
        .core
        .create_note(
            input(
                "text",
                "Fake log",
                json!({}),
                json!({"body": "fake line one"}),
            ),
            None,
        )
        .unwrap();
    let empty = app
        .core
        .create_note(input("text", "Fake empty", json!({}), json!({})), None)
        .unwrap();
    let login = app
        .core
        .create_note(
            input("login", "Fake", json!({}), json!({"password": "fake-pass"})),
            None,
        )
        .unwrap();
    let backup = app.temp.path().join("fake-backup.enote");
    app.core.set_backup_destination(backup).unwrap();
    app.core.write_backup().unwrap();
    let created = app.core.get_note(&text).unwrap().updated_at;

    app.advance(5_000);
    app.core
        .append_text(&text, pw("fake line two\nfake line three"))
        .unwrap();
    app.core.append_text(&empty, pw("fake only line")).unwrap();
    // Nothing was shown or copied to do it, and it was saved as a change.
    let status = app.core.app_status();
    assert_eq!(status.reveal, None);
    assert!(nothing_copied(&app));
    assert!(status.backup.unbacked_changes);
    assert!(app.core.get_note(&text).unwrap().updated_at > created);

    // Another Kind has no body to add to; an empty addition is nothing to add.
    assert_eq!(
        code(app.core.append_text(&login, pw("fake line"))),
        "invalid_input"
    );
    let refused = app.core.append_text(&text, pw("")).unwrap_err();
    assert_eq!(
        (refused.code, refused.message.as_str()),
        ("invalid_input", "Type the text to add first.")
    );
    assert_eq!(
        code(app.core.append_text(&"0".repeat(32), pw("fake line"))),
        "not_found"
    );

    let mut app = app.restart();
    assert_eq!(code(app.core.append_text(&text, pw("fake line"))), "locked");
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(
        app.core.show_field(&text, "body").unwrap().as_str(),
        "fake line one\nfake line two\nfake line three"
    );
    assert_eq!(
        app.core.show_field(&empty, "body").unwrap().as_str(),
        "fake only line"
    );
    assert_eq!(
        app.core.show_field(&login, "password").unwrap().as_str(),
        "fake-pass"
    );
}

#[test]
fn a_wallet_kind_value_is_shown_and_copied_with_one_master_password_entry() {
    let mut app = App::new();
    unlocked(&mut app);
    // The longer clipboard time is chosen; a Wallet Kind copy still gets 30 s.
    app.core
        .set_settings(settings(json!({
            "idle_minutes": 5,
            "lock_on_app_switch": false,
            "clipboard_clear_seconds": 60,
        })))
        .unwrap();
    let n = sample_notes(&mut app);

    let shown = app
        .core
        .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        .unwrap();
    assert_eq!(shown.as_str(), "fake-private-key");
    assert_eq!(app.clipboard().as_deref(), Some("fake-private-key"));
    let status = app.core.app_status();
    assert_eq!(status.clipboard_clear_in_ms, Some(30_000));
    assert_eq!(
        serde_json::to_value(status.reveal).unwrap(),
        json!({
            "note_id": n.pk,
            "field": "key",
            "expires_in_ms": 20_000,
            "can_extend": true,
            "extensions_left": 3,
        })
    );
    // A timed reveal, never a held one: it ends by itself after 20 s, the copy after 30.
    app.advance(19_999);
    app.core.tick();
    assert!(app.core.app_status().reveal.is_some());
    app.advance(1);
    app.core.tick();
    assert_eq!(app.core.app_status().reveal, None);
    assert_eq!(app.clipboard().as_deref(), Some("fake-private-key"));
    app.advance(10_000);
    app.core.tick();
    assert!(nothing_copied(&app));

    // The Wallet Key was used for that one action and dropped: the next one asks again.
    assert_eq!(
        code(app.core.copy_field(&n.pk, "key", None)),
        "invalid_input"
    );
    assert!(nothing_copied(&app));

    // A wrong password neither shows nor copies, and is one wrong attempt, not two.
    let refused = app
        .core
        .show_and_copy_wallet_field(&n.pk, "key", pw(WRONG_PASSWORD))
        .unwrap_err();
    assert_eq!(refused.code, "wrong_credential");
    let status = app.core.app_status();
    assert_eq!(status.reveal, None);
    assert_eq!(status.failed_attempts, 1);
    assert!(nothing_copied(&app));

    // Only Wallet Kinds, and only their Hidden Fields.
    assert_eq!(
        code(
            app.core
                .show_and_copy_wallet_field(&n.text, "body", pw(PASSWORD))
        ),
        "invalid_input"
    );
    assert_eq!(
        code(
            app.core
                .show_and_copy_wallet_field(&n.pk, "address", pw(PASSWORD))
        ),
        "invalid_input"
    );
    assert_eq!(app.core.app_status().reveal, None);
    assert!(nothing_copied(&app));

    // The Sharing Guard blocks it like any Show or Copy.
    app.core.guard_update(&zoom());
    assert_eq!(
        code(
            app.core
                .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        ),
        "sharing_guard_up"
    );
    app.core.guard_update(&[]);
    assert_eq!(app.core.app_status().reveal, None);
    assert!(nothing_copied(&app));

    // A Seed Phrase the same way; then Locked.
    let words = app
        .core
        .show_and_copy_wallet_field(&n.seed, "words", pw(PASSWORD))
        .unwrap();
    assert_eq!(app.clipboard().as_deref(), Some(words.as_str()));
    app.core.lock();
    assert!(nothing_copied(&app));
    assert_eq!(
        code(
            app.core
                .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        ),
        "locked"
    );
}

#[test]
fn a_refused_reveal_leaves_nothing_on_the_clipboard() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);
    let logins: Vec<String> = (0..8)
        .map(|i| {
            let note = input(
                "login",
                &format!("Fake {i}"),
                json!({}),
                json!({"password": format!("fake-value-{i}")}),
            );
            app.core.create_note(note, None).unwrap()
        })
        .collect();

    // Seven different Hidden Fields are shown; the Private Key is the eighth, and counts.
    for id in &logins[..7] {
        app.core.show_field(id, "password").unwrap();
        app.advance(1_000);
    }
    app.core
        .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        .unwrap();
    assert_eq!(
        code(app.core.show_field(&logins[7], "password")),
        "too_many"
    );
    // Showing the same one again costs nothing.
    app.core
        .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        .unwrap();

    // Locking empties the clipboard but hands out no new allowance: a ninth field is refused,
    // and with the reveal refused nothing is copied either.
    app.core.lock();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(nothing_copied(&app));
    let refused = app
        .core
        .show_and_copy_wallet_field(&n.seed, "words", pw(PASSWORD))
        .unwrap_err();
    assert_eq!(refused.code, "too_many");
    assert_eq!(app.core.app_status().reveal, None);
    assert!(nothing_copied(&app));

    // Once the minute has passed it is shown and copied.
    app.advance(60_000);
    app.core.activity();
    app.core
        .show_and_copy_wallet_field(&n.seed, "words", pw(PASSWORD))
        .unwrap();
    assert_eq!(
        app.clipboard().as_deref(),
        Some(["zoo"; 12].join(" ").as_str())
    );
}

#[test]
fn a_wallet_kind_value_is_not_handed_over_when_the_clipboard_refuses_the_copy() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);

    app.board.lock().unwrap().write_failures = 1;
    let refused = app
        .core
        .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        .unwrap_err();
    assert_eq!(refused.code, "io");
    assert!(!refused.message.contains("fake-private-key"));
    // Neither half happened: nothing is shown, nothing is copied, no clear is waiting.
    let status = app.core.app_status();
    assert_eq!(status.reveal, None);
    assert!(nothing_copied(&app));
    assert_eq!(code(app.core.extend_reveal()), "not_found");
    // The Master Password was right, so this is not a wrong attempt, and the next try works.
    assert_eq!(status.failed_attempts, 0);
    let shown = app
        .core
        .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        .unwrap();
    assert_eq!(shown.as_str(), "fake-private-key");
    assert_eq!(app.clipboard().as_deref(), Some("fake-private-key"));
    assert!(app.core.app_status().reveal.is_some());
}

// ---------- key convenience: more time, settings, previews ----------

#[test]
fn a_shown_value_can_be_given_more_time() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);

    // Nothing is shown yet.
    assert_eq!(code(app.core.extend_reveal()), "not_found");

    // An ordinary Kind: another 30 s from the moment of asking, as often as asked.
    app.core.show_field(&n.login, "password").unwrap();
    assert_eq!(
        serde_json::to_value(app.core.app_status().reveal).unwrap(),
        json!({
            "note_id": n.login,
            "field": "password",
            "expires_in_ms": 30_000,
            "can_extend": true,
            "extensions_left": null,
        })
    );
    for _ in 0..6 {
        app.advance(25_000);
        app.core.activity();
        app.core.extend_reveal().unwrap();
        let reveal = app.core.app_status().reveal.unwrap();
        assert_eq!(
            (reveal.expires_in_ms, reveal.can_extend),
            (Some(30_000), true)
        );
    }
    app.advance(29_999);
    app.core.tick();
    assert!(app.core.app_status().reveal.is_some());
    app.advance(1);
    app.core.tick();
    assert_eq!(app.core.app_status().reveal, None);
    // Once it has ended, asking for more time does not bring it back.
    let refused = app.core.extend_reveal().unwrap_err();
    assert_eq!(refused.code, "not_found");
    assert_eq!(app.core.app_status().reveal, None);

    // A Wallet Kind value shown for 20 s: another 20 s, three times.
    app.core.activity();
    app.core
        .show_wallet_field(&n.pk, "key", pw(PASSWORD), ShowMode::Accessible)
        .unwrap();
    for left in [2, 1, 0] {
        app.advance(15_000);
        app.core.extend_reveal().unwrap();
        let reveal = app.core.app_status().reveal.unwrap();
        assert_eq!(
            (
                reveal.expires_in_ms,
                reveal.extensions_left,
                reveal.can_extend
            ),
            (Some(20_000), Some(left), left > 0)
        );
    }
    app.advance(15_000);
    let refused = app.core.extend_reveal().unwrap_err();
    assert_eq!(refused.code, "invalid_input");
    assert!(
        refused.message.contains("Master Password"),
        "{}",
        refused.message
    );
    // It still ends 20 s after the third time.
    assert_eq!(
        app.core.app_status().reveal.unwrap().expires_in_ms,
        Some(5_000)
    );
    app.advance(5_000);
    app.core.tick();
    assert_eq!(app.core.app_status().reveal, None);
    // Shown again with the Master Password, it can be given more time again.
    app.core
        .show_and_copy_wallet_field(&n.pk, "key", pw(PASSWORD))
        .unwrap();
    assert_eq!(
        app.core.app_status().reveal.unwrap().extensions_left,
        Some(3)
    );
    app.core.extend_reveal().unwrap();
    app.core.hide_field();
    assert_eq!(code(app.core.extend_reveal()), "not_found");

    // A value shown while held has no timer to extend, and stays as it was.
    app.core
        .show_wallet_field(&n.pk, "key", pw(PASSWORD), ShowMode::Held)
        .unwrap();
    assert!(!app.core.app_status().reveal.unwrap().can_extend);
    assert_eq!(code(app.core.extend_reveal()), "invalid_input");
    assert_eq!(app.core.app_status().reveal.unwrap().expires_in_ms, None);

    // Hidden by the Sharing Guard, or by a lock: not brought back.
    app.core.guard_update(&zoom());
    assert_eq!(code(app.core.extend_reveal()), "sharing_guard_up");
    app.core.guard_update(&[]);
    assert_eq!(code(app.core.extend_reveal()), "not_found");
    app.core.show_field(&n.login, "password").unwrap();
    app.core.lock();
    assert_eq!(code(app.core.extend_reveal()), "locked");
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(code(app.core.extend_reveal()), "not_found");
    assert_eq!(app.core.app_status().reveal, None);
}

#[test]
fn copies_stay_for_the_chosen_30_or_60_seconds_and_wallet_kind_values_always_for_30() {
    let mut app = App::new();
    unlocked(&mut app);
    let n = sample_notes(&mut app);
    let clear_in = |app: &App| app.core.app_status().clipboard_clear_in_ms;
    let chosen = |app: &App| app.core.app_status().settings.clipboard_clear_seconds;
    assert_eq!(chosen(&app), 30);

    app.core
        .set_settings(settings(json!({
            "idle_minutes": 5,
            "lock_on_app_switch": false,
            "clipboard_clear_seconds": 60,
        })))
        .unwrap();
    assert_eq!(chosen(&app), 60);

    // Every ordinary copy: a Hidden Field, a visible field, one line, NAME=value.
    app.core.copy_field(&n.login, "password", None).unwrap();
    assert_eq!(clear_in(&app), Some(60_000));
    app.advance(59_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-login-pass"));
    app.advance(1);
    app.core.tick();
    assert!(nothing_copied(&app));
    app.core.copy_visible_field(&n.login, "username").unwrap();
    assert_eq!(clear_in(&app), Some(60_000));
    app.core.copy_field_line(&n.text, "body", 1).unwrap();
    assert_eq!(clear_in(&app), Some(60_000));
    app.core.copy_field_env(&n.api).unwrap();
    assert_eq!(clear_in(&app), Some(60_000));
    // A Wallet Kind's visible field is an ordinary copy too.
    app.core.copy_visible_field(&n.pk, "address").unwrap();
    assert_eq!(clear_in(&app), Some(60_000));

    // A Wallet Kind value: 30 s, by either way of copying it.
    app.core
        .copy_field(&n.pk, "key", Some(pw(PASSWORD)))
        .unwrap();
    assert_eq!(clear_in(&app), Some(30_000));
    app.advance(29_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-private-key"));
    app.advance(1);
    app.core.tick();
    assert!(nothing_copied(&app));
    app.core
        .show_and_copy_wallet_field(&n.seed, "passphrase", pw(PASSWORD))
        .unwrap();
    assert_eq!(clear_in(&app), Some(30_000));
    app.core.hide_field();

    // The lock rule sent alone, as the Settings screen has always sent it, keeps the choice.
    app.core
        .set_settings(settings(
            json!({"idle_minutes": 10, "lock_on_app_switch": false}),
        ))
        .unwrap();
    let view = app.core.app_status().settings;
    assert_eq!((view.idle_minutes, view.clipboard_clear_seconds), (10, 60));

    // The choice survives a restart.
    app.core.lock();
    let mut app = app.restart();
    assert_eq!(chosen(&app), 60);
    app.core.unlock(password(PASSWORD)).unwrap();

    // Anything that is not one of the two choices becomes 30.
    for other in [0, 45, 61, 3_600] {
        app.core
            .set_settings(settings(json!({
                "idle_minutes": 10,
                "lock_on_app_switch": false,
                "clipboard_clear_seconds": 60,
            })))
            .unwrap();
        app.core
            .set_settings(settings(json!({
                "idle_minutes": 10,
                "lock_on_app_switch": false,
                "clipboard_clear_seconds": other,
            })))
            .unwrap();
        assert_eq!(chosen(&app), 30, "{other}");
    }
    app.core.copy_field(&n.login, "password", None).unwrap();
    assert_eq!(clear_in(&app), Some(30_000));
}

#[test]
fn the_status_counts_each_copy_and_a_clear_that_is_tried_again_is_not_one() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .set_settings(settings(json!({
            "idle_minutes": 5,
            "lock_on_app_switch": false,
            "clipboard_clear_seconds": 60,
        })))
        .unwrap();
    let n = sample_notes(&mut app);
    // The copies so far, and the milliseconds until the clipboard is cleared.
    let seen = |app: &App| {
        let status = app.core.app_status();
        (status.copies, status.clipboard_clear_in_ms)
    };
    assert_eq!(seen(&app), (0, None));

    app.core.copy_visible_field(&n.login, "username").unwrap();
    assert_eq!(seen(&app), (1, Some(60_000)));
    // A Wallet Kind value copied while that one waits has less time than is left on it, and
    // is a copy all the same: the time left alone would not say so.
    app.advance(10_000);
    app.core
        .copy_field(&n.pk, "key", Some(pw(PASSWORD)))
        .unwrap();
    assert_eq!(seen(&app), (2, Some(30_000)));
    // So is the same value copied once more in the same moment, with the very same time.
    app.core
        .copy_field(&n.pk, "key", Some(pw(PASSWORD)))
        .unwrap();
    assert_eq!(seen(&app), (3, Some(30_000)));

    // A copy that was refused is not one: no Master Password, or the clipboard said no.
    assert_eq!(
        code(app.core.copy_field(&n.pk, "key", None)),
        "invalid_input"
    );
    app.board.lock().unwrap().write_failures = 1;
    assert_eq!(code(app.core.copy_field(&n.login, "password", None)), "io");
    assert_eq!(seen(&app).0, 3);

    // A clear that has to be tried again has time left once more, and is not a copy.
    app.core.copy_field(&n.login, "password", None).unwrap();
    assert_eq!(seen(&app), (4, Some(60_000)));
    app.board.lock().unwrap().clear_failures = 2;
    app.advance(60_000);
    app.core.tick();
    assert_eq!(seen(&app), (4, Some(250)));
    app.advance(100);
    assert_eq!(seen(&app), (4, Some(150)));
    app.advance(150);
    app.core.tick();
    assert_eq!(seen(&app), (4, Some(250)));
    app.advance(250);
    app.core.tick();
    assert_eq!(seen(&app), (4, None));
    assert!(nothing_copied(&app));

    // The count belongs to this run of the app, not to one unlock: the screens that come back
    // after a lock compare with it.
    app.core.lock();
    app.core.unlock(password(PASSWORD)).unwrap();
    assert_eq!(seen(&app), (4, None));
    app.core.copy_visible_field(&n.login, "username").unwrap();
    assert_eq!(seen(&app), (5, Some(60_000)));
}

#[test]
fn locking_on_app_switch_leaves_a_60_second_copy_long_enough_to_paste_it() {
    let mut app = App::new();
    unlocked(&mut app);
    app.core
        .set_settings(settings(json!({
            "idle_minutes": 5,
            "lock_on_app_switch": true,
            "clipboard_clear_seconds": 60,
        })))
        .unwrap();
    let n = sample_notes(&mut app);

    app.core.copy_field(&n.login, "password", None).unwrap();
    app.advance(1_000);
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(app.phase(), "locked");
    assert_eq!(app.clipboard().as_deref(), Some("fake-login-pass"));
    app.advance(58_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-login-pass"));
    app.advance(1);
    app.core.tick();
    assert!(nothing_copied(&app));

    // A Wallet Kind value keeps its 30 s through the same lock.
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core
        .copy_field(&n.pk, "key", Some(pw(PASSWORD)))
        .unwrap();
    app.advance(1_000);
    app.core.os_event(OsEvent::FocusLost);
    assert_eq!(app.phase(), "locked");
    app.advance(28_999);
    app.core.tick();
    assert_eq!(app.clipboard().as_deref(), Some("fake-private-key"));
    app.advance(1);
    app.core.tick();
    assert!(nothing_copied(&app));

    // Every other lock still clears a copy at once.
    for event in [OsEvent::Sleep, OsEvent::ScreenLocked, OsEvent::UserSwitched] {
        app.core.unlock(password(PASSWORD)).unwrap();
        app.core.copy_field(&n.login, "password", None).unwrap();
        app.core.os_event(event);
        assert_eq!(app.phase(), "locked", "{event:?}");
        assert!(nothing_copied(&app), "{event:?}");
    }
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.copy_visible_field(&n.login, "username").unwrap();
    app.core.lock();
    assert!(nothing_copied(&app));
}

#[test]
fn counting_activity_anywhere_keeps_the_app_unlocked_but_never_past_60_minutes() {
    const MINUTE: u64 = 60_000;
    let mut app = App::new();
    unlocked(&mut app);
    let view = app.core.app_status().settings;
    assert!(!view.count_activity_anywhere && view.count_activity_anywhere_supported);

    // Off unless the owner turns it on: input elsewhere changes nothing.
    assert!(!app.core.counts_activity_anywhere());
    app.advance(4 * MINUTE);
    app.core.activity_elsewhere(0);
    assert_eq!(app.core.app_status().lock_in_ms, Some(MINUTE));
    app.advance(MINUTE);
    app.core.tick();
    assert_eq!(app.phase(), "locked");

    app.core.unlock(password(PASSWORD)).unwrap();
    app.core
        .set_settings(settings(json!({
            "idle_minutes": 5,
            "lock_on_app_switch": false,
            "count_activity_anywhere": true,
        })))
        .unwrap();
    assert!(app.core.app_status().settings.count_activity_anywhere);
    assert!(app.core.counts_activity_anywhere());

    // The owner works in another app for an hour and never touches this one: the OS last saw
    // input a second ago, every time it is asked.
    for half_minutes in 1..120 {
        app.advance(MINUTE / 2);
        app.core.activity_elsewhere(1_000);
        app.core.tick();
        assert_eq!(app.phase(), "unlocked", "after {half_minutes} half minutes");
        // The countdown is the real one: the idle time, or what is left of the hour.
        let left_of_the_hour = 60 * MINUTE - half_minutes * (MINUTE / 2);
        assert_eq!(
            app.core.app_status().lock_in_ms,
            Some((5 * MINUTE - 1_000).min(left_of_the_hour)),
            "after {half_minutes} half minutes"
        );
    }
    // Sixty minutes after the last activity inside the window it locks all the same.
    app.advance(MINUTE / 2);
    app.core.activity_elsewhere(0);
    app.core.tick();
    assert_eq!(app.phase(), "locked");
    assert!(!app.core.counts_activity_anywhere());

    // Activity inside the window is what starts the hour again: touched once, 45 minutes in,
    // the app stays Unlocked past the first hour and locks 60 minutes after that touch.
    app.core.unlock(password(PASSWORD)).unwrap();
    for minutes in 1..=105 {
        app.advance(MINUTE);
        app.core.activity_elsewhere(0);
        if minutes == 45 {
            app.core.activity();
        }
        app.core.tick();
        let expected = if minutes < 105 { "unlocked" } else { "locked" };
        assert_eq!(app.phase(), expected, "after {minutes} minutes");
    }

    // Sleep, screen lock and a user switch lock at once, whatever was just typed elsewhere.
    for event in [OsEvent::Sleep, OsEvent::ScreenLocked, OsEvent::UserSwitched] {
        app.core.unlock(password(PASSWORD)).unwrap();
        app.core.activity_elsewhere(0);
        app.core.os_event(event);
        assert_eq!(app.phase(), "locked", "{event:?}");
    }

    // The choice is kept across a restart, and the old lock-rule-only input keeps it too.
    let mut app = app.restart();
    assert!(app.core.app_status().settings.count_activity_anywhere);
    app.core.unlock(password(PASSWORD)).unwrap();
    app.core.set_settings(lock_rule(5, false)).unwrap();
    assert!(app.core.app_status().settings.count_activity_anywhere);
    app.core.lock();

    // Read on a computer where the OS can't say when input was last seen, the same settings
    // file leaves it off.
    app.activity_anywhere_supported = false;
    let mut app = app.restart();
    let view = app.core.app_status().settings;
    assert!(!view.count_activity_anywhere && !view.count_activity_anywhere_supported);
    app.core.unlock(password(PASSWORD)).unwrap();
    assert!(!app.core.counts_activity_anywhere());
    app.advance(4 * MINUTE);
    app.core.activity_elsewhere(0);
    app.advance(MINUTE);
    app.core.tick();
    assert_eq!(app.phase(), "locked");
}

#[test]
fn activity_anywhere_cannot_be_turned_on_where_the_computer_does_not_support_it() {
    let mut app = App::on_a_computer(false);
    unlocked(&mut app);
    app.core
        .set_settings(settings(json!({
            "idle_minutes": 5,
            "lock_on_app_switch": false,
            "count_activity_anywhere": true,
        })))
        .unwrap();
    let view = app.core.app_status().settings;
    assert!(!view.count_activity_anywhere && !view.count_activity_anywhere_supported);
    assert!(!app.core.counts_activity_anywhere());
    // Nor did it get into the settings file.
    let mut app = app.restart();
    app.activity_anywhere_supported = true;
    let app = app.restart();
    assert!(!app.core.app_status().settings.count_activity_anywhere);
}

#[test]
fn a_preview_says_what_this_computer_has_that_the_copy_lacks() {
    let mut app = App::new();
    unlocked(&mut app);
    let note = |app: &mut App, title: &str| {
        app.core
            .create_note(input("text", title, json!({}), json!({})), None)
            .unwrap()
    };
    note(&mut app, "Fake kept");
    let trashed_later = note(&mut app, "Fake trashed later");
    let restored_later = note(&mut app, "Fake restored later");
    app.core.trash_note(&restored_later, None).unwrap();
    let backup = app.temp.path().join("fake-earlier.enote");
    app.core.set_backup_destination(backup.clone()).unwrap();
    app.core.write_backup().unwrap();

    // Since that Backup: two new Notes, one Note moved to Trash, one brought back from it.
    app.advance(5_000);
    note(&mut app, "Fake new one");
    note(&mut app, "Fake new two");
    app.core.trash_note(&trashed_later, None).unwrap();
    app.core.restore_note(&restored_later).unwrap();
    let changed_at = app.core.unix_secs();
    app.advance(5_000);

    app.core.set_backup_to_open(backup.clone()).unwrap();
    let preview = app.core.inspect_backup(password(PASSWORD)).unwrap();
    assert_eq!(
        (preview.note_count, preview.older_than_current),
        (2, Some(true))
    );
    // Here, not in Trash: the kept one, the restored one and the two new ones.
    assert_eq!(preview.current_note_count, Some(4));
    assert_eq!(preview.current_changed_at, Some(changed_at));
    // Only the two new Notes are missing from the copy. The restored one is in the copy's
    // Trash, and the one in Trash here is not counted.
    assert_eq!(preview.only_here, Some(2));
    // Numbers and dates only: no titles.
    let shape = serde_json::to_value(&preview).unwrap();
    let mut keys: Vec<&str> = shape
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "changed_at",
            "current_changed_at",
            "current_note_count",
            "note_count",
            "older_than_current",
            "only_here",
            "replaces_vault",
        ]
    );
    assert!(!shape.to_string().contains("Fake"));
    app.core.discard_inspection();

    // Safety Copies of this Vault are compared the same way. The newest is the Vault as it
    // was before the last change (the restore): it holds every Note that is here.
    let copies = app.core.list_safety_copies().unwrap();
    let preview = app
        .core
        .inspect_safety_copy(&copies[0].id, password(PASSWORD))
        .unwrap();
    assert_eq!(
        (
            preview.note_count,
            preview.current_note_count,
            preview.only_here
        ),
        (3, Some(4), Some(0))
    );
    // The oldest is the Vault before its first Note.
    let oldest = &copies[copies.len() - 1];
    let preview = app
        .core
        .inspect_safety_copy(&oldest.id, password(PASSWORD))
        .unwrap();
    assert_eq!(
        (
            preview.note_count,
            preview.current_note_count,
            preview.current_changed_at,
            preview.only_here
        ),
        (0, Some(4), Some(changed_at), Some(4))
    );
    app.core.discard_inspection();

    // A copy of another Vault: its Notes are not compared with the ones here.
    let (_source, foreign) = a_backup();
    app.core.set_backup_to_open(foreign.clone()).unwrap();
    let preview = app.core.inspect_backup(password(PASSWORD)).unwrap();
    assert!(preview.replaces_vault);
    assert_eq!(
        (
            preview.current_note_count,
            preview.current_changed_at,
            preview.only_here
        ),
        (None, None, None)
    );

    // While the Vault here is Locked, nothing about it is told, although the same Master
    // Password lets the app say that the Backup is older.
    app.core.lock();
    app.core.set_backup_to_open(backup).unwrap();
    let preview = app.core.inspect_backup(password(PASSWORD)).unwrap();
    assert_eq!(preview.older_than_current, Some(true));
    assert_eq!(
        (
            preview.current_note_count,
            preview.current_changed_at,
            preview.only_here
        ),
        (None, None, None)
    );

    // On a computer with no Vault there is nothing to compare.
    let mut fresh = App::new();
    fresh.core.set_backup_to_open(foreign).unwrap();
    let preview = fresh.core.inspect_backup(password(PASSWORD)).unwrap();
    assert_eq!(
        serde_json::to_value(&preview).unwrap()["only_here"],
        Value::Null
    );
    assert_eq!(
        (preview.current_note_count, preview.current_changed_at),
        (None, None)
    );
}

#[test]
fn the_vault_location_can_be_read_in_every_phase() {
    let mut app = App::new();
    let folder = app.data_dir().display().to_string();
    assert!(folder.ends_with("Vault.noindex"));
    assert_eq!(app.phase(), "no_vault");
    assert_eq!(app.core.vault_location(), folder);

    let key = app
        .core
        .create_vault(pw(PASSWORD), KdfParams::TESTING)
        .unwrap()
        .recovery_key
        .unwrap();
    assert_eq!(app.phase(), "confirm_recovery_kit");
    assert_eq!(app.core.vault_location(), folder);
    app.core.confirm_recovery_kit(&last_groups(&key)).unwrap();
    assert_eq!(app.phase(), "unlocked");
    assert_eq!(app.core.vault_location(), folder);
    app.core.lock();
    assert_eq!(app.phase(), "locked");
    assert_eq!(app.core.vault_location(), folder);
    app.core.unlock(recovery(&key)).unwrap();
    assert_eq!(app.phase(), "needs_new_password");
    assert_eq!(app.core.vault_location(), folder);
}

#[test]
fn a_private_key_is_checked_under_whatever_chain_name_the_note_has() {
    let mut app = App::new();
    unlocked(&mut app);
    let not_a_key = "fake-not-a-key";
    let hint = |app: &App, chain: &str, key: &str| app.core.check_private_key(chain, key).unwrap();

    assert!(hint(&app, "evm", not_a_key).unwrap().contains("Ethereum"));
    assert!(hint(&app, "solana", not_a_key).unwrap().contains("Solana"));
    assert!(
        hint(&app, "bitcoin", not_a_key)
            .unwrap()
            .contains("Bitcoin")
    );
    // A name of a chain the app knows is found whatever its case or the spaces around it.
    assert!(hint(&app, " EVM ", not_a_key).is_some());
    assert_eq!(hint(&app, "evm", &"ab".repeat(32)), None);
    // Any other name, typed by the owner, is no check at all, and never a failed call.
    for chain in ["other", "Fake Chain", "fake-l2", "", "   ", "evm\nfake"] {
        assert_eq!(hint(&app, chain, not_a_key), None, "{chain:?}");
    }

    app.core.lock();
    assert_eq!(code(app.core.check_private_key("evm", not_a_key)), "locked");
    assert_eq!(
        code(app.core.check_private_key("Fake Chain", not_a_key)),
        "locked"
    );
}
