//! Turning the Vault document into the text the Emergency Reader prints.
//!
//! Every Note is printed, Trash included and marked: Kind, title, visible fields, then each
//! Hidden Field as [`MASK`] unless revealing. The whole text is built in one wiping buffer before
//! anything is printed.

use std::fmt::{self, Write as _};

use notes::{Document, Filter, Kind, NoteSummary, NotesError, WalletCipher, field};
use zeroize::Zeroizing;

/// How a Hidden Field with a value looks without `--reveal`.
const MASK: &str = "••••••";
const EMPTY: &str = "(empty)";
const INDENT: &str = "    ";

/// True if revealing would need the Wallet Key: some Wallet Kind Note has a Hidden Field value.
pub fn needs_wallet_key(document: &Document) -> bool {
    every_note(document).iter().any(|summary| {
        summary.kind.is_wallet()
            && document
                .get(&summary.id)
                .is_some_and(|view| view.hidden.iter().any(|(_, has_value)| *has_value))
    })
}

/// The text, and how many Hidden Fields couldn't be decrypted. `wallet` opens Wallet Kind fields;
/// it is only consulted when `reveal` is set.
pub fn render(
    document: &Document,
    reveal: bool,
    wallet: Option<&impl WalletCipher>,
) -> (Zeroizing<String>, usize) {
    let (mut masked, mut undecryptable) = (0, 0);
    let mut out = Buffer(Zeroizing::new(String::with_capacity(4096)));

    let notes = every_note(document);
    let in_trash = notes.iter().filter(|n| n.deleted_at.is_some()).count();
    let active = notes.len() - in_trash;
    if notes.is_empty() {
        out.push_str("This Vault has no Notes.\n");
    } else {
        let s = if active == 1 { "" } else { "s" };
        out.write(format_args!("{active} Note{s}"));
        if in_trash > 0 {
            out.write(format_args!(", plus {in_trash} in Trash"));
        }
        out.push_str(".\n");
    }

    for view in notes.iter().filter_map(|summary| document.get(&summary.id)) {
        out.push_str("\n");
        if view.deleted_at.is_some() {
            out.push_str("[In Trash] ");
        }
        out.push_str(kind_name(view.kind));
        out.push_str(": ");
        out.push_clean(&view.title);
        if view.favorite {
            out.push_str(" (Favorite)");
        }
        out.push_str("\n");

        for name in view.kind.visible_fields() {
            if let Some(value) = view.visible.get(*name) {
                out.field(name, value);
            }
        }

        for (name, has_value) in &view.hidden {
            if *has_value && !reveal {
                masked += 1;
                out.field(name, MASK);
                continue;
            }
            let revealed = if !has_value {
                Err(NotesError::EmptyField)
            } else if view.kind.is_wallet() {
                wallet
                    .ok_or(NotesError::NeedsWalletKey)
                    .and_then(|cipher| document.reveal_wallet(&view.id, name, cipher))
            } else {
                document.reveal(&view.id, name)
            };
            match revealed {
                Ok(words) if view.kind == Kind::SeedPhrase && name == field::WORDS => {
                    out.seed_words(name, &words);
                }
                Ok(value) => out.field(name, &value),
                Err(NotesError::EmptyField) => out.field(name, EMPTY),
                Err(_) => {
                    undecryptable += 1;
                    out.field(name, "(damaged: couldn't be decrypted)");
                }
            }
        }
    }

    if masked > 0 {
        out.write(format_args!(
            "\nHidden Fields are shown as {MASK}. To show them, run again with --reveal.\n"
        ));
    }
    (out.0, undecryptable)
}

/// Every Note: not in Trash first (Favorites, then most recently updated), then Trash.
fn every_note(document: &Document) -> Vec<NoteSummary> {
    let mut notes = document.list(Filter::All, "");
    notes.extend(document.list(Filter::Trash, ""));
    notes
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::SeedPhrase => "Seed Phrase",
        Kind::PrivateKey => "Private Key",
        Kind::Login => "Login",
        Kind::ApiKey => "API Key",
        Kind::Text => "Text",
    }
}

fn label(name: &str) -> &str {
    match name {
        field::WALLET_NAME => "Wallet name",
        field::WORD_COUNT => "Word count",
        field::WORDS => "Words",
        field::PASSPHRASE => "Passphrase",
        field::CHAIN => "Chain",
        field::ADDRESS => "Address",
        field::KEY => "Key",
        field::WEBSITE => "Website",
        field::USERNAME => "Username",
        field::PASSWORD => "Password",
        field::SERVICE => "Service",
        field::BODY => "Body",
        other => other,
    }
}

/// A growing text buffer that wipes every copy of its contents: when it needs more room it
/// moves into a bigger wiping buffer and the old one is wiped as it drops.
struct Buffer(Zeroizing<String>);

impl Buffer {
    fn reserve(&mut self, extra: usize) {
        let needed = self.0.len().saturating_add(extra);
        if needed > self.0.capacity() {
            let capacity = needed.max(self.0.capacity().saturating_mul(2));
            let mut bigger = Zeroizing::new(String::with_capacity(capacity));
            bigger.push_str(&self.0);
            self.0 = bigger;
        }
    }

    fn push_str(&mut self, s: &str) {
        self.reserve(s.len());
        self.0.push_str(s);
    }

    /// Text from the Vault, with control characters (which could drive the terminal) shown as
    /// U+FFFD. Tabs are kept.
    fn push_clean(&mut self, s: &str) {
        for c in s.chars() {
            let c = if c.is_control() && c != '\t' {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            };
            self.reserve(c.len_utf8());
            self.0.push(c);
        }
    }

    fn write(&mut self, args: fmt::Arguments<'_>) {
        // Writing into this buffer cannot fail.
        let _ = self.write_fmt(args);
    }

    /// `    Label: value`, or for several lines `    Label:` followed by the lines indented.
    fn field(&mut self, name: &str, value: &str) {
        self.push_str(INDENT);
        self.push_str(label(name));
        self.push_str(":");
        if value.contains('\n') {
            for line in value.lines() {
                self.push_str("\n");
                self.push_str(INDENT);
                self.push_str(INDENT);
                self.push_clean(line);
            }
        } else {
            self.push_str(" ");
            self.push_clean(value);
        }
        self.push_str("\n");
    }

    /// The words as a numbered grid, like the app shows them.
    fn seed_words(&mut self, name: &str, words: &str) {
        const PER_ROW: usize = 4;
        const COLUMN: usize = 10;
        self.push_str(INDENT);
        self.push_str(label(name));
        self.push_str(":");
        let mut previous_len = 0;
        for (i, word) in words.split_whitespace().enumerate() {
            if i % PER_ROW == 0 {
                self.push_str("\n");
                self.push_str(INDENT);
                self.push_str(INDENT);
            } else {
                for _ in previous_len..COLUMN {
                    self.push_str(" ");
                }
            }
            self.write(format_args!("{:>2}. ", i + 1));
            self.push_clean(word);
            previous_len = word.chars().count();
        }
        self.push_str("\n");
    }
}

impl fmt::Write for Buffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}
