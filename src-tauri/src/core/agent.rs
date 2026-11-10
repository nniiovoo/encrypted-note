//! The agent route's side of the core (ADR-0012): an agent may store keys, list and edit the
//! details of the Notes it added, and ask to replace a value or delete one. It never sees or
//! uses a value. These are plain functions; the local route (`agent_route.rs`) calls them and
//! sends back only the fixed shapes below.
//!
//! What this module guarantees (and `tests/agent.rs` checks):
//! * **A grant.** Every request names the grant the owner started: at most one hour, only while
//!   Unlocked, and only for the unlock session it was started in. Every lock path, every phase
//!   change, Auto-lock, sleep, screen lock, user switch and quit end it (`forget_open_vaults`
//!   and `set_phase` call [`AppCore::agent_end`]), and so does its hour running out.
//! * **Only marked Notes.** Everything goes through `Document::agent_notes` and
//!   `Document::agent_note`: a Note the owner added, a Note in Trash and an id that doesn't exist
//!   all get the same [`AgentError::NotFound`].
//! * **Never a value.** No reply carries a Hidden Field, whether one has a value, an
//!   [`ApiError`] message or any other free text. Replies are [`AgentError`] codes,
//!   [`AgentNote`]s, request ids and [`AgentOutcome`]s.
//! * **The owner decides.** A store, a replace, a delete and an edit that says where a key is
//!   used wait as one pending request until the owner completes it in the app (the WebView
//!   editor or confirm, then the native Master Password panel where one is needed), cancels it,
//!   or five minutes pass. A delete always needs the Master Password, and so does every edit of
//!   a Seed Phrase or Private Key.
//! * **No interference.** Agent requests never count as activity, so they don't keep Auto-lock
//!   away. An edit that applies at once is refused while the owner has a panel, a preview or an
//!   agent request open, so it can't cancel what the owner is doing.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher, RandomState};

use notes::{AgentEditRule, AgentNote, agent_edit_needs, agent_edit_rule};

use super::native::{ActionDescription, NativeAction, NativeResult};
use super::*;

/// The longest grant the owner can start.
pub const AGENT_GRANT_MAX_MS: u64 = 60 * 60_000;
/// How long one agent request waits for the owner before it is cancelled.
pub const AGENT_REQUEST_MS: u64 = 5 * 60_000;
/// The longest title or visible value an agent may suggest. Far above any real one.
const MAX_AGENT_TEXT_CHARS: usize = 1_000;
/// Outcomes kept for the route to collect. One request is pending at a time, so a few suffice.
const KEPT_OUTCOMES: usize = 8;

const MSG_AGENT_REQUEST_GONE: &str =
    "That request from the agent has ended, so nothing was changed. The agent can ask again.";
const MSG_AGENT_WRONG_KIND: &str =
    "The agent asked to store a different Kind of key. Nothing was saved.";
const MSG_AGENT_NOTE_CHANGED: &str =
    "This key changed after the agent asked, so nothing was changed. The agent can ask again.";
const MSG_AGENT_VALUE_ONLY: &str =
    "Only the value can be changed here. Change the name or details in the Editor yourself.";
const MSG_AGENT_NO_VALUE: &str = "Type the new value first, then choose Save.";
const MSG_AGENT_ACCOUNT_NAME: &str = "That isn't a Mac account name. Use the Account name from System Settings → Users & Groups, such as enote-agent.";
const MSG_AGENT_TOOL_HASH: &str = "That isn't a code hash. Paste the CDHash line that scripts/agent-keys-setup.sh prints: 40 letters and digits, 0-9 and a-f.";

// ---------- shapes ----------

/// What an agent suggests storing: the Kind, a name and visible details. There is no place for
/// a value: the owner types it in the app's editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentStoreSuggestion {
    pub kind: Kind,
    pub title: String,
    pub visible: BTreeMap<String, String>,
}

/// An agent's change to a Note it added: a new title and/or visible fields (`""` clears one).
/// No Hidden Field: values change only by the owner typing a replacement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentEdit {
    pub title: Option<String>,
    pub visible: BTreeMap<String, String>,
}

/// A grant just started. `id` stays in Rust: the route uses it, the screens never need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentGrantView {
    pub id: u64,
    pub expires_in_ms: u64,
}

/// For the owner's Settings: `state` is `off`, `running`, `ended_by_lock` or `expired`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentGrantStatus {
    pub state: &'static str,
    pub expires_in_ms: Option<u64>,
}

