//! The Vault document: the plaintext that `vault-format` encrypts as the body.
//!
//! Vocabulary follows CONTEXT.md: a **Note** has one **Kind** (Seed Phrase, Private Key, Login,
//! API Key, Text). Its parts are either visible (safe to list, search and show) or **Hidden
//! Fields**. Hidden Fields of **Wallet Kinds** (Seed Phrase, Private Key) are stored already
//! encrypted under the Wallet Key (ADR-0004); this crate never sees that key. It asks a
//! [`WalletCipher`] to seal or open them.
//!
//! Rules this module guarantees (and tests):
//! * Listing and searching only ever touch visible parts. Search never matches Hidden Field contents.
//! * Hidden Fields are replace-only when editing: an edit sets a Hidden Field only if a new value
//!   is supplied, so editing never has to reveal the old value.
//! * Every mutation bumps `changed_at` and `change_counter` (used to spot an older Backup).
//! * A trashed Note stays in the document until 30 days after `deleted_at`, then
//!   [`Document::purge_expired`] removes it.
//!
//! Serialisation (`to_json` / `from_json`) is the body format documented in FORMAT.md. Wallet
//! field ciphertexts are stored as lowercase hex strings.

#![forbid(unsafe_code)]

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub mod keys;
mod note;
pub mod seed;

use note::{Body, StoredNote};

/// Seconds since the Unix epoch. Passed in by callers (no clock inside this crate).
pub type Timestamp = i64;
/// 16 random bytes, lowercase hex (32 chars).
pub type NoteId = String;

/// How long a Note stays in Trash.
pub const TRASH_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;
/// Current document schema.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    SeedPhrase,
    PrivateKey,
    Login,
    ApiKey,
    Text,
}

/// Field names. Visible and hidden field sets per Kind are fixed (see [`Kind::visible_fields`]).
pub mod field {
    pub const WALLET_NAME: &str = "wallet_name";
    pub const WORD_COUNT: &str = "word_count";
    pub const WORDS: &str = "words";
    pub const PASSPHRASE: &str = "passphrase";
    pub const CHAIN: &str = "chain";
    pub const ADDRESS: &str = "address";
    pub const KEY: &str = "key";
    pub const WEBSITE: &str = "website";
    pub const USERNAME: &str = "username";
    pub const PASSWORD: &str = "password";
    pub const SERVICE: &str = "service";
    pub const BODY: &str = "body";
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::SeedPhrase,
        Kind::PrivateKey,
        Kind::Login,
        Kind::ApiKey,
        Kind::Text,
    ];

    /// Seed Phrase and Private Key.
    pub fn is_wallet(self) -> bool {
        matches!(self, Kind::SeedPhrase | Kind::PrivateKey)
    }

    /// Seed Phrase: wallet_name, word_count. Private Key: chain, address. Login: website, username.
    /// API Key: service. Text: none. (The title is separate and always visible.)
    pub fn visible_fields(self) -> &'static [&'static str] {
        match self {
            Kind::SeedPhrase => &[field::WALLET_NAME, field::WORD_COUNT],
            Kind::PrivateKey => &[field::CHAIN, field::ADDRESS],
            Kind::Login => &[field::WEBSITE, field::USERNAME],
            Kind::ApiKey => &[field::SERVICE],
            Kind::Text => &[],
        }
    }

    /// Seed Phrase: words, passphrase. Private Key: key. Login: password. API Key: key. Text: body.
    pub fn hidden_fields(self) -> &'static [&'static str] {
        match self {
            Kind::SeedPhrase => &[field::WORDS, field::PASSPHRASE],
            Kind::PrivateKey => &[field::KEY],
            Kind::Login => &[field::PASSWORD],
            Kind::ApiKey => &[field::KEY],
            Kind::Text => &[field::BODY],
        }
    }
}

