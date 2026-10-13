//! Turning the Vault document into the text the Emergency Reader prints.
//!
//! Every Note is listed with its number, Trash included and marked: Kind, title, visible
//! fields, then each Hidden Field as [`MASK`] unless revealing. When one Note is chosen by its
//! number, only that Note is printed, and only its Hidden Fields are opened. The whole text is
//! built in one wiping buffer before anything is printed.

use std::fmt::{self, Write as _};

use notes::{Document, Filter, Kind, NoteView, NotesError, WalletCipher, field};
use zeroize::Zeroizing;

/// How a Hidden Field with a value looks without `--reveal`.
const MASK: &str = "••••••";
const EMPTY: &str = "(empty)";
const INDENT: &str = "    ";

/// A Note with its number in the list: 1 for the first Note printed.
pub struct Numbered {
    pub number: usize,
    pub view: NoteView,
}

/// Every Note with its number, in the order printed: not in Trash first (Favorites, then most
/// recently updated), then Trash. The order comes from the document alone, so a number names
/// the same Note every time the same file is read.
pub fn numbered(document: &Document) -> Vec<Numbered> {
    let mut summaries = document.list(Filter::All, "");
    summaries.extend(document.list(Filter::Trash, ""));
    summaries
        .iter()
        .filter_map(|summary| document.get(&summary.id))
        .enumerate()
        .map(|(i, view)| Numbered {
            number: i + 1,
            view,
        })
        .collect()
}