/// Why an agent request was refused. The route sends only `code()`: never a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentError {
    Locked,
    Off,
    NotFound,
    NotAllowed,
    Busy,
    NotNow,
    Cancelled,
    Invalid,
    Internal,
}

impl AgentError {
    pub fn code(self) -> &'static str {
        match self {
            AgentError::Locked => "locked",
            AgentError::Off => "off",
            AgentError::NotFound => "not_found",
            AgentError::NotAllowed => "not_allowed",
            AgentError::Busy => "busy",
            AgentError::NotNow => "not_now",
            AgentError::Cancelled => "cancelled",
            AgentError::Invalid => "invalid",
            AgentError::Internal => "internal",
        }
    }
}

/// How a pending request ended, for the agent. `id` is the Note's id, never anything else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AgentOutcome {
    Saved { id: String },
    Done,
    Deleted,
    Cancelled,
}

/// An edit applied at once, or one waiting for the owner under this request id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEditReply {
    Done,
    Pending(String),
}

/// What the owner's screens show for the pending request. Visible parts only, never a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentPendingView {
    pub request_id: String,
    /// `store`, `edit`, `replace` or `delete`.
    pub request: &'static str,
    pub kind: Kind,
    /// The Note the request is about; `None` for a store.
    pub note_id: Option<String>,
    /// The suggested title (store), or the Note's title now.
    pub title: String,
    /// The suggested visible details (store), or the Note's visible details now.
    pub visible: BTreeMap<String, String>,
    /// An edit's changes, field by field (`title` for the title). Empty for other requests.
    pub changes: Vec<AgentChange>,
    /// Whether completing it asks for the Master Password.
    pub needs_password: bool,
    pub expires_in_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentChange {
    pub field: String,
    pub from: String,
    pub to: String,
}

// ---------- state ----------

/// All the agent route's state in the core. Nothing here is a value or a credential.
#[derive(Default)]
pub(super) struct AgentState {
    grant: Option<Grant>,
    /// Why the last grant ended, for the status; `None` when the owner stopped it.
    ended: Option<&'static str>,
    pending: Option<Pending>,
    outcomes: VecDeque<(String, AgentOutcome)>,
    /// The native panel opened to complete the pending request, if one is open.
    native_id: Option<u64>,
    ids: Ids,
}

struct Grant {
    id: u64,
    generation: u64,
    expires_at_ms: u64,
}

struct Pending {
    request_id: String,
    grant_id: u64,
    created_at_ms: u64,
    request: Request,
}

enum Request {
    Store(AgentStoreSuggestion),
    Edit {
        id: String,
        /// The Note as the agent saw it when it asked, and as the edit would leave it.
        before: Box<AgentNote>,
        after: Box<AgentNote>,
        title: Option<String>,
        visible: BTreeMap<String, String>,
        rule: AgentEditRule,
    },
    Replace {
        id: String,
    },
    Delete {
        id: String,
    },
}

impl Request {
    fn target(&self) -> Option<&str> {
        match self {
            Request::Store(_) => None,
            Request::Edit { id, .. } | Request::Replace { id } | Request::Delete { id } => Some(id),
        }
    }
}

/// Ids nobody outside can guess: SipHash under the keys std draws from the OS for each process,
/// over a counter, so they are also never repeated. They authorize nothing by themselves.
struct Ids {
    keys: RandomState,
    count: u64,
}

impl Default for Ids {
    fn default() -> Ids {
        Ids {
            keys: RandomState::new(),
            count: 0,
        }
    }
}

impl Ids {
    fn next(&mut self) -> u64 {
        self.count = self.count.wrapping_add(1);
        let mut hasher = self.keys.build_hasher();
        hasher.write_u64(self.count);
        hasher.finish()
    }

    /// 16 bytes as 32 lowercase hex characters.
    fn request_id(&mut self) -> String {
        format!("{:016x}{:016x}", self.next(), self.next())
    }
}

/// A refused completion: what the owner is told, and whether the request is over.
struct Refusal {
    error: ApiError,
    cancel: bool,
}

fn refuse(error: ApiError, cancel: bool) -> Refusal {
    Refusal { error, cancel }
}

/// Whether `action` completes an agent request.
pub(super) fn is_agent_action(action: &NativeAction) -> bool {
    matches!(
        action,
        NativeAction::AgentStore { .. }
            | NativeAction::AgentEdit { .. }
            | NativeAction::AgentReplace { .. }
            | NativeAction::AgentTrash { .. }
    )
}

// ---------- the core ----------

