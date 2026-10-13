//! Private storage of one Note and of the whole body, in the serialised shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::keys::Chain;
use crate::{
    BASE_SCHEMA_VERSION, Kind, NoteId, NoteSummary, NoteView, NotesError, SCHEMA_VERSION,
    Timestamp, WalletCipher, field, rules,
};

/// Visible fields that may be used as a list row's subtitle, in preference order.
const SUBTITLE_FIELDS: [&str; 5] = [
    field::WALLET_NAME,
    field::ADDRESS,
    field::WEBSITE,
    field::USERNAME,
    field::SERVICE,
];

/// The body as it is held in memory. It has no schema version of its own: the version is
/// checked when a body is read ([`ParsedBody`]) and worked out from the Notes whenever one is
/// written ([`Body::schema_version`]), so it can never be out of step with them.
pub(crate) struct Body {
    pub changed_at: Timestamp,
    pub change_counter: u64,
    pub notes: Vec<StoredNote>,
}

/// The body as it is read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParsedBody {
    pub schema_version: u32,
    pub changed_at: Timestamp,
    pub change_counter: u64,
    pub notes: Vec<StoredNote>,
}

/// The body as it is written: the keys of [`ParsedBody`], in the same order, borrowing the
/// Notes so that nothing is copied.
#[derive(Serialize)]
pub(crate) struct WrittenBody<'a> {
    schema_version: u32,
    changed_at: Timestamp,
    change_counter: u64,
    notes: &'a [StoredNote],
}

impl Body {
    /// The oldest schema that can hold these Notes, which is the one written: 2 if any Note
    /// has a value in a field schema 2 added, otherwise 1.
    pub fn schema_version(&self) -> u32 {
        if self.notes.iter().any(StoredNote::uses_schema_2) {
            SCHEMA_VERSION
        } else {
            BASE_SCHEMA_VERSION
        }
    }

