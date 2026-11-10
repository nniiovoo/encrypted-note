//! `enote-agent-keys`: the MCP tool an agent (Claude Code or Codex, ADR-0013) runs in its own
//! Mac account to ask the encrypted-note app to store and look after keys (ADR-0012).
//!
//! What it guarantees:
//! * **It never sees a value.** No tool takes one and no reply holds one: the owner types every
//!   value in the app. A detail named like a value (`value`, `key`, `password`, `secret`,
//!   `words`, `passphrase`, `body`) is refused here, before anything is sent, and the app
//!   refuses it again.
//! * **It talks only to the owner's app.** Each call connects to the app's socket, checks with
//!   the kernel (`getpeereid`) that the other end runs as the owner's uid (`--owner-uid` or
//!   `ENOTE_OWNER_UID`), sends one request line and reads one reply line. No network, no other
//!   files, nothing kept between calls.
//! * **Plain answers.** Replies become short tool results: what happened, or why not, from a
//!   fixed list of codes.
//!
//! The MCP side is JSON-RPC 2.0, one message per line on stdin and stdout: `initialize`,
//! `notifications/initialized`, `ping`, `tools/list` and `tools/call`.
//!
//! A request that waits for the owner can take up to 5 minutes, longer than some clients wait
//! for a tool (Codex and the Claude app give up after 60 s by default). So the binary answers
//! each `tools/call` on its own thread, and other calls (`ping`, `list_keys`) are answered
//! while one waits; and while one waits it sends `notifications/progress` every
//! [`PROGRESS_EVERY`] when the client gave a `progressToken`, for clients that wait longer on
//! progress. A client that still gives up leaves the request open in the app: the agent then
//! checks with `list_keys` before asking again.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value, json};

#[cfg(unix)]
#[allow(unsafe_code)]
pub mod peer;

/// Where the app listens (the folder the owner's setup script makes).
pub const SOCKET_PATH: &str = "/Users/Shared/encrypted-note-keys/agent.sock";
/// The newest MCP version first; an `initialize` asking for one of these gets it back.
pub const MCP_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// The route's protocol version (`agent_route.rs` in the app).
pub const ROUTE_VERSION: u64 = 1;
/// The longest MCP message this tool reads, and the longest reply it takes from the app.
pub const MAX_LINE_BYTES: usize = 1 << 20;
/// How often a call waiting for the owner tells the client it is still going.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(15);
/// What the progress notifications say. Fixed text, never anything from the app.
pub const WAITING_TEXT: &str = "Waiting for the owner in the encrypted-note app.";

/// Names a value goes under. Never a detail: the owner types values in the app.
pub const VALUE_NAMES: [&str; 7] = [
    "value",
    "key",
    "password",
    "secret",
    "words",
    "passphrase",
    "body",
];

/// The Kinds, and the details each may have (the app's visible fields, less `word_count`).
pub const KINDS: [(&str, &[&str]); 5] = [
    (
        "api_key",
        &["service", "key_id", "purpose", "url", "expires", "env_var"],
    ),
    ("login", &["website", "username", "purpose"]),
    ("text", &[]),
    (
        "seed_phrase",
        &["wallet_name", "address", "derivation_path"],
    ),
    ("private_key", &["chain", "address"]),
];

#[derive(Clone, Debug)]
pub struct Config {
    pub socket: PathBuf,
    /// The owner's uid: the app must run as it. `None` until the owner's setup names it.
    pub owner_uid: Option<u32>,
    /// How long a request waits for the owner to finish it in the app (the app gives up at 5
    /// minutes).
    pub owner_wait: Duration,
    /// How long anything else may take.
    pub quick_wait: Duration,
}

impl Config {
    pub fn new(owner_uid: Option<u32>) -> Config {
        Config {
            socket: PathBuf::from(SOCKET_PATH),
            owner_uid,
            owner_wait: Duration::from_secs(5 * 60 + 30),
            quick_wait: Duration::from_secs(30),
        }
    }
}

/// Why a request didn't reach the app, or got no answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteFailure {
    /// No `--owner-uid`: the tool can't tell the owner's app from anyone else's.
    NoOwner,
    /// Nothing listens: the app is closed, locked or not letting agents in.
    NotRunning,
    /// Something listens, but not as the owner: never talked to.
    WrongServer,
    /// The app's answer was missing, too long or not the protocol.
    Broken,
}

/// Sends one request line to the app and returns its reply.
pub trait Route {
    fn send(&self, request: &Value, wait_for_owner: bool) -> Result<Value, RouteFailure>;
}

