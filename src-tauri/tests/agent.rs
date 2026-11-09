//! The agent route's side of the core (ADR-0012), through AppCore's public interface: a temp
//! data folder, tiny Argon2 parameters, a fake clock and a fake clipboard. Every value below is
//! an obviously fake placeholder.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use encrypted_note_app::core::agent::{
    AGENT_GRANT_MAX_MS, AGENT_REQUEST_MS, AgentEdit, AgentEditReply, AgentError, AgentOutcome,
    AgentStoreSuggestion,
};
use encrypted_note_app::core::native::{NativeAction, NativeCredentials, NativeResult};
use encrypted_note_app::core::{
    ApiError, AppCore, Clipboard, Clock, Config, NoteEditInput, NoteInput,
};
use serde_json::{Value, json};
use session::OsEvent;
use vault_format::{KdfParams, Limits};
use zeroize::Zeroizing;

const PASSWORD: &str = "fake-lantern-quartz-mosaic-velvet-orbit";
const WRONG_PASSWORD: &str = "fake-wrong-guess-for-tests-only-7";
const START_UNIX: i64 = 1_790_000_000;
const MINUTE_MS: u64 = 60_000;
/// No Note with this id exists in any test.
const MISSING_ID: &str = "00000000000000000000000000000000";

/// The fake values typed into Hidden Fields, and the owner's own titles: none may ever appear in
/// anything the agent gets back.
const SECRETS: &[&str] = &[
    "fake-owner-key-4501",
    "fake-agent-key-4502",
    "fake-agent-private-key-4503",
    "fake-agent-passphrase-4504",
    "fake-agent-password-4505",
    "fake-agent-body-4506",
    "abandon",
    "Fake owner exchange",
    "Fake owner diary",
];

// ---------- fakes ----------

struct FakeClock(Arc<AtomicU64>);

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn unix_secs(&self) -> i64 {
        START_UNIX + i64::try_from(self.now_ms() / 1000).unwrap()
    }
    fn continuous_ms(&self) -> Option<u64> {
        Some(self.now_ms())
    }
}

struct FakeClipboard(Arc<Mutex<u64>>);

impl Clipboard for FakeClipboard {
    fn write(&mut self, _text: &str) -> Result<u64, ApiError> {
        let mut count = self.0.lock().unwrap();
        *count += 1;
        Ok(*count)
    }
    fn clear_if(&mut self, _change_count: u64) -> bool {
        true
    }
}

struct App {
    core: AppCore,
    ms: Arc<AtomicU64>,
    _temp: tempfile::TempDir,
}

impl App {
    /// A new Vault, confirmed and Unlocked.
    fn unlocked() -> App {
        let temp = tempfile::tempdir().unwrap();
        let ms = Arc::new(AtomicU64::new(1_000));
        let mut core = AppCore::new(Config {
            data_dir: temp.path().join("Vault.noindex"),
            home_dir: temp.path().join("home"),
            limits: Limits::TESTING,
            clock: Box::new(FakeClock(ms.clone())),
            clipboard: Box::new(FakeClipboard(Arc::new(Mutex::new(0)))),
            activity_anywhere_supported: true,
        })
        .unwrap();
        let key = core
            .create_vault(pw(PASSWORD), KdfParams::TESTING)
            .unwrap()
            .recovery_key
            .unwrap();
        core.confirm_recovery_kit(&key[key.len() - 9..]).unwrap();
        assert_eq!(core.app_status().phase, "unlocked");
        App {
            core,
            ms,
            _temp: temp,
        }
    }

    fn advance(&self, ms: u64) {
        self.ms.fetch_add(ms, Ordering::SeqCst);
    }

    /// Time passes while the owner keeps using the app, so Auto-lock stays away.
    fn advance_with_owner_activity(&mut self, ms: u64) {
        let mut left = ms;
        while left > 0 {
            let step = left.min(MINUTE_MS);
            self.core.activity();
            self.advance(step);
            self.core.tick();
            left -= step;
        }
    }

    fn grant(&mut self) -> u64 {
        self.core.agent_grant_start().unwrap().id
    }

    /// The owner's own Note, added in the app as today.
    fn owner_adds(&mut self, kind: &str, title: &str, visible: Value, hidden: Value) -> String {
        let password = is_wallet(kind).then(|| pw(PASSWORD));
        self.core
            .create_note(input(kind, title, visible, hidden), password)
            .unwrap()
    }

    /// A Note the agent asked to store and the owner saved.
    fn agent_adds(
        &mut self,
        grant: u64,
        kind: &str,
        title: &str,
        visible: Value,
        hidden: Value,
    ) -> String {
        let request_id = self
            .core
            .agent_store(grant, suggestion(kind, title, visible.clone()))
            .unwrap();
        let result = native_run(
            &mut self.core,
            NativeAction::AgentStore {
                request_id: request_id.clone(),
                input: input(kind, title, visible, hidden),
            },
            credentials_for(kind),
        )
        .unwrap();
        let NativeResult::NoteId(id) = result else {
            panic!("a Note id expected")
        };
        assert_eq!(
            self.core.agent_take_outcome(&request_id),
            Some(AgentOutcome::Saved { id: id.clone() })
        );
        id
    }

    fn agent_ids(&mut self, grant: u64) -> Vec<String> {
        let mut ids: Vec<String> = self
            .core
            .agent_list(grant)
            .unwrap()
            .into_iter()
            .map(|n| n.id)
            .collect();
        ids.sort();
        ids
    }

    fn visible(&mut self, id: &str) -> BTreeMap<String, String> {
        self.core.get_note(id).unwrap().visible
    }
}

// ---------- helpers ----------

fn pw(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_owned())
}

fn is_wallet(kind: &str) -> bool {
    matches!(kind, "seed_phrase" | "private_key")
}

fn credentials_for(kind: &str) -> NativeCredentials {
    if is_wallet(kind) {
        NativeCredentials::Password(pw(PASSWORD))
    } else {
        NativeCredentials::None
    }
}

fn input(kind: &str, title: &str, visible: Value, hidden: Value) -> NoteInput {
    serde_json::from_value(
        json!({ "kind": kind, "title": title, "visible": visible, "hidden": hidden }),
    )
    .unwrap()
}