/// Seals/opens Wallet Kind Hidden Fields. Implemented by the app on top of
/// `vault_format::seal_wallet_field` with a freshly unlocked Wallet Key; tests use a fake.
pub trait WalletCipher {
    fn seal(&self, note_id: &str, field: &str, plaintext: &[u8]) -> Result<Vec<u8>, NotesError>;
    fn open(
        &self,
        note_id: &str,
        field: &str,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, NotesError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesError {
    NotFound,
    /// A visible or hidden field name that doesn't belong to this Kind.
    UnknownField(String),
    /// A Wallet Kind was created/edited/revealed without a [`WalletCipher`].
    NeedsWalletKey,
    /// Asked for a Hidden Field that has no value.
    EmptyField,
    /// Title is empty after trimming.
    EmptyTitle,
    /// Seed Phrase with a word count other than 12 or 24.
    BadWordCount(usize),
    /// Wallet field failed to decrypt.
    WalletCipher,
    /// Body JSON didn't parse, or has an unsupported schema.
    Corrupt(String),
    Randomness,
}

/// Input for a new Note.
pub struct NewNote {
    pub kind: Kind,
    pub title: String,
    pub visible: BTreeMap<String, String>,
    pub hidden: BTreeMap<String, Zeroizing<String>>,
}

/// An edit. `None`/absent means "keep". For Hidden Fields, an empty new value clears the field.
#[derive(Default)]
pub struct NoteEdit {
    pub title: Option<String>,
    pub visible: BTreeMap<String, String>,
    pub hidden: BTreeMap<String, Zeroizing<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "kind", rename_all = "snake_case")]
pub enum Filter {
    /// Every Note not in Trash.
    All,
    Favorites,
    Kind(Kind),
    Trash,
}

/// What a list row may show. Visible parts only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NoteSummary {
    pub id: NoteId,
    pub kind: Kind,
    pub title: String,
    /// One visible detail for the row: wallet name, address, website, username or service (first non-empty).
    pub subtitle: Option<String>,
    pub favorite: bool,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

/// What the detail view may show before anything is revealed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NoteView {
    pub id: NoteId,
    pub kind: Kind,
    pub title: String,
    pub visible: BTreeMap<String, String>,
    /// Every Hidden Field of the Kind, in `Kind::hidden_fields` order, and whether it has a value.
    pub hidden: Vec<(String, bool)>,
    pub favorite: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

/// The whole Vault content. Implementers choose the private representation, but it must
/// round-trip through `to_json`/`from_json` and keep plain Hidden Field values in zeroizing types.
pub struct Document {
    body: Body,
}

impl Document {
    pub fn new(now: Timestamp) -> Document {
        Document {
            body: Body {
                schema_version: SCHEMA_VERSION,
                changed_at: now,
                change_counter: 0,
                notes: Vec::new(),
            },
        }
    }

    /// Parse a decrypted body. Rejects unknown schema versions and Kind/field mismatches.
    pub fn from_json(bytes: &[u8]) -> Result<Document, NotesError> {
        // The version is read on its own first, so a body from another schema gets a clear
        // "unsupported schema" error instead of failing somewhere inside the Notes.
        #[derive(Deserialize)]
        struct Version {
            schema_version: u32,
        }
        let Version { schema_version } = serde_json::from_slice(bytes).map_err(parse_error)?;
        if schema_version != SCHEMA_VERSION {
            let msg = format!("unsupported schema version {schema_version}");
            return Err(NotesError::Corrupt(msg));
        }
        let body: Body = serde_json::from_slice(bytes).map_err(parse_error)?;
        let mut ids = BTreeSet::new();
        for note in &body.notes {
            note.validate()?;
            if !ids.insert(note.id.as_str()) {
                let msg = format!("two Notes share the id {}", note.id);
                return Err(NotesError::Corrupt(msg));
            }
        }
        Ok(Document { body })
    }

    pub fn to_json(&self) -> Zeroizing<Vec<u8>> {
        // Measure first, then write into one exact allocation: a growing Vec would free earlier
        // copies of the plaintext without wiping them.
        let mut counter = ByteCounter(0);
        // Serialising plain structs with string keys cannot fail, and neither can these writers.
        serde_json::to_writer(&mut counter, &self.body)
            .expect("serialising the Vault document cannot fail");
        let mut out = Zeroizing::new(Vec::with_capacity(counter.0));
        serde_json::to_writer(&mut *out, &self.body)
            .expect("serialising the Vault document cannot fail");
        debug_assert_eq!(out.len(), counter.0);
        out
    }

