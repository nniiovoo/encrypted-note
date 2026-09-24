//! Private storage of one Note and of the whole body, in the serialised shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{Kind, NoteId, NoteSummary, NoteView, NotesError, Timestamp, WalletCipher, field};

/// Visible fields that may be used as a list row's subtitle, in preference order.
const SUBTITLE_FIELDS: [&str; 5] = [
    field::WALLET_NAME,
    field::ADDRESS,
    field::WEBSITE,
    field::USERNAME,
    field::SERVICE,
];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub schema_version: u32,
    pub changed_at: Timestamp,
    pub change_counter: u64,
    pub notes: Vec<StoredNote>,
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

    /// Title and visible values a search may match. Never Hidden Fields; `word_count` is a number,
    /// not a description, so it is left out.
    pub fn searchable_parts(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str()).chain(
            self.visible
                .iter()
                .filter(|(k, _)| k.as_str() != field::WORD_COUNT)
                .map(|(_, v)| v.as_str()),
        )
    }

    pub fn summary(&self) -> NoteSummary {
        NoteSummary {
            id: self.id.clone(),
            kind: self.kind,
            title: self.title.clone(),
            subtitle: SUBTITLE_FIELDS
                .iter()
                .find_map(|f| self.visible.get(*f).filter(|v| !v.is_empty()).cloned()),
            favorite: self.favorite,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
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
                .map(|f| {
                    (
                        f.to_string(),
                        self.plain.contains_key(*f) || self.sealed.contains_key(*f),
                    )
                })
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