fn edit_input(title: &str, visible: Value, hidden: Value) -> NoteEditInput {
    serde_json::from_value(json!({ "title": title, "visible": visible, "hidden": hidden })).unwrap()
}

fn suggestion(kind: &str, title: &str, visible: Value) -> AgentStoreSuggestion {
    AgentStoreSuggestion {
        kind: serde_json::from_value(json!(kind)).unwrap(),
        title: title.to_owned(),
        visible: serde_json::from_value(visible).unwrap(),
    }
}

fn agent_edit(title: Option<&str>, visible: Value) -> AgentEdit {
    AgentEdit {
        title: title.map(str::to_owned),
        visible: serde_json::from_value(visible).unwrap(),
    }
}

fn twelve_words() -> String {
    ["abandon"; 11].join(" ") + " about"
}

fn native_run(
    core: &mut AppCore,
    action: NativeAction,
    credentials: NativeCredentials,
) -> Result<NativeResult, ApiError> {
    let operation = core.begin_native(action)?;
    let job = core.prepare_native(operation.id, credentials)?;
    core.finish_native(job.run())
}

/// Runs `action` and, if it fails before the panel's work finishes, closes the panel as the
/// coordinator would, so the next step starts clean.
fn native_try(
    core: &mut AppCore,
    action: NativeAction,
    credentials: NativeCredentials,
) -> Result<NativeResult, ApiError> {
    let result = native_run(core, action, credentials);
    core.cancel_all_native();
    result
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

fn agent_code<T>(result: Result<T, AgentError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(e) => e.code(),
    }
}

fn pending_request(reply: AgentEditReply) -> String {
    match reply {
        AgentEditReply::Pending(request_id) => request_id,
        AgentEditReply::Done => panic!("a pending request expected"),
    }
}

fn assert_no_secret(text: &str) {
    for secret in SECRETS {
        assert!(!text.contains(secret), "{secret} leaked into {text}");
    }
}

/// An owner API Key and Text, and agent-added Notes of every Kind. Returns
/// (owner ids, agent ids by Kind: api_key, login, text, seed_phrase, private_key).
fn a_mixed_vault(app: &mut App, grant: u64) -> (Vec<String>, Vec<String>) {
    let owner = vec![
        app.owner_adds(
            "api_key",
            "Fake owner exchange",
            json!({"service": "owner.example.test"}),
            json!({"key": "fake-owner-key-4501"}),
        ),
        app.owner_adds(
            "text",
            "Fake owner diary",
            json!({}),
            json!({"body": "fake owner text"}),
        ),
    ];
    let agent = vec![
        app.agent_adds(
            grant,
            "api_key",
            "Fake agent RPC",
            json!({"service": "rpc.example.test", "purpose": "fake testing"}),
            json!({"key": "fake-agent-key-4502"}),
        ),
        app.agent_adds(
            grant,
            "login",
            "Fake agent login",
            json!({"website": "https://login.example.test", "username": "fake-user"}),
            json!({"password": "fake-agent-password-4505"}),
        ),
        app.agent_adds(
            grant,
            "text",
            "Fake agent text",
            json!({}),
            json!({"body": "fake-agent-body-4506"}),
        ),
        app.agent_adds(
            grant,
            "seed_phrase",
            "Fake agent seed",
            json!({"wallet_name": "Fake wallet"}),
            json!({"words": twelve_words(), "passphrase": "fake-agent-passphrase-4504"}),
        ),
        app.agent_adds(
            grant,
            "private_key",
            "Fake agent EVM key",
            json!({"chain": "evm", "address": "0x00000000000000000000000000000000000000a1"}),
            json!({"key": "fake-agent-private-key-4503"}),
        ),
    ];
    (owner, agent)
}

// ---------- what the agent sees ----------

#[test]
fn the_agent_sees_only_the_notes_it_added_and_never_a_value() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (owner, agent) = a_mixed_vault(&mut app, grant);

    let mut expected = agent.clone();
    expected.sort();
    assert_eq!(app.agent_ids(grant), expected);
    let listed = app.core.agent_list(grant).unwrap();
    let text = serde_json::to_string(&listed).unwrap();
    assert_no_secret(&text);
    for leak in [
        "passphrase",
        "has_hidden",
        "favorite",
        "word_count",
        "hidden",
    ] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    // A Seed Phrase without a Passphrase looks the same.
    let without = app.agent_adds(
        grant,
        "seed_phrase",
        "Fake agent seed two",
        json!({"wallet_name": "Fake wallet"}),
        json!({"words": twelve_words()}),
    );
    let one = |app: &mut App, id: &str| {
        let note = app
            .core
            .agent_list(grant)
            .unwrap()
            .into_iter()
            .find(|n| n.id == id)
            .unwrap();
        serde_json::to_value(note).unwrap()
    };
    let (with, without) = (one(&mut app, &agent[3]), one(&mut app, &without));
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(&with), keys(&without));
    assert_eq!(with["visible"], without["visible"]);

    // The owner's screens show the mark; the owner's own Notes don't carry it.
    for id in &agent {
        assert!(app.core.get_note(id).unwrap().added_by_agent);
    }
    for id in &owner {
        assert!(!app.core.get_note(id).unwrap().added_by_agent);
    }

    // What the owner's screens are told about a pending request holds no value either.
    type Ask = fn(&mut AppCore, u64, &str) -> Result<String, AgentError>;
    let asks: [(Ask, &str); 4] = [
        (AppCore::agent_replace, &agent[4]),
        (AppCore::agent_replace, &agent[3]),
        (AppCore::agent_delete, &agent[3]),
        (
            |core, grant, id| {
                let edit = agent_edit(None, json!({"address": "fake-address-a1"}));
                match core.agent_edit(grant, id, edit) {
                    Ok(AgentEditReply::Pending(request)) => Ok(request),
                    _ => Err(AgentError::Internal),
                }
            },
            &agent[3],
        ),
    ];
    for (ask, id) in asks {
        let request = ask(&mut app.core, grant, id).unwrap();
        let view = app.core.agent_pending().unwrap();
        assert_no_secret(&serde_json::to_string(&view).unwrap());
        app.core.agent_cancel_request(&request);
    }
    let outcome = serde_json::to_string(&AgentOutcome::Saved {
        id: agent[0].clone(),
    })
    .unwrap();
    assert_eq!(
        outcome,
        format!(r#"{{"result":"saved","id":"{}"}}"#, agent[0])
    );
}