/// The real route: the app's socket.
#[cfg(unix)]
pub struct SocketRoute(pub Config);

#[cfg(unix)]
impl Route for SocketRoute {
    fn send(&self, request: &Value, wait_for_owner: bool) -> Result<Value, RouteFailure> {
        use std::io::{BufRead, BufReader, Read, Write};
        let owner = self.0.owner_uid.ok_or(RouteFailure::NoOwner)?;
        let mut stream = std::os::unix::net::UnixStream::connect(&self.0.socket)
            .map_err(|_| RouteFailure::NotRunning)?;
        // Before a single byte is sent: is that the owner's app?
        if peer::server_uid(&stream) != Some(owner) {
            return Err(RouteFailure::WrongServer);
        }
        let wait = if wait_for_owner {
            self.0.owner_wait
        } else {
            self.0.quick_wait
        };
        stream
            .set_write_timeout(Some(self.0.quick_wait))
            .and_then(|()| stream.set_read_timeout(Some(wait)))
            .map_err(|_| RouteFailure::Broken)?;
        let mut line = request.to_string();
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .map_err(|_| RouteFailure::Broken)?;
        let mut reply = Vec::new();
        BufReader::new(stream.take(MAX_LINE_BYTES as u64 + 1))
            .read_until(b'\n', &mut reply)
            .map_err(|_| RouteFailure::Broken)?;
        if reply.last() != Some(&b'\n') {
            return Err(RouteFailure::Broken);
        }
        let reply: Value = serde_json::from_slice(&reply).map_err(|_| RouteFailure::Broken)?;
        if reply.get("v") != Some(&json!(ROUTE_VERSION)) {
            return Err(RouteFailure::Broken);
        }
        Ok(reply)
    }
}

// ---------- the tools ----------

const NO_VALUE: &str = "The owner types every value (password, key, secret, seed words, passphrase, private key, note text) in the encrypted-note app; this tool never sees or returns one. Never put a value in `name` or `details`, or in the chat.";

/// For the calls that wait for the owner.
const IF_TIMED_OUT: &str = "If your client gives up on this call before the owner answers (a tool timeout), the request is still open in the app: don't ask again, check with list_keys a little later.";

fn kind_names() -> Vec<&'static str> {
    KINDS.iter().map(|(kind, _)| *kind).collect()
}

fn details_schema() -> Value {
    json!({
        "type": "object",
        "description": "Visible details only, by Kind. api_key: service, key_id, purpose, url, expires (YYYY-MM-DD), env_var. login: website, username, purpose. seed_phrase: wallet_name, address (public), derivation_path. private_key: chain (evm, solana, bitcoin or other), address (public). text: none. These reach the AI company as part of the chat: never put a secret here.",
        "additionalProperties": { "type": "string" }
    })
}

/// The five tools, as `tools/list` shows them.
pub fn tools() -> Value {
    let id =
        json!({ "type": "string", "description": "The key's id, from list_keys or store_key." });
    json!([
        {
            "name": "store_key",
            "description": format!("Ask the owner to store a key in the encrypted-note app. The app comes to the front with its editor filled in from your kind, name and details; the owner types or pastes the value there and chooses Save (a Seed Phrase or Private Key also needs their Master Password). Waits up to 5 minutes, then returns \"saved\" and the key's id, or \"cancelled\". {IF_TIMED_OUT} {NO_VALUE}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": kind_names() },
                    "name": { "type": "string", "description": "A name for the key, such as \"Fake CI deploy key\". It is not secret, but it reaches the AI company." },
                    "details": details_schema()
                },
                "required": ["kind", "name"],
                "additionalProperties": false
            }
        },
        {
            "name": "list_keys",
            "description": "List the keys you (an agent) added: each one's id, kind, name, visible details and when it was added or changed. Never a value, and never the owner's own keys.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "edit_key_details",
            "description": format!("Change the name or visible details of a key you added. A name, purpose or expiry date changes at once. Anything that says where a key is used (website, username, service, key id, URL, env var name, and every detail of a Seed Phrase or Private Key) waits up to 5 minutes for the owner to allow it in the app. Returns \"done\", or why not. An empty detail clears it. {IF_TIMED_OUT} {NO_VALUE}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": id,
                    "name": { "type": "string", "description": "The new name." },
                    "details": details_schema()
                },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "ask_to_replace_value",
            "description": format!("Ask the owner to type a new value for a key you added. The app opens that key with only its value editable. Waits up to 5 minutes, then returns \"saved\" or \"cancelled\". {IF_TIMED_OUT} {NO_VALUE}"),
            "inputSchema": {
                "type": "object",
                "properties": { "id": id },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "ask_to_delete",
            "description": format!("Ask the owner to delete a key you added. The app asks them to confirm and to type their Master Password, then moves the key to Trash, where it stays 30 days. Waits up to 5 minutes, then returns \"deleted\" or \"cancelled\". {IF_TIMED_OUT}"),
            "inputSchema": {
                "type": "object",
                "properties": { "id": id },
                "required": ["id"],
                "additionalProperties": false
            }
        }
    ])
}