impl AppCore {
    // ----- the grant (owner) -----

    /// Starts a grant for one hour, replacing any other (its pending request is cancelled).
    /// Only while Unlocked.
    pub fn agent_grant_start(&mut self) -> Result<AgentGrantView, ApiError> {
        self.tick();
        self.open()?;
        self.agent_end(None);
        let id = self.agent.ids.next();
        self.agent.grant = Some(Grant {
            id,
            generation: self.session_generation,
            expires_at_ms: self.clock.now_ms().saturating_add(AGENT_GRANT_MAX_MS),
        });
        Ok(AgentGrantView {
            id,
            expires_in_ms: AGENT_GRANT_MAX_MS,
        })
    }

    /// Ends the grant, and cancels its pending request.
    pub fn agent_grant_stop(&mut self) {
        self.agent_end(None);
    }

    pub fn agent_grant_status(&self) -> AgentGrantStatus {
        let now = self.clock.now_ms();
        match &self.agent.grant {
            Some(grant) if self.agent_grant_live(grant.id) => AgentGrantStatus {
                state: "running",
                expires_in_ms: Some(grant.expires_at_ms.saturating_sub(now)),
            },
            Some(grant) if now >= grant.expires_at_ms => AgentGrantStatus {
                state: "expired",
                expires_in_ms: None,
            },
            Some(_) => AgentGrantStatus {
                state: "ended_by_lock",
                expires_in_ms: None,
            },
            None => AgentGrantStatus {
                state: self.agent.ended.unwrap_or("off"),
                expires_in_ms: None,
            },
        }
    }

    /// The Mac account the owner named for agents storing keys, if any. Not a secret.
    pub fn agent_account(&self) -> Option<String> {
        self.settings.agent_account.clone()
    }