    pub fn as_written(&self) -> WrittenBody<'_> {
        WrittenBody {
            schema_version: self.schema_version(),
            changed_at: self.changed_at,
            change_counter: self.change_counter,
            notes: &self.notes,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredNote {
    pub id: NoteId,
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub visible: BTreeMap<String, String>,
    /// Hidden Fields of ordinary Kinds.
    #[serde(default, rename = "hidden")]
    pub plain: BTreeMap<String, Zeroizing<String>>,
    /// Hidden Fields of Wallet Kinds: Wallet Key ciphertext, lowercase hex.
    #[serde(default)]
    pub sealed: BTreeMap<String, String>,
    #[serde(default)]
    pub favorite: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub deleted_at: Option<Timestamp>,
}

pub(crate) enum HiddenChange {
    Clear(String),
    Plain(String, Zeroizing<String>),
    Sealed(String, String),
}

impl StoredNote {
    pub fn new(id: NoteId, kind: Kind, title: String, now: Timestamp) -> StoredNote {
        StoredNote {
            id,
            kind,
            title,
            visible: BTreeMap::new(),
            plain: BTreeMap::new(),
            sealed: BTreeMap::new(),
            favorite: false,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    pub fn in_trash(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// In Trash for [`crate::TRASH_RETENTION_SECS`] or longer at `now`.
    pub fn expired(&self, now: Timestamp) -> bool {
        self.deleted_at
            .is_some_and(|d| d.saturating_add(crate::TRASH_RETENTION_SECS) <= now)
    }

    /// True if this Note has a value in a field that document schema 2 added.
    pub fn uses_schema_2(&self) -> bool {
        let added = self.kind.schema_2_fields();
        self.visible
            .keys()
            .chain(self.plain.keys())
            .chain(self.sealed.keys())
            .any(|name| added.contains(&name.as_str()))
    }

    /// True if this Note has a value in the Hidden Field `name`.
    fn has_value(&self, name: &str) -> bool {
        self.plain.contains_key(name) || self.sealed.contains_key(name)
    }

    /// True if setting this title and these checked visible values would change nothing. (An
    /// empty value asks for its field to be removed, so it matches a field that isn't there.)
    pub fn already_has(&self, title: Option<&str>, visible: &[(String, String)]) -> bool {
        title.is_none_or(|title| title == self.title)
            && visible.iter().all(|(name, value)| {
                self.visible.get(name).map_or("", String::as_str) == value.as_str()
            })
    }

    /// Applies checked changes. An empty visible value removes the field.
    pub fn apply(
        &mut self,
        visible: Vec<(String, String)>,
        word_count: Option<usize>,
        hidden: Vec<HiddenChange>,
    ) {
        let word_count = word_count.map(|n| (field::WORD_COUNT.to_string(), n.to_string()));
        for (name, value) in visible.into_iter().chain(word_count) {
            if value.is_empty() {
                self.visible.remove(&name);
            } else {
                self.visible.insert(name, value);
            }
        }
        for change in hidden {
            match change {
                HiddenChange::Clear(name) => {
                    self.plain.remove(&name);
                    self.sealed.remove(&name);
                }
                HiddenChange::Plain(name, value) => {
                    self.plain.insert(name, value);
                }
                HiddenChange::Sealed(name, hex) => {
                    self.sealed.insert(name, hex);
                }
            }
        }
    }

    /// Opens one sealed field (`EmptyField` if it has no value).
    pub fn open(
        &self,
        name: &str,
        cipher: &dyn WalletCipher,
    ) -> Result<Zeroizing<Vec<u8>>, NotesError> {
        let hex = self.sealed.get(name).ok_or(NotesError::EmptyField)?;
        let sealed = from_hex(hex).ok_or(NotesError::WalletCipher)?;
        cipher.open(&self.id, name, &sealed)
    }

    /// Title and visible values a search may match, plus the chain's name in words for a
    /// Private Key. Never Hidden Fields; `word_count` is a number, not a description, so it is
    /// left out.
    pub fn searchable_parts(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str())
            .chain(
                self.visible
                    .iter()
                    .filter(|(k, _)| k.as_str() != field::WORD_COUNT)
                    .map(|(_, v)| v.as_str()),
            )
            .chain(self.chain_words().iter().copied())
    }

    /// For a Private Key on a chain the app knows, that chain's name in words: the stored
    /// value is the short form (`evm`), and someone looking for the key types "ethereum".
    fn chain_words(&self) -> &'static [&'static str] {
        if self.kind != Kind::PrivateKey {
            return &[];
        }
        self.visible
            .get(field::CHAIN)
            .and_then(|chain| Chain::parse(chain))
            .map_or(&[], Chain::search_words)
    }

    /// True if every term (already lowercase) is found in some searchable part. The terms may
    /// be in different parts. No terms at all matches every Note.
    pub fn matches(&self, terms: &[&str]) -> bool {
        if terms.is_empty() {
            return true;
        }
        let parts: Vec<String> = self.searchable_parts().map(str::to_lowercase).collect();
        terms
            .iter()
            .all(|term| parts.iter().any(|part| part.contains(term)))
    }

    /// The first non-empty subtitle field. A Seed Phrase row shows its wallet name or nothing:
    /// an address there would read as the wallet's name.
    fn subtitle(&self) -> Option<String> {
        SUBTITLE_FIELDS
            .iter()
            .filter(|f| !(self.kind == Kind::SeedPhrase && **f == field::ADDRESS))
            .find_map(|f| self.visible.get(*f).filter(|v| !v.is_empty()).cloned())
    }

    pub fn summary(&self) -> NoteSummary {
        NoteSummary {
            id: self.id.clone(),
            kind: self.kind,
            title: self.title.clone(),
            subtitle: self.subtitle(),
            favorite: self.favorite,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
            expires: self
                .visible
                .get(field::EXPIRES)
                .filter(|_| self.kind == Kind::ApiKey)
                .cloned(),
            has_hidden: self
                .kind
                .hidden_fields()
                .first()
                .is_some_and(|name| self.has_value(name)),
        }
    }

    pub fn view(&self) -> NoteView {
        NoteView {
            id: self.id.clone(),
            kind: self.kind,
            title: self.title.clone(),
            visible: self.visible.clone(),
            hidden: self
                .kind
                .hidden_fields()
                .iter()
                .map(|f| (f.to_string(), self.has_value(f)))
                .collect(),
            favorite: self.favorite,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }

    pub fn validate(&self) -> Result<(), NotesError> {
        // Messages name fields and Kinds only, never values read from the body.
        let corrupt = |msg: String| Err(NotesError::Corrupt(msg));
        if self.id.len() != 32 || from_hex(&self.id).is_none() {
            return corrupt("a Note has a malformed id".into());
        }
        let kind = self.kind;
        let bad = |what: &str| corrupt(format!("a {kind:?} Note has {what}"));
        if self.title.trim().is_empty() {
            return bad("a blank title");
        }
        for (name, value) in &self.visible {
            if !kind.visible_fields().contains(&name.as_str()) {
                return bad("an unexpected visible field");
            }
            if value.is_empty() {
                return bad(&format!("an empty visible field {name}"));
            }
            if !rules::allows(kind, name, value) {
                return bad(&format!("a {name} that breaks the rule for that field"));
            }
        }
        let wallet = kind.is_wallet();
        for (name, value) in &self.plain {
            if wallet || !kind.hidden_fields().contains(&name.as_str()) {
                return bad("an unexpected Hidden Field");
            }
            if value.is_empty() {
                return bad(&format!("an empty Hidden Field {name}"));
            }
        }
        for (name, hex) in &self.sealed {
            if !wallet || !kind.hidden_fields().contains(&name.as_str()) {
                return bad("an unexpected wallet field");
            }
            if hex.is_empty() || from_hex(hex).is_none() {
                return corrupt(format!(
                    "a {kind:?} Note's wallet field {name} isn't non-empty lowercase hex"
                ));
            }
        }
        if kind == Kind::SeedPhrase {
            // Editing always keeps the words and derives word_count from them.
            if !self.sealed.contains_key(field::WORDS) {
                return corrupt("a Seed Phrase Note has no words".into());
            }
            let count_ok = self.visible.get(field::WORD_COUNT).is_some_and(|c| {
                crate::seed::ALLOWED_WORD_COUNTS
                    .iter()
                    .any(|n| n.to_string() == *c)
            });
            if !count_ok {
                return corrupt("a Seed Phrase Note's word_count isn't 12 or 24".into());
            }
        }
        Ok(())
    }
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase hex only.
pub(crate) fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}