/// Turns a tool call into the route request, and whether it may wait for the owner. An
/// argument this tool doesn't take, or a detail named like a value, is refused with the reason.
pub fn route_request(tool: &str, arguments: &Value) -> Result<(Value, bool), String> {
    let empty = Map::new();
    let arguments = match arguments {
        Value::Null => &empty,
        Value::Object(arguments) => arguments,
        _ => return Err("The arguments must be an object.".to_owned()),
    };
    let allowed: &[&str] = match tool {
        "store_key" => &["kind", "name", "details"],
        "list_keys" => &[],
        "edit_key_details" => &["id", "name", "details"],
        "ask_to_replace_value" | "ask_to_delete" => &["id"],
        _ => return Err(format!("There is no tool called {tool}.")),
    };
    // A value is named as such whatever else is wrong with the call.
    if let Some(name) = arguments.keys().find(|name| is_value_name(name)) {
        return Err(format!("Refused: `{name}` looks like a value. {NO_VALUE}"));
    }
    if let Some(Value::Object(details)) = arguments.get("details")
        && let Some(name) = details.keys().find(|name| is_value_name(name))
    {
        return Err(format!(
            "Refused: the detail `{name}` is a value. {NO_VALUE}"
        ));
    }
    for name in arguments.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(format!(
                "{tool} doesn't take `{name}`. It takes: {}.",
                if allowed.is_empty() {
                    "nothing".to_owned()
                } else {
                    allowed.join(", ")
                }
            ));
        }
    }
    let text = |name: &str, required: bool| -> Result<Option<String>, String> {
        match arguments.get(name) {
            None if required => Err(format!("{tool} needs `{name}`.")),
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("`{name}` must be text.")),
        }
    };
    let details = |kind: Option<&str>| -> Result<Option<BTreeMap<String, String>>, String> {
        let Some(found) = arguments.get("details") else {
            return Ok(None);
        };
        let Value::Object(found) = found else {
            return Err("`details` must be an object of text values.".to_owned());
        };
        let known: Vec<&str> = match kind {
            Some(kind) => KINDS
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, fields)| fields.to_vec())
                .unwrap_or_default(),
            None => KINDS
                .iter()
                .flat_map(|(_, fields)| fields.iter().copied())
                .collect(),
        };
        let mut out = BTreeMap::new();
        for (name, value) in found {
            if is_value_name(name) {
                return Err(format!(
                    "Refused: the detail `{name}` is a value. {NO_VALUE}"
                ));
            }
            if !known.contains(&name.as_str()) {
                return Err(match kind {
                    Some("text") => {
                        "A text key has no details: give only its kind and name.".to_owned()
                    }
                    Some(kind) => format!(
                        "A {kind} has no detail called `{name}`. Its details are: {}.",
                        known.join(", ")
                    ),
                    None => format!("No kind of key has a detail called `{name}`."),
                });
            }
            let Value::String(value) = value else {
                return Err(format!("The detail `{name}` must be text."));
            };
            out.insert(name.clone(), value.clone());
        }
        Ok(Some(out))
    };
    let request = match tool {
        "store_key" => {
            let kind = text("kind", true)?.unwrap_or_default();
            if !kind_names().contains(&kind.as_str()) {
                return Err(format!(
                    "`kind` must be one of: {}.",
                    kind_names().join(", ")
                ));
            }
            let title = text("name", true)?.unwrap_or_default();
            let details = details(Some(&kind))?.unwrap_or_default();
            (
                json!({ "v": ROUTE_VERSION, "op": "store", "kind": kind, "title": title, "details": details }),
                true,
            )
        }
        "list_keys" => (json!({ "v": ROUTE_VERSION, "op": "list" }), false),
        "edit_key_details" => {
            let id = text("id", true)?.unwrap_or_default();
            let mut request = json!({ "v": ROUTE_VERSION, "op": "edit", "id": id });
            if let Some(title) = text("name", false)? {
                request["title"] = json!(title);
            }
            if let Some(details) = details(None)? {
                request["details"] = json!(details);
            }
            if request.get("title").is_none() && request.get("details").is_none() {
                return Err("Give a new `name` or `details` to change.".to_owned());
            }
            (request, true)
        }
        "ask_to_replace_value" => {
            let id = text("id", true)?.unwrap_or_default();
            (
                json!({ "v": ROUTE_VERSION, "op": "replace", "id": id }),
                true,
            )
        }
        _ => {
            let id = text("id", true)?.unwrap_or_default();
            (
                json!({ "v": ROUTE_VERSION, "op": "delete", "id": id }),
                true,
            )
        }
    };
    Ok(request)
}