    /// The owner names the agent's Mac account (or clears it with `None` or `""`). Only while
    /// Unlocked, like every other setting. A grant started for another account ends, so a new
    /// name takes effect only through a new Start, whose native confirm names it.
    pub fn set_agent_account(&mut self, name: Option<String>) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        let name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        if name.as_deref().is_some_and(|name| !is_account_name(name)) {
            return Err(err("invalid_input", MSG_AGENT_ACCOUNT_NAME));
        }
        if name == self.settings.agent_account {
            return Ok(());
        }
        let before = self.settings.agent_account.clone();
        self.settings.agent_account = name;
        if let Err(error) = self.write_settings() {
            self.settings.agent_account = before;
            return Err(error);
        }
        self.agent_end(None);
        Ok(())
    }

    /// The code hash of the agent's tool the owner approved (ADR-0013), as 40 lowercase hex
    /// digits. Not a secret: it identifies a program, it doesn't unlock anything.
    pub fn agent_tool_hash(&self) -> Option<String> {
        self.settings.agent_tool_hash.clone()
    }

    /// The owner approves the agent tool's code hash (or clears it with `None` or `""`). Only
    /// while Unlocked. Like a new account name, a new hash ends a running grant, so it takes
    /// effect only through a new Start.
    pub fn set_agent_tool_hash(&mut self, hash: Option<String>) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        let hash = match hash
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            None => None,
            Some(text) => Some(tool_hash_text(
                &parse_tool_hash(text).ok_or_else(|| err("invalid_input", MSG_AGENT_TOOL_HASH))?,
            )),
        };
        if hash == self.settings.agent_tool_hash {
            return Ok(());
        }
        let before = self.settings.agent_tool_hash.clone();
        self.settings.agent_tool_hash = hash;
        if let Err(error) = self.write_settings() {
            self.settings.agent_tool_hash = before;
            return Err(error);
        }
        self.agent_end(None);
        Ok(())
    }

    // ----- the five requests (agent) -----

    /// Request 1: asks the owner to store a key the agent describes. Returns the request id;
    /// the outcome is `saved` with the new Note's id, or `cancelled`.
    pub fn agent_store(
        &mut self,
        grant: u64,
        suggestion: AgentStoreSuggestion,
    ) -> Result<String, AgentError> {
        self.agent_begin(grant)?;
        check_suggestion(&suggestion)?;
        self.agent_new_request(grant, Request::Store(suggestion))
    }

    /// Request 2: the Notes the agent added, outside Trash.
    pub fn agent_list(&mut self, grant: u64) -> Result<Vec<AgentNote>, AgentError> {
        self.agent_begin(grant)?;
        Ok(self.agent_doc()?.agent_notes())
    }

    /// Request 3: changes the title or visible details of a Note the agent added, at once or
    /// after the owner allows it, by the rules in `notes::agent_edit_rule`.
    pub fn agent_edit(
        &mut self,
        grant: u64,
        id: &str,
        edit: AgentEdit,
    ) -> Result<AgentEditReply, AgentError> {
        self.agent_begin(grant)?;
        let doc = self.agent_doc()?;
        let before = doc.agent_note(id).ok_or(AgentError::NotFound)?;
        let kind = before.kind;
        // A name the agent may never change is refused whatever its value, so asking can't
        // tell whether the value is there.
        if edit
            .visible
            .keys()
            .any(|name| agent_edit_rule(kind, name) == AgentEditRule::Never)
        {
            return Err(AgentError::NotAllowed);
        }
        let fits = |text: &str| text.chars().count() <= MAX_AGENT_TEXT_CHARS;
        if !edit.title.as_deref().is_none_or(fits) || !edit.visible.values().all(|v| fits(v)) {
            return Err(AgentError::Invalid);
        }
        // Tried on a copy first: the notes crate's own checks and trimming decide what would
        // change, so the rule is the rule for what really changes.
        let mut copy = Document::from_json(&doc.to_json()).map_err(|_| AgentError::Internal)?;
        copy.update(
            id,
            NoteEdit {
                title: edit.title.clone(),
                visible: edit.visible.clone(),
                hidden: BTreeMap::new(),
            },
            None,
            self.clock.unix_secs(),
        )
        .map_err(|_| AgentError::Invalid)?;
        let after = copy.agent_note(id).ok_or(AgentError::Internal)?;
        drop(copy);
        let changes = changes(&before, &after);
        let title_changed = changes.iter().any(|c| c.field == "title");
        let fields: Vec<&str> = changes
            .iter()
            .map(|c| c.field.as_str())
            .filter(|f| *f != "title")
            .collect();
        match agent_edit_needs(kind, title_changed, &fields) {
            AgentEditRule::Never => Err(AgentError::NotAllowed),
            AgentEditRule::AtOnce => {
                if changes.is_empty() {
                    return Ok(AgentEditReply::Done);
                }
                if self.agent_at_once_blocked() {
                    return Err(AgentError::NotNow);
                }
                let AgentEdit { title, visible } = edit;
                self.mutate(|doc, now| {
                    doc.update(
                        id,
                        NoteEdit {
                            title,
                            visible,
                            hidden: BTreeMap::new(),
                        },
                        None,
                        now,
                    )
                })
                .map_err(agent_error)?;
                Ok(AgentEditReply::Done)
            }
            rule => {
                let AgentEdit { title, visible } = edit;
                let request = Request::Edit {
                    id: id.to_owned(),
                    before: Box::new(before),
                    after: Box::new(after),
                    title,
                    visible,
                    rule,
                };
                self.agent_new_request(grant, request)
                    .map(AgentEditReply::Pending)
            }
        }
    }

    /// Request 4: asks the owner to type a new value for a Note the agent added. The outcome
    /// is `saved` or `cancelled`.
    pub fn agent_replace(&mut self, grant: u64, id: &str) -> Result<String, AgentError> {
        self.agent_begin(grant)?;
        self.agent_doc()?
            .agent_note(id)
            .ok_or(AgentError::NotFound)?;
        self.agent_new_request(grant, Request::Replace { id: id.to_owned() })
    }

    /// Request 5: asks the owner to move a Note the agent added to Trash, which always needs
    /// the Master Password. The outcome is `deleted` or `cancelled`.
    pub fn agent_delete(&mut self, grant: u64, id: &str) -> Result<String, AgentError> {
        self.agent_begin(grant)?;
        self.agent_doc()?
            .agent_note(id)
            .ok_or(AgentError::NotFound)?;
        self.agent_new_request(grant, Request::Delete { id: id.to_owned() })
    }

    /// For the route waiting on a request: `None` while it is still pending, then its outcome
    /// once. An id this core doesn't know (any more) is `cancelled`.
    pub fn agent_take_outcome(&mut self, request_id: &str) -> Option<AgentOutcome> {
        self.tick();
        if let Some(at) = self
            .agent
            .outcomes
            .iter()
            .position(|(id, _)| id == request_id)
        {
            return self.agent.outcomes.remove(at).map(|(_, outcome)| outcome);
        }
        if self
            .agent
            .pending
            .as_ref()
            .is_some_and(|p| p.request_id == request_id)
        {
            return None;
        }
        Some(AgentOutcome::Cancelled)
    }

    // ----- the owner's side (WebView commands) -----

    /// What the screens should show for the pending request, if there is one.
    pub fn agent_pending(&mut self) -> Option<AgentPendingView> {
        self.tick();
        let pending = self.agent.pending.as_ref()?;
        if !self.agent_grant_live(pending.grant_id) {
            return None;
        }
        let doc = &self.open().ok()?.doc;
        let expires_in_ms = pending
            .created_at_ms
            .saturating_add(AGENT_REQUEST_MS)
            .saturating_sub(self.clock.now_ms());
        let of_note = |request: &'static str, id: &str, needs_password: Option<bool>| {
            let note = doc.agent_note(id)?;
            Some(AgentPendingView {
                request_id: pending.request_id.clone(),
                request,
                kind: note.kind,
                note_id: Some(note.id),
                title: note.title,
                visible: note.visible,
                changes: Vec::new(),
                needs_password: needs_password.unwrap_or(note.kind.is_wallet()),
                expires_in_ms,
            })
        };
        match &pending.request {
            Request::Store(suggestion) => Some(AgentPendingView {
                request_id: pending.request_id.clone(),
                request: "store",
                kind: suggestion.kind,
                note_id: None,
                title: suggestion.title.clone(),
                visible: suggestion.visible.clone(),
                changes: Vec::new(),
                needs_password: suggestion.kind.is_wallet(),
                expires_in_ms,
            }),
            Request::Edit {
                id,
                before,
                after,
                rule,
                ..
            } => of_note(
                "edit",
                id,
                Some(*rule == AgentEditRule::OwnerConfirmsWithPassword),
            )
            .map(|view| AgentPendingView {
                changes: changes(before, after),
                ..view
            }),
            Request::Replace { id } => of_note("replace", id, None),
            Request::Delete { id } => of_note("delete", id, Some(true)),
        }
    }

    /// The owner's Cancel. An id that isn't pending changes nothing.
    pub fn agent_cancel_request(&mut self, request_id: &str) {
        self.tick();
        if self
            .agent
            .pending
            .as_ref()
            .is_some_and(|p| p.request_id == request_id)
        {
            self.agent_cancel_pending();
        }
    }

    /// The owner takes the "added by an agent" mark off a Note: the agent can't see or touch it
    /// from now on, and a request about it is cancelled.
    pub fn remove_agent_mark(&mut self, id: &str) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.clear_agent_mark(id, now))?;
        if self
            .agent
            .pending
            .as_ref()
            .is_some_and(|p| p.request.target() == Some(id))
        {
            self.agent_cancel_pending();
        }
        Ok(())
    }

    // ----- hooks for the rest of the core -----

    /// Ends the grant and cancels its pending request. `reason` is what the status says
    /// afterwards (`None`: "off").
    pub(super) fn agent_end(&mut self, reason: Option<&'static str>) {
        if self.agent.grant.take().is_some() {
            self.agent.ended = reason;
        } else if reason.is_none() {
            self.agent.ended = None;
        }
        self.agent_cancel_pending();
    }

    /// From `tick`: a grant whose hour is over ends, and a request older than five minutes, one
    /// whose grant ended, or one about a Note that is no longer the agent's, is cancelled.
    pub(super) fn agent_tick(&mut self, now: u64) {
        let grant_over = self
            .agent
            .grant
            .as_ref()
            .is_some_and(|g| now >= g.expires_at_ms || g.generation != self.session_generation);
        if grant_over {
            let expired = self
                .agent
                .grant
                .as_ref()
                .is_some_and(|g| now >= g.expires_at_ms);
            self.agent_end(Some(if expired { "expired" } else { "ended_by_lock" }));
        }
        let grant_id = self.agent.grant.as_ref().map(|g| g.id);
        // A request about a Note the owner moved to Trash, deleted or otherwise took out of the
        // agent's reach can't be shown or completed, so it ends at once rather than keeping
        // every other request `busy` for the rest of its five minutes.
        let target_gone = |p: &Pending| {
            p.request.target().is_some_and(|id| {
                self.open()
                    .is_ok_and(|open| open.doc.agent_note(id).is_none())
            })
        };
        if self.agent.pending.as_ref().is_some_and(|p| {
            Some(p.grant_id) != grant_id
                || now >= p.created_at_ms.saturating_add(AGENT_REQUEST_MS)
                || target_gone(p)
        }) {
            self.agent_cancel_pending();
        }
    }

    /// From `begin_native`, before any panel opens: the request is still pending, its grant
    /// is live and the input fits it. A request that can no longer be completed is cancelled.
    pub(super) fn agent_check_native(&mut self, action: &NativeAction) -> Result<(), ApiError> {
        match self.agent_completion(action) {
            Ok(_) => Ok(()),
            Err(refusal) => {
                if refusal.cancel {
                    self.agent_cancel_pending();
                }
                Err(refusal.error)
            }
        }
    }

    /// From `begin_native` once the panel is open, so ending the request closes it.
    pub(super) fn agent_native_opened(&mut self, native_id: u64) {
        self.agent.native_id = Some(native_id);
    }

    /// The label, target, Kind and whether the Master Password is needed.
    pub(super) fn agent_description(
        &self,
        action: &NativeAction,
    ) -> Result<ActionDescription, ApiError> {
        self.agent_completion(action).map_err(|r| r.error)
    }

    /// From `commit_native`, after the panel: checked again, then applied. `authenticated`:
    /// the Master Password was typed and checked for this action.
    pub(super) fn agent_commit(
        &mut self,
        action: NativeAction,
        authenticated: bool,
    ) -> Result<NativeResult, ApiError> {
        let (_, _, kind, needs_password) = match self.agent_completion(&action) {
            Ok(described) => described,
            Err(refusal) => {
                if refusal.cancel {
                    self.agent_cancel_pending();
                }
                return Err(refusal.error);
            }
        };
        // The panel asked for the password exactly when it was needed; this holds even if a
        // future change to the panels didn't.
        if needs_password && !authenticated {
            return Err(not_now());
        }
        let wallet_kind = kind.is_some_and(Kind::is_wallet);
        let wallet = if wallet_kind {
            Some(self.wallet(None)?)
        } else {
            None
        };
        let cipher = wallet.as_ref().map(|w| w as &dyn WalletCipher);
        match action {
            NativeAction::AgentStore { request_id, input } => {
                let new = NewNote {
                    kind: input.kind,
                    title: input.title,
                    visible: input.visible,
                    hidden: input.hidden,
                };
                let id = self.mutate(|doc, now| doc.create_by_agent(new, cipher, now))?;
                self.agent_resolve(&request_id, AgentOutcome::Saved { id: id.clone() });
                Ok(NativeResult::NoteId(id))
            }
            NativeAction::AgentEdit { request_id } => {
                let Some(Request::Edit {
                    id, title, visible, ..
                }) = self.agent.pending.as_ref().map(|p| &p.request)
                else {
                    return Err(not_now());
                };
                let (id, edit) = (
                    id.clone(),
                    NoteEdit {
                        title: title.clone(),
                        visible: visible.clone(),
                        hidden: BTreeMap::new(),
                    },
                );
                self.mutate(|doc, now| doc.update(&id, edit, cipher, now))?;
                self.agent_resolve(&request_id, AgentOutcome::Done);
                Ok(NativeResult::Unit)
            }
            NativeAction::AgentReplace { request_id, input } => {
                let id = self.agent_target()?;
                let edit = NoteEdit {
                    title: None,
                    visible: BTreeMap::new(),
                    hidden: input.hidden,
                };
                self.mutate(|doc, now| doc.update(&id, edit, cipher, now))?;
                self.agent_resolve(&request_id, AgentOutcome::Saved { id });
                Ok(NativeResult::Unit)
            }
            NativeAction::AgentTrash { request_id } => {
                let id = self.agent_target()?;
                // The Wallet Key this panel derived is handed back for `trash_note` to use.
                self.native_wallet = wallet;
                self.trash_note(&id, None)?;
                self.agent_resolve(&request_id, AgentOutcome::Deleted);
                Ok(NativeResult::Unit)
            }
            _ => Err(not_now()),
        }
    }

    // ----- internals -----

    /// Every agent request starts here: due work first (Auto-lock among it), then the grant,
    /// then the phase. Never `session.activity`: agent requests don't keep Auto-lock away.
    fn agent_begin(&mut self, grant: u64) -> Result<(), AgentError> {
        self.tick();
        if !self.agent_grant_live(grant) {
            return Err(AgentError::Off);
        }
        self.open().map_err(|_| AgentError::Locked)?;
        Ok(())
    }

    fn agent_doc(&self) -> Result<&Document, AgentError> {
        self.open()
            .map(|open| &open.doc)
            .map_err(|_| AgentError::Locked)
    }

    fn agent_grant_live(&self, grant_id: u64) -> bool {
        self.agent.grant.as_ref().is_some_and(|g| {
            g.id == grant_id
                && g.generation == self.session_generation
                && self.clock.now_ms() < g.expires_at_ms
        })
    }

    /// An edit applied at once saves the Vault, which ends any panel or preview the owner has
    /// open. So it waits for those, and for a pending agent request the owner may be reading.
    fn agent_at_once_blocked(&self) -> bool {
        self.native_pending.is_some()
            || self.quick_pending.is_some()
            || self.inspected.is_some()
            || self.agent.pending.is_some()
    }

    fn agent_new_request(&mut self, grant: u64, request: Request) -> Result<String, AgentError> {
        if self.agent.pending.is_some() {
            return Err(AgentError::Busy);
        }
        let request_id = self.agent.ids.request_id();
        self.agent.pending = Some(Pending {
            request_id: request_id.clone(),
            grant_id: grant,
            created_at_ms: self.clock.now_ms(),
            request,
        });
        Ok(request_id)
    }

    fn agent_target(&self) -> Result<String, ApiError> {
        self.agent
            .pending
            .as_ref()
            .and_then(|p| p.request.target())
            .map(str::to_owned)
            .ok_or_else(not_now)
    }

    /// The pending request ended as `outcome`.
    fn agent_resolve(&mut self, request_id: &str, outcome: AgentOutcome) {
        if self
            .agent
            .pending
            .as_ref()
            .is_some_and(|p| p.request_id == request_id)
        {
            self.agent.pending = None;
            self.agent.native_id = None;
            self.agent_record(request_id.to_owned(), outcome);
        }
    }

    /// The pending request (if any) is cancelled, and a panel open to complete it closes.
    fn agent_cancel_pending(&mut self) {
        let Some(pending) = self.agent.pending.take() else {
            return;
        };
        // Only a panel opened for this request: native ids are never reused.
        let native_id = self.agent.native_id.take();
        if native_id.is_some() && self.native_pending_id() == native_id {
            self.invalidate_native();
        }
        self.agent_record(pending.request_id, AgentOutcome::Cancelled);
    }

    fn agent_record(&mut self, request_id: String, outcome: AgentOutcome) {
        if self.agent.outcomes.len() >= KEPT_OUTCOMES {
            self.agent.outcomes.pop_front();
        }
        self.agent.outcomes.push_back((request_id, outcome));
    }

    /// Whether `action` can complete the pending request now, and how it is described.
    fn agent_completion(&self, action: &NativeAction) -> Result<ActionDescription, Refusal> {
        let gone = || err("not_found", MSG_AGENT_REQUEST_GONE);
        let doc = &self.open().map_err(|e| refuse(e, false))?.doc;
        let request_id = match action {
            NativeAction::AgentStore { request_id, .. }
            | NativeAction::AgentEdit { request_id }
            | NativeAction::AgentReplace { request_id, .. }
            | NativeAction::AgentTrash { request_id } => request_id,
            _ => return Err(refuse(not_now(), false)),
        };
        let pending = self
            .agent
            .pending
            .as_ref()
            .filter(|p| p.request_id == *request_id)
            .ok_or_else(|| refuse(gone(), false))?;
        let now = self.clock.now_ms();
        if !self.agent_grant_live(pending.grant_id)
            || now >= pending.created_at_ms.saturating_add(AGENT_REQUEST_MS)
        {
            return Err(refuse(gone(), true));
        }
        let note = match pending.request.target() {
            Some(id) => Some(
                doc.agent_note(id)
                    .ok_or_else(|| refuse(note_not_found(), true))?,
            ),
            None => None,
        };
        match (action, &pending.request, note) {
            (NativeAction::AgentStore { input, .. }, Request::Store(suggestion), None) => {
                if input.kind != suggestion.kind {
                    return Err(refuse(err("invalid_input", MSG_AGENT_WRONG_KIND), false));
                }
                Ok((
                    "agent_store",
                    Some(input.title.clone()),
                    Some(input.kind),
                    input.kind.is_wallet(),
                ))
            }
            (NativeAction::AgentEdit { .. }, Request::Edit { before, rule, .. }, Some(note)) => {
                // The owner was shown "from" values: if they aren't there any more, the
                // request is about a Note that no longer exists in that form.
                if note != **before {
                    return Err(refuse(err("invalid_input", MSG_AGENT_NOTE_CHANGED), true));
                }
                Ok((
                    "agent_edit",
                    Some(note.title),
                    Some(note.kind),
                    *rule == AgentEditRule::OwnerConfirmsWithPassword,
                ))
            }
            (NativeAction::AgentReplace { input, .. }, Request::Replace { .. }, Some(note)) => {
                check_value_only(doc, &note, input).map_err(|e| refuse(e, false))?;
                Ok((
                    "agent_replace",
                    Some(note.title),
                    Some(note.kind),
                    note.kind.is_wallet(),
                ))
            }
            (NativeAction::AgentTrash { .. }, Request::Delete { .. }, Some(note)) => {
                Ok(("agent_trash", Some(note.title), Some(note.kind), true))
            }
            _ => Err(refuse(gone(), false)),
        }
    }
}