#[test]
fn an_owners_note_a_missing_id_and_one_in_trash_look_the_same() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (owner, agent) = a_mixed_vault(&mut app, grant);
    let trashed = agent[0].clone();
    app.core.trash_note(&trashed, None).unwrap();
    assert!(!app.agent_ids(grant).contains(&trashed));

    for id in [owner[0].as_str(), owner[1].as_str(), MISSING_ID, &trashed] {
        let rename = agent_edit(Some("Fake taken over"), json!({}));
        assert_eq!(
            agent_code(app.core.agent_edit(grant, id, rename)),
            "not_found"
        );
        let service = agent_edit(None, json!({"service": "evil.example.test"}));
        assert_eq!(
            agent_code(app.core.agent_edit(grant, id, service)),
            "not_found"
        );
        assert_eq!(agent_code(app.core.agent_replace(grant, id)), "not_found");
        assert_eq!(agent_code(app.core.agent_delete(grant, id)), "not_found");
    }
    assert!(app.core.agent_pending().is_none());
    assert_eq!(
        app.core.get_note(&owner[0]).unwrap().title,
        "Fake owner exchange"
    );
    // The Note the owner restores from Trash is the agent's again.
    app.core.restore_note(&trashed).unwrap();
    assert!(app.agent_ids(grant).contains(&trashed));
}

#[test]
fn removing_the_mark_takes_a_note_out_of_the_agents_reach_at_once() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let id = app.agent_adds(
        grant,
        "api_key",
        "Fake agent RPC",
        json!({"service": "rpc.example.test"}),
        json!({"key": "fake-agent-key-4502"}),
    );
    let request = pending_request(
        app.core
            .agent_edit(
                grant,
                &id,
                agent_edit(None, json!({"service": "x.example.test"})),
            )
            .unwrap(),
    );
    app.core.remove_agent_mark(&id).unwrap();
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    assert!(app.agent_ids(grant).is_empty());
    assert_eq!(agent_code(app.core.agent_delete(grant, &id)), "not_found");
    assert!(!app.core.get_note(&id).unwrap().added_by_agent);
    assert_eq!(app.visible(&id)["service"], "rpc.example.test");
    // Nothing puts it back: removing it again changes nothing.
    app.core.remove_agent_mark(&id).unwrap();
    assert!(app.agent_ids(grant).is_empty());
}

// ---------- the grant ----------

#[test]
fn the_grant_ends_on_every_lock_and_after_an_hour() {
    type End = fn(&mut App);
    let ends: [(&str, End); 7] = [
        ("lock", |app| app.core.lock()),
        ("sleep", |app| app.core.os_event(OsEvent::Sleep)),
        ("screen lock", |app| {
            app.core.os_event(OsEvent::ScreenLocked)
        }),
        ("user switch", |app| {
            app.core.os_event(OsEvent::UserSwitched)
        }),
        ("quit", |app| app.core.os_event(OsEvent::Quit)),
        ("Auto-lock", |app| {
            app.advance(u64::from(session::DEFAULT_IDLE_MINUTES) * MINUTE_MS)
        }),
        ("the hour", |app| {
            app.advance_with_owner_activity(AGENT_GRANT_MAX_MS)
        }),
    ];
    for (name, end) in ends {
        let mut app = App::unlocked();
        let grant = app.grant();
        assert_eq!(app.core.agent_grant_status().state, "running", "{name}");
        let request = app
            .core
            .agent_store(grant, suggestion("text", "Fake note", json!({})))
            .unwrap();
        end(&mut app);
        assert_eq!(agent_code(app.core.agent_list(grant)), "off", "{name}");
        assert_eq!(
            app.core.agent_take_outcome(&request),
            Some(AgentOutcome::Cancelled),
            "{name}"
        );
        let expected = if name == "the hour" {
            "expired"
        } else {
            "ended_by_lock"
        };
        assert_eq!(app.core.agent_grant_status().state, expected, "{name}");
        assert_eq!(app.core.agent_grant_status().expires_in_ms, None);
        if app.core.app_status().phase == "locked" {
            app.core
                .unlock(serde_json::from_value(json!({ "master_password": PASSWORD })).unwrap())
                .unwrap();
        }
        // Unlocking again doesn't bring the old grant back.
        assert_eq!(agent_code(app.core.agent_list(grant)), "off", "{name}");
        let completion = native_try(
            &mut app.core,
            NativeAction::AgentStore {
                request_id: request,
                input: input("text", "Fake note", json!({}), json!({"body": "fake"})),
            },
            NativeCredentials::None,
        );
        assert_eq!(code(completion), "not_found", "{name}");
    }
}

#[test]
fn the_grant_lasts_at_most_an_hour_and_only_while_unlocked() {
    let mut app = App::unlocked();
    assert_eq!(app.core.agent_grant_status().state, "off");
    assert_eq!(agent_code(app.core.agent_list(0)), "off");
    let started = app.core.agent_grant_start().unwrap();
    assert_eq!(started.expires_in_ms, AGENT_GRANT_MAX_MS);
    assert_eq!(
        app.core.agent_grant_status().expires_in_ms,
        Some(AGENT_GRANT_MAX_MS)
    );
    // A grant that isn't the current one is off.
    assert_eq!(agent_code(app.core.agent_list(started.id ^ 1)), "off");
    app.advance_with_owner_activity(AGENT_GRANT_MAX_MS - MINUTE_MS);
    assert_eq!(agent_code(app.core.agent_list(started.id)), "ok");
    assert_eq!(app.core.agent_grant_status().expires_in_ms, Some(MINUTE_MS));

    // Starting again replaces the grant, and cancels its request.
    let request = app
        .core
        .agent_store(started.id, suggestion("text", "Fake note", json!({})))
        .unwrap();
    let again = app.grant();
    assert_ne!(again, started.id);
    assert_eq!(agent_code(app.core.agent_list(started.id)), "off");
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    app.core.agent_grant_stop();
    assert_eq!(app.core.agent_grant_status().state, "off");
    assert_eq!(agent_code(app.core.agent_list(again)), "off");

    app.core.lock();
    assert_eq!(code(app.core.agent_grant_start()), "locked");
    assert_eq!(app.core.agent_grant_status().state, "off");
}

