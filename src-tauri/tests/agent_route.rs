//! The agents' local route (ADR-0012): the request protocol, the requests run against a real
//! AppCore (temp data folder, tiny Argon2 parameters, fake clock and clipboard), and on a Mac the
//! listener itself on a socket in a temp folder, answering only the uid it was given. Every value
//! below is an obviously fake placeholder.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use encrypted_note_app::agent_route::{
    self, Host, Limits, Request, RouteError, check_account, error_reply, handle, parse_request,
};
use encrypted_note_app::core::agent::{AgentEdit, AgentError, AgentStoreSuggestion};
use encrypted_note_app::core::native::{NativeAction, NativeCredentials, NativeResult};
use encrypted_note_app::core::{ApiError, AppCore, Clipboard, Clock, Config, NoteInput};
use serde_json::{Value, json};
use vault_format::{KdfParams, Limits as VaultLimits};
use zeroize::Zeroizing;

const PASSWORD: &str = "fake-lantern-quartz-mosaic-velvet-orbit";
const START_UNIX: i64 = 1_790_000_000;
const OWNER_SECRET: &str = "fake-owner-key-7701";
const AGENT_SECRET: &str = "fake-agent-key-7702";
const OWNER_TITLE: &str = "Fake owner exchange";

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

struct FakeClipboard;

impl Clipboard for FakeClipboard {
    fn write(&mut self, _text: &str) -> Result<u64, ApiError> {
        Ok(1)
    }
    fn clear_if(&mut self, _change_count: u64) -> bool {
        true
    }
}

/// The app as the route sees it: one core behind a lock, and a signal when a request waits for
/// the owner.
struct TestHost {
    core: Mutex<AppCore>,
    arrived: Mutex<Sender<()>>,
    changes: AtomicU64,
    _temp: tempfile::TempDir,
}

impl Host for TestHost {
    fn with_core<T>(&self, f: impl FnOnce(&mut AppCore) -> T) -> T {
        f(&mut self.core.lock().unwrap())
    }
    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }
    fn request_arrived(&self) {
        let _ = self.arrived.lock().unwrap().send(());
    }
}

/// A new Vault, Unlocked, with one owner Note and one agent-added Note, and a running grant.
fn unlocked() -> (Arc<TestHost>, u64, String, Receiver<()>) {
    let temp = tempfile::tempdir().unwrap();
    let mut core = AppCore::new(Config {
        data_dir: temp.path().join("Vault.noindex"),
        home_dir: temp.path().join("home"),
        limits: VaultLimits::TESTING,
        clock: Box::new(FakeClock(Arc::new(AtomicU64::new(1_000)))),
        clipboard: Box::new(FakeClipboard),
        activity_anywhere_supported: true,
    })
    .unwrap();
    let key = core
        .create_vault(pw(PASSWORD), KdfParams::TESTING)
        .unwrap()
        .recovery_key
        .unwrap();
    core.confirm_recovery_kit(&key[key.len() - 9..]).unwrap();
    core.create_note(
        input(
            "api_key",
            OWNER_TITLE,
            json!({"service": "fake-owner-service"}),
            json!({"key": OWNER_SECRET}),
        ),
        None,
    )
    .unwrap();
    let grant = core.agent_grant_start().unwrap().id;
    let request_id = core
        .agent_store(grant, suggestion("api_key", "Fake agent key"))
        .unwrap();
    let NativeResult::NoteId(agent_id) = native_run(
        &mut core,
        NativeAction::AgentStore {
            request_id,
            input: input(
                "api_key",
                "Fake agent key",
                json!({"service": "fake-agent-service"}),
                json!({"key": AGENT_SECRET}),
            ),
        },
    ) else {
        panic!("a Note id expected")
    };
    let (sender, receiver) = mpsc::channel();
    let host = Arc::new(TestHost {
        core: Mutex::new(core),
        arrived: Mutex::new(sender),
        changes: AtomicU64::new(0),
        _temp: temp,
    });
    (host, grant, agent_id, receiver)
}

fn pw(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_owned())
}

fn input(kind: &str, title: &str, visible: Value, hidden: Value) -> NoteInput {
    serde_json::from_value(
        json!({ "kind": kind, "title": title, "visible": visible, "hidden": hidden }),
    )
    .unwrap()
}

