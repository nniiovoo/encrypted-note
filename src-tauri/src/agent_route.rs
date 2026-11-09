//! The local route for agents storing keys (ADR-0012): a Unix domain socket in an owner-made
//! folder, open only while the owner's grant runs, that takes the five agent requests to the
//! core (`core::agent`) and sends back fixed replies. It is not a network socket, it shares
//! nothing with the Agent Companion, and it never carries a value: no request has a field that
//! could hold one, and every reply is a fixed shape (`AgentNote`s, ids, results, error codes).
//!
//! What it guarantees (checked by `tests/agent_route.rs`):
//! * **Only the owner's folder.** `/Users/Shared/encrypted-note-keys` must be a real folder
//!   owned by this account, mode 0700, under parents that are root's (sticky where anyone may
//!   write) or this account's own. The owner's setup script gives the agent account one ACL
//!   entry to reach the socket; nothing else is trusted about the folder.
//! * **Only the agent account.** Every connection's peer uid comes from the kernel
//!   (`getpeereid`). Anything but the expected uid is closed without a reply, including the
//!   owner's own programs. The expected uid is the account the owner named, refused if it is
//!   the owner's own, below 501, or in the admin, wheel or operator group.
//! * **Bounded.** One request per connection: one UTF-8 JSON line of at most 16 KiB, read within
//!   10 s; at most 4 connections at once. Requests the owner completes wait at most 5 minutes.
//! * **Only while the grant runs.** The listener starts with the grant and stops with it, and
//!   on lock, sleep, screen lock, user switch and quit (`lib.rs`). It removes its socket only if
//!   the file there is still the one it made.
//!
//! Developers only, debug builds only: `ENOTE_DEV_AGENT_UID` names the uid to accept (so the
//! route can be tried with a fictional Vault in one account) and `ENOTE_DEV_AGENT_SOCKET_DIR`
//! moves the folder. Release builds ignore both, as they ignore `ENOTE_DEV_DATA_DIR`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use notes::Kind;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::core::agent::{
    AGENT_REQUEST_MS, AgentEdit, AgentEditReply, AgentError, AgentOutcome, AgentStoreSuggestion,
};
use crate::core::{ApiError, AppCore};

/// The folder the owner's setup script makes, and the only place the app may write a socket
/// (the one path exception in `Entitlements.plist`).
pub const SOCKET_DIR: &str = "/Users/Shared/encrypted-note-keys";
pub const SOCKET_NAME: &str = "agent.sock";
pub const PROTOCOL_VERSION: u64 = 1;
/// The Tauri event that tells the screens an agent request is waiting (`agent_pending`).
pub const AGENT_REQUEST_EVENT: &str = "agent-request";

/// How long and how much the route waits for. Tests shorten them.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The longest request line, newline excluded.
    pub max_line_bytes: usize,
    /// The whole request line must arrive within this.
    pub read_timeout: Duration,
    /// How long a request the owner completes waits for the owner.
    pub owner_wait: Duration,
    pub max_connections: usize,
    /// How often a waiting request asks the core for its outcome.
    pub poll: Duration,
}

impl Limits {
    pub const PRODUCTION: Limits = Limits {
        max_line_bytes: 16 * 1024,
        read_timeout: Duration::from_secs(10),
        // The core cancels the request at five minutes; a little more lets that reply arrive.
        owner_wait: Duration::from_millis(AGENT_REQUEST_MS + 5_000),
        max_connections: 4,
        poll: Duration::from_millis(200),
    };
}

// ---------- why the route can't start (for the owner) ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteError {
    /// The folder is missing, or isn't the owner's private folder, or something else is where
    /// the socket goes.
    NotSetUp,
    /// No agent account is named in Settings.
    NoAccount,
    /// No Mac account has that name.
    AccountNotFound,
    /// The named account is the owner's own.
    AccountIsOwner,
    /// The named account is a system account or can administer the Mac.
    AccountNotStandard,
    /// This platform has no route yet.
    Unavailable,
    /// Anything else (a bind or a thread that failed).
    Failed,
}