/// Whether `name` is, by its letters alone, one a value goes under.
pub fn is_value_name(name: &str) -> bool {
    let plain = name.trim().to_ascii_lowercase();
    VALUE_NAMES.contains(&plain.as_str())
}

/// What the agent reads for the app's reply, and whether it is an error.
pub fn describe(tool: &str, reply: &Value) -> (String, bool) {
    if let Some(code) = reply.get("error").and_then(Value::as_str) {
        return (error_text(code).to_owned(), true);
    }
    if reply.get("ok") != Some(&json!(true)) {
        return (error_text("internal").to_owned(), true);
    }
    match (tool, reply.get("result").and_then(Value::as_str)) {
        ("list_keys", _) => {
            let keys = reply.get("keys").cloned().unwrap_or(json!([]));
            let count = keys.as_array().map_or(0, Vec::len);
            let shown = serde_json::to_string_pretty(&keys).unwrap_or_default();
            let what = match count {
                0 => "You haven't added any keys (or the owner took them out of your reach)."
                    .to_owned(),
                1 => "1 key you added (values are never shown):".to_owned(),
                n => format!("{n} keys you added (values are never shown):"),
            };
            if count == 0 {
                (what, false)
            } else {
                (format!("{what}\n{shown}"), false)
            }
        }
        ("store_key", Some("saved")) => {
            let id = reply.get("id").and_then(Value::as_str).unwrap_or("");
            (
                format!(
                    "Saved. The owner typed the value in the app; this tool never saw it. The key's id is {id}."
                ),
                false,
            )
        }
        ("ask_to_replace_value", Some("saved")) => (
            "Saved. The owner typed the new value in the app; this tool never saw it.".to_owned(),
            false,
        ),
        ("edit_key_details", Some("done")) => ("Done.".to_owned(), false),
        ("ask_to_delete", Some("deleted")) => (
            "Deleted: the key is in the app's Trash, where the owner can restore it for 30 days."
                .to_owned(),
            false,
        ),
        _ => (error_text("internal").to_owned(), true),
    }
}

/// The fixed codes the app replies with, in words.
pub fn error_text(code: &str) -> &'static str {
    match code {
        "locked" => "The encrypted-note app is locked. Ask the owner to unlock it.",
        "off" => {
            "Agents aren't let in right now. Ask the owner to choose Start in the app's Settings (it lasts at most one hour and ends when the Vault locks)."
        }
        "not_found" => "No key with that id among the keys you added. Use list_keys.",
        "not_allowed" => {
            "That isn't allowed. Values are changed only by the owner typing them in the app (use ask_to_replace_value), and some details can't be changed by an agent."
        }
        "busy" => {
            "The owner already has a request from you open in the app. Wait for it to finish, then try again."
        }
        "not_now" => "The owner is in the middle of something in the app. Try again in a moment.",
        "cancelled" => {
            "Cancelled: the owner said no, or didn't answer within 5 minutes. Nothing was changed."
        }
        "invalid" => {
            "The app couldn't take that request: check the kind, the name and the details (for example, expires is YYYY-MM-DD and env_var is a variable name)."
        }
        _ => "The app couldn't do that. Nothing was changed.",
    }
}

fn failure_text(failure: RouteFailure) -> &'static str {
    match failure {
        RouteFailure::NoOwner => {
            "This tool isn't set up: it must be started with --owner-uid <the owner's uid> (see the owner's setup script)."
        }
        RouteFailure::NotRunning => {
            "The encrypted-note app isn't letting agents in. Ask the owner to open it, unlock it and choose Start in Settings."
        }
        RouteFailure::WrongServer => {
            "Refused: the socket isn't run by the owner's account, so nothing was sent. Tell the owner."
        }
        RouteFailure::Broken => {
            "The app's answer didn't arrive. If you asked to store, replace or delete, check with list_keys before asking again."
        }
    }
}