/// A replacement changes Hidden Fields only: the title and visible details sent with it must be
/// what the Note already has, and at least one value must be typed.
fn check_value_only(
    doc: &Document,
    note: &AgentNote,
    input: &NoteEditInput,
) -> Result<(), ApiError> {
    let view = doc.get(&note.id).ok_or_else(note_not_found)?;
    let same = |typed: &str, stored: Option<&String>| {
        stored.is_some_and(|s| s == typed) || typed.trim() == stored.map_or("", |s| s.as_str())
    };
    if !same(&input.title, Some(&view.title))
        || !input
            .visible
            .iter()
            .all(|(name, value)| same(value, view.visible.get(name)))
    {
        return Err(err("invalid_input", MSG_AGENT_VALUE_ONLY));
    }
    if input.hidden.is_empty() {
        return Err(err("invalid_input", MSG_AGENT_NO_VALUE));
    }
    if let Some(name) = input
        .hidden
        .keys()
        .find(|name| !note.kind.hidden_fields().contains(&name.as_str()))
    {
        return Err(notes_error(NotesError::UnknownField(name.clone())));
    }
    Ok(())
}

/// A Mac account short name: a lowercase letter, then up to 30 lowercase letters, digits, `_` or
/// `-` (the rule `scripts/agent-keys-setup.sh` and `scripts/companion-setup.sh` apply).
pub fn is_account_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && name.len() <= 31
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// The agent tool's code hash as the owner pastes it (ADR-0013): the 20-byte hash `codesign`
/// prints on its `CDHash=` line, as 40 hex digits, with or without that prefix.
pub fn parse_tool_hash(text: &str) -> Option<[u8; 20]> {
    let text = text.trim();
    let hex = text
        .strip_prefix("CDHash=")
        .or_else(|| text.strip_prefix("cdhash="))
        .unwrap_or(text);
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut hash = [0u8; 20];
    for (i, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(hash)
}

/// The same hash written the one way it is kept: 40 lowercase hex digits.
pub fn tool_hash_text(hash: &[u8; 20]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// A suggestion names only visible fields an agent may describe, with a title.
fn check_suggestion(suggestion: &AgentStoreSuggestion) -> Result<(), AgentError> {
    let fits = |text: &str| text.chars().count() <= MAX_AGENT_TEXT_CHARS;
    if suggestion.title.trim().is_empty() || !fits(&suggestion.title) {
        return Err(AgentError::Invalid);
    }
    for (name, value) in &suggestion.visible {
        if name == notes::field::WORD_COUNT
            || !suggestion.kind.visible_fields().contains(&name.as_str())
        {
            return Err(AgentError::NotAllowed);
        }
        if !fits(value) {
            return Err(AgentError::Invalid);
        }
    }
    Ok(())
}

/// Field by field, what `after` changes in `before` (`title` for the title).
fn changes(before: &AgentNote, after: &AgentNote) -> Vec<AgentChange> {
    let mut out = Vec::new();
    if before.title != after.title {
        out.push(AgentChange {
            field: "title".to_owned(),
            from: before.title.clone(),
            to: after.title.clone(),
        });
    }
    let names: BTreeSet<&String> = before.visible.keys().chain(after.visible.keys()).collect();
    for name in names {
        let (from, to) = (before.visible.get(name), after.visible.get(name));
        if from != to {
            out.push(AgentChange {
                field: name.clone(),
                from: from.cloned().unwrap_or_default(),
                to: to.cloned().unwrap_or_default(),
            });
        }
    }
    out
}

/// An owner-facing error turned into a code the agent may get. Never its message.
fn agent_error(error: ApiError) -> AgentError {
    match error.code {
        "locked" => AgentError::Locked,
        "not_found" => AgentError::NotFound,
        "invalid_input" => AgentError::Invalid,
        _ => AgentError::Internal,
    }
}
