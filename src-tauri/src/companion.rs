//! The optional Agent Companion (ADR-0009), supervised apart from the Vault core.
//!
//! The helper is a separate XPC service with its own App Sandbox container: it cannot open the
//! Vault's container and holds only a separately created Agent Key, never anything from the
//! Vault. This module only asks it for status, start, stop and owner setup, and turns its four
//! numbers (state, readiness flags, grant expiry, error code) into a fixed view for the screens.
//! Nothing it returns is free text: no path, key, token, helper output or provider error.
//!
//! The broker stops when the owner presses Stop, when the Vault is not Unlocked (lock, sleep,
//! screen lock, user switch), when the grant ends, and when the app exits. Nothing here starts
//! it again by itself: a crash or a helper that started over is reported, not restarted.

use std::fs::File;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

/// The one approved operation, as pinned in `src-tauri/companion/companion.lock.json`. The
/// packaged helper refuses to run unless its bundled policy matches that pinned hash.
pub const OPERATION: OperationView = OperationView {
    name: "generate_text",
    destination: "OpenAI (api.openai.com/v1/responses)",
    model: "gpt-6-luna",
    max_input_bytes: 4096,
    max_output_tokens: 512,
    concurrent: 1,
    per_minute: 6,
    per_hour: 60,
    per_run: 100,
    grant_minutes: 60,
};

const MAX_GRANT_SECONDS: i64 = 3600;
const STATUS_TIMEOUT_MS: i32 = 20_000;
const START_TIMEOUT_MS: i32 = 45_000;
const STOP_TIMEOUT_MS: i32 = 20_000;
const INSTALL_TIMEOUT_MS: i32 = 30_000;

const RUNTIME: u32 = 1 << 0;
const PLAN: u32 = 1 << 1;
const CREDENTIAL: u32 = 1 << 2;
const HANDOFF: u32 = 1 << 3;
const CODE: u32 = 1 << 4;
const AGENT: u32 = 1 << 5;
const ALL_FLAGS: u32 = RUNTIME | PLAN | CREDENTIAL | HANDOFF | CODE | AGENT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Status = 1,
    Start = 2,
    Stop = 3,
    Install = 4,
}

/// The helper's reply, exactly as received.
// Off macOS the helper is never reachable, so no reply or transport failure is ever built there.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raw {
    pub state: i32,
    pub flags: u32,
    pub expires: i64,
    pub error: i32,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// Not a Mac build, or no helper inside this app bundle.
    Unavailable,
    Connection,
    Timeout,
    Invalid,
}

/// How this process reaches the helper. Production: XPC (macOS). Tests: a fake.
pub trait Transport: Send + Sync {
    fn bundled(&self) -> bool;
    fn call(
        &self,
        op: Op,
        files: Option<(&File, &File)>,
        timeout_ms: i32,
    ) -> Result<Raw, TransportError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OperationView {
    pub name: &'static str,
    pub destination: &'static str,
    pub model: &'static str,
    pub max_input_bytes: u32,
    pub max_output_tokens: u32,
    pub concurrent: u32,
    pub per_minute: u32,
    pub per_hour: u32,
    pub per_run: u32,
    pub grant_minutes: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Readiness {
    /// Every part below is ready.
    pub ready: bool,
    pub runtime: bool,
    pub plan: bool,
    pub credential: bool,
    pub handoff: bool,
    pub agent: bool,
    pub code: bool,
}

/// What the screens get: fixed strings, booleans and one timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CompanionView {
    pub platform_supported: bool,
    pub helper_bundled: bool,
    /// unsupported | unavailable | stopped | starting | running | stopping | expired | error
    pub state: &'static str,
    pub error: Option<&'static str>,
    /// Unix seconds; only while running or just expired, and only within one grant.
    pub expires_at: Option<i64>,
    pub local: Readiness,
    /// The private ChatGPT connection is set up and checked outside this app.
    pub chatgpt: &'static str,
    pub operation: OperationView,
}

fn state_name(state: i32) -> Option<&'static str> {
    Some(match state {
        1 => "stopped",
        2 => "starting",
        3 => "running",
        4 => "stopping",
        5 => "expired",
        6 => "error",
        _ => return None,
    })
}