#[test]
fn agent_requests_do_not_keep_auto_lock_away() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let id = app.agent_adds(
        grant,
        "api_key",
        "Fake agent RPC",
        json!({"service": "rpc.example.test"}),
        json!({"key": "fake-agent-key-4502"}),
    );
    app.core.activity();
    let idle_ms = u64::from(session::DEFAULT_IDLE_MINUTES) * MINUTE_MS;
    let step = 10_000;
    let mut waited = 0;
    while waited + step < idle_ms {
        app.advance(step);
        waited += step;
        assert_eq!(agent_code(app.core.agent_list(grant)), "ok");
        let rename = agent_edit(Some(&format!("Fake agent RPC {waited}")), json!({}));
        assert_eq!(agent_code(app.core.agent_edit(grant, &id, rename)), "ok");
        let request = app.core.agent_replace(grant, &id).unwrap();
        assert_eq!(app.core.agent_take_outcome(&request), None);
        app.core.agent_cancel_request(&request);
    }
    app.advance(idle_ms - waited);
    assert_eq!(agent_code(app.core.agent_list(grant)), "off");
    assert_eq!(app.core.app_status().phase, "locked");
}

// ---------- edits ----------

#[test]
fn names_purposes_and_expiry_dates_change_at_once() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    let (api, login, text) = (&agent[0], &agent[1], &agent[2]);

    let reply = app.core.agent_edit(
        grant,
        api,
        agent_edit(
            Some("Fake agent RPC renamed"),
            json!({"purpose": "fake staging", "expires": "2027-01-31"}),
        ),
    );
    assert_eq!(reply, Ok(AgentEditReply::Done));
    let view = app.core.get_note(api).unwrap();
    assert_eq!(view.title, "Fake agent RPC renamed");
    assert_eq!(view.visible["purpose"], "fake staging");
    assert_eq!(view.visible["expires"], "2027-01-31");
    let reply = app.core.agent_edit(
        grant,
        login,
        agent_edit(Some("Fake agent login 2"), json!({"purpose": "fake"})),
    );
    assert_eq!(reply, Ok(AgentEditReply::Done));
    assert_eq!(app.visible(login)["purpose"], "fake");
    let reply = app.core.agent_edit(
        grant,
        text,
        agent_edit(Some("Fake agent text 2"), json!({})),
    );
    assert_eq!(reply, Ok(AgentEditReply::Done));
    assert_eq!(app.core.get_note(text).unwrap().title, "Fake agent text 2");

    // A purpose cleared, and an edit that changes nothing.
    let clear = agent_edit(None, json!({"purpose": ""}));
    assert_eq!(
        app.core.agent_edit(grant, api, clear),
        Ok(AgentEditReply::Done)
    );
    assert!(!app.visible(api).contains_key("purpose"));
    let same = agent_edit(Some("Fake agent RPC renamed"), json!({}));
    assert_eq!(
        app.core.agent_edit(grant, api, same),
        Ok(AgentEditReply::Done)
    );

    // A value that breaks its field's rule is refused, and nothing changes.
    for bad in [
        agent_edit(None, json!({"expires": "fake-not-a-date"})),
        agent_edit(None, json!({"purpose": "fake\nsecond line"})),
        agent_edit(Some("   "), json!({})),
        agent_edit(Some(&"x".repeat(1_001)), json!({})),
    ] {
        assert_eq!(agent_code(app.core.agent_edit(grant, api, bad)), "invalid");
    }
    assert_eq!(
        app.core.get_note(api).unwrap().title,
        "Fake agent RPC renamed"
    );
    assert!(app.core.agent_pending().is_none());
}

#[test]
fn where_a_key_is_used_changes_only_after_the_owner_allows_it() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    let (api, login) = (&agent[0], &agent[1]);

    for (id, change, field) in [
        (api, json!({"service": "evil.example.test"}), "service"),
        (api, json!({"key_id": "fake-key-id"}), "key_id"),
        (api, json!({"url": "https://evil.example.test"}), "url"),
        (api, json!({"env_var": "FAKE_ENV"}), "env_var"),
        (
            login,
            json!({"website": "https://evil.example.test"}),
            "website",
        ),
        (login, json!({"username": "fake-evil"}), "username"),
    ] {
        let before = app.visible(id);
        let reply = app
            .core
            .agent_edit(grant, id, agent_edit(None, change.clone()));
        let request = pending_request(reply.unwrap());
        assert_eq!(
            app.visible(id),
            before,
            "{field} changed before the owner allowed it"
        );
        let view = app.core.agent_pending().unwrap();
        assert_eq!((view.request, view.needs_password), ("edit", false));
        assert_eq!(view.note_id.as_deref(), Some(id.as_str()));
        assert_eq!(view.changes.len(), 1);
        assert_eq!(view.changes[0].field, field);
        assert_eq!(
            view.changes[0].from,
            before.get(field).cloned().unwrap_or_default()
        );
        assert_eq!(view.changes[0].to, change[field].as_str().unwrap());
        assert_eq!(app.core.agent_take_outcome(&request), None);

        // Cancel: nothing changes.
        app.core.agent_cancel_request(&request);
        assert_eq!(
            app.core.agent_take_outcome(&request),
            Some(AgentOutcome::Cancelled)
        );
        assert_eq!(app.visible(id), before);

        // Allow: changed.
        let request = pending_request(
            app.core
                .agent_edit(grant, id, agent_edit(None, change.clone()))
                .unwrap(),
        );
        let result = native_run(
            &mut app.core,
            NativeAction::AgentEdit {
                request_id: request.clone(),
            },
            NativeCredentials::None,
        );
        assert!(matches!(result, Ok(NativeResult::Unit)));
        assert_eq!(
            app.core.agent_take_outcome(&request),
            Some(AgentOutcome::Done)
        );
        assert_eq!(app.visible(id)[field], change[field].as_str().unwrap());
    }

    // A name together with a service waits for the owner as a whole.
    let both = agent_edit(
        Some("Fake renamed"),
        json!({"service": "both.example.test"}),
    );
    let request = pending_request(app.core.agent_edit(grant, api, both).unwrap());
    assert_ne!(app.core.get_note(api).unwrap().title, "Fake renamed");
    assert_eq!(app.core.agent_pending().unwrap().changes.len(), 2);
    app.core.agent_cancel_request(&request);
}