/// What to print.
pub enum Selection<'a> {
    /// Every Note, under a line saying how many there are.
    All(&'a [Numbered]),
    /// The one Note chosen with `--note`, and nothing about the others.
    One(&'a Numbered),
}

impl<'a> Selection<'a> {
    /// `number` is the one given with `--note`; `None` selects every Note. `Err` if no Note has
    /// that number.
    pub fn of(all: &'a [Numbered], number: Option<usize>) -> Result<Selection<'a>, NoSuchNote> {
        let Some(number) = number else {
            return Ok(Selection::All(all));
        };
        all.iter()
            .find(|note| note.number == number)
            .map(Selection::One)
            .ok_or(NoSuchNote {
                number,
                count: all.len(),
            })
    }

    fn notes(&self) -> &'a [Numbered] {
        match *self {
            Selection::All(all) => all,
            Selection::One(one) => std::slice::from_ref(one),
        }
    }

    /// True if revealing the selected Notes would need the Wallet Key: one of them is a Wallet
    /// Kind with a Hidden Field value. Never because of a Note that isn't selected.
    pub fn needs_wallet_key(&self) -> bool {
        self.notes().iter().any(|note| {
            note.view.kind.is_wallet() && note.view.hidden.iter().any(|(_, has_value)| *has_value)
        })
    }
}

/// `--note` was given a number that no Note has.
pub struct NoSuchNote {
    number: usize,
    count: usize,
}

impl NoSuchNote {
    /// For stderr. Says which numbers exist, and nothing about any Note.
    pub fn message(&self) -> String {
        let number = self.number;
        match self.count {
            0 => format!("There is no Note {number}: this Vault has no Notes."),
            1 => format!(
                "There is no Note {number}: this Vault has one Note, number 1. Run again \
                 without --note to list it."
            ),
            count => format!(
                "There is no Note {number}: this Vault's Notes are numbered 1 to {count}. \
                 Run again without --note to list them."
            ),
        }
    }
}

/// The text, and how many Hidden Fields couldn't be decrypted. `wallet` opens Wallet Kind fields;
/// it is only consulted when `reveal` is set, and only for the selected Notes.
pub fn render(
    document: &Document,
    selection: &Selection<'_>,
    reveal: bool,
    wallet: Option<&impl WalletCipher>,
) -> (Zeroizing<String>, usize) {
    let (mut masked, mut undecryptable) = (0, 0);
    let mut out = Buffer(Zeroizing::new(String::with_capacity(4096)));

    if let Selection::All(all) = selection {
        out.count_line(all);
    }
    for (i, Numbered { number, view }) in selection.notes().iter().enumerate() {
        // A blank line between Notes, and between the count line and the first one.
        if i > 0 || matches!(selection, Selection::All(_)) {
            out.push_str("\n");
        }
        out.write(format_args!("{number}. "));
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
        out.write(format_args!("\nHidden Fields are shown as {MASK}. "));
        match selection {
            Selection::All(_) => out.push_str(
                "To show one Note's, run again with --note <its number> --reveal. To show \
                 every Note's at once, use --reveal alone.\n",
            ),
            Selection::One(note) => out.write(format_args!(
                "To show them, run again with --note {} --reveal.\n",
                note.number
            )),
        }
    }
    (out.0, undecryptable)
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
        field::DERIVATION_PATH => "Derivation path",
        field::PURPOSE => "Purpose",
        field::KEY_ID => "Key ID",
        field::URL => "Managed at",
        field::EXPIRES => "Expires",
        field::ENV_VAR => "Environment variable",
        field::SECRET => "Secret",
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

    /// Text from the Vault, with two sorts of character shown as U+FFFD: control characters
    /// (which could drive the terminal) and Unicode format characters (which are invisible or
    /// change the order text is displayed in, see [`is_format`]). Tabs are kept.
    fn push_clean(&mut self, s: &str) {
        for c in s.chars() {
            let c = if (c.is_control() && c != '\t') || is_format(c) {
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

    /// The first line of the full list: how many Notes there are, and how many are in Trash.
    fn count_line(&mut self, all: &[Numbered]) {
        let in_trash = all.iter().filter(|n| n.view.deleted_at.is_some()).count();
        let active = all.len() - in_trash;
        if all.is_empty() {
            self.push_str("This Vault has no Notes.\n");
            return;
        }
        let s = if active == 1 { "" } else { "s" };
        self.write(format_args!("{active} Note{s}"));
        if in_trash > 0 {
            self.write(format_args!(", plus {in_trash} in Trash"));
        }
        self.push_str(".\n");
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

/// Characters that take no space or change the order the surrounding text is displayed in, so
/// a printed key, address or password could read differently from what is stored.
///
/// This is every code point with Unicode General_Category Cf (Format) as of Unicode 16.0,
/// plus the line and paragraph separators and the unassigned code points inside the two
/// invisible blocks listed. A later Unicode version that adds format characters needs them
/// added here.
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'                    // soft hyphen
        | '\u{0600}'..='\u{0605}'     // Arabic number signs
        | '\u{061C}'                  // Arabic letter mark (direction)
        | '\u{06DD}'                  // Arabic end of ayah
        | '\u{070F}'                  // Syriac abbreviation mark
        | '\u{0890}'..='\u{0891}'     // Arabic pound and piastre marks above
        | '\u{08E2}'                  // Arabic disputed end of ayah
        | '\u{180E}'                  // Mongolian vowel separator
        | '\u{200B}'..='\u{200F}'     // zero-width space, non-joiner, joiner; direction marks
        | '\u{2028}'..='\u{2029}'     // line and paragraph separators
        | '\u{202A}'..='\u{202E}'     // direction embeddings and overrides
        | '\u{2060}'..='\u{206F}'     // word joiner, invisible operators, direction isolates
        | '\u{FEFF}'                  // zero-width no-break space (byte order mark)
        | '\u{FFF9}'..='\u{FFFB}'     // interlinear annotation marks
        | '\u{110BD}' | '\u{110CD}'   // Kaithi number signs
        | '\u{13430}'..='\u{1343F}'   // Egyptian hieroglyph format controls
        | '\u{1BCA0}'..='\u{1BCA3}'   // shorthand format controls
        | '\u{1D173}'..='\u{1D17A}'   // musical symbol format controls
        | '\u{E0000}'..='\u{E007F}'   // tag characters (invisible)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> String {
        let mut out = Buffer(Zeroizing::new(String::new()));
        out.push_clean(s);
        out.0.to_string()
    }

    #[test]
    fn control_and_format_characters_are_replaced_and_tabs_kept() {
        // Escape, a C1 control, a right-to-left override, a zero-width space and a direction
        // isolate, around fake text.
        let dirty = "fake\u{1b}[2J-a\u{9b}b\u{202E}dcba\u{200B}-\u{2066}x\u{2069}\tend";
        assert_eq!(
            clean(dirty),
            "fake\u{FFFD}[2J-a\u{FFFD}b\u{FFFD}dcba\u{FFFD}-\u{FFFD}x\u{FFFD}\tend"
        );
    }

    #[test]
    fn every_listed_format_range_is_replaced_at_both_ends() {
        let ranges: [(u32, u32); 19] = [
            (0x00AD, 0x00AD),
            (0x0600, 0x0605),
            (0x061C, 0x061C),
            (0x06DD, 0x06DD),
            (0x070F, 0x070F),
            (0x0890, 0x0891),
            (0x08E2, 0x08E2),
            (0x180E, 0x180E),
            (0x200B, 0x200F),
            (0x2028, 0x202E),
            (0x2060, 0x206F),
            (0xFEFF, 0xFEFF),
            (0xFFF9, 0xFFFB),
            (0x110BD, 0x110BD),
            (0x110CD, 0x110CD),
            (0x13430, 0x1343F),
            (0x1BCA0, 0x1BCA3),
            (0x1D173, 0x1D17A),
            (0xE0000, 0xE007F),
        ];
        for (first, last) in ranges {
            for code in [first, last] {
                let c = char::from_u32(code).unwrap();
                assert_eq!(clean(&c.to_string()), "\u{FFFD}", "U+{code:04X}");
            }
            // The code points just outside each range are not format characters and are kept.
            for code in [first - 1, last + 1] {
                let c = char::from_u32(code).unwrap();
                assert_eq!(clean(&c.to_string()), c.to_string(), "U+{code:04X}");
            }
        }
    }

    #[test]
    fn ordinary_text_in_any_script_is_kept() {
        for text in [
            "fake-password-123 !@#$%^&*()",
            "caf\u{e9} na\u{ef}ve \u{2014} \u{201c}quoted\u{201d}",
            "\u{65e5}\u{672c}\u{8a9e} \u{d55c}\u{ae00} \u{0639}\u{0631}\u{0628}\u{064a}",
            "no-break\u{a0}space and narrow\u{202f}space",
            "\u{1f511} key",
        ] {
            assert_eq!(clean(text), text);
        }
    }

    // --- Numbering, and printing one Note. Every value below is an obvious fake. ---

    const NOW: i64 = 1_790_000_000;
    /// The public BIP39 test vector: not anyone's wallet.
    const SEED_WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const LOGIN_PASSWORD: &str = "fake-login-password-123";
    const API_KEY: &str = "fake-api-key-0000";
    const API_SECRET: &str = "fake-api-secret-0000";
    const PASSPHRASE: &str = "fake-test-passphrase";
    const PRIVATE_KEY: &str = "fake-private-key-1";
    const TRASHED_BODY: &str = "fake-trashed-body";

    /// A reversible stand-in for the Wallet Key that binds each value to its Note and field,
    /// and counts how many values it was asked to open.
    struct FakeCipher {
        opened: std::cell::Cell<usize>,
    }

    impl FakeCipher {
        fn new() -> FakeCipher {
            FakeCipher {
                opened: std::cell::Cell::new(0),
            }
        }
    }

    impl WalletCipher for FakeCipher {
        fn seal(&self, id: &str, field: &str, plaintext: &[u8]) -> Result<Vec<u8>, NotesError> {
            let mut out = format!("{id}:{field}|").into_bytes();
            out.extend(plaintext.iter().map(|b| b ^ 0x5a));
            Ok(out)
        }

        fn open(
            &self,
            id: &str,
            field: &str,
            sealed: &[u8],
        ) -> Result<Zeroizing<Vec<u8>>, NotesError> {
            self.opened.set(self.opened.get() + 1);
            let tag = format!("{id}:{field}|");
            let rest = sealed
                .strip_prefix(tag.as_bytes())
                .ok_or(NotesError::WalletCipher)?;
            Ok(Zeroizing::new(rest.iter().map(|b| b ^ 0x5a).collect()))
        }
    }

    fn new_note(
        kind: Kind,
        title: &str,
        visible: &[(&str, &str)],
        hidden: &[(&str, &str)],
    ) -> notes::NewNote {
        notes::NewNote {
            kind,
            title: title.into(),
            visible: visible
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            hidden: hidden
                .iter()
                .map(|(k, v)| (k.to_string(), Zeroizing::new(v.to_string())))
                .collect(),
        }
    }

    /// Five Notes, created oldest first, so they are listed newest first: 1 a Login, 2 an API
    /// Key, 3 a Seed Phrase, 4 a Private Key, and 5 a Text Note in Trash.
    fn fake_vault(cipher: &FakeCipher) -> Document {
        let mut doc = Document::new(NOW);
        let notes = [
            new_note(
                Kind::PrivateKey,
                "Fake key",
                &[(field::CHAIN, "evm"), (field::ADDRESS, "0xfake-address")],
                &[(field::KEY, PRIVATE_KEY)],
            ),
            new_note(
                Kind::SeedPhrase,
                "Fake wallet",
                &[
                    (field::WALLET_NAME, "Fake Wallet One"),
                    (field::ADDRESS, "0xFAKESEEDADDR"),
                    (field::DERIVATION_PATH, "m/44'/60'/0'/0/0"),
                ],
                &[(field::WORDS, SEED_WORDS), (field::PASSPHRASE, PASSPHRASE)],
            ),
            new_note(
                Kind::ApiKey,
                "Fake API key",
                &[
                    (field::SERVICE, "Example Test Service"),
                    (field::KEY_ID, "FAKEKEYID0001"),
                    (field::PURPOSE, "Nightly fake uploads"),
                    (field::URL, "https://console.example.test/keys"),
                    (field::EXPIRES, "2027-01-31"),
                    (field::ENV_VAR, "FAKE_DEPLOY_KEY"),
                ],
                &[(field::KEY, API_KEY), (field::SECRET, API_SECRET)],
            ),
            new_note(
                Kind::Login,
                "Fake login",
                &[
                    (field::WEBSITE, "https://login.example.test"),
                    (field::USERNAME, "fake-user"),
                    (field::PURPOSE, "Shared with the fake team"),
                ],
                &[(field::PASSWORD, LOGIN_PASSWORD)],
            ),
        ];
        for (i, note) in notes.into_iter().enumerate() {
            doc.create(note, Some(cipher), NOW + i as i64).unwrap();
        }
        let trashed = doc
            .create(
                new_note(
                    Kind::Text,
                    "Fake old text",
                    &[],
                    &[(field::BODY, TRASHED_BODY)],
                ),
                None,
                NOW + 10,
            )
            .unwrap();
        doc.trash(&trashed, NOW + 11).unwrap();
        doc
    }

    const TITLES: [&str; 5] = [
        "Fake login",
        "Fake API key",
        "Fake wallet",
        "Fake key",
        "Fake old text",
    ];
    const EVERY_HIDDEN_VALUE: [&str; 7] = [
        LOGIN_PASSWORD,
        API_KEY,
        API_SECRET,
        "abandon",
        PASSPHRASE,
        PRIVATE_KEY,
        TRASHED_BODY,
    ];

    fn one(all: &[Numbered], number: usize) -> Selection<'_> {
        match Selection::of(all, Some(number)) {
            Ok(selection) => selection,
            Err(_) => panic!("there should be a Note {number}"),
        }
    }

    fn no_cipher() -> Option<&'static FakeCipher> {
        None
    }

    #[test]
    fn notes_are_numbered_from_1_in_the_order_printed_with_trash_last() {
        let cipher = FakeCipher::new();
        let doc = fake_vault(&cipher);
        let all = numbered(&doc);
        let listed: Vec<(usize, &str)> = all
            .iter()
            .map(|n| (n.number, n.view.title.as_str()))
            .collect();
        let expected: Vec<(usize, &str)> = (1..).zip(TITLES).collect();
        assert_eq!(listed, expected);

        // The same file gives every Note the same number each time it is read.
        let reread = Document::from_json(&doc.to_json()).unwrap();
        let again: Vec<(usize, String)> = numbered(&reread)
            .into_iter()
            .map(|n| (n.number, n.view.id))
            .collect();
        let first: Vec<(usize, String)> =
            all.iter().map(|n| (n.number, n.view.id.clone())).collect();
        assert_eq!(again, first);

        let (text, undecryptable) = render(&doc, &Selection::All(&all), false, no_cipher());
        assert_eq!(undecryptable, 0);
        assert!(text.starts_with("4 Notes, plus 1 in Trash.\n\n1. Login: Fake login\n"));
        assert!(text.contains("\n2. API Key: Fake API key\n"));
        assert!(text.contains("\n3. Seed Phrase: Fake wallet\n"));
        assert!(text.contains("\n4. Private Key: Fake key\n"));
        assert!(text.contains("\n5. [In Trash] Text: Fake old text\n"));
        // It says how to show one Note's Hidden Fields, not only how to show them all.
        assert!(text.contains("--note <its number> --reveal"));
        for value in EVERY_HIDDEN_VALUE {
            assert!(!text.contains(value));
        }
    }

    #[test]
    fn a_number_no_note_has_selects_nothing_and_the_message_says_which_numbers_exist() {
        let cipher = FakeCipher::new();
        let doc = fake_vault(&cipher);
        let all = numbered(&doc);
        assert!(matches!(Selection::of(&all, None), Ok(Selection::All(_))));
        for number in 1..=5 {
            assert!(matches!(
                Selection::of(&all, Some(number)),
                Ok(Selection::One(note)) if note.number == number
            ));
        }
        for number in [0, 6, 60, usize::MAX] {
            let Err(none) = Selection::of(&all, Some(number)) else {
                panic!("there is no Note {number}");
            };
            let message = none.message();
            assert!(message.contains(&format!("no Note {number}")), "{message}");
            assert!(message.contains("1 to 5"), "{message}");
            // Nothing from the Vault is in it.
            for text in TITLES.iter().chain(&EVERY_HIDDEN_VALUE) {
                assert!(!message.contains(text), "{message}");
            }
        }

        let empty = numbered(&Document::new(NOW));
        let Err(none) = Selection::of(&empty, Some(1)) else {
            panic!("an empty Vault has no Note 1");
        };
        assert!(none.message().contains("has no Notes"));
    }

    #[test]
    fn the_wallet_key_is_needed_only_for_a_selected_wallet_kind_with_a_hidden_value() {
        let cipher = FakeCipher::new();
        let mut doc = fake_vault(&cipher);
        let all = numbered(&doc);
        assert!(Selection::All(&all).needs_wallet_key());
        for (number, needed) in [(1, false), (2, false), (3, true), (4, true), (5, false)] {
            assert_eq!(
                one(&all, number).needs_wallet_key(),
                needed,
                "Note {number}"
            );
        }

        // A Wallet Kind with nothing to open doesn't need it either.
        let key_note = all[3].view.id.clone();
        let clear = notes::NoteEdit {
            hidden: [(field::KEY.to_string(), Zeroizing::new(String::new()))].into(),
            ..Default::default()
        };
        doc.update(&key_note, clear, Some(&cipher), NOW + 20)
            .unwrap();
        let all = numbered(&doc);
        let cleared = all.iter().find(|n| n.view.id == key_note).unwrap();
        assert!(!Selection::One(cleared).needs_wallet_key());
    }

    #[test]
    fn one_selected_note_is_printed_alone_and_masked_without_reveal() {
        let cipher = FakeCipher::new();
        let doc = fake_vault(&cipher);
        let all = numbered(&doc);
        for (number, title) in (1..).zip(TITLES) {
            let (text, undecryptable) = render(&doc, &one(&all, number), false, no_cipher());
            assert_eq!(undecryptable, 0);
            // Its own heading comes first: no count line, no blank line, no other Note.
            assert!(text.starts_with(&format!("{number}. ")), "{text:?}");
            assert!(text.lines().next().unwrap().ends_with(title), "{text:?}");
            for other in TITLES.iter().filter(|t| **t != title) {
                assert!(!text.contains(other), "Note {number} printed {other}");
            }
            assert!(!text.contains("Notes"), "{text:?}");
            assert!(text.contains(MASK));
            for value in EVERY_HIDDEN_VALUE {
                assert!(!text.contains(value));
            }
            // It says how to show this Note's Hidden Fields.
            assert!(text.contains(&format!("--note {number} --reveal")));
        }
        assert_eq!(cipher.opened.get(), 0);
    }

    #[test]
    fn revealing_one_note_opens_only_its_hidden_fields() {
        let cipher = FakeCipher::new();
        let doc = fake_vault(&cipher);
        let all = numbered(&doc);
        // (number, what it must print, how many Wallet Kind fields that takes)
        let cases: [(usize, &[&str], usize); 5] = [
            (1, &[LOGIN_PASSWORD], 0),
            (2, &[API_KEY, API_SECRET], 0),
            (3, &["abandon", "about", PASSPHRASE], 2),
            (4, &[PRIVATE_KEY], 1),
            (5, &[TRASHED_BODY], 0),
        ];
        for (number, shown, wallet_fields) in cases {
            cipher.opened.set(0);
            let selection = one(&all, number);
            let wallet = selection.needs_wallet_key().then_some(&cipher);
            let (text, undecryptable) = render(&doc, &selection, true, wallet);
            assert_eq!(undecryptable, 0, "Note {number}");
            for value in shown {
                assert!(text.contains(value), "Note {number} should show {value}");
            }
            for value in EVERY_HIDDEN_VALUE.iter().filter(|v| !shown.contains(v)) {
                assert!(!text.contains(value), "Note {number} showed {value}");
            }
            assert!(!text.contains(MASK));
            assert_eq!(cipher.opened.get(), wallet_fields, "Note {number}");
        }
    }

    #[test]
    fn the_fields_added_in_schema_2_are_printed_under_their_names() {
        let cipher = FakeCipher::new();
        let doc = fake_vault(&cipher);
        assert_eq!(doc.schema_version(), 2);
        let all = numbered(&doc);
        let (text, _) = render(&doc, &Selection::All(&all), true, Some(&cipher));
        for line in [
            "    Key ID: FAKEKEYID0001\n",
            "    Purpose: Nightly fake uploads\n",
            "    Managed at: https://console.example.test/keys\n",
            "    Expires: 2027-01-31\n",
            "    Environment variable: FAKE_DEPLOY_KEY\n",
            "    Secret: fake-api-secret-0000\n",
            "    Purpose: Shared with the fake team\n",
            "    Address: 0xFAKESEEDADDR\n",
            "    Derivation path: m/44'/60'/0'/0/0\n",
        ] {
            assert!(text.contains(line), "missing {line:?} in:\n{}", *text);
        }
        let (masked, _) = render(&doc, &one(&all, 2), false, no_cipher());
        assert!(masked.contains(&format!("    Key: {MASK}\n    Secret: {MASK}\n")));
    }

    #[test]
    fn every_field_of_every_kind_has_a_label_of_its_own() {
        for kind in Kind::ALL {
            for name in kind.visible_fields().iter().chain(kind.hidden_fields()) {
                let shown = label(name);
                assert_ne!(shown, *name, "{kind:?} {name} has no label");
                assert!(shown.starts_with(char::is_uppercase), "{shown}");
            }
        }
    }
}