fn error_name(code: i32) -> Option<&'static str> {
    Some(match code {
        0 => return None,
        1 => "plan_missing",
        2 => "plan_invalid",
        3 => "wrong_identity",
        4 => "code_unprotected",
        5 => "agent_unverified",
        6 => "credential_missing",
        7 => "credential_invalid",
        8 => "handoff_unavailable",
        9 => "port_unavailable",
        10 => "grant_inactive",
        11 => "control_invalid",
        20 => "runtime_missing",
        21 => "policy_mismatch",
        22 => "launch_failed",
        23 => "start_timeout",
        24 => "unexpected_exit",
        25 => "protocol_invalid",
        26 => "busy",
        27 => "install_invalid",
        _ => "internal",
    })
}

fn readiness(flags: u32) -> Readiness {
    let flags = flags & ALL_FLAGS;
    Readiness {
        ready: flags == ALL_FLAGS,
        runtime: flags & RUNTIME != 0,
        plan: flags & PLAN != 0,
        credential: flags & CREDENTIAL != 0,
        handoff: flags & HANDOFF != 0,
        agent: flags & AGENT != 0,
        code: flags & CODE != 0,
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

struct Inner {
    /// The owner started a broker that has not been seen to stop yet.
    owner_started: bool,
    last: Option<Raw>,
    /// An error noticed here rather than reported by the helper.
    noticed: Option<&'static str>,
}

pub struct Companion {
    transport: Box<dyn Transport>,
    now: fn() -> i64,
    inner: Mutex<Inner>,
    /// A broker may be running. Lifecycle stops make a helper call only while this is set.
    active: AtomicBool,
}

impl Companion {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self::with_clock(transport, now_unix)
    }

    fn with_clock(transport: Box<dyn Transport>, now: fn() -> i64) -> Self {
        Self {
            transport,
            now,
            inner: Mutex::new(Inner {
                owner_started: false,
                last: None,
                noticed: None,
            }),
            active: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> CompanionView {
        self.run(Op::Status, None, STATUS_TIMEOUT_MS)
    }

    /// The owner's explicit Start: one new grant of at most an hour.
    pub fn start(&self) -> CompanionView {
        self.run(Op::Start, None, START_TIMEOUT_MS)
    }

    pub fn stop(&self) -> CompanionView {
        self.active.store(false, Ordering::SeqCst);
        self.run(Op::Stop, None, STOP_TIMEOUT_MS)
    }

    /// Owner setup: two files the owner chose in OS panels, passed as open descriptors. This
    /// process never reads them.
    pub fn install(&self, plan: &File, credential: &File) -> CompanionView {
        self.run(Op::Install, Some((plan, credential)), INSTALL_TIMEOUT_MS)
    }

    /// Lifecycle stop (lock, sleep, screen lock, user switch, exit): only when a broker may run.
    pub fn stop_if_active(&self) {
        if self.active.load(Ordering::SeqCst) {
            self.stop();
        }
    }

    fn run(&self, op: Op, files: Option<(&File, &File)>, timeout_ms: i32) -> CompanionView {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.transport.bundled() {
            return self.view(&mut inner, Err(TransportError::Unavailable));
        }
        match op {
            Op::Start => {
                inner.owner_started = true;
                inner.noticed = None;
                self.active.store(true, Ordering::SeqCst);
            }
            // An owner action acknowledges an earlier unexpected stop; status refreshes don't.
            Op::Stop | Op::Install => {
                inner.owner_started = false;
                inner.noticed = None;
            }
            Op::Status => {}
        }
        let reply = self.transport.call(op, files, timeout_ms);
        self.view(&mut inner, reply)
    }

    fn view(&self, inner: &mut Inner, reply: Result<Raw, TransportError>) -> CompanionView {
        let platform_supported = cfg!(target_os = "macos");
        let bundled = reply != Err(TransportError::Unavailable);
        let mut view = CompanionView {
            platform_supported,
            helper_bundled: platform_supported && bundled,
            state: if platform_supported {
                "unavailable"
            } else {
                "unsupported"
            },
            error: None,
            expires_at: None,
            local: Readiness::default(),
            chatgpt: "not_verified",
            operation: OPERATION,
        };
        let raw = match reply {
            Ok(raw) => raw,
            Err(error) => {
                if error != TransportError::Unavailable {
                    view.state = "error";
                    view.error = Some(match error {
                        TransportError::Timeout => "helper_timeout",
                        TransportError::Invalid => "invalid_reply",
                        _ => "helper_unreachable",
                    });
                    // The broker's state is unknown: keep lifecycle stops armed while the
                    // owner expected it to run.
                    self.active.store(inner.owner_started, Ordering::SeqCst);
                } else {
                    inner.owner_started = false;
                    self.active.store(false, Ordering::SeqCst);
                }
                return view;
            }
        };
        let Some(state) = state_name(raw.state) else {
            view.state = "error";
            view.error = Some("invalid_reply");
            self.active.store(inner.owner_started, Ordering::SeqCst);
            return view;
        };
        // The helper answers "stopped" with no error after it started over (it was killed or
        // crashed) while a broker should have been running: say so, never restart it.
        if inner.owner_started && state == "stopped" && raw.error == 0 {
            inner.noticed = Some("helper_restarted");
        }
        let running = matches!(state, "starting" | "running" | "stopping");
        if !running {
            inner.owner_started = false;
        }
        self.active.store(running, Ordering::SeqCst);
        inner.last = Some(raw);
        let now = (self.now)();
        view.state = state;
        view.error = error_name(raw.error).or(if state == "stopped" {
            inner.noticed
        } else {
            None
        });
        if view.error == Some("helper_restarted") {
            view.state = "error";
        }
        if matches!(state, "running" | "expired")
            && raw.expires > now - 2 * MAX_GRANT_SECONDS
            && raw.expires <= now + MAX_GRANT_SECONDS + 60
        {
            view.expires_at = Some(raw.expires);
        }
        view.local = readiness(raw.flags);
        view
    }
}

/// Stops a broker that may be running, off the calling thread (lock/sleep events arrive on the
/// main thread or the 500 ms status loop and must not wait for the helper).
pub fn stop_for_lifecycle<R: Runtime>(app: &AppHandle<R>) {
    let Some(companion) = app.try_state::<Companion>() else {
        return;
    };
    if !companion.active.load(Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(companion) = app.try_state::<Companion>() {
            companion.stop_if_active();
        }
    });
}

/// App exit: stop synchronously (bounded by the stop timeout). The helper also stops its
/// broker by itself when this process's connection goes away.
pub fn stop_on_exit<R: Runtime>(app: &AppHandle<R>) {
    if let Some(companion) = app.try_state::<Companion>() {
        companion.stop_if_active();
    }
}

pub fn system_transport() -> Box<dyn Transport> {
    #[cfg(target_os = "macos")]
    {
        Box::new(xpc::XpcTransport)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(Unsupported)
    }
}

/// Windows and other platforms: the helper's boundary is not built or verified there.
#[cfg(not(target_os = "macos"))]
struct Unsupported;

#[cfg(not(target_os = "macos"))]
impl Transport for Unsupported {
    fn bundled(&self) -> bool {
        false
    }

    fn call(&self, _: Op, _: Option<(&File, &File)>, _: i32) -> Result<Raw, TransportError> {
        Err(TransportError::Unavailable)
    }
}

#[cfg(target_os = "macos")]
mod xpc {
    use std::fs::File;
    use std::os::fd::AsRawFd;

    use super::{Op, Raw, Transport, TransportError};

    unsafe extern "C" {
        fn enote_companion_bundled() -> i32;
        fn enote_companion_call(
            operation: i32,
            plan_fd: i32,
            credential_fd: i32,
            timeout_ms: i32,
            state: *mut i32,
            flags: *mut u32,
            expires: *mut i64,
            error: *mut i32,
        ) -> i32;
    }

    pub struct XpcTransport;

    impl Transport for XpcTransport {
        fn bundled(&self) -> bool {
            // SAFETY: no arguments; reads only this app's bundle.
            unsafe { enote_companion_bundled() == 1 }
        }

        fn call(
            &self,
            op: Op,
            files: Option<(&File, &File)>,
            timeout_ms: i32,
        ) -> Result<Raw, TransportError> {
            let (plan, credential) = files.map_or((-1, -1), |(plan, credential)| {
                (plan.as_raw_fd(), credential.as_raw_fd())
            });
            let mut raw = Raw::default();
            // SAFETY: four valid out-pointers to this stack frame; the descriptors stay open
            // (borrowed) for the whole call, and the Swift side duplicates rather than owns them.
            let rc = unsafe {
                enote_companion_call(
                    op as i32,
                    plan,
                    credential,
                    timeout_ms,
                    &mut raw.state,
                    &mut raw.flags,
                    &mut raw.expires,
                    &mut raw.error,
                )
            };
            match rc {
                0 => Ok(raw),
                1 => Err(TransportError::Unavailable),
                3 => Err(TransportError::Timeout),
                4 => Err(TransportError::Invalid),
                _ => Err(TransportError::Connection),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Script {
        replies: Mutex<Vec<Result<Raw, TransportError>>>,
        calls: Mutex<Vec<Op>>,
        bundled: bool,
    }

    struct Fake(Arc<Script>);

    impl Transport for Fake {
        fn bundled(&self) -> bool {
            self.0.bundled
        }
        fn call(&self, op: Op, _: Option<(&File, &File)>, _: i32) -> Result<Raw, TransportError> {
            self.0.calls.lock().unwrap().push(op);
            let mut replies = self.0.replies.lock().unwrap();
            if replies.is_empty() {
                Err(TransportError::Connection)
            } else {
                replies.remove(0)
            }
        }
    }

    const NOW: i64 = 1_793_000_000;

    fn companion(replies: Vec<Result<Raw, TransportError>>) -> (Companion, Arc<Script>) {
        let script = Arc::new(Script {
            replies: Mutex::new(replies),
            calls: Mutex::new(Vec::new()),
            bundled: true,
        });
        (
            Companion::with_clock(Box::new(Fake(Arc::clone(&script))), || NOW),
            script,
        )
    }

    fn raw(state: i32, flags: u32, expires: i64, error: i32) -> Result<Raw, TransportError> {
        Ok(Raw {
            state,
            flags,
            expires,
            error,
        })
    }

    #[test]
    fn a_build_without_the_helper_is_unavailable_and_makes_no_call() {
        let script = Arc::new(Script::default());
        let companion = Companion::with_clock(Box::new(Fake(Arc::clone(&script))), || NOW);
        let view = companion.start();
        assert!(!view.helper_bundled);
        assert_eq!(
            view.state,
            if cfg!(target_os = "macos") {
                "unavailable"
            } else {
                "unsupported"
            }
        );
        assert!(script.calls.lock().unwrap().is_empty());
        assert!(!companion.active.load(Ordering::SeqCst));
    }

    #[test]
    fn readiness_needs_every_part_and_ignores_unknown_bits() {
        let (companion, _) = companion(vec![raw(1, 0b11_1111, 0, 0), raw(1, 0b1101_0111, 0, 0)]);
        let ready = companion.status();
        assert!(ready.local.ready && ready.local.handoff && ready.local.agent);
        let partial = companion.status();
        assert!(!partial.local.ready);
        assert!(partial.local.runtime && partial.local.plan && partial.local.credential);
        assert!(!partial.local.handoff && partial.local.code && !partial.local.agent);
        assert_eq!(partial.chatgpt, "not_verified");
    }

    #[test]
    fn unknown_states_and_codes_become_fixed_errors() {
        let (companion, _) = companion(vec![
            raw(99, 0, 0, 0),
            raw(6, 0, 0, 12345),
            raw(6, 0, 0, 12),
        ]);
        let unknown_state = companion.status();
        assert_eq!(
            (unknown_state.state, unknown_state.error),
            ("error", Some("invalid_reply"))
        );
        assert_eq!(companion.status().error, Some("internal"));
        assert_eq!(companion.status().error, Some("internal"));
    }

    #[test]
    fn expiry_is_shown_only_within_one_grant() {
        let (companion, _) = companion(vec![
            raw(3, 0, NOW + 3599, 0),
            raw(3, 0, NOW + 100_000, 0),
            raw(1, 0, NOW + 60, 0),
            raw(5, 0, NOW - 10, 0),
        ]);
        assert_eq!(companion.status().expires_at, Some(NOW + 3599));
        assert_eq!(companion.status().expires_at, None);
        assert_eq!(companion.status().expires_at, None);
        assert_eq!(companion.status().expires_at, Some(NOW - 10));
    }

    #[test]
    fn start_arms_lifecycle_stops_only_while_a_broker_runs() {
        let (companion, script) = companion(vec![raw(3, 0, NOW + 3600, 0), raw(1, 0, 0, 0)]);
        assert_eq!(companion.start().state, "running");
        assert!(companion.active.load(Ordering::SeqCst));
        companion.stop_if_active();
        assert!(!companion.active.load(Ordering::SeqCst));
        companion.stop_if_active(); // nothing running: no call
        assert_eq!(*script.calls.lock().unwrap(), vec![Op::Start, Op::Stop]);
        let (companion, _) = self::companion(vec![raw(6, 0, 0, 5)]);
        let refused = companion.start();
        assert_eq!(
            (refused.state, refused.error),
            ("error", Some("agent_unverified"))
        );
        assert!(!companion.active.load(Ordering::SeqCst));
    }

    #[test]
    fn a_helper_that_started_over_is_reported_and_never_restarted() {
        let (companion, script) = companion(vec![
            raw(3, 0, NOW + 3600, 0),
            raw(1, 63, 0, 0),
            raw(1, 63, 0, 0),
        ]);
        companion.start();
        let after = companion.status();
        assert_eq!(
            (after.state, after.error),
            ("error", Some("helper_restarted"))
        );
        assert!(!companion.active.load(Ordering::SeqCst));
        // The notice stays until the owner acts, and nothing is started again meanwhile.
        let later = companion.status();
        assert_eq!(
            (later.state, later.error),
            ("error", Some("helper_restarted"))
        );
        assert_eq!(
            *script.calls.lock().unwrap(),
            vec![Op::Start, Op::Status, Op::Status]
        );
    }

    #[test]
    fn crashes_and_expiry_end_the_grant_without_a_restart() {
        let (companion, script) = companion(vec![raw(3, 0, NOW + 3600, 0), raw(6, 0, 0, 24)]);
        companion.start();
        let crashed = companion.status();
        assert_eq!(
            (crashed.state, crashed.error),
            ("error", Some("unexpected_exit"))
        );
        assert!(!companion.active.load(Ordering::SeqCst));
        assert_eq!(*script.calls.lock().unwrap(), vec![Op::Start, Op::Status]);
        let (companion, _) = self::companion(vec![raw(3, 0, NOW + 3600, 0), raw(5, 0, NOW, 0)]);
        companion.start();
        assert_eq!(companion.status().state, "expired");
        assert!(!companion.active.load(Ordering::SeqCst));
    }

    #[test]
    fn transport_failures_are_fixed_errors_and_keep_lifecycle_stops_armed() {
        let (companion, _) = companion(vec![
            raw(3, 0, NOW + 3600, 0),
            Err(TransportError::Timeout),
            Err(TransportError::Connection),
        ]);
        companion.start();
        let timed_out = companion.status();
        assert_eq!(
            (timed_out.state, timed_out.error),
            ("error", Some("helper_timeout"))
        );
        assert!(companion.active.load(Ordering::SeqCst));
        assert_eq!(companion.status().error, Some("helper_unreachable"));
    }

    #[test]
    fn the_view_holds_only_fixed_fields() {
        let (companion, _) = companion(vec![raw(3, 63, NOW + 3000, 0)]);
        let json = serde_json::to_value(companion.start()).unwrap();
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "chatgpt",
                "error",
                "expires_at",
                "helper_bundled",
                "local",
                "operation",
                "platform_supported",
                "state"
            ]
        );
        assert_eq!(json["operation"]["name"], "generate_text");
        assert_eq!(json["operation"]["model"], "gpt-6-luna");
    }
}
