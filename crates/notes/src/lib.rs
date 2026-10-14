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
//!   is supplied, so editing never has to reveal the old value. Adding a line to a Text body
//!   ([`Document::append_text`]) joins old and new inside this crate for the same reason.
//! * Every mutation bumps `changed_at` and `change_counter` (used to spot an older Backup). An
//!   edit that changes nothing is not a mutation.
//! * A trashed Note stays in the document until 30 days after `deleted_at`, then
//!   [`Document::purge_expired`] removes it.
//! * The document is written as the oldest schema that can hold it: schema 2 only while some
//!   Note uses a field schema 2 added ([`Kind::schema_2_fields`]), so a Vault that doesn't use
//!   them stays readable by older builds and older Emergency Readers.
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
mod rules;
pub mod seed;

use note::{Body, ParsedBody, StoredNote};

pub use rules::{MAX_ENV_VAR_CHARS, MAX_PURPOSE_CHARS};

/// Seconds since the Unix epoch. Passed in by callers (no clock inside this crate).
pub type Timestamp = i64;
/// 16 random bytes, lowercase hex (32 chars).
pub type NoteId = String;

/// How long a Note stays in Trash.
pub const TRASH_RETENTION_SECS: i64 = 30 * 24 * 60 * 60;
/// The newest document schema this build reads and writes.
pub const SCHEMA_VERSION: u32 = 2;
/// The oldest document schema, and the one written while no Note uses a field that schema 2
/// added.
pub const BASE_SCHEMA_VERSION: u32 = 1;
/// The largest `change_counter` a body may carry. No real Vault gets near it (one count per
/// saved change); a body claiming more is rejected, because a counter that can't grow any more
/// would make every later change look like "nothing changed".
pub const MAX_CHANGE_COUNTER: u64 = 1 << 53;

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
    // Added by document schema 2 (all optional).
    pub const DERIVATION_PATH: &str = "derivation_path";
    pub const PURPOSE: &str = "purpose";
    pub const KEY_ID: &str = "key_id";
    pub const URL: &str = "url";
    pub const EXPIRES: &str = "expires";
    pub const ENV_VAR: &str = "env_var";
    pub const SECRET: &str = "secret";
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

    /// Seed Phrase: wallet_name, word_count, address, derivation_path. Private Key: chain,
    /// address. Login: website, username, purpose. API Key: service, key_id, purpose, url,
    /// expires, env_var. Text: none. (The title is separate and always visible.)
    pub fn visible_fields(self) -> &'static [&'static str] {
        match self {
            Kind::SeedPhrase => &[
                field::WALLET_NAME,
                field::WORD_COUNT,
                field::ADDRESS,
                field::DERIVATION_PATH,
            ],
            Kind::PrivateKey => &[field::CHAIN, field::ADDRESS],
            Kind::Login => &[field::WEBSITE, field::USERNAME, field::PURPOSE],
            Kind::ApiKey => &[
                field::SERVICE,
                field::KEY_ID,
                field::PURPOSE,
                field::URL,
                field::EXPIRES,
                field::ENV_VAR,
            ],
            Kind::Text => &[],
        }
    }

    /// Seed Phrase: words, passphrase. Private Key: key. Login: password. API Key: key, secret.
    /// Text: body. The first one is the Kind's main Hidden Field.
    pub fn hidden_fields(self) -> &'static [&'static str] {
        match self {
            Kind::SeedPhrase => &[field::WORDS, field::PASSPHRASE],
            Kind::PrivateKey => &[field::KEY],
            Kind::Login => &[field::PASSWORD],
            Kind::ApiKey => &[field::KEY, field::SECRET],
            Kind::Text => &[field::BODY],
        }
    }

    /// The optional fields document schema 2 added to this Kind, visible and hidden together.
    /// Seed Phrase: address, derivation_path. Login: purpose. API Key: key_id, purpose, url,
    /// expires, env_var and the Hidden Field secret. A document in which no Note has a value in
    /// one of these is written as schema 1.
    pub fn schema_2_fields(self) -> &'static [&'static str] {
        match self {
            Kind::SeedPhrase => &[field::ADDRESS, field::DERIVATION_PATH],
            Kind::Login => &[field::PURPOSE],
            Kind::ApiKey => &[
                field::KEY_ID,
                field::PURPOSE,
                field::URL,
                field::EXPIRES,
                field::ENV_VAR,
                field::SECRET,
            ],
            Kind::PrivateKey | Kind::Text => &[],
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
    /// A supplied value breaks the rule for its field (a date that doesn't exist, a line break
    /// where one line is expected, an empty addition to a Text body). Holds the field's name
    /// and never the value.
    InvalidValue(String),
    /// Wallet field failed to decrypt.
    WalletCipher,
    /// Body JSON didn't parse, or breaks a rule of the schema.
    Corrupt(String),
    /// The body was written with another (newer) document schema.
    UnsupportedSchema(u32),
    Randomness,
}

/// Input for a new Note. Visible values are trimmed; a pasted key is tidied (see
/// [`Document::create`]).
pub struct NewNote {
    pub kind: Kind,
    pub title: String,
    pub visible: BTreeMap<String, String>,
    pub hidden: BTreeMap<String, Zeroizing<String>>,
}

/// An edit. `None`/absent means "keep", and so does a visible value that is exactly the stored
/// one. An empty visible value removes the field, and so does one that is only whitespace. For
/// Hidden Fields, an empty new value clears the field.
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
    /// One visible detail for the row: wallet name, address, website, username or service (first
    /// non-empty). A Seed Phrase shows its wallet name or nothing, never its address.
    pub subtitle: Option<String>,
    pub favorite: bool,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
    /// The `expires` date of an API Key (`YYYY-MM-DD`), so a row can show it. `None` for a Note
    /// without one and for every other Kind.
    pub expires: Option<String>,
    /// True if the Kind's first Hidden Field has a value, so a row knows whether there is
    /// anything to copy.
    pub has_hidden: bool,
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
                changed_at: now,
                change_counter: 0,
                notes: Vec::new(),
            },
        }
    }

    /// Parse a decrypted body. Schema 1 and 2 are read; any other version is
    /// `UnsupportedSchema`. Rejects Kind/field mismatches, values that break the original
    /// stored rules for their field, and a body that says schema 1 but uses a field schema 2
    /// added. Legacy Unicode line and paragraph separators in visible values are preserved;
    /// new or changed values must exclude them.
    pub fn from_json(bytes: &[u8]) -> Result<Document, NotesError> {
        // The version is read on its own first, so a body from another schema gets a clear
        // "unsupported schema" error instead of failing somewhere inside the Notes.
        #[derive(Deserialize)]
        struct Version {
            schema_version: u32,
        }
        let Version { schema_version } =
            serde_json::from_slice_zeroizing(bytes).map_err(parse_error)?;
        if !(BASE_SCHEMA_VERSION..=SCHEMA_VERSION).contains(&schema_version) {
            return Err(NotesError::UnsupportedSchema(schema_version));
        }
        let parsed: ParsedBody = serde_json::from_slice_zeroizing(bytes).map_err(parse_error)?;
        if parsed.change_counter > MAX_CHANGE_COUNTER {
            return Err(NotesError::Corrupt(
                "the change counter is out of range".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for note in &parsed.notes {
            note.validate()?;
            if !ids.insert(note.id.as_str()) {
                let msg = format!("two Notes share the id {}", note.id);
                return Err(NotesError::Corrupt(msg));
            }
        }
        let body = Body {
            changed_at: parsed.changed_at,
            change_counter: parsed.change_counter,
            notes: parsed.notes,
        };
        // An older build refuses schema 2 as a whole. A body that claims the older schema while
        // using a newer field was not written by any build, so it is not trusted.
        if parsed.schema_version < body.schema_version() {
            return Err(NotesError::Corrupt(format!(
                "the document says schema {} but a Note uses a field added in schema {}",
                parsed.schema_version,
                body.schema_version()
            )));
        }
        Ok(Document { body })
    }

    /// The body, written as [`Document::schema_version`].
    pub fn to_json(&self) -> Zeroizing<Vec<u8>> {
        let body = self.body.as_written();
        // Measure first, then write into one exact allocation: a growing Vec would free earlier
        // copies of the plaintext without wiping them.
        let mut counter = ByteCounter(0);
        // Serialising plain structs with string keys cannot fail, and neither can these writers.
        serde_json::to_writer(&mut counter, &body)
            .expect("serialising the Vault document cannot fail");
        let mut out = Zeroizing::new(Vec::with_capacity(counter.0));
        serde_json::to_writer(&mut *out, &body)
            .expect("serialising the Vault document cannot fail");
        debug_assert_eq!(out.len(), counter.0);
        out
    }

    /// The schema version [`Document::to_json`] writes right now: 2 while at least one Note
    /// (Trash included) has a value in a field schema 2 added, otherwise 1. It is worked out
    /// from the Notes each time, never remembered, so it follows every change: clearing the
    /// last such field, or removing the last Note that had one, brings it back to 1.
    pub fn schema_version(&self) -> u32 {
        self.body.schema_version()
    }

    pub fn changed_at(&self) -> Timestamp {
        self.body.changed_at
    }

    pub fn change_counter(&self) -> u64 {
        self.body.change_counter
    }

    /// True if `other` (a copy of the same Vault) may hold something `self` lacks: it has more
    /// saved changes, or its last change is later.
    ///
    /// Neither signal is enough alone. The counter counts changes, not time, so two copies that
    /// were both edited since they parted can't be ordered by it (ADR-0001: "the copy opened
    /// last wins"). The clock can be wrong, or corrected backwards. So either one saying "older"
    /// is reported; the cost is a needless warning now and then, never a missing one. Callers
    /// comparing different Vaults must use timestamps only: their counters are unrelated.
    pub fn is_older_than(&self, other: &Document) -> bool {
        self.change_counter() < other.change_counter() || self.changed_at() < other.changed_at()
    }

    /// Notes not in Trash.
    pub fn count(&self) -> usize {
        self.body.notes.iter().filter(|n| !n.in_trash()).count()
    }

    /// Wallet Kinds need `cipher` (else `NeedsWalletKey`). Seed Phrase words must number 12 or 24
    /// (`BadWordCount`); `word_count` is set from the words, not trusted from input.
    ///
    /// Visible values are trimmed, and a value that breaks the rule for its field is refused
    /// (`InvalidValue`). A pasted key loses stray whitespace around it, and one that is only
    /// whitespace is left out (see `checked_hidden`).
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
    ///
    /// An edit that leaves the title and every visible value as they were and supplies no
    /// Hidden Field changes nothing: `updated_at`, `changed_at` and the change counter stay.
    /// A supplied Hidden Field value always counts as a change, even one equal to the stored
    /// value (telling would mean opening the stored one).
    ///
    /// A visible value sent back byte for byte as it is stored means "keep", and is set aside
    /// before anything is trimmed. Builds from before visible values were trimmed stored them
    /// as typed, and an Editor saved untouched sends such a value back as it is: trimming it
    /// first would make that save look like an edit. A value that differs from the stored one
    /// in any way is a real edit of that field: it is trimmed, checked and counted.
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
        let mut supplied = edit.visible;
        supplied.retain(|name, value| note.visible.get(name) != Some(&*value));
        let visible = checked_visible(kind, supplied)?;
        let (hidden, word_count) = checked_hidden(kind, edit.hidden)?;
        if kind.is_wallet() && cipher.is_none() && !hidden.is_empty() {
            return Err(NotesError::NeedsWalletKey);
        }
        if hidden.is_empty() && note.already_has(title.as_deref(), &visible) {
            return Ok(());
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

    /// Adds `extra` to the end of a Text Note's body without the caller having to supply the
    /// rest: the new body is the old one, a line break unless the old body is empty or already
    /// ends with one, then `extra` exactly as given. Counts as a change.
    ///
    /// Text only: another Kind has no body (`UnknownField`). An empty `extra` is refused
    /// (`InvalidValue`).
    pub fn append_text(&mut self, id: &str, extra: &str, now: Timestamp) -> Result<(), NotesError> {
        let note = self.find_mut(id)?;
        if note.kind != Kind::Text {
            return Err(NotesError::UnknownField(field::BODY.to_string()));
        }
        if extra.is_empty() {
            return Err(NotesError::InvalidValue(field::BODY.to_string()));
        }
        let old = note.plain.get(field::BODY).map_or("", |body| body.as_str());
        let line_break = if old.is_empty() || old.ends_with(['\n', '\r']) {
            ""
        } else {
            "\n"
        };
        // One buffer of the exact size: a growing one would free shorter copies of the body
        // without wiping them.
        let mut body = Zeroizing::new(String::with_capacity(
            old.len() + line_break.len() + extra.len(),
        ));
        body.push_str(old);
        body.push_str(line_break);
        body.push_str(extra);
        note.plain.insert(field::BODY.to_string(), body);
        note.updated_at = now;
        self.touch(now);
        Ok(())
    }

    /// Trashing a Note already in Trash changes nothing (its `deleted_at` is kept). The 30 days
    /// count from `now`, or from the Vault's last change if the clock is behind that, so a clock
    /// that is wrong today can't back-date the deletion.
    pub fn trash(&mut self, id: &str, now: Timestamp) -> Result<(), NotesError> {
        let deleted_at = now.max(self.body.changed_at);
        let note = self.find_mut(id)?;
        if note.deleted_at.is_none() {
            note.deleted_at = Some(deleted_at);
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
        if self.expired_count(now) == 0 {
            return 0;
        }
        self.remove_where(now, |n| n.expired(now))
    }

    /// How many Notes [`Document::purge_expired`] would remove at `now`. None while the clock is
    /// behind the Vault's last change: that clock is wrong, and nothing is removed on its word.
    pub fn expired_count(&self, now: Timestamp) -> usize {
        if now < self.body.changed_at {
            return 0;
        }
        self.body.notes.iter().filter(|n| n.expired(now)).count()
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

    /// Filtered, then searched. `query` is split on whitespace into terms, and a Note matches
    /// when every term is found, ignoring case, somewhere in its searchable parts: the title,
    /// the visible values, and for a Private Key the chain's name in words (`ethereum` for
    /// `evm`). The terms may sit in different parts and come in any order. Hidden Fields are
    /// never searched. An empty `query` matches everything.
    ///
    /// Order: Favorites first, then most recently updated. Trash is ordered by `deleted_at`,
    /// newest first. Searching never changes the order.
    pub fn list(&self, filter: Filter, query: &str) -> Vec<NoteSummary> {
        let query = query.to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
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
            .filter(|n| n.matches(&terms))
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

    /// Mark a Vault-level change that does not alter an individual Note, such as a credential change.
    pub fn touch(&mut self, now: Timestamp) {
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

/// Checks visible field names and values. Every value is trimmed; one that is empty or only
/// whitespace comes back empty, which removes the field. A value that breaks the rule for its
/// field is `InvalidValue`. `word_count` is derived from the words, so input for it is dropped.
fn checked_visible(
    kind: Kind,
    visible: BTreeMap<String, String>,
) -> Result<Vec<(String, String)>, NotesError> {
    let mut out = Vec::new();
    for (name, value) in visible {
        if !kind.visible_fields().contains(&name.as_str()) {
            return Err(NotesError::UnknownField(name));
        }
        if name == field::WORD_COUNT {
            continue;
        }
        let value = value.trim();
        if !value.is_empty() && !rules::allows(kind, &name, value) {
            return Err(NotesError::InvalidValue(name));
        }
        out.push((name, value.to_string()));
    }
    Ok(out)
}

/// A Hidden Field name and its new plain value.
type HiddenValue = (String, Zeroizing<String>);

/// Checks Hidden Field names; tidies and counts Seed Phrase words (returning the count when
/// words were supplied). Empty values mean "clear", except that Seed Phrase words can't be empty.
///
/// A pasted key ([`is_pasted_key`]) is tidied by [`tidied_key`], and one that is only
/// whitespace is left out of the result, exactly as if it had not been supplied. Every other
/// Hidden Field (a Login password, a Passphrase, a Text body) is kept exactly as received.
fn checked_hidden(
    kind: Kind,
    hidden: BTreeMap<String, Zeroizing<String>>,
) -> Result<(Vec<HiddenValue>, Option<usize>), NotesError> {
    let mut out = Vec::new();
    let mut word_count = None;
    for (name, value) in hidden {
        hidden_field_of(kind, &name)?;
        if kind == Kind::SeedPhrase && name == field::WORDS {
            let words = Zeroizing::new(seed::for_storage(&value));
            let count = words.split_whitespace().count();
            if !seed::ALLOWED_WORD_COUNTS.contains(&count) {
                return Err(NotesError::BadWordCount(count));
            }
            word_count = Some(count);
            out.push((name, words));
        } else if is_pasted_key(kind, &name) {
            if let Some(key) = tidied_key(value) {
                out.push((name, key));
            }
        } else {
            out.push((name, value));
        }
    }
    Ok((out, word_count))
}

/// The Hidden Fields that hold a key someone pastes in and later copies out to a service: the
/// `key` of an API Key or a Private Key, and the `secret` of an API Key.
fn is_pasted_key(kind: Kind, name: &str) -> bool {
    matches!(
        (kind, name),
        (Kind::ApiKey | Kind::PrivateKey, field::KEY) | (Kind::ApiKey, field::SECRET)
    )
}

/// A pasted key as it is stored. A stray space or line break around a one-line key makes the
/// service reject the copy, so a key that is one line loses its leading and trailing
/// whitespace. A key of several lines (a PEM block) is kept exactly as received, final line
/// break included.
///
/// `None` for a value that is only whitespace: it is treated as not supplied, so on an edit
/// the stored key is kept, never cleared or replaced by blanks. An empty value is passed on:
/// that is the explicit "clear".
fn tidied_key(value: Zeroizing<String>) -> Option<Zeroizing<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return value.is_empty().then_some(value);
    }
    if trimmed.len() == value.len() || trimmed.contains(['\n', '\r']) {
        return Some(value);
    }
    // An exact-size wiping copy; the received value is wiped as it drops.
    let mut exact = Zeroizing::new(String::with_capacity(trimmed.len()));
    exact.push_str(trimmed);
    Some(exact)
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