    pub fn changed_at(&self) -> Timestamp {
        self.body.changed_at
    }

    pub fn change_counter(&self) -> u64 {
        self.body.change_counter
    }

    /// True if `self` was last changed before `other` (by `changed_at`, ties broken by `change_counter`).
    pub fn is_older_than(&self, other: &Document) -> bool {
        (self.changed_at(), self.change_counter()) < (other.changed_at(), other.change_counter())
    }

    /// Notes not in Trash.
    pub fn count(&self) -> usize {
        self.body.notes.iter().filter(|n| !n.in_trash()).count()
    }

    /// Wallet Kinds need `cipher` (else `NeedsWalletKey`). Seed Phrase words must number 12 or 24
    /// (`BadWordCount`); `word_count` is set from the words, not trusted from input.
    pub fn create(
        &mut self,
        new: NewNote,
        cipher: Option<&dyn WalletCipher>,
        now: Timestamp,
    ) -> Result<NoteId, NotesError> {
        let kind = new.kind;
        let title = checked_title(&new.title)?;
        if kind.is_wallet() && cipher.is_none() {
            return Err(NotesError::NeedsWalletKey);
        }
        let visible = checked_visible(kind, new.visible)?;
        let (hidden, word_count) = checked_hidden(kind, new.hidden)?;
        if kind == Kind::SeedPhrase && word_count.is_none() {
            return Err(NotesError::BadWordCount(0));
        }
        let id = new_note_id()?;
        let mut note = StoredNote::new(id.clone(), kind, title, now);
        let changes = seal_changes(&note, hidden, cipher)?;
        note.apply(visible, word_count, changes);
        self.body.notes.push(note);
        self.touch(now);
        Ok(id)
    }

    /// Wallet Kinds need `cipher` only if the edit touches a Hidden Field.
    pub fn update(
        &mut self,
        id: &str,
        edit: NoteEdit,
        cipher: Option<&dyn WalletCipher>,
        now: Timestamp,
    ) -> Result<(), NotesError> {
        let note = self.find(id)?;
        let kind = note.kind;
        let title = edit.title.map(|t| checked_title(&t)).transpose()?;
        let visible = checked_visible(kind, edit.visible)?;
        let (hidden, word_count) = checked_hidden(kind, edit.hidden)?;
        if kind.is_wallet() && cipher.is_none() && !hidden.is_empty() {
            return Err(NotesError::NeedsWalletKey);
        }
        let changes = seal_changes(note, hidden, cipher)?;

        // Everything is checked; apply.
        let note = self.find_mut(id)?;
        if let Some(title) = title {
            note.title = title;
        }
        note.apply(visible, word_count, changes);
        note.updated_at = now;
        self.touch(now);
        Ok(())
    }

    /// Trashing a Note already in Trash changes nothing (its `deleted_at` is kept).
    pub fn trash(&mut self, id: &str, now: Timestamp) -> Result<(), NotesError> {
        let note = self.find_mut(id)?;
        if note.deleted_at.is_none() {
            note.deleted_at = Some(now);
            self.touch(now);
        }
        Ok(())
    }

    /// Restoring a Note that isn't in Trash changes nothing.
    pub fn restore(&mut self, id: &str, now: Timestamp) -> Result<(), NotesError> {
        let note = self.find_mut(id)?;
        if note.deleted_at.take().is_some() {
            self.touch(now);
        }
        Ok(())
    }

    /// Only for Notes already in Trash.
    pub fn delete_forever(&mut self, id: &str, now: Timestamp) -> Result<(), NotesError> {
        let pos = self
            .body
            .notes
            .iter()
            .position(|n| n.id == id && n.in_trash())
            .ok_or(NotesError::NotFound)?;
        self.body.notes.remove(pos);
        self.touch(now);
        Ok(())
    }

    /// Returns how many Notes were removed.
    pub fn empty_trash(&mut self, now: Timestamp) -> usize {
        self.remove_where(now, |n| n.in_trash())
    }

    /// Remove Notes whose `deleted_at + TRASH_RETENTION_SECS <= now`. Returns how many.
    pub fn purge_expired(&mut self, now: Timestamp) -> usize {
        self.remove_where(now, |n| {
            n.deleted_at
                .is_some_and(|d| d.saturating_add(TRASH_RETENTION_SECS) <= now)
        })
    }

