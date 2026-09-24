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

struct FakeClock(Arc<AtomicU64>);

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn unix_secs(&self) -> i64 {
        START_UNIX + i64::try_from(self.now_ms() / 1000).unwrap()
    }
}

#[derive(Default)]
struct Board {
    text: Option<String>,
    change_count: u64,
}

struct FakeClipboard(Arc<Mutex<Board>>);

impl Clipboard for FakeClipboard {
    fn write(&mut self, text: &str) -> Result<u64, ApiError> {
        let mut board = self.0.lock().unwrap();
        board.change_count += 1;
        board.text = Some(text.to_owned());
        Ok(board.change_count)
    }
    fn clear_if(&mut self, change_count: u64) {
        let mut board = self.0.lock().unwrap();
        if board.change_count == change_count {
            board.change_count += 1;
            board.text = None;
        }
    }
}

struct App {
    core: AppCore,
    ms: Arc<AtomicU64>,
    board: Arc<Mutex<Board>>,
    temp: tempfile::TempDir,
}

impl App {
    fn new() -> App {
        let temp = tempfile::tempdir().unwrap();
        let (ms, board) = (
            Arc::new(AtomicU64::new(1_000)),
            Arc::new(Mutex::new(Board::default())),
        );
        let core = start(temp.path(), &ms, &board);
        App {
            core,
            ms,
            board,
            temp,
        }
    }

    /// Quit and start again on the same data folder (same clock and clipboard).
    fn restart(self) -> App {
        let App {
            core,
            ms,
            board,
            temp,
        } = self;
        drop(core);
        let core = start(temp.path(), &ms, &board);
        App {
            core,
            ms,
            board,
            temp,
        }
    }

    fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, Ordering::SeqCst);
    }

    fn data_dir(&self) -> PathBuf {
        self.temp.path().join("Vault.noindex")
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

fn start(root: &Path, ms: &Arc<AtomicU64>, board: &Arc<Mutex<Board>>) -> AppCore {
    AppCore::new(Config {
        data_dir: root.join("Vault.noindex"),
        home_dir: root.join("home"),
        limits: Limits::TESTING,
        clock: Box::new(FakeClock(ms.clone())),
        clipboard: Box::new(FakeClipboard(board.clone())),
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
    app.core
        .set_settings(SettingsInput {
            idle_minutes: 5,
            lock_on_app_switch: true,
        })
        .unwrap();
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
    core.trash_note(&text).unwrap();
    core.trash_note(&login).unwrap();
    assert_eq!(core.list_notes(all(), "").unwrap().len(), 3);
    assert_eq!(
        core.list_notes(filter(json!({"type": "trash"})), "")
            .unwrap()
            .len(),
        2
    );
    core.restore_note(&text).unwrap();
    assert_eq!(code(core.delete_forever(&api)), "not_found"); // only from Trash
    core.delete_forever(&login).unwrap();
    core.trash_note(&text).unwrap();
    assert_eq!(core.empty_trash().unwrap(), 1);
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
    core.trash_note(&ids[2]).unwrap();

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
    app.core
        .set_settings(SettingsInput {
            idle_minutes: 1,
            lock_on_app_switch: true,
        })
        .unwrap();
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
    app.core.trash_note(&id).unwrap();
    app.core.lock();
    app.advance(30 * DAY_MS);
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
    assert_eq!(code(app.core.backup_to_open_allowed()), "locked");
    assert_eq!(code(app.core.list_safety_copies()), "locked");
    assert_eq!(code(app.core.write_backup()), "locked");
}

// ---------- account care ----------

#[test]
fn changing_the_master_password_keeps_the_recovery_key() {
    let mut app = App::new();
    let key = unlocked(&mut app);
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
        "phase",
        "platform",
        "capture_hiding_active",
        "sharing_guard",
        "lock_in_ms",
        "clipboard_clear_in_ms",
        "reveal",
        "failed_attempts",
        "retry_in_ms",
        "show_recovery_hint",
        "backup",
        "settings",
    ];
    expected.sort_unstable();
    let mut keys = keys;
    keys.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(
        status["settings"],
        json!({"idle_minutes": 5, "lock_on_app_switch": false, "seed_explainer_seen": false})
    );
    assert_eq!(
        status["backup"],
        json!({"last_backup_at": null, "reminder_due": false, "unbacked_changes": false})
    );
    assert_eq!(status["sharing_guard"], json!({"up": false, "apps": []}));
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
fn an_inspected_backup_is_dropped_after_the_idle_time_even_without_a_vault() {
    let (_source, backup) = a_backup();
    let mut app = App::new();
    app.core.set_backup_to_open(backup.clone()).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    app.advance(5 * 60_000);
    app.core.tick();
    assert_eq!(code(app.core.replace_with_backup()), "invalid_input");
    assert_eq!(app.phase(), "no_vault");

    // Within the idle time it can still be installed.
    app.core.set_backup_to_open(backup).unwrap();
    app.core.inspect_backup(password(PASSWORD)).unwrap();
    app.advance(5 * 60_000 - 1);
    app.core.tick();
    app.core.replace_with_backup().unwrap();
    assert_eq!(app.phase(), "unlocked");
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
    assert_eq!(code(app.core.list_safety_copies()), "locked");
    let e = app.core.unlock(password(PASSWORD)).unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (
            "damaged",
            "This Vault file is damaged. Restore a Safety Copy or Backup."
        )
    );
    // Now the Safety Copies (and Backups) are offered.
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
    assert_eq!(app.core.empty_trash().unwrap(), 0);
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