fn suggestion(kind: &str, title: &str) -> AgentStoreSuggestion {
    AgentStoreSuggestion {
        kind: serde_json::from_value(json!(kind)).unwrap(),
        title: title.to_owned(),
        visible: Default::default(),
    }
}

fn native_run(core: &mut AppCore, action: NativeAction) -> NativeResult {
    let operation = core.begin_native(action).unwrap();
    let job = core
        .prepare_native(operation.id, NativeCredentials::None)
        .unwrap();
    core.finish_native(job.run()).unwrap()
}

fn fast() -> Limits {
    Limits {
        max_line_bytes: 16 * 1024,
        read_timeout: Duration::from_secs(5),
        owner_wait: Duration::from_secs(20),
        max_connections: 4,
        poll: Duration::from_millis(10),
    }
}

fn reply(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

/// Nothing the agent gets back may hold a value or anything about the owner's own Notes.
fn assert_clean(line: &str) {
    for secret in [
        OWNER_SECRET,
        AGENT_SECRET,
        OWNER_TITLE,
        "fake-owner-service",
        PASSWORD,
    ] {
        assert!(!line.contains(secret), "{secret} leaked in {line}");
    }
}

fn run(host: &Arc<TestHost>, grant: u64, line: &str) -> String {
    let request = match parse_request(line.as_bytes()) {
        Ok(request) => request,
        Err(error) => return error_reply(error),
    };
    handle(&**host, grant, request, &fast(), &AtomicBool::new(false))
}

// ---------- the protocol ----------

#[test]
fn each_of_the_five_requests_is_read() {
    assert_eq!(parse_request(br#"{"v":1,"op":"list"}"#), Ok(Request::List));
    assert_eq!(
        parse_request(
            br#"{"v":1,"op":"store","kind":"login","title":"Fake site","details":{"website":"example.invalid"}}"#
        ),
        Ok(Request::Store(AgentStoreSuggestion {
            kind: serde_json::from_value(json!("login")).unwrap(),
            title: "Fake site".to_owned(),
            visible: [("website".to_owned(), "example.invalid".to_owned())].into(),
        }))
    );
    assert_eq!(
        parse_request(br#"{"op":"edit","v":1,"id":"fake-id","title":"New name"}"#),
        Ok(Request::Edit {
            id: "fake-id".to_owned(),
            edit: AgentEdit {
                title: Some("New name".to_owned()),
                visible: Default::default(),
            },
        })
    );
    assert_eq!(
        parse_request(br#"{"v":1,"op":"replace","id":"fake-id"}"#),
        Ok(Request::Replace {
            id: "fake-id".to_owned()
        })
    );
    assert_eq!(
        parse_request(br#"{"v":1,"op":"delete","id":"fake-id"}"#),
        Ok(Request::Delete {
            id: "fake-id".to_owned()
        })
    );
}

#[test]
fn anything_else_is_invalid_and_no_field_can_carry_a_value() {
    for line in [
        // Not JSON, not an object, not UTF-8.
        &b"not json"[..],
        b"[1]",
        b"{\"v\":1,\"op\":\"list\",\"x\":\"\xff\"}",
        // Wrong or missing version, missing or unknown op.
        br#"{"op":"list"}"#,
        br#"{"v":2,"op":"list"}"#,
        br#"{"v":"1","op":"list"}"#,
        br#"{"v":1}"#,
        br#"{"v":1,"op":"show"}"#,
        br#"{"v":1,"op":"copy","id":"fake-id"}"#,
        br#"{"v":1,"op":"export"}"#,
        br#"{"v":1,"op":"sign","id":"fake-id"}"#,
        // A value, in any field name, is never part of a request.
        br#"{"v":1,"op":"store","kind":"api_key","title":"t","value":"fake-value-1"}"#,
        br#"{"v":1,"op":"store","kind":"api_key","title":"t","hidden":{"key":"fake-value-2"}}"#,
        br#"{"v":1,"op":"replace","id":"fake-id","value":"fake-value-3"}"#,
        br#"{"v":1,"op":"edit","id":"fake-id","secret":"fake-value-4"}"#,
        br#"{"v":1,"op":"list","all":true}"#,
        br#"{"v":1,"op":"delete","id":"fake-id","forever":true}"#,
        // Details are text, and the Kind must be one of the five.
        br#"{"v":1,"op":"store","kind":"api_key","title":"t","details":{"purpose":5}}"#,
        br#"{"v":1,"op":"store","kind":"wallet","title":"t"}"#,
        br#"{"v":1,"op":"store","title":"t"}"#,
        br#"{"v":1,"op":"edit","title":"t"}"#,
    ] {
        assert_eq!(
            parse_request(line),
            Err(AgentError::Invalid),
            "{}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn a_refusal_is_one_fixed_code() {
    for error in [
        AgentError::Locked,
        AgentError::Off,
        AgentError::NotFound,
        AgentError::NotAllowed,
        AgentError::Busy,
        AgentError::NotNow,
        AgentError::Cancelled,
        AgentError::Invalid,
        AgentError::Internal,
    ] {
        assert_eq!(
            reply(&error_reply(error)),
            json!({"v": 1, "error": error.code()})
        );
    }
}

// ---------- the requests against the core ----------

#[test]
fn list_shows_only_the_agents_notes_and_never_a_value() {
    let (host, grant, agent_id, _arrived) = unlocked();
    let line = run(&host, grant, r#"{"v":1,"op":"list"}"#);
    assert_clean(&line);
    let reply = reply(&line);
    assert_eq!(reply["ok"], json!(true));
    let keys = reply["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["id"], json!(agent_id));
    assert_eq!(keys[0]["visible"], json!({"service": "fake-agent-service"}));
}

#[test]
fn requests_need_the_running_grant() {
    let (host, grant, agent_id, _arrived) = unlocked();
    assert_eq!(
        reply(&run(&host, grant + 1, r#"{"v":1,"op":"list"}"#)),
        json!({"v":1,"error":"off"})
    );
    host.with_core(AppCore::agent_grant_stop);
    for line in [
        r#"{"v":1,"op":"list"}"#.to_owned(),
        format!(r#"{{"v":1,"op":"delete","id":"{agent_id}"}}"#),
    ] {
        assert_eq!(
            reply(&run(&host, grant, &line)),
            json!({"v":1,"error":"off"})
        );
    }
}

#[test]
fn the_owners_notes_and_missing_ids_are_not_found_alike() {
    let (host, grant, _agent_id, _arrived) = unlocked();
    let owner_id = host.with_core(|core| {
        core.list_notes(notes::Filter::All, "")
            .unwrap()
            .into_iter()
            .find(|n| !n.added_by_agent)
            .unwrap()
            .id
    });
    for id in [owner_id.as_str(), "00000000000000000000000000000000"] {
        for op in ["replace", "delete"] {
            let line = run(
                &host,
                grant,
                &format!(r#"{{"v":1,"op":"{op}","id":"{id}"}}"#),
            );
            assert_eq!(reply(&line), json!({"v":1,"error":"not_found"}));
        }
        let line = run(
            &host,
            grant,
            &format!(r#"{{"v":1,"op":"edit","id":"{id}","title":"Fake renamed"}}"#),
        );
        assert_eq!(reply(&line), json!({"v":1,"error":"not_found"}));
    }
}

#[test]
fn an_edit_that_applies_at_once_is_done() {
    let (host, grant, agent_id, _arrived) = unlocked();
    let line = run(
        &host,
        grant,
        &format!(r#"{{"v":1,"op":"edit","id":"{agent_id}","details":{{"purpose":"Fake CI"}}}}"#),
    );
    assert_eq!(reply(&line), json!({"v":1,"ok":true,"result":"done"}));
    assert!(host.changes.load(Ordering::SeqCst) >= 1);
    let note = host.with_core(|core| core.get_note(&agent_id).unwrap());
    assert_eq!(note.visible["purpose"], "Fake CI");
}

#[test]
fn a_hidden_field_in_details_is_not_allowed() {
    let (host, grant, agent_id, _arrived) = unlocked();
    for name in ["key", "secret"] {
        let line = run(
            &host,
            grant,
            &format!(r#"{{"v":1,"op":"edit","id":"{agent_id}","details":{{"{name}":"fake-v"}}}}"#),
        );
        assert_eq!(reply(&line), json!({"v":1,"error":"not_allowed"}));
        let line = run(
            &host,
            grant,
            &format!(
                r#"{{"v":1,"op":"store","kind":"api_key","title":"Fake","details":{{"{name}":"fake-v"}}}}"#
            ),
        );
        assert_eq!(reply(&line), json!({"v":1,"error":"not_allowed"}));
    }
}

/// A store waits for the owner; the reply is the new Note's id and nothing about the value.
#[test]
fn a_store_waits_for_the_owner_then_says_saved() {
    let (host, grant, _agent_id, arrived) = unlocked();
    let waiting = {
        let host = host.clone();
        std::thread::spawn(move || {
            run(
                &host,
                grant,
                r#"{"v":1,"op":"store","kind":"api_key","title":"Fake second key","details":{"service":"fake-svc"}}"#,
            )
        })
    };
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    let new_id = host.with_core(|core| {
        let pending = core.agent_pending().unwrap();
        assert_eq!(pending.request, "store");
        let NativeResult::NoteId(id) = native_run(
            core,
            NativeAction::AgentStore {
                request_id: pending.request_id,
                input: input(
                    "api_key",
                    "Fake second key",
                    json!({"service": "fake-svc"}),
                    json!({"key": "fake-typed-by-owner-7703"}),
                ),
            },
        ) else {
            panic!("a Note id expected")
        };
        id
    });
    let line = waiting.join().unwrap();
    assert!(!line.contains("fake-typed-by-owner-7703"));
    assert_eq!(
        reply(&line),
        json!({"v":1,"ok":true,"result":"saved","id":new_id})
    );
}

#[test]
fn the_owners_cancel_is_cancelled() {
    let (host, grant, agent_id, arrived) = unlocked();
    let waiting = {
        let host = host.clone();
        std::thread::spawn(move || {
            run(
                &host,
                grant,
                &format!(r#"{{"v":1,"op":"delete","id":"{agent_id}"}}"#),
            )
        })
    };
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    host.with_core(|core| {
        let id = core.agent_pending().unwrap().request_id;
        core.agent_cancel_request(&id);
    });
    assert_eq!(
        reply(&waiting.join().unwrap()),
        json!({"v":1,"error":"cancelled"})
    );
}

#[test]
fn a_request_nobody_answers_is_cancelled_in_the_core_too() {
    let (host, grant, agent_id, _arrived) = unlocked();
    let limits = Limits {
        owner_wait: Duration::from_millis(50),
        ..fast()
    };
    let line = handle(
        &*host,
        grant,
        Request::Replace { id: agent_id },
        &limits,
        &AtomicBool::new(false),
    );
    assert_eq!(reply(&line), json!({"v":1,"error":"cancelled"}));
    assert!(host.with_core(AppCore::agent_pending).is_none());
}

#[test]
fn stopping_the_route_cancels_a_waiting_request() {
    let (host, grant, agent_id, _arrived) = unlocked();
    let line = handle(
        &*host,
        grant,
        Request::Delete { id: agent_id },
        &fast(),
        &AtomicBool::new(true),
    );
    assert_eq!(reply(&line), json!({"v":1,"error":"cancelled"}));
    assert!(host.with_core(AppCore::agent_pending).is_none());
}

#[test]
fn a_second_request_while_one_waits_is_busy() {
    let (host, grant, agent_id, arrived) = unlocked();
    let waiting = {
        let host = host.clone();
        let id = agent_id.clone();
        std::thread::spawn(move || {
            run(
                &host,
                grant,
                &format!(r#"{{"v":1,"op":"replace","id":"{id}"}}"#),
            )
        })
    };
    arrived.recv_timeout(Duration::from_secs(10)).unwrap();
    let line = run(
        &host,
        grant,
        r#"{"v":1,"op":"store","kind":"text","title":"Fake note"}"#,
    );
    assert_eq!(reply(&line), json!({"v":1,"error":"busy"}));
    host.with_core(AppCore::agent_grant_stop);
    assert_eq!(
        reply(&waiting.join().unwrap()),
        json!({"v":1,"error":"cancelled"})
    );
}

// ---------- the agent account and its settings ----------

#[test]
fn only_a_standard_account_other_than_the_owners_is_accepted() {
    assert_eq!(check_account(502, 501, &[20]), Ok(()));
    assert_eq!(
        check_account(501, 501, &[20]),
        Err(RouteError::AccountIsOwner)
    );
    for (uid, groups) in [
        (0, &[0u32][..]),
        (248, &[20][..]),
        (500, &[20][..]),
        (2_147_483_648, &[20][..]),
        (502, &[20, 80][..]),
        (502, &[0][..]),
        (502, &[5, 20][..]),
    ] {
        assert_eq!(
            check_account(uid, 501, groups),
            Err(RouteError::AccountNotStandard),
            "{uid} {groups:?}"
        );
    }
}

#[test]
fn the_agent_account_is_saved_checked_and_ends_the_grant() {
    let (host, _grant, _agent_id, _arrived) = unlocked();
    host.with_core(|core| {
        assert_eq!(core.agent_account(), None);
        for bad in [
            "Root",
            "1agent",
            "fake agent",
            "fake/agent",
            &"a".repeat(32),
        ] {
            assert_eq!(
                core.set_agent_account(Some(bad.to_owned()))
                    .map_err(|e| e.code),
                Err("invalid_input"),
                "{bad}"
            );
        }
        assert_eq!(core.agent_grant_status().state, "running");
        core.set_agent_account(Some(" fake-agent ".to_owned()))
            .unwrap();
        assert_eq!(core.agent_account().as_deref(), Some("fake-agent"));
        assert_eq!(core.agent_grant_status().state, "off");
        core.set_agent_account(Some(String::new())).unwrap();
        assert_eq!(core.agent_account(), None);
        core.lock();
        assert_eq!(
            core.set_agent_account(Some("fake-agent".to_owned()))
                .map_err(|e| e.code),
            Err("locked")
        );
    });
}

// ---------- the listener (macOS) ----------

#[cfg(target_os = "macos")]
mod listener {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    use encrypted_note_app::agent_route::{AgentRoute, SOCKET_NAME, check_folder, expected_uid};

    use super::*;

    /// This test's own uid: the owner of a file it just made.
    fn own_uid() -> u32 {
        use std::os::unix::fs::MetadataExt;
        tempfile::tempfile().unwrap().metadata().unwrap().uid()
    }

    /// A private folder like the owner's setup script makes, inside a private temp folder.
    fn folder() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let dir = root.join("keys");
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        (temp, dir)
    }

    fn ask(dir: &Path, line: &str) -> String {
        let mut stream = UnixStream::connect(dir.join(SOCKET_NAME)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        reply
    }

    #[test]
    fn the_folder_must_be_the_owners_private_folder() {
        let (_temp, dir) = folder();
        assert_eq!(check_folder(&dir), Ok(()));
        // Relative, missing, open to others, or a link to a private folder: refused.
        assert_eq!(check_folder(Path::new("keys")), Err(RouteError::NotSetUp));
        assert_eq!(
            check_folder(&dir.join("missing")),
            Err(RouteError::NotSetUp)
        );
        for mode in [0o750, 0o755, 0o701, 0o1700, 0o600] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(check_folder(&dir), Err(RouteError::NotSetUp), "{mode:o}");
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.parent().unwrap().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert_eq!(check_folder(&link), Err(RouteError::NotSetUp));
        // A parent anyone can write in (and not root's sticky one) could swap the folder out.
        let parent = dir.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(check_folder(&dir), Err(RouteError::NotSetUp));
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(check_folder(&dir), Ok(()));
    }

    #[test]
    fn the_real_folder_path_is_fixed() {
        assert_eq!(agent_route::SOCKET_DIR, "/Users/Shared/encrypted-note-keys");
        if std::env::var_os("ENOTE_DEV_AGENT_SOCKET_DIR").is_none() {
            assert_eq!(
                agent_route::socket_dir(),
                Path::new(agent_route::SOCKET_DIR)
            );
        }
    }

    #[test]
    fn accounts_are_looked_up_and_refused_by_the_rules() {
        if std::env::var_os("ENOTE_DEV_AGENT_UID").is_some() {
            return;
        }
        assert_eq!(expected_uid(None), Err(RouteError::NoAccount));
        assert_eq!(
            expected_uid(Some("fake-no-such-account-7704")),
            Err(RouteError::AccountNotFound)
        );
        assert_eq!(
            expected_uid(Some("Not A Name")),
            Err(RouteError::AccountNotFound)
        );
        assert_eq!(
            expected_uid(Some("root")),
            Err(RouteError::AccountNotStandard)
        );
        assert_eq!(
            expected_uid(Some("daemon")),
            Err(RouteError::AccountNotStandard)
        );
        if let Ok(me) = std::env::var("USER")
            && encrypted_note_app::core::agent::is_account_name(&me)
            && agent_route::lookup_account(&me).is_ok_and(|(uid, _)| uid == own_uid())
        {
            assert_eq!(expected_uid(Some(&me)), Err(RouteError::AccountIsOwner));
        }
    }

    #[test]
    fn the_route_answers_the_expected_uid_and_cleans_up() {
        let (_temp, dir) = folder();
        let (host, grant, agent_id, _arrived) = unlocked();
        let route = AgentRoute::default();
        route
            .start(host.clone(), grant, own_uid(), &dir, fast())
            .unwrap();
        assert!(route.is_active());
        let socket = dir.join(SOCKET_NAME);
        let meta = std::fs::symlink_metadata(&socket).unwrap();
        assert!(std::os::unix::fs::FileTypeExt::is_socket(&meta.file_type()));
        assert_eq!(meta.permissions().mode() & 0o777, 0o666);

        let line = ask(&dir, r#"{"v":1,"op":"list"}"#);
        assert!(line.ends_with('\n'));
        assert_clean(&line);
        assert_eq!(reply(&line)["keys"][0]["id"], json!(agent_id));
        assert_eq!(
            reply(&ask(&dir, r#"{"v":1,"op":"show","id":"x"}"#)),
            json!({"v":1,"error":"invalid"})
        );

        route.stop();
        assert!(!route.is_active());
        assert!(std::fs::symlink_metadata(&socket).is_err());
        assert!(UnixStream::connect(&socket).is_err());
    }

    #[test]
    fn anyone_else_is_closed_without_a_reply() {
        let (_temp, dir) = folder();
        let (host, grant, _agent_id, _arrived) = unlocked();
        let route = AgentRoute::default();
        // As if the agent account were someone else: this test's own uid is then a stranger,
        // exactly as the owner's own programs are to the real route.
        route
            .start(host, grant, own_uid() + 1, &dir, fast())
            .unwrap();
        let mut stream = UnixStream::connect(dir.join(SOCKET_NAME)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let _ = stream.write_all(b"{\"v\":1,\"op\":\"list\"}\n");
        let mut got = Vec::new();
        let _ = stream.read_to_end(&mut got);
        assert!(got.is_empty(), "{}", String::from_utf8_lossy(&got));
        route.stop();
    }

    #[test]
    fn a_long_or_slow_line_is_invalid() {
        let (_temp, dir) = folder();
        let (host, grant, _agent_id, _arrived) = unlocked();
        let route = AgentRoute::default();
        let limits = Limits {
            read_timeout: Duration::from_millis(300),
            ..fast()
        };
        route.start(host, grant, own_uid(), &dir, limits).unwrap();
        let long = format!(
            r#"{{"v":1,"op":"list","pad":"{}"}}"#,
            "x".repeat(16 * 1024 + 1)
        );
        assert_eq!(reply(&ask(&dir, &long)), json!({"v":1,"error":"invalid"}));
        // Connected, but the line never comes.
        let stream = UnixStream::connect(dir.join(SOCKET_NAME)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reply_line = String::new();
        BufReader::new(stream).read_line(&mut reply_line).unwrap();
        assert_eq!(reply(&reply_line), json!({"v":1,"error":"invalid"}));
        route.stop();
    }

    #[test]
    fn connections_over_the_limit_are_busy() {
        let (_temp, dir) = folder();
        let (host, grant, _agent_id, _arrived) = unlocked();
        let route = AgentRoute::default();
        let limits = Limits {
            max_connections: 1,
            ..fast()
        };
        route.start(host, grant, own_uid(), &dir, limits).unwrap();
        // Holds the one slot by never sending its line.
        let _first = UnixStream::connect(dir.join(SOCKET_NAME)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            reply(&ask(&dir, r#"{"v":1,"op":"list"}"#)),
            json!({"v":1,"error":"busy"})
        );
        route.stop();
    }

    #[test]
    fn a_stale_socket_of_ours_is_replaced_but_nothing_else_is() {
        let (_temp, dir) = folder();
        let (host, grant, _agent_id, _arrived) = unlocked();
        let socket = dir.join(SOCKET_NAME);
        // Left behind by a crash: a socket nobody listens on.
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        let route = AgentRoute::default();
        route
            .start(host.clone(), grant, own_uid(), &dir, fast())
            .unwrap();
        route.stop();
        // A file that isn't a socket where the socket goes: refused, and left alone.
        std::fs::write(&socket, "fake-not-a-socket").unwrap();
        assert_eq!(
            route.start(host.clone(), grant, own_uid(), &dir, fast()),
            Err(RouteError::NotSetUp)
        );
        assert_eq!(
            std::fs::read_to_string(&socket).unwrap(),
            "fake-not-a-socket"
        );
        std::fs::remove_file(&socket).unwrap();
        // The socket replaced by something else while running: stop leaves that alone.
        route.start(host, grant, own_uid(), &dir, fast()).unwrap();
        std::fs::remove_file(&socket).unwrap();
        std::fs::write(&socket, "fake-replacement").unwrap();
        route.stop();
        assert_eq!(
            std::fs::read_to_string(&socket).unwrap(),
            "fake-replacement"
        );
    }

    #[test]
    fn a_route_in_an_unprepared_folder_does_not_start() {
        let (_temp, dir) = folder();
        let (host, grant, _agent_id, _arrived) = unlocked();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let route = AgentRoute::default();
        assert_eq!(
            route.start(host, grant, own_uid(), &dir, fast()),
            Err(RouteError::NotSetUp)
        );
        assert!(!route.is_active());
        assert!(std::fs::symlink_metadata(dir.join(SOCKET_NAME)).is_err());
    }

    /// The agent's real tool against the real listener, when it has been built (a workspace
    /// test run builds it; a run of this crate alone may not, and then this checks nothing).
    #[test]
    fn the_agents_tool_talks_to_the_route() {
        use std::process::{Command, Stdio};
        let tool = std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(Path::parent)
            .map(|debug| debug.join("enote-agent-keys"));
        let Some(tool) = tool.filter(|tool| tool.is_file()) else {
            return;
        };
        let (_temp, dir) = folder();
        let (host, grant, agent_id, _arrived) = unlocked();
        let route = AgentRoute::default();
        route
            .start(host.clone(), grant, own_uid(), &dir, fast())
            .unwrap();
        let mut child = Command::new(tool)
            .args(["--owner-uid", &own_uid().to_string(), "--socket"])
            .arg(dir.join(SOCKET_NAME))
            .env_remove("ENOTE_OWNER_UID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        for message in [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_keys","arguments":{}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"edit_key_details","arguments":{"id":agent_id,"details":{"purpose":"Fake from the tool"}}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"edit_key_details","arguments":{"id":agent_id,"details":{"key":"fake-value-7707"}}}}),
        ] {
            writeln!(stdin, "{message}").unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        route.stop();
        let out = String::from_utf8(output.stdout).unwrap();
        assert_clean(&out);
        let lines: Vec<Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4, "{out}");
        // Each tools/call is answered on its own thread, so replies come in any order: by id.
        let reply = |id: u64| {
            lines
                .iter()
                .find(|line| line["id"] == json!(id))
                .unwrap_or_else(|| panic!("no reply {id}: {out}"))
        };
        let text = |id: u64| {
            reply(id)["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert!(text(2).contains(&agent_id), "{}", text(2));
        assert_eq!(text(3), "Done.");
        assert_eq!(reply(4)["result"]["isError"], json!(true));
        assert!(!text(4).contains("fake-value-7707"));
        let note = host.with_core(|core| core.get_note(&agent_id).unwrap());
        assert_eq!(note.visible["purpose"], "Fake from the tool");
    }

    #[test]
    fn a_store_over_the_socket_waits_for_the_owner() {
        let (_temp, dir) = folder();
        let (host, grant, _agent_id, arrived) = unlocked();
        let route = AgentRoute::default();
        route
            .start(host.clone(), grant, own_uid(), &dir, fast())
            .unwrap();
        let asking = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                ask(
                    &dir,
                    r#"{"v":1,"op":"store","kind":"text","title":"Fake agent note"}"#,
                )
            })
        };
        arrived.recv_timeout(Duration::from_secs(10)).unwrap();
        host.with_core(|core| {
            let id = core.agent_pending().unwrap().request_id;
            core.agent_cancel_request(&id);
        });
        assert_eq!(
            reply(&asking.join().unwrap()),
            json!({"v":1,"error":"cancelled"})
        );
        route.stop();
    }
}