    pub fn set_favorite(
        &mut self,
        id: &str,
        favorite: bool,
        now: Timestamp,
    ) -> Result<(), NotesError> {
        let note = self.find_mut(id)?;
        if note.favorite != favorite {
            note.favorite = favorite;
            self.touch(now);
        }
        Ok(())
    }

    /// Filtered, then matched case-insensitively against the title and visible values (substring).
    /// Empty `query` matches everything. Order: Favorites first, then most recently updated.
    /// Trash is ordered by `deleted_at`, newest first.
    pub fn list(&self, filter: Filter, query: &str) -> Vec<NoteSummary> {
        let query = query.trim().to_lowercase();
        let mut found: Vec<&StoredNote> = self
            .body
            .notes
            .iter()
            .filter(|n| match filter {
                Filter::All => !n.in_trash(),
                Filter::Favorites => !n.in_trash() && n.favorite,
                Filter::Kind(k) => !n.in_trash() && n.kind == k,
                Filter::Trash => n.in_trash(),
            })
            .filter(|n| {
                query.is_empty()
                    || n.searchable_parts()
                        .any(|p| p.to_lowercase().contains(&query))
            })
            .collect();
        match filter {
            Filter::Trash => found.sort_by_key(|&n| (Reverse(n.deleted_at), &n.id)),
            _ => found.sort_by_key(|&n| (Reverse(n.favorite), Reverse(n.updated_at), &n.id)),
        }
        found.into_iter().map(StoredNote::summary).collect()
    }

    pub fn get(&self, id: &str) -> Option<NoteView> {
        self.find(id).ok().map(StoredNote::view)
    }

    /// The value of one Hidden Field of a non-wallet Kind. Wallet Kinds -> `NeedsWalletKey`.
    pub fn reveal(&self, id: &str, field: &str) -> Result<Zeroizing<String>, NotesError> {
        let note = self.find(id)?;
        let field = hidden_field_of(note.kind, field)?;
        if note.kind.is_wallet() {
            return Err(NotesError::NeedsWalletKey);
        }
        note.plain.get(field).cloned().ok_or(NotesError::EmptyField)
    }

    /// The value of one Hidden Field of a Wallet Kind, opened with `cipher`.
    pub fn reveal_wallet(
        &self,
        id: &str,
        field: &str,
        cipher: &dyn WalletCipher,
    ) -> Result<Zeroizing<String>, NotesError> {
        let note = self.find(id)?;
        let field = hidden_field_of(note.kind, field)?;
        if !note.kind.is_wallet() {
            // Ordinary Kinds have no wallet fields; use `reveal`.
            return Err(NotesError::UnknownField(field.to_string()));
        }
        let plain = note.open(field, cipher)?;
        let text = std::str::from_utf8(&plain)
            .map_err(|_| NotesError::Corrupt("a wallet field isn't valid text".into()))?;
        Ok(Zeroizing::new(text.to_string()))
    }

    /// Key Rotation support: open every wallet field with `old`, seal it with `new`.
    pub fn reencrypt_wallet_fields(
        &mut self,
        old: &dyn WalletCipher,
        new: &dyn WalletCipher,
        now: Timestamp,
    ) -> Result<(), NotesError> {
        // Re-seal everything first so a failure leaves the document untouched.
        let mut resealed: Vec<(usize, String, String)> = Vec::new();
        for (i, note) in self.body.notes.iter().enumerate() {
            for name in note.sealed.keys() {
                let plain = note.open(name, old)?;
                let sealed = new.seal(&note.id, name, &plain)?;
                resealed.push((i, name.clone(), note::to_hex(&sealed)));
            }
        }
        for (i, name, hex) in resealed {
            self.body.notes[i].sealed.insert(name, hex);
        }
        self.touch(now);
        Ok(())
    }

    fn touch(&mut self, now: Timestamp) {
        self.body.changed_at = now;
        self.body.change_counter = self.body.change_counter.saturating_add(1);
    }