#[test]
fn a_pending_edit_is_not_applied_once_the_note_changed() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let id = app.agent_adds(
        grant,
        "api_key",
        "Fake agent RPC",
        json!({"service": "rpc.example.test"}),
        json!({"key": "fake-agent-key-4502"}),
    );
    let request = pending_request(
        app.core
            .agent_edit(
                grant,
                &id,
                agent_edit(None, json!({"url": "https://x.example.test"})),
            )
            .unwrap(),
    );
    app.core
        .update_note(
            &id,
            edit_input(
                "Fake agent RPC",
                json!({"service": "owner.example.test"}),
                json!({}),
            ),
            None,
        )
        .unwrap();
    let result = native_try(
        &mut app.core,
        NativeAction::AgentEdit {
            request_id: request.clone(),
        },
        NativeCredentials::None,
    );
    assert_eq!(code(result), "invalid_input");
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    assert!(!app.visible(&id).contains_key("url"));
}

#[test]
fn values_word_counts_and_unknown_fields_are_never_edited() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    for (id, change) in [
        (&agent[0], json!({"key": "fake-replaced"})),
        (&agent[1], json!({"password": "fake-replaced"})),
        (&agent[2], json!({"body": "fake-replaced"})),
        (&agent[3], json!({"words": "fake-replaced"})),
        (&agent[3], json!({"passphrase": "fake-replaced"})),
        (&agent[3], json!({"word_count": "24"})),
        (&agent[4], json!({"key": "fake-replaced"})),
        (&agent[0], json!({"fake_field": "fake"})),
        // Allowed together with a name that never is: refused as a whole.
        (
            &agent[0],
            json!({"purpose": "fake", "key": "fake-replaced"}),
        ),
    ] {
        let before = app.core.agent_list(grant).unwrap();
        let reply = app
            .core
            .agent_edit(grant, id, agent_edit(None, change.clone()));
        assert_eq!(agent_code(reply), "not_allowed", "{change}");
        assert_eq!(app.core.agent_list(grant).unwrap(), before);
    }
    assert!(app.core.agent_pending().is_none());
}

#[test]
fn every_edit_of_a_wallet_kind_needs_the_master_password() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    let (seed, key) = (&agent[3], &agent[4]);
    for (id, change) in [
        (key, agent_edit(Some("Fake renamed key"), json!({}))),
        (key, agent_edit(None, json!({"chain": "solana"}))),
        (
            key,
            agent_edit(
                None,
                json!({"address": "0x00000000000000000000000000000000000000b2"}),
            ),
        ),
        (seed, agent_edit(Some("Fake renamed seed"), json!({}))),
        (
            seed,
            agent_edit(None, json!({"wallet_name": "Fake other wallet"})),
        ),
        (
            seed,
            agent_edit(None, json!({"derivation_path": "m/44'/60'/0'/0/1"})),
        ),
    ] {
        let before = app.core.get_note(id).unwrap();
        let request = pending_request(app.core.agent_edit(grant, id, change).unwrap());
        assert!(app.core.agent_pending().unwrap().needs_password);
        let action = || NativeAction::AgentEdit {
            request_id: request.clone(),
        };
        assert_eq!(
            code(native_try(&mut app.core, action(), NativeCredentials::None)),
            "invalid_input"
        );
        let wrong = NativeCredentials::Password(pw(WRONG_PASSWORD));
        assert_eq!(
            code(native_try(&mut app.core, action(), wrong)),
            "wrong_credential"
        );
        let after = app.core.get_note(id).unwrap();
        assert_eq!((after.title, after.visible), (before.title, before.visible));
        let right = NativeCredentials::Password(pw(PASSWORD));
        assert!(matches!(
            native_try(&mut app.core, action(), right),
            Ok(NativeResult::Unit)
        ));
        assert_eq!(
            app.core.agent_take_outcome(&request),
            Some(AgentOutcome::Done)
        );
    }
    assert_eq!(app.core.get_note(key).unwrap().title, "Fake renamed key");
    assert_eq!(app.visible(key)["chain"], "solana");
    assert_eq!(app.visible(seed)["wallet_name"], "Fake other wallet");
}

// ---------- store, replace, delete ----------

#[test]
fn every_kind_can_be_stored_and_wallet_kinds_need_the_master_password() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    assert_eq!(agent.len(), 5);

    for (kind, visible, hidden) in [
        (
            "seed_phrase",
            json!({"wallet_name": "Fake wallet"}),
            json!({"words": twelve_words()}),
        ),
        (
            "private_key",
            json!({"chain": "bitcoin"}),
            json!({"key": "fake-agent-private-key-4503"}),
        ),
    ] {
        let request = app
            .core
            .agent_store(grant, suggestion(kind, "Fake wallet key", visible.clone()))
            .unwrap();
        let view = app.core.agent_pending().unwrap();
        assert_eq!((view.request, view.needs_password), ("store", true));
        assert_eq!(view.title, "Fake wallet key");
        let action = || NativeAction::AgentStore {
            request_id: request.clone(),
            input: input(kind, "Fake wallet key", visible.clone(), hidden.clone()),
        };
        assert_eq!(
            code(native_try(&mut app.core, action(), NativeCredentials::None)),
            "invalid_input"
        );
        let wrong = NativeCredentials::Password(pw(WRONG_PASSWORD));
        assert_eq!(
            code(native_try(&mut app.core, action(), wrong)),
            "wrong_credential"
        );
        let right = NativeCredentials::Password(pw(PASSWORD));
        let Ok(NativeResult::NoteId(id)) = native_try(&mut app.core, action(), right) else {
            panic!("stored")
        };
        assert!(app.core.get_note(&id).unwrap().added_by_agent);
    }
    // The owner's own Add never marks a Note.
    let owner = app.owner_adds("text", "Fake owner diary", json!({}), json!({"body": "x"}));
    assert!(!app.core.get_note(&owner).unwrap().added_by_agent);
    assert!(!app.agent_ids(grant).contains(&owner));
}