impl RouteError {
    pub fn api_error(self) -> ApiError {
        let (code, message) = match self {
            RouteError::NotSetUp => (
                "not_set_up",
                "Agents can't connect yet: the folder /Users/Shared/encrypted-note-keys isn't set up. Run scripts/agent-keys-setup.sh in your own account, then choose Start again.",
            ),
            RouteError::NoAccount => (
                "not_set_up",
                "Type the agent's Mac account name first, then choose Start.",
            ),
            RouteError::AccountNotFound => (
                "not_set_up",
                "There's no Mac account with that name. Check its Account name in System Settings → Users & Groups.",
            ),
            RouteError::AccountIsOwner => (
                "not_allowed",
                "That's your own account. Name the separate Standard account the agent runs in.",
            ),
            RouteError::AccountNotStandard => (
                "not_allowed",
                "That account is an administrator or a system account. The agent must run in a Standard account.",
            ),
            RouteError::Unavailable => (
                "unavailable",
                "Agents storing keys works only on a Mac for now.",
            ),
            RouteError::Failed => (
                "internal",
                "Agents couldn't be let in. Nothing was changed. Choose Start again.",
            ),
        };
        ApiError {
            code,
            message: message.to_owned(),
        }
    }
}

/// The groups whose members may never be the agent account: wheel, operator and admin.
pub const PRIVILEGED_GROUPS: [u32; 3] = [0, 5, 80];

/// Whether `uid` may be the agent account: not the owner's own, an ordinary user (501 and up,
/// below 2^31), and in none of `groups` that can administer the Mac.
pub fn check_account(uid: u32, owner_uid: u32, groups: &[u32]) -> Result<(), RouteError> {
    if uid == owner_uid {
        return Err(RouteError::AccountIsOwner);
    }
    if !(501..2_147_483_648).contains(&uid) || groups.iter().any(|g| PRIVILEGED_GROUPS.contains(g))
    {
        return Err(RouteError::AccountNotStandard);
    }
    Ok(())
}

/// Where the socket goes. Debug builds may move it with `ENOTE_DEV_AGENT_SOCKET_DIR`.
pub fn socket_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("ENOTE_DEV_AGENT_SOCKET_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(SOCKET_DIR)
}

// ---------- the protocol (every platform) ----------

