//! What an agent may see and change of the Vault (ADR-0012): only the Notes it added, which
//! carry the "added by an agent" mark, and only their visible parts.
//!
//! Rules this module guarantees (and tests):
//! * The mark is set only by [`Document::create_by_agent`]. Nothing puts it on an existing Note,
//!   so an agent can't take over a Note the owner added. The owner can take it off
//!   ([`Document::clear_agent_mark`]).
//! * [`Document::agent_notes`] and [`Document::agent_note`] hold only marked Notes outside
//!   Trash. A Note without the mark, a Note in Trash and an id that doesn't exist all look the
//!   same: absent.
//! * An [`AgentNote`] holds the id, Kind, title, visible fields (never `word_count`, which is
//!   counted from the words) and two timestamps. Never a Hidden Field, never whether a Hidden
//!   Field has a value (that would tell whether a Seed Phrase has a Passphrase), never
//!   `favorite`.
//! * [`agent_edit_rule`] decides, per field, whether an agent's edit applies at once, needs the
//!   owner's confirmation, needs the confirmation and the Master Password, or is never allowed.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use serde::Serialize;

use crate::note::StoredNote;
use crate::{Document, Kind, NewNote, NoteId, NotesError, Timestamp, WalletCipher, field};

/// The name [`agent_edit_rule`] gives a Note's title, which is not one of the Kind's fields.
const TITLE: &str = "title";

/// What an agent is told about one Note it added. Built only from visible parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentNote {
    pub id: NoteId,
    pub kind: Kind,
    pub title: String,
    /// The Kind's visible fields that have a value, except `word_count`.
    pub visible: BTreeMap<String, String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl AgentNote {
    fn of(note: &StoredNote) -> AgentNote {
        let visible = note
            .kind
            .visible_fields()
            .iter()
            .filter(|name| **name != field::WORD_COUNT)
            .filter_map(|name| {
                let value = note.visible.get(*name)?;
                Some((name.to_string(), value.clone()))
            })
            .collect();
        AgentNote {
            id: note.id.clone(),
            kind: note.kind,
            title: note.title.clone(),
            visible,
            created_at: note.created_at,
            updated_at: note.updated_at,
        }
    }
}

/// How an agent's change to one field of a Note it added may happen. Ordered from least to
/// most strict, so the strictest of several is their maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgentEditRule {
    /// Applied when asked: a name, a purpose or an expiry date can only confuse the owner.
    AtOnce,
    /// Applied only after the owner allows it in the app: the field says where a key is used.
    OwnerConfirms,
    /// Applied only after the owner allows it and types the Master Password: every edit of a
    /// Wallet Kind (Seed Phrase, Private Key), its title included.
    OwnerConfirmsWithPassword,
    /// Never changed by an agent: `word_count`, every Hidden Field (values change only by the
    /// owner typing a replacement), and any name that isn't a visible field of the Kind.
    Never,
}

/// The rule for an agent changing `field` of a `kind` Note it added. `"title"` names the
/// title. Any other name that isn't a visible field of the Kind is [`AgentEditRule::Never`].
pub fn agent_edit_rule(kind: Kind, field: &str) -> AgentEditRule {
    use AgentEditRule::{AtOnce, Never, OwnerConfirms, OwnerConfirmsWithPassword};
    if field == TITLE {
        return if kind.is_wallet() {
            OwnerConfirmsWithPassword
        } else {
            AtOnce
        };
    }
    if field == field::WORD_COUNT || !kind.visible_fields().contains(&field) {
        return Never;
    }
    if kind.is_wallet() {
        return OwnerConfirmsWithPassword;
    }
    match (kind, field) {
        (Kind::Login, field::PURPOSE) | (Kind::ApiKey, field::PURPOSE | field::EXPIRES) => AtOnce,
        (Kind::Login, field::WEBSITE | field::USERNAME)
        | (Kind::ApiKey, field::SERVICE | field::KEY_ID | field::URL | field::ENV_VAR) => {
            OwnerConfirms
        }
        // A visible field added later gets no agent edits until it is given a rule here.
        _ => Never,
    }
}

/// The strictest rule among a title change (if `title_changed`) and changes to `fields`. An
/// edit that changes nothing is [`AgentEditRule::AtOnce`].
pub fn agent_edit_needs(kind: Kind, title_changed: bool, fields: &[&str]) -> AgentEditRule {
    title_changed
        .then_some(TITLE)
        .into_iter()
        .chain(fields.iter().copied())
        .map(|name| agent_edit_rule(kind, name))
        .max()
        .unwrap_or(AgentEditRule::AtOnce)
}

impl Document {
    /// [`Document::create`], with the Note marked "added by an agent" from its first save. The
    /// only way the mark is ever set. The same checks apply: a Wallet Kind needs `cipher`.
    pub fn create_by_agent(
        &mut self,
        new: NewNote,
        cipher: Option<&dyn WalletCipher>,
        now: Timestamp,
    ) -> Result<NoteId, NotesError> {
        self.create_marked(new, cipher, now, true)
    }

    /// The Notes an agent added, outside Trash, most recently updated first (then by id). Not
    /// ordered by `favorite`, which would tell the agent which ones the owner starred.
    pub fn agent_notes(&self) -> Vec<AgentNote> {
        let mut found: Vec<&StoredNote> = self.agent_visible().collect();
        found.sort_by_key(|&n| (Reverse(n.updated_at), &n.id));
        found.into_iter().map(AgentNote::of).collect()
    }

    /// One Note an agent added. `None` for an unmarked Note, a Note in Trash and a missing id
    /// alike, so the answer never tells that an owner's Note exists.
    pub fn agent_note(&self, id: &str) -> Option<AgentNote> {
        self.agent_visible().find(|n| n.id == id).map(AgentNote::of)
    }

    /// The owner takes the mark off a Note, Trash included, which takes it out of the agent's
    /// reach at once. A change to the Vault, but not to the Note's content, so `updated_at`
    /// stays. A Note without the mark changes nothing.
    pub fn clear_agent_mark(&mut self, id: &str, now: Timestamp) -> Result<(), NotesError> {
        let note = self.find_mut(id)?;
        if note.added_by_agent {
            note.added_by_agent = false;
            self.touch(now);
        }
        Ok(())
    }

    /// Marked Notes outside Trash: everything an agent may know of.
    fn agent_visible(&self) -> impl Iterator<Item = &StoredNote> {
        self.body
            .notes
            .iter()
            .filter(|n| n.added_by_agent && !n.in_trash())
    }
}