#[test]
fn a_suggestion_has_a_title_and_only_visible_details() {
    let mut app = App::unlocked();
    let grant = app.grant();
    for (bad, expected) in [
        (suggestion("api_key", " ", json!({})), "invalid"),
        (
            suggestion("api_key", &"x".repeat(1_001), json!({})),
            "invalid",
        ),
        (
            suggestion("api_key", "Fake", json!({"service": "x".repeat(1_001)})),
            "invalid",
        ),
        (
            suggestion("api_key", "Fake", json!({"key": "fake-agent-key-4502"})),
            "not_allowed",
        ),
        (
            suggestion("seed_phrase", "Fake", json!({"word_count": "12"})),
            "not_allowed",
        ),
        (
            suggestion("text", "Fake", json!({"body": "fake-agent-body-4506"})),
            "not_allowed",
        ),
        (
            suggestion("login", "Fake", json!({"fake_field": "fake"})),
            "not_allowed",
        ),
    ] {
        assert_eq!(agent_code(app.core.agent_store(grant, bad)), expected);
    }
    assert!(app.core.agent_pending().is_none());
}

#[test]
fn the_owner_can_store_only_the_kind_the_agent_asked_for() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let request = app
        .core
        .agent_store(grant, suggestion("api_key", "Fake agent RPC", json!({})))
        .unwrap();
    let wrong_kind = NativeAction::AgentStore {
        request_id: request.clone(),
        input: input("text", "Fake agent RPC", json!({}), json!({"body": "fake"})),
    };
    assert_eq!(
        code(native_try(
            &mut app.core,
            wrong_kind,
            NativeCredentials::None
        )),
        "invalid_input"
    );
    // Still waiting: the owner can save it as asked.
    assert_eq!(app.core.agent_take_outcome(&request), None);
    let right = NativeAction::AgentStore {
        request_id: request.clone(),
        input: input(
            "api_key",
            "Fake agent RPC",
            json!({}),
            json!({"key": "fake-agent-key-4502"}),
        ),
    };
    assert!(matches!(
        native_try(&mut app.core, right, NativeCredentials::None),
        Ok(NativeResult::NoteId(_))
    ));
    assert!(matches!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Saved { .. })
    ));
}

#[test]
fn a_replacement_changes_only_the_value() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    let (api, key) = (&agent[0], &agent[4]);

    let request = app.core.agent_replace(grant, api).unwrap();
    let view = app.core.agent_pending().unwrap();
    assert_eq!((view.request, view.needs_password), ("replace", false));
    let service = json!({"service": "rpc.example.test", "purpose": "fake testing"});
    let replace = |title: &str, visible: Value, hidden: Value| NativeAction::AgentReplace {
        request_id: request.clone(),
        input: edit_input(title, visible, hidden),
    };
    for (action, expected) in [
        (
            replace(
                "Fake renamed",
                service.clone(),
                json!({"key": "fake-new-4510"}),
            ),
            "invalid_input",
        ),
        (
            replace(
                "Fake agent RPC",
                json!({"service": "evil.example.test"}),
                json!({"key": "fake-new-4510"}),
            ),
            "invalid_input",
        ),
        (
            replace("Fake agent RPC", service.clone(), json!({})),
            "invalid_input",
        ),
        (
            replace(
                "Fake agent RPC",
                service.clone(),
                json!({"fake_field": "x"}),
            ),
            "invalid_input",
        ),
    ] {
        assert_eq!(
            code(native_try(&mut app.core, action, NativeCredentials::None)),
            expected
        );
    }
    let done = native_try(
        &mut app.core,
        replace(
            "Fake agent RPC",
            service.clone(),
            json!({"key": "fake-new-4510"}),
        ),
        NativeCredentials::None,
    );
    assert!(matches!(done, Ok(NativeResult::Unit)));
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Saved { id: api.clone() })
    );
    assert_eq!(&*app.core.show_field(api, "key").unwrap(), "fake-new-4510");
    app.core.hide_field();

    // A Private Key: the Master Password too.
    let request = app.core.agent_replace(grant, key).unwrap();
    assert!(app.core.agent_pending().unwrap().needs_password);
    let action = || NativeAction::AgentReplace {
        request_id: request.clone(),
        input: edit_input(
            "Fake agent EVM key",
            json!({}),
            json!({"key": "fake-new-private-4511"}),
        ),
    };
    assert_eq!(
        code(native_try(&mut app.core, action(), NativeCredentials::None)),
        "invalid_input"
    );
    let right = NativeCredentials::Password(pw(PASSWORD));
    assert!(matches!(
        native_try(&mut app.core, action(), right),
        Ok(NativeResult::Unit)
    ));
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Saved { id: key.clone() })
    );
}

#[test]
fn a_delete_always_needs_the_master_password() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    for id in [&agent[0], &agent[2], &agent[4]] {
        let request = app.core.agent_delete(grant, id).unwrap();
        let view = app.core.agent_pending().unwrap();
        assert_eq!((view.request, view.needs_password), ("delete", true));
        let action = || NativeAction::AgentTrash {
            request_id: request.clone(),
        };
        assert_eq!(
            code(native_try(&mut app.core, action(), NativeCredentials::None)),
            "invalid_input"
        );
        let wrong = NativeCredentials::Password(pw(WRONG_PASSWORD));
        assert_eq!(
            code(native_try(&mut app.core, action(), wrong)),
            "wrong_credential"
        );
        assert!(app.core.get_note(id).unwrap().deleted_at.is_none());
        let right = NativeCredentials::Password(pw(PASSWORD));
        assert!(matches!(
            native_try(&mut app.core, action(), right),
            Ok(NativeResult::Unit)
        ));
        assert_eq!(
            app.core.agent_take_outcome(&request),
            Some(AgentOutcome::Deleted)
        );
        assert!(app.core.get_note(id).unwrap().deleted_at.is_some());
        assert!(!app.agent_ids(grant).contains(id));
        // Three wrong passwords in a row would make the next try wait; this test isn't about that.
        app.advance_with_owner_activity(MINUTE_MS);
    }
    // The owner's Cancel leaves the Note where it was.
    let request = app.core.agent_delete(grant, &agent[1]).unwrap();
    app.core.agent_cancel_request(&request);
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    assert!(app.core.get_note(&agent[1]).unwrap().deleted_at.is_none());
}