/// One agent request, as the core takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Store(AgentStoreSuggestion),
    List,
    Edit { id: String, edit: AgentEdit },
    Replace { id: String },
    Delete { id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreWire {
    kind: Kind,
    title: String,
    #[serde(default)]
    details: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListWire {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditWire {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    details: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdWire {
    id: String,
}

/// Reads one request line: `{"v":1,"op":"store"|"list"|"edit"|"replace"|"delete", ...}`.
///
/// * store: `kind`, `title`, `details` (visible field name → text).
/// * list: nothing else.
/// * edit: `id`, and `title` and/or `details`.
/// * replace, delete: `id`.
///
/// Any other field, version or op, a line that isn't UTF-8 JSON, or a detail that isn't text is
/// [`AgentError::Invalid`]. There is no field for a value: a detail named after a Hidden Field
/// is refused by the core (`not_allowed`).
pub fn parse_request(line: &[u8]) -> Result<Request, AgentError> {
    let invalid = |_| AgentError::Invalid;
    let mut object: Map<String, Value> = serde_json::from_slice(line).map_err(invalid)?;
    if object.remove("v") != Some(json!(PROTOCOL_VERSION)) {
        return Err(AgentError::Invalid);
    }
    let Some(Value::String(op)) = object.remove("op") else {
        return Err(AgentError::Invalid);
    };
    let rest = Value::Object(object);
    Ok(match op.as_str() {
        "store" => {
            let wire: StoreWire = serde_json::from_value(rest).map_err(invalid)?;
            Request::Store(AgentStoreSuggestion {
                kind: wire.kind,
                title: wire.title,
                visible: wire.details,
            })
        }
        "list" => {
            let ListWire {} = serde_json::from_value(rest).map_err(invalid)?;
            Request::List
        }
        "edit" => {
            let wire: EditWire = serde_json::from_value(rest).map_err(invalid)?;
            Request::Edit {
                id: wire.id,
                edit: AgentEdit {
                    title: wire.title,
                    visible: wire.details,
                },
            }
        }
        "replace" => Request::Replace {
            id: serde_json::from_value::<IdWire>(rest).map_err(invalid)?.id,
        },
        "delete" => Request::Delete {
            id: serde_json::from_value::<IdWire>(rest).map_err(invalid)?.id,
        },
        _ => return Err(AgentError::Invalid),
    })
}

/// `{"v":1,"error":"<code>"}`, the only shape a refusal takes.
pub fn error_reply(error: AgentError) -> String {
    json!({ "v": PROTOCOL_VERSION, "error": error.code() }).to_string()
}

fn ok_reply(mut fields: Map<String, Value>) -> String {
    fields.insert("v".to_owned(), json!(PROTOCOL_VERSION));
    fields.insert("ok".to_owned(), json!(true));
    Value::Object(fields).to_string()
}

fn outcome_reply(outcome: AgentOutcome) -> String {
    match outcome {
        AgentOutcome::Cancelled => error_reply(AgentError::Cancelled),
        outcome => match serde_json::to_value(outcome) {
            Ok(Value::Object(fields)) => ok_reply(fields),
            _ => error_reply(AgentError::Internal),
        },
    }
}

/// What the route needs from the app: the core, and a way to tell the screens.
pub trait Host: Send + Sync + 'static {
    fn with_core<T>(&self, f: impl FnOnce(&mut AppCore) -> T) -> T;
    /// Something may have changed: the screens refresh.
    fn changed(&self);
    /// A request now waits for the owner: the app comes to the front and the screens show it.
    fn request_arrived(&self);
}

/// Runs one request under `grant` and returns the reply line (no newline). Requests the owner
/// completes wait here until the core has their outcome, `limits.owner_wait` passes, or `stop`
/// is set; the last two cancel the request.
pub fn handle<H: Host>(
    host: &H,
    grant: u64,
    request: Request,
    limits: &Limits,
    stop: &AtomicBool,
) -> String {
    let waiting = match request {
        Request::List => {
            return match host.with_core(|core| core.agent_list(grant)) {
                Ok(keys) => match serde_json::to_value(keys) {
                    Ok(keys) => ok_reply(Map::from_iter([("keys".to_owned(), keys)])),
                    Err(_) => error_reply(AgentError::Internal),
                },
                Err(error) => error_reply(error),
            };
        }
        Request::Store(suggestion) => host.with_core(|core| core.agent_store(grant, suggestion)),
        Request::Edit { id, edit } => {
            match host.with_core(|core| core.agent_edit(grant, &id, edit)) {
                Ok(AgentEditReply::Done) => {
                    host.changed();
                    return outcome_reply(AgentOutcome::Done);
                }
                Ok(AgentEditReply::Pending(request_id)) => Ok(request_id),
                Err(error) => Err(error),
            }
        }
        Request::Replace { id } => host.with_core(|core| core.agent_replace(grant, &id)),
        Request::Delete { id } => host.with_core(|core| core.agent_delete(grant, &id)),
    };
    let request_id = match waiting {
        Ok(request_id) => request_id,
        Err(error) => return error_reply(error),
    };
    host.changed();
    host.request_arrived();
    outcome_reply(wait_for_owner(host, &request_id, limits, stop))
}

fn wait_for_owner<H: Host>(
    host: &H,
    request_id: &str,
    limits: &Limits,
    stop: &AtomicBool,
) -> AgentOutcome {
    let deadline = std::time::Instant::now() + limits.owner_wait;
    loop {
        if let Some(outcome) = host.with_core(|core| core.agent_take_outcome(request_id)) {
            return outcome;
        }
        if stop.load(std::sync::atomic::Ordering::SeqCst) || std::time::Instant::now() >= deadline {
            host.with_core(|core| core.agent_cancel_request(request_id));
            host.changed();
            return AgentOutcome::Cancelled;
        }
        std::thread::sleep(limits.poll);
    }
}

// ---------- the listener (macOS) ----------

#[cfg(target_os = "macos")]
pub use mac::{AgentRoute, check_folder, expected_uid, lookup_account, peer_uid};

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CString, c_char, c_int};
    use std::io::{ErrorKind, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use super::*;

    fn own_uid() -> u32 {
        // SAFETY: geteuid has no arguments and cannot fail.
        unsafe { libc::geteuid() }
    }

    /// The kernel's word for who is at the other end of `stream`.
    pub fn peer_uid(stream: &UnixStream) -> Option<u32> {
        let mut uid: libc::uid_t = u32::MAX;
        let mut gid: libc::gid_t = u32::MAX;
        // SAFETY: the descriptor is open for as long as `stream` is borrowed, and both out
        // pointers are valid for writes.
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        (rc == 0 && uid != u32::MAX).then_some(uid)
    }

    /// The folder must be a real folder owned by this account with mode 0700 exactly, and every
    /// folder above it a real folder owned by root or this account; one that others may write
    /// in must be root's and sticky (as `/Users/Shared` is), so nobody can swap ours out.
    pub fn check_folder(dir: &Path) -> Result<(), RouteError> {
        let me = own_uid();
        if !dir.is_absolute() {
            return Err(RouteError::NotSetUp);
        }
        let meta = std::fs::symlink_metadata(dir).map_err(|_| RouteError::NotSetUp)?;
        if !meta.file_type().is_dir() || meta.uid() != me || meta.mode() & 0o7777 != 0o700 {
            return Err(RouteError::NotSetUp);
        }
        for parent in dir.ancestors().skip(1) {
            let meta = std::fs::symlink_metadata(parent).map_err(|_| RouteError::NotSetUp)?;
            let owner_ok = meta.uid() == 0 || meta.uid() == me;
            let others_write = meta.mode() & 0o022 != 0;
            let sticky_root = meta.uid() == 0 && meta.mode() & 0o1000 != 0;
            if !meta.file_type().is_dir() || !owner_ok || (others_write && !sticky_root) {
                return Err(RouteError::NotSetUp);
            }
        }
        Ok(())
    }

    /// The uid and primary group of the account called `name`.
    pub fn lookup_account(name: &str) -> Result<(u32, u32), RouteError> {
        if !crate::core::agent::is_account_name(name) {
            return Err(RouteError::AccountNotFound);
        }
        let name = CString::new(name).map_err(|_| RouteError::AccountNotFound)?;
        let mut size = 4096;
        while size <= 1 << 20 {
            let mut buffer = vec![0 as c_char; size];
            // SAFETY: an all-zero passwd is a valid value of a plain C struct; the call fills it.
            let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
            let mut found: *mut libc::passwd = std::ptr::null_mut();
            // SAFETY: every pointer is valid for the call, and `buffer` is `size` bytes long.
            let rc = unsafe {
                libc::getpwnam_r(
                    name.as_ptr(),
                    &mut entry,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut found,
                )
            };
            if rc == libc::ERANGE {
                size *= 4;
                continue;
            }
            if rc != 0 || found.is_null() {
                return Err(RouteError::AccountNotFound);
            }
            return Ok((entry.pw_uid, entry.pw_gid));
        }
        Err(RouteError::AccountNotFound)
    }

    /// Every group the account is in, by `getgrouplist`.
    fn groups_of(name: &CString, primary: u32) -> Result<Vec<u32>, RouteError> {
        let base = c_int::try_from(primary).map_err(|_| RouteError::AccountNotStandard)?;
        let mut size: c_int = 64;
        while size <= 4096 {
            let mut groups = vec![0 as c_int; usize::try_from(size).unwrap_or(0)];
            let mut count = size;
            // SAFETY: `groups` holds `count` entries, and the call writes at most that many.
            let rc =
                unsafe { libc::getgrouplist(name.as_ptr(), base, groups.as_mut_ptr(), &mut count) };
            if rc == 0 {
                groups.truncate(usize::try_from(count).unwrap_or(0));
                return Ok(groups.into_iter().map(|g| g as u32).collect());
            }
            size *= 4;
        }
        Err(RouteError::AccountNotStandard)
    }

    // The Open Directory membership check, which also sees nested groups (`dseditgroup` uses
    // it). Declared here because the libc crate doesn't have it.
    unsafe extern "C" {
        fn mbr_uid_to_uuid(uid: libc::uid_t, uu: *mut u8) -> c_int;
        fn mbr_gid_to_uuid(gid: libc::gid_t, uu: *mut u8) -> c_int;
        fn mbr_check_membership(user: *const u8, group: *const u8, ismember: *mut c_int) -> c_int;
    }

    /// Whether Open Directory says `uid` is in `gid`. An answer it can't give counts as yes.
    fn member_of(uid: u32, gid: u32) -> bool {
        let mut user = [0u8; 16];
        let mut group = [0u8; 16];
        let mut member: c_int = 1;
        // SAFETY: each uuid buffer is the 16 bytes a uuid_t is, and `member` is valid for writes.
        let ok = unsafe {
            mbr_uid_to_uuid(uid, user.as_mut_ptr()) == 0
                && mbr_gid_to_uuid(gid, group.as_mut_ptr()) == 0
                && mbr_check_membership(user.as_ptr(), group.as_ptr(), &mut member) == 0
        };
        !ok || member != 0
    }

    /// The uid the route will accept, from the account the owner named. Debug builds take
    /// `ENOTE_DEV_AGENT_UID` instead, with no checks, so one account can try it end to end.
    pub fn expected_uid(account: Option<&str>) -> Result<u32, RouteError> {
        #[cfg(debug_assertions)]
        if let Ok(uid) = std::env::var("ENOTE_DEV_AGENT_UID") {
            return uid.trim().parse().map_err(|_| RouteError::AccountNotFound);
        }
        let name = account.ok_or(RouteError::NoAccount)?;
        let (uid, primary) = lookup_account(name)?;
        let cname = CString::new(name).map_err(|_| RouteError::AccountNotFound)?;
        let mut groups = groups_of(&cname, primary)?;
        if PRIVILEGED_GROUPS.iter().any(|&g| member_of(uid, g)) {
            groups.extend(PRIVILEGED_GROUPS);
        }
        check_account(uid, own_uid(), &groups)?;
        Ok(uid)
    }

    /// The socket this route made: removed on stop only if it is still the same file.
    struct Socket {
        path: PathBuf,
        dev: u64,
        ino: u64,
    }

    impl Socket {
        fn bind(dir: &Path) -> Result<(UnixListener, Socket), RouteError> {
            check_folder(dir)?;
            let path = dir.join(SOCKET_NAME);
            match std::fs::symlink_metadata(&path) {
                // A socket this account left behind (a crash): ours to replace.
                Ok(meta) if meta.file_type().is_socket() && meta.uid() == own_uid() => {
                    std::fs::remove_file(&path).map_err(|_| RouteError::NotSetUp)?;
                }
                Ok(_) => return Err(RouteError::NotSetUp),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => return Err(RouteError::NotSetUp),
            }
            let listener = UnixListener::bind(&path).map_err(|_| RouteError::Failed)?;
            let made = std::fs::symlink_metadata(&path).map_err(|_| RouteError::Failed)?;
            let socket = Socket {
                path,
                dev: made.dev(),
                ino: made.ino(),
            };
            // Anyone who can reach the folder may connect; the folder's one ACL entry decides
            // who can reach it, and the peer uid decides who is answered.
            if !made.file_type().is_socket()
                || std::fs::set_permissions(&socket.path, std::fs::Permissions::from_mode(0o666))
                    .is_err()
                || listener.set_nonblocking(true).is_err()
            {
                socket.remove();
                return Err(RouteError::Failed);
            }
            Ok((listener, socket))
        }

        fn remove(&self) {
            if std::fs::symlink_metadata(&self.path)
                .is_ok_and(|meta| meta.dev() == self.dev && meta.ino() == self.ino)
            {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }

    struct Running {
        stop: Arc<AtomicBool>,
        thread: JoinHandle<()>,
        socket: Socket,
    }

    /// The listener, managed beside the core (never inside it). At most one runs.
    #[derive(Default)]
    pub struct AgentRoute {
        running: Mutex<Option<Running>>,
        active: AtomicBool,
    }

    impl AgentRoute {
        /// Listens in `dir` for `grant`, answering only `expected_uid`. Replaces a listener
        /// already running.
        pub fn start<H: Host>(
            &self,
            host: Arc<H>,
            grant: u64,
            expected_uid: u32,
            dir: &Path,
            limits: Limits,
        ) -> Result<(), RouteError> {
            let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(old) = running.take() {
                old.end();
            }
            self.active.store(false, Ordering::SeqCst);
            let (listener, socket) = Socket::bind(dir)?;
            let stop = Arc::new(AtomicBool::new(false));
            let thread = {
                let stop = stop.clone();
                std::thread::Builder::new()
                    .name("agent-route".to_owned())
                    .spawn(move || accept_loop(listener, host, grant, expected_uid, limits, stop))
            };
            let thread = match thread {
                Ok(thread) => thread,
                Err(_) => {
                    socket.remove();
                    return Err(RouteError::Failed);
                }
            };
            *running = Some(Running {
                stop,
                thread,
                socket,
            });
            self.active.store(true, Ordering::SeqCst);
            Ok(())
        }

        /// Stops listening and removes the socket. Requests waiting for the owner end as
        /// `cancelled`. Nothing happens if it isn't running.
        pub fn stop(&self) {
            let running = self
                .running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.active.store(false, Ordering::SeqCst);
            if let Some(running) = running {
                running.end();
            }
        }

        /// An atomic read: whether a listener is running.
        pub fn is_active(&self) -> bool {
            self.active.load(Ordering::SeqCst)
        }
    }

    impl Running {
        fn end(self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.thread.join();
            self.socket.remove();
        }
    }

    /// Keeps the count of open connections right however a connection ends.
    struct Slot(Arc<AtomicUsize>);

    impl Drop for Slot {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn accept_loop<H: Host>(
        listener: UnixListener,
        host: Arc<H>,
        grant: u64,
        expected_uid: u32,
        limits: Limits,
        stop: Arc<AtomicBool>,
    ) {
        let open = Arc::new(AtomicUsize::new(0));
        while !stop.load(Ordering::SeqCst) {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            // Anyone else, the owner's own programs included, is closed without a word.
            if peer_uid(&stream) != Some(expected_uid) || stream.set_nonblocking(false).is_err() {
                continue;
            }
            let _ = stream.set_write_timeout(Some(limits.read_timeout));
            if open.fetch_add(1, Ordering::SeqCst) >= limits.max_connections {
                open.fetch_sub(1, Ordering::SeqCst);
                send(&stream, &error_reply(AgentError::Busy));
                continue;
            }
            let slot = Slot(open.clone());
            let (host, stop) = (host.clone(), stop.clone());
            // A thread that can't start drops the closure, and with it the slot.
            let _ = std::thread::Builder::new()
                .name("agent-request".to_owned())
                .spawn(move || {
                    let _slot = slot;
                    let reply = match read_line(&stream, &limits) {
                        Ok(line) => match parse_request(&line) {
                            Ok(request) => handle(&*host, grant, request, &limits, &stop),
                            Err(error) => error_reply(error),
                        },
                        Err(error) => error_reply(error),
                    };
                    send(&stream, &reply);
                });
        }
    }

    fn send(mut stream: &UnixStream, reply: &str) {
        let _ = stream.write_all(reply.as_bytes());
        let _ = stream.write_all(b"\n");
        let _ = stream.flush();
    }

    /// One line, within the size and the time limit. End of input also ends the line.
    fn read_line(mut stream: &UnixStream, limits: &Limits) -> Result<Vec<u8>, AgentError> {
        let deadline = Instant::now() + limits.read_timeout;
        let mut line = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || stream.set_read_timeout(Some(left)).is_err() {
                return Err(AgentError::Invalid);
            }
            let read = match stream.read(&mut chunk) {
                Ok(read) => read,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(AgentError::Invalid),
            };
            if read == 0 {
                return if line.is_empty() {
                    Err(AgentError::Invalid)
                } else {
                    Ok(line)
                };
            }
            let chunk = &chunk[..read];
            if let Some(end) = chunk.iter().position(|&b| b == b'\n') {
                line.extend_from_slice(&chunk[..end]);
                return if line.len() > limits.max_line_bytes {
                    Err(AgentError::Invalid)
                } else {
                    Ok(line)
                };
            }
            line.extend_from_slice(chunk);
            if line.len() > limits.max_line_bytes {
                return Err(AgentError::Invalid);
            }
        }
    }
}

// ---------- other platforms: no route yet ----------

#[cfg(not(target_os = "macos"))]
pub use other::{AgentRoute, expected_uid};

#[cfg(not(target_os = "macos"))]
mod other {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;

    pub fn expected_uid(_account: Option<&str>) -> Result<u32, RouteError> {
        Err(RouteError::Unavailable)
    }

    /// No listener here. Like the Mac route, it is built only through `Default`.
    #[derive(Default)]
    pub struct AgentRoute {
        _private: (),
    }

    impl AgentRoute {
        pub fn start<H: Host>(
            &self,
            _host: Arc<H>,
            _grant: u64,
            _expected_uid: u32,
            _dir: &Path,
            _limits: Limits,
        ) -> Result<(), RouteError> {
            Err(RouteError::Unavailable)
        }

        pub fn stop(&self) {}

        pub fn is_active(&self) -> bool {
            false
        }
    }
}

// ---------- the app's side (Tauri) ----------

/// The route's host in the running app.
pub struct TauriHost(pub tauri::AppHandle);

impl Host for TauriHost {
    fn with_core<T>(&self, f: impl FnOnce(&mut AppCore) -> T) -> T {
        f(&mut crate::commands::core(&self.0))
    }

    fn changed(&self) {
        crate::commands::with_core(&self.0, |_| ());
    }

    fn request_arrived(&self) {
        use tauri::{Emitter, Manager};
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        let _ = self.0.emit(AGENT_REQUEST_EVENT, ());
    }
}

/// From the 500 ms background loop: a listener whose grant is no longer running stops.
pub fn sync(app: &tauri::AppHandle, grant_running: bool) {
    use tauri::Manager;
    let route = app.state::<AgentRoute>();
    if !grant_running && route.is_active() {
        route.stop();
    }
}

/// Sleep, screen lock, user switch and quit: stop at once, off the calling (main) thread.
pub fn stop_for_lifecycle(app: &tauri::AppHandle) {
    use tauri::Manager;
    if app.state::<AgentRoute>().is_active() {
        let app = app.clone();
        std::thread::spawn(move || app.state::<AgentRoute>().stop());
    }
}

/// The app is quitting: stop now, so the socket is gone before the process is.
pub fn stop_on_exit(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(route) = app.try_state::<AgentRoute>() {
        route.stop();
    }
}