    fn find(&self, id: &str) -> Result<&StoredNote, NotesError> {
        self.body
            .notes
            .iter()
            .find(|n| n.id == id)
            .ok_or(NotesError::NotFound)
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut StoredNote, NotesError> {
        self.body
            .notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(NotesError::NotFound)
    }

    fn remove_where(&mut self, now: Timestamp, doomed: impl Fn(&StoredNote) -> bool) -> usize {
        let before = self.body.notes.len();
        self.body.notes.retain(|n| !doomed(n));
        let removed = before - self.body.notes.len();
        if removed > 0 {
            self.touch(now);
        }
        removed
    }
}

/// A parse failure, described without serde_json's message: that message can quote the
/// offending value, which may be a Hidden Field.
fn parse_error(e: serde_json::Error) -> NotesError {
    use serde_json::error::Category;
    let what = match e.classify() {
        Category::Io => "couldn't be read",
        Category::Syntax => "isn't valid JSON",
        Category::Data => "doesn't have the expected shape",
        Category::Eof => "ends too early",
    };
    NotesError::Corrupt(format!(
        "the Vault document {what} (line {}, column {})",
        e.line(),
        e.column()
    ))
}

/// An `io::Write` that only counts bytes.
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn checked_title(title: &str) -> Result<String, NotesError> {
    let title = title.trim();
    if title.is_empty() {
        Err(NotesError::EmptyTitle)
    } else {
        Ok(title.to_string())
    }
}

fn hidden_field_of(kind: Kind, name: &str) -> Result<&'static str, NotesError> {
    kind.hidden_fields()
        .iter()
        .copied()
        .find(|f| *f == name)
        .ok_or_else(|| NotesError::UnknownField(name.to_string()))
}

/// Checks visible field names. `word_count` is derived from the words, so input for it is dropped.
fn checked_visible(
    kind: Kind,
    visible: BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, NotesError> {
    let mut out = Vec::new();
    for (name, value) in visible {
        if !kind.visible_fields().contains(&name.as_str()) {
            return Err(NotesError::UnknownField(name));
        }
        if name != field::WORD_COUNT {
            out.push((name, value));
        }
    }
    Ok(out)
}

/// A Hidden Field name and its new plain value.
type HiddenValue = (String, Zeroizing<String>);

/// Checks Hidden Field names; normalises and counts Seed Phrase words (returning the count when
/// words were supplied). Empty values mean "clear", except that Seed Phrase words can't be empty.
fn checked_hidden(
    kind: Kind,
    hidden: BTreeMap<String, Zeroizing<String>>,
) -> Result<(Vec<HiddenValue>, Option<usize>), NotesError> {
    let mut out = Vec::new();
    let mut word_count = None;
    for (name, value) in hidden {
        hidden_field_of(kind, &name)?;
        if kind == Kind::SeedPhrase && name == field::WORDS {
            let words = Zeroizing::new(seed::normalize(&value));
            let count = words.split_whitespace().count();
            if !seed::ALLOWED_WORD_COUNTS.contains(&count) {
                return Err(NotesError::BadWordCount(count));
            }
            word_count = Some(count);
            out.push((name, words));
        } else {
            out.push((name, value));
        }
    }
    Ok((out, word_count))
}

/// Turns checked Hidden Field values into ready-to-apply changes, sealing Wallet Kind fields.
fn seal_changes(
    note: &StoredNote,
    hidden: Vec<HiddenValue>,
    cipher: Option<&dyn WalletCipher>,
) -> Result<Vec<note::HiddenChange>, NotesError> {
    let mut changes = Vec::with_capacity(hidden.len());
    for (name, value) in hidden {
        let change = if value.is_empty() {
            note::HiddenChange::Clear(name)
        } else if note.kind.is_wallet() {
            let cipher = cipher.ok_or(NotesError::NeedsWalletKey)?;
            let sealed = cipher.seal(&note.id, &name, value.as_bytes())?;
            note::HiddenChange::Sealed(name, note::to_hex(&sealed))
        } else {
            note::HiddenChange::Plain(name, value)
        };
        changes.push(change);
    }
    Ok(changes)
}

fn new_note_id() -> Result<NoteId, NotesError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| NotesError::Randomness)?;
    Ok(note::to_hex(&bytes))
}