// ---------- requests and the owner's commands ----------

#[test]
fn a_request_that_does_not_exist_or_ended_cannot_be_completed() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let id = app.agent_adds(
        grant,
        "text",
        "Fake agent text",
        json!({}),
        json!({"body": "fake-agent-body-4506"}),
    );
    let made_up = "fake-request-id".to_owned();
    for action in [
        NativeAction::AgentStore {
            request_id: made_up.clone(),
            input: input("text", "Fake", json!({}), json!({"body": "fake"})),
        },
        NativeAction::AgentEdit {
            request_id: made_up.clone(),
        },
        NativeAction::AgentReplace {
            request_id: made_up.clone(),
            input: edit_input("Fake agent text", json!({}), json!({"body": "fake"})),
        },
        NativeAction::AgentTrash {
            request_id: made_up.clone(),
        },
    ] {
        let password = NativeCredentials::Password(pw(PASSWORD));
        assert_eq!(
            code(native_try(&mut app.core, action, password)),
            "not_found"
        );
    }

    // A request completed by a different action is refused, and stays pending.
    let request = app.core.agent_delete(grant, &id).unwrap();
    let edit = NativeAction::AgentEdit {
        request_id: request.clone(),
    };
    assert_eq!(
        code(native_try(&mut app.core, edit, NativeCredentials::None)),
        "not_found"
    );
    let replace = NativeAction::AgentReplace {
        request_id: request.clone(),
        input: edit_input("Fake agent text", json!({}), json!({"body": "fake"})),
    };
    assert_eq!(
        code(native_try(&mut app.core, replace, NativeCredentials::None)),
        "not_found"
    );
    assert_eq!(app.core.agent_take_outcome(&request), None);

    // Once its grant ends, not even the right completion works.
    app.core.agent_grant_stop();
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    let _ = app.grant();
    let trash = NativeAction::AgentTrash {
        request_id: request.clone(),
    };
    let password = NativeCredentials::Password(pw(PASSWORD));
    assert_eq!(
        code(native_try(&mut app.core, trash, password)),
        "not_found"
    );
    assert!(app.core.get_note(&id).unwrap().deleted_at.is_none());
    // An outcome is given once; an id the core doesn't know is "cancelled".
    assert_eq!(
        app.core.agent_take_outcome("fake-unknown-request"),
        Some(AgentOutcome::Cancelled)
    );
}

#[test]
fn one_request_at_a_time_and_edits_at_once_wait_for_the_owner() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (owner, agent) = a_mixed_vault(&mut app, grant);
    let request = app
        .core
        .agent_store(grant, suggestion("text", "Fake", json!({})))
        .unwrap();
    assert_eq!(
        agent_code(
            app.core
                .agent_store(grant, suggestion("text", "Fake 2", json!({})))
        ),
        "busy"
    );
    assert_eq!(agent_code(app.core.agent_replace(grant, &agent[0])), "busy");
    assert_eq!(agent_code(app.core.agent_delete(grant, &agent[0])), "busy");
    let service = agent_edit(None, json!({"service": "x.example.test"}));
    assert_eq!(
        agent_code(app.core.agent_edit(grant, &agent[0], service)),
        "busy"
    );
    let rename = || agent_edit(Some("Fake renamed"), json!({}));
    assert_eq!(
        agent_code(app.core.agent_edit(grant, &agent[0], rename())),
        "not_now"
    );
    assert_eq!(agent_code(app.core.agent_list(grant)), "ok");
    app.core.agent_cancel_request(&request);

    // While the owner has a panel open, an edit at once waits, and a new request doesn't end
    // the owner's panel.
    let operation = app
        .core
        .begin_native(NativeAction::TrashNote {
            id: owner[1].clone(),
        })
        .unwrap();
    assert_eq!(
        agent_code(app.core.agent_edit(grant, &agent[0], rename())),
        "not_now"
    );
    let request = app.core.agent_replace(grant, &agent[0]).unwrap();
    let job = app
        .core
        .prepare_native(operation.id, NativeCredentials::None)
        .unwrap();
    assert!(matches!(
        app.core.finish_native(job.run()),
        Ok(NativeResult::Unit)
    ));
    assert!(app.core.get_note(&owner[1]).unwrap().deleted_at.is_some());
    app.core.agent_cancel_request(&request);
    assert_eq!(
        app.core.agent_edit(grant, &agent[0], rename()),
        Ok(AgentEditReply::Done)
    );
}

#[test]
fn a_request_is_cancelled_after_five_minutes() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let request = app
        .core
        .agent_store(grant, suggestion("text", "Fake", json!({})))
        .unwrap();
    app.advance_with_owner_activity(AGENT_REQUEST_MS - 1_000);
    assert_eq!(app.core.agent_take_outcome(&request), None);
    assert!(app.core.agent_pending().unwrap().expires_in_ms <= 1_000);
    app.advance_with_owner_activity(1_000);
    assert!(app.core.agent_pending().is_none());
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    let late = NativeAction::AgentStore {
        request_id: request,
        input: input("text", "Fake", json!({}), json!({"body": "fake"})),
    };
    assert_eq!(
        code(native_try(&mut app.core, late, NativeCredentials::None)),
        "not_found"
    );
    // The grant itself goes on.
    assert_eq!(agent_code(app.core.agent_list(grant)), "ok");
    assert!(app.agent_ids(grant).is_empty());
}