// ---------- the MCP server ----------

pub struct Server<R: Route> {
    route: R,
    progress_every: Duration,
}

/// Whether `line` is a `tools/call`, which the binary answers on its own thread.
pub fn is_tool_call(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .is_ok_and(|message| message.get("method") == Some(&json!("tools/call")))
}

impl<R: Route> Server<R> {
    pub fn new(route: R) -> Server<R> {
        Server {
            route,
            progress_every: PROGRESS_EVERY,
        }
    }

    /// How often a waiting call sends progress (shorter in tests).
    pub fn with_progress_every(mut self, every: Duration) -> Server<R> {
        self.progress_every = every;
        self
    }

    /// Answers one JSON-RPC message, or `None` for a notification or a client's response.
    pub fn handle_line(&self, line: &str) -> Option<String> {
        self.handle_line_notifying(line, &|_| {})
    }

    /// As [`Server::handle_line`], and while a call waits for the owner, hands `notify` a
    /// `notifications/progress` line every so often if the call carried a `progressToken`.
    pub fn handle_line_notifying(
        &self,
        line: &str,
        notify: &(dyn Fn(String) + Sync),
    ) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(_) => return Some(parse_error()),
        };
        let Value::Object(message) = message else {
            return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let (Some(id), Some(method)) = (id, method) else {
            // A notification (no id) needs no answer, and a response (no method) isn't ours.
            return None;
        };
        if message.get("jsonrpc") != Some(&json!("2.0")) || !(id.is_string() || id.is_number()) {
            return Some(rpc_error(id, -32600, "Invalid Request"));
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| MCP_VERSIONS.contains(v))
                    .unwrap_or(MCP_VERSIONS[0]);
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "enote-agent-keys", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": format!("These tools let you ask the owner's encrypted-note app to store keys and look after the ones you added. {NO_VALUE} If the owner pastes a value into the chat, tell them not to, and use store_key so they type it in the app instead.")
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools() }),
            "tools/call" => {
                let Some(tool) = params.get("name").and_then(Value::as_str) else {
                    return Some(rpc_error(id, -32602, "tools/call needs a tool name"));
                };
                if !tools()
                    .as_array()
                    .is_some_and(|all| all.iter().any(|t| t["name"] == json!(tool)))
                {
                    return Some(rpc_error(id, -32602, "Unknown tool"));
                }
                let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
                let (text, is_error) = match route_request(tool, &arguments) {
                    Err(reason) => (reason, true),
                    Ok((request, wait)) => {
                        let token = params
                            .get("_meta")
                            .and_then(|meta| meta.get("progressToken"))
                            .filter(|token| token.is_string() || token.is_number())
                            .filter(|_| wait)
                            .cloned();
                        match self.send_with_progress(&request, wait, token, notify) {
                            Ok(reply) => describe(tool, &reply),
                            Err(failure) => (failure_text(failure).to_owned(), true),
                        }
                    }
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
            }
            _ => return Some(rpc_error(id, -32601, "Method not found")),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string())
    }
}

impl<R: Route> Server<R> {
    /// Sends `request`; with a progress token, a second thread says "still waiting" until the
    /// app answers. The notification holds the token, a count and fixed text only.
    fn send_with_progress(
        &self,
        request: &Value,
        wait: bool,
        token: Option<Value>,
        notify: &(dyn Fn(String) + Sync),
    ) -> Result<Value, RouteFailure> {
        let Some(token) = token else {
            return self.route.send(request, wait);
        };
        let every = self.progress_every;
        std::thread::scope(|scope| {
            let (answered, waiting) = std::sync::mpsc::channel::<()>();
            scope.spawn(move || {
                let mut count = 0u64;
                while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                    waiting.recv_timeout(every)
                {
                    count += 1;
                    notify(
                        json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/progress",
                            "params": { "progressToken": token, "progress": count, "message": WAITING_TEXT }
                        })
                        .to_string(),
                    );
                }
            });
            let reply = self.route.send(request, wait);
            drop(answered);
            reply
        })
    }
}

/// The answer to a message that isn't JSON (or is too long to read).
pub fn parse_error() -> String {
    rpc_error(Value::Null, -32700, "Parse error")
}

fn rpc_error(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}