#[test]
fn a_request_about_a_note_the_owner_trashed_or_deleted_ends_at_once() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (_, agent) = a_mixed_vault(&mut app, grant);
    let website = || agent_edit(None, json!({"website": "https://other.example.test"}));
    let asks: [(&str, &str); 3] = [
        ("delete", "api_key"),
        ("replace", "text"),
        ("edit", "login"),
    ];
    for (ask, kind) in asks {
        let id = match kind {
            "api_key" => agent[0].clone(),
            "login" => agent[1].clone(),
            _ => agent[2].clone(),
        };
        let request = match ask {
            "delete" => app.core.agent_delete(grant, &id).unwrap(),
            "replace" => app.core.agent_replace(grant, &id).unwrap(),
            _ => pending_request(app.core.agent_edit(grant, &id, website()).unwrap()),
        };
        assert_eq!(app.core.agent_pending().unwrap().request, ask);
        // The owner moves the Note to Trash in the normal way, not through the request.
        app.core.trash_note(&id, None).unwrap();
        assert!(app.core.agent_pending().is_none(), "{ask}");
        assert_eq!(
            app.core.agent_take_outcome(&request),
            Some(AgentOutcome::Cancelled),
            "{ask}"
        );
        // Nothing is left holding the route: a new request isn't `busy`, an edit at once isn't
        // `not_now`.
        let store = app
            .core
            .agent_store(grant, suggestion("text", "Fake next", json!({})))
            .unwrap();
        app.core.agent_cancel_request(&store);
        app.core.restore_note(&id).unwrap();
        assert_eq!(
            app.core
                .agent_edit(grant, &id, agent_edit(Some("Fake renamed"), json!({}))),
            Ok(AgentEditReply::Done),
            "{ask}"
        );
    }
    // Deleted for good: the same.
    let request = app.core.agent_delete(grant, &agent[0]).unwrap();
    app.core.trash_note(&agent[0], None).unwrap();
    app.core.delete_forever(&agent[0], None).unwrap();
    assert!(app.core.agent_pending().is_none());
    assert_eq!(
        app.core.agent_take_outcome(&request),
        Some(AgentOutcome::Cancelled)
    );
    let store = app
        .core
        .agent_store(grant, suggestion("text", "Fake last", json!({})))
        .unwrap();
    app.core.agent_cancel_request(&store);
}

#[test]
fn a_request_ending_closes_the_panel_opened_for_it() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let id = app.agent_adds(
        grant,
        "api_key",
        "Fake agent RPC",
        json!({}),
        json!({"key": "fake-agent-key-4502"}),
    );
    let request = app.core.agent_delete(grant, &id).unwrap();
    let operation = app
        .core
        .begin_native(NativeAction::AgentTrash {
            request_id: request.clone(),
        })
        .unwrap();
    assert!(operation.requires_credentials);
    assert_eq!(operation.action, "agent_trash");
    app.core.agent_grant_stop();
    assert_eq!(app.core.native_pending_id(), None);
    assert!(
        app.core
            .prepare_native(operation.id, NativeCredentials::Password(pw(PASSWORD)))
            .is_err()
    );
    assert!(app.core.get_note(&id).unwrap().deleted_at.is_none());
}

// ---------- what the owner's screens get (src/api.ts) ----------

/// The keys of a JSON object, sorted.
fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

/// The request waiting now, as the screens get it.
fn pending_json(app: &mut App) -> Value {
    serde_json::to_value(app.core.agent_pending().expect("a request waiting")).unwrap()
}

#[test]
fn the_screens_get_the_shapes_api_ts_expects_and_never_a_value() {
    let mut app = App::unlocked();
    let grant = app.grant();
    let (owner, agent) = a_mixed_vault(&mut app, grant);
    let pending_keys = [
        "changes",
        "expires_in_ms",
        "kind",
        "needs_password",
        "note_id",
        "request",
        "request_id",
        "title",
        "visible",
    ];

    // A store: the suggestion, no Note yet.
    let request = app
        .core
        .agent_store(
            grant,
            suggestion(
                "private_key",
                "Fake agent Solana key",
                json!({"chain": "solana"}),
            ),
        )
        .unwrap();
    let store = pending_json(&mut app);
    assert_eq!(keys(&store), pending_keys);
    assert_eq!(store["request"], "store");
    assert_eq!(store["request_id"], request.as_str());
    assert_eq!(store["kind"], "private_key");
    assert_eq!(store["note_id"], Value::Null);
    assert_eq!(store["visible"], json!({"chain": "solana"}));
    assert_eq!(store["changes"], json!([]));
    assert_eq!(store["needs_password"], true);
    app.core.agent_cancel_request(&request);
    assert!(app.core.agent_pending().is_none());

    // An edit that says where a key is used: field by field, from and to.
    let (login, seed) = (&agent[1], &agent[3]);
    let request = pending_request(
        app.core
            .agent_edit(
                grant,
                login,
                agent_edit(None, json!({"website": "https://other.example.test"})),
            )
            .unwrap(),
    );
    let edit = pending_json(&mut app);
    assert_eq!(keys(&edit), pending_keys);
    assert_eq!(edit["request"], "edit");
    assert_eq!(edit["note_id"], login.as_str());
    assert_eq!(
        edit["changes"],
        json!([{"field": "website", "from": "https://login.example.test", "to": "https://other.example.test"}])
    );
    assert_eq!(edit["needs_password"], false);
    app.core.agent_cancel_request(&request);

    // Replace and delete: the Note's title and visible details, and a delete needs the password.
    for request_name in ["replace", "delete"] {
        // One request at a time: each is asked only once the one before is cancelled.
        let request = if request_name == "replace" {
            app.core.agent_replace(grant, seed)
        } else {
            app.core.agent_delete(grant, seed)
        }
        .unwrap();
        let needs_password = true;
        let shown = pending_json(&mut app);
        assert_eq!(keys(&shown), pending_keys);
        assert_eq!(shown["request"], request_name);
        assert_eq!(shown["needs_password"], needs_password);
        assert_no_secret(&shown.to_string());
        // A Seed Phrase's view never says whether it has a Passphrase, or how many words.
        assert!(!shown.to_string().contains("passphrase"));
        assert!(!shown.to_string().contains("word_count"));
        app.core.agent_cancel_request(&request);
    }

    // Rows and the detail view carry the mark, as a boolean, for the agent's Notes only.
    let rows = serde_json::to_value(app.core.list_notes(notes::Filter::All, "").unwrap()).unwrap();
    for row in rows.as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        assert_eq!(row["added_by_agent"], json!(agent.iter().any(|a| a == id)));
    }
    for (id, marked) in [(&owner[0], false), (&agent[0], true)] {
        let detail = serde_json::to_value(app.core.get_note(id).unwrap()).unwrap();
        assert_eq!(detail["added_by_agent"], json!(marked));
    }
}
