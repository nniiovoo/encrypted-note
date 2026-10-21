//! Emergency Reader (CONTEXT.md): prints a Vault or Backup's Notes to the terminal and writes
//! nothing to disk. Flags and exit codes are in [`USAGE`] and docs/FORMAT.md section 13.

mod credential;
mod report;

use std::ffi::{OsStr, OsString};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use notes::{Document, NotesError, WalletCipher};
use vault_format::{Error, Limits, WalletKey};
use zeroize::Zeroizing;

use credential::Secret;
use report::Selection;

const USAGE: &str = "\
Usage: enote-reader <vault-or-backup-file> [--recovery-key] [--note <number>] [--reveal]
                    [--allow-non-terminal] [--credential-stdin]

Emergency Reader: prints the Notes in a Vault or Backup, for when the encrypted-note app
no longer works. It asks for the Master Password without showing what you type, and it
writes nothing to disk.

On its own it lists every Note with a number, and shows Hidden Fields as dots. To read one
Note's Hidden Fields, find its number in that list, then run again with --note <number>
--reveal: nothing of the other Notes is printed.

Options:
  --recovery-key        Ask for the Recovery Key (from your Recovery Kit) instead.
  --note <number>       Print only the Note with this number in the list (--note=<number>
                        works too). A Note keeps its number for as long as the file stays
                        the same.
  --reveal              Also print Hidden Fields: passwords, keys, Seed Phrase words and
                        Passphrases, and the text of Text Notes. With --note, only that
                        Note's; without it, every Note's. Only prints to a terminal: if the
                        output is redirected to a file or piped into another program, it
                        stops without printing anything.
  --allow-non-terminal  With --reveal, print even when the output is redirected or piped.
                        Whatever receives it then holds your secrets as plain text.
  --credential-stdin    Read the Master Password or Recovery Key from the first line of
                        a pipe or file on stdin instead of asking (for scripts).
  -h, --help            Show this help.

Exit codes: 0 ok, 1 wrong credential, 2 damaged or unsupported file, 3 usage or I/O error,
            4 no Note has the number given with --note.
";

const DAMAGED: &str = "This Vault file is damaged. Restore a Safety Copy or Backup.";

const REVEAL_NEEDS_TERMINAL: &str = "\
Nothing was printed. --reveal prints passwords, keys and Seed Phrases as plain text, and the \
output of this command is not going to a terminal: it is redirected to a file or piped into \
another program, which would then hold them.
Run it again in a terminal without redirecting, and read the Notes on screen. If you do mean \
to send revealed Notes somewhere else, add --allow-non-terminal.";

/// Why the reader stopped (exit code 1, 2, 3 or 4), with a message for stderr that never
/// includes credentials or Note contents.
enum Failure {
    WrongCredential(String),
    Damaged(String),
    UsageOrIo(String),
    /// The file opened, but no Note has the number given with `--note`.
    NoSuchNote(String),
}

struct Options {
    file: PathBuf,
    recovery_key: bool,
    /// The number given with `--note`: print that Note only.
    note: Option<usize>,
    reveal: bool,
    allow_non_terminal: bool,
    credential_stdin: bool,
}

fn main() -> ExitCode {
    let (code, message) = match run() {
        Ok(()) => return ExitCode::SUCCESS,
        Err(Failure::WrongCredential(message)) => (1, message),
        Err(Failure::Damaged(message)) => (2, message),
        Err(Failure::UsageOrIo(message)) => (3, message),
        Err(Failure::NoSuchNote(message)) => (4, message),
    };
    // Nothing more can be done if stderr itself is gone.
    let _ = writeln!(io::stderr(), "{message}");
    ExitCode::from(code)
}

fn run() -> Result<(), Failure> {
    let Some(options) = parse(std::env::args_os().skip(1))? else {
        return print(USAGE);
    };
    // Checked first: before the file is read and before the owner types a credential.
    if reveal_refused(&options, io::stdout().is_terminal()) {
        return Err(Failure::UsageOrIo(REVEAL_NEEDS_TERMINAL.to_string()));
    }
    let _ = writeln!(
        io::stderr(),
        "Warning: this prints your Notes as plain text. Read them on screen only; don't \
         redirect or pipe the output into a file."
    );

    let bytes = std::fs::read(&options.file).map_err(|e| {
        Failure::UsageOrIo(format!("Couldn't read {}: {e}.", options.file.display()))
    })?;
    // A damaged or unsupported file is reported before the owner types anything.
    vault_format::inspect(&bytes, &Limits::PRODUCTION).map_err(|e| failure(e, None))?;

    let secret = credential::read(&options)?;
    let (unlocked, body) = vault_format::open(&bytes, secret.credential(), &Limits::PRODUCTION)
        .map_err(|e| failure(e, Some(&secret)))?;
    let document = Document::from_json(&body).map_err(|e| {
        Failure::Damaged(match e {
            NotesError::Corrupt(why) => format!("{DAMAGED} ({why})"),
            // Not damage: a newer app wrote it.
            NotesError::UnsupportedSchema(version) => format!(
                "This Vault's Notes are in document schema {version}, which this Emergency \
                 Reader doesn't know. Use the Emergency Reader from a newer encrypted-note."
            ),
            _ => DAMAGED.to_string(),
        })
    })?;
    drop(body);

    let all = report::numbered(&document);
    let selection =
        Selection::of(&all, options.note).map_err(|none| Failure::NoSuchNote(none.message()))?;

    let wallet = wallet_key_if_needed(&options, &selection, || {
        // A fresh derivation from the same credential that opened the Vault (ADR-0004).
        let key =
            vault_format::unlock_wallet_key(&unlocked, secret.credential(), &Limits::PRODUCTION)
                .map_err(|e| failure(e, Some(&secret)))?;
        Ok(WalletFields {
            key,
            vault_id: unlocked.vault_id(),
        })
    })?;
    drop(secret);

    let (text, undecryptable) =
        report::render(&document, &selection, options.reveal, wallet.as_ref());
    print(&text)?;
    match undecryptable {
        0 => Ok(()),
        n => Err(Failure::Damaged(format!(
            "{n} Hidden {} couldn't be decrypted, so part of this Vault file is damaged. \
             Restore a Safety Copy or Backup to read {}.",
            if n == 1 { "Field" } else { "Fields" },
            if n == 1 { "it" } else { "them" },
        ))),
    }
}

/// `None` for `--help`. Flags may come before or after the file; `--` ends the flags (for a
/// file named like one). `--note` takes its number as the next argument, or as `--note=3`.
fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Option<Options>, Failure> {
    let (mut file, mut note) = (None, None);
    let (mut recovery_key, mut reveal, mut allow_non_terminal, mut credential_stdin) =
        (false, false, false, false);
    let (mut help, mut flags_ended) = (false, false);
    while let Some(arg) = args.next() {
        if !flags_ended && arg.len() > 1 && arg.as_encoded_bytes().starts_with(b"-") {
            match arg.to_str() {
                Some("--") => flags_ended = true,
                Some("--recovery-key") => recovery_key = true,
                Some("--reveal") => reveal = true,
                Some("--allow-non-terminal") => allow_non_terminal = true,
                Some("--credential-stdin") => credential_stdin = true,
                Some("-h" | "--help") => help = true,
                Some("--note") => {
                    let next = args.next();
                    set_note(&mut note, next.as_deref().and_then(OsStr::to_str))?;
                }
                Some(joined) if joined.starts_with("--note=") => {
                    set_note(&mut note, joined.strip_prefix("--note="))?;
                }
                _ => return Err(usage(&format!("Unknown option {}.", arg.to_string_lossy()))),
            }
        } else if file.replace(PathBuf::from(arg)).is_some() {
            return Err(usage("Give one Vault or Backup file at a time."));
        }
    }
    if help {
        return Ok(None);
    }
    let file = file.ok_or_else(|| usage("Which file? Give the path of a Vault or Backup."))?;
    Ok(Some(Options {
        file,
        recovery_key,
        note,
        reveal,
        allow_non_terminal,
        credential_stdin,
    }))
}

/// Records the number that follows `--note`: a whole number from 1 up, as the list shows it.
/// Anything else, or a second `--note`, is a usage error. Whether a Note has that number is
/// only known once the file is open.
fn set_note(note: &mut Option<usize>, given: Option<&str>) -> Result<(), Failure> {
    let number = given
        .and_then(|number| number.parse::<usize>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            usage("--note needs the number of a Note as the list shows it, for example --note 3.")
        })?;
    if note.replace(number).is_some() {
        return Err(usage("Give --note once: it prints one Note."));
    }
    Ok(())
}

/// `--reveal` puts Hidden Fields in the output (one Note's with `--note`, otherwise every
/// Note's), so it is refused when the output is not a terminal (a file or another program
/// would keep a plain-text copy), unless the owner asked for exactly that with
/// `--allow-non-terminal`.
fn reveal_refused(options: &Options, stdout_is_terminal: bool) -> bool {
    options.reveal && !options.allow_non_terminal && !stdout_is_terminal
}

/// Runs `derive`, which derives the Wallet Key, only when this run is about to reveal a Wallet
/// Kind: `--reveal` was given and a selected Note is a Seed Phrase or Private Key with a Hidden
/// Field value. In every other run `derive` is not called, so the Wallet Key never comes into
/// memory: not to list Notes without `--reveal`, and not to reveal a Login, an API Key or a
/// Text, whatever Wallet Kinds the rest of the Vault holds.
fn wallet_key_if_needed<W>(
    options: &Options,
    selection: &Selection<'_>,
    derive: impl FnOnce() -> Result<W, Failure>,
) -> Result<Option<W>, Failure> {
    if options.reveal && selection.needs_wallet_key() {
        derive().map(Some)
    } else {
        Ok(None)
    }
}

fn usage(problem: &str) -> Failure {
    Failure::UsageOrIo(format!("{problem}\n\n{USAGE}"))
}

/// Write to stdout in one go. A closed pipe is an I/O error, not a panic.
fn print(text: &str) -> Result<(), Failure> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|e| Failure::UsageOrIo(format!("Couldn't print the Notes: {e}.")))
}

/// What to tell the owner about a `vault_format` error. `secret` is the credential tried, or
/// `None` while only the header has been read.
fn failure(error: Error, secret: Option<&Secret>) -> Failure {
    match (error, secret) {
        (Error::WrongCredential, Some(secret)) => {
            Failure::WrongCredential(format!("That {} didn't unlock this Vault.", secret.name()))
        }
        // The key derivation couldn't get its working memory. With a credential in hand the
        // limits were already checked, so `KdfOutOfLimits` there is the hash itself failing.
        (Error::OutOfMemory, _) | (Error::KdfOutOfLimits, Some(_)) => Failure::UsageOrIo(
            "There isn't enough free memory to derive this Vault's key. Close other apps and \
             try again."
                .to_string(),
        ),
        (Error::KdfOutOfLimits, None) => Failure::Damaged(
            "This Vault file's key settings are outside the safe range, so it is damaged or \
             was tampered with. Restore a Safety Copy or Backup."
                .to_string(),
        ),
        (Error::Malformed(what), _) => Failure::Damaged(format!(
            "This isn't a readable Vault file ({what}). If it is one, it's damaged: restore a \
             Safety Copy or Backup."
        )),
        (Error::UnsupportedVersion(version), _) => Failure::Damaged(format!(
            "This Vault file uses format version {version}, which this Emergency Reader \
             doesn't know. Use the Emergency Reader from a newer encrypted-note."
        )),
        (Error::Randomness, _) => {
            Failure::UsageOrIo("The system's random number generator failed.".to_string())
        }
        // Only writing a credential slot can report this, and the Emergency Reader never writes.
        (Error::SlotCheckFailed, _) => Failure::UsageOrIo(
            "The Emergency Reader hit an internal error it should never reach while only \
             reading. Nothing was changed; try again."
                .to_string(),
        ),
        (Error::WrongCredential | Error::Damaged, _) => Failure::Damaged(DAMAGED.to_string()),
    }
}

/// Opens Wallet Kind Hidden Fields with the Wallet Key (ADR-0004).
struct WalletFields {
    key: WalletKey,
    vault_id: [u8; 16],
}

impl WalletCipher for WalletFields {
    /// The Emergency Reader only reads.
    fn seal(&self, _: &str, _: &str, _: &[u8]) -> Result<Vec<u8>, NotesError> {
        Err(NotesError::WalletCipher)
    }

    fn open(
        &self,
        note_id: &str,
        field: &str,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, NotesError> {
        vault_format::open_wallet_field(&self.key, &self.vault_id, note_id, field, sealed)
            .map_err(|_| NotesError::WalletCipher)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use notes::{Kind, NewNote, NoteEdit, field};

    use super::*;
    use report::Numbered;

    fn options(args: &[&str]) -> Options {
        match parse(args.iter().map(OsString::from)) {
            Ok(Some(options)) => options,
            _ => panic!("{args:?} should parse"),
        }
    }

    /// tests/reader.rs runs the piped cases for real. A test can't hand the reader a terminal,
    /// so the terminal cases are checked here.
    #[test]
    fn reveal_is_refused_only_off_a_terminal_and_without_the_override() {
        let (terminal, not_terminal) = (true, false);
        let cases: [(&[&str], bool, bool); 7] = [
            (&["f"], terminal, false),
            (&["f"], not_terminal, false),
            (&["f", "--reveal"], terminal, false),
            (&["f", "--reveal"], not_terminal, true),
            (&["f", "--reveal", "--allow-non-terminal"], terminal, false),
            (
                &["f", "--allow-non-terminal", "--reveal"],
                not_terminal,
                false,
            ),
            (&["f", "--allow-non-terminal"], not_terminal, false),
        ];
        for (args, stdout_is_terminal, refused) in cases {
            assert_eq!(
                reveal_refused(&options(args), stdout_is_terminal),
                refused,
                "{args:?}, terminal: {stdout_is_terminal}"
            );
        }
    }

    #[test]
    fn revealing_one_note_is_refused_off_a_terminal_like_revealing_them_all() {
        let one_note: &[&str] = &["f", "--note", "2", "--reveal"];
        assert!(reveal_refused(&options(one_note), false));
        assert!(!reveal_refused(&options(one_note), true));
        assert!(!reveal_refused(&options(&["f", "--note", "2"]), false));
    }

    #[test]
    fn note_takes_a_number_from_1_up_before_or_after_the_file() {
        assert_eq!(options(&["f"]).note, None);
        assert_eq!(options(&["f", "--note", "3"]).note, Some(3));
        assert_eq!(options(&["--note", "3", "f"]).note, Some(3));
        assert_eq!(options(&["--note=12", "f", "--reveal"]).note, Some(12));
        // The argument after --note is its number, even when a file could have that name.
        let both = options(&["--note", "7", "7"]);
        assert_eq!((both.note, both.file), (Some(7), PathBuf::from("7")));
    }

    #[test]
    fn a_note_number_that_is_missing_or_not_a_number_from_1_up_is_a_usage_error() {
        let bad: [&[&str]; 10] = [
            &["f", "--note"],
            &["f", "--note", "0"],
            &["f", "--note=0"],
            &["f", "--note", "abc"],
            &["f", "--note", "-1"],
            &["f", "--note", "1.5"],
            &["f", "--note", "--reveal"],
            &["f", "--note="],
            &["f", "--note", "1", "--note", "2"],
            // The number was taken by --note, so no file is left.
            &["--note", "3"],
        ];
        for args in bad {
            assert!(
                matches!(
                    parse(args.iter().map(OsString::from)),
                    Err(Failure::UsageOrIo(_))
                ),
                "{args:?} should be a usage error"
            );
        }
    }

    // --- Whether the Wallet Key is derived. Every value below is an obvious fake. ---

    const NOW: i64 = 1_790_000_000;
    /// The public BIP39 test vector: not anyone's wallet.
    const SEED_WORDS: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    /// What the stand-in derivation hands back in place of a Wallet Key.
    const STAND_IN_KEY: &str = "fake-stand-in-for-the-wallet-key";

    /// Seals Wallet Kind fields while a fake Vault is built. Nothing in these tests opens one.
    struct FakeSeal;

    impl WalletCipher for FakeSeal {
        fn seal(&self, _: &str, _: &str, plaintext: &[u8]) -> Result<Vec<u8>, NotesError> {
            Ok(plaintext.iter().map(|b| b ^ 0x5a).collect())
        }

        fn open(&self, _: &str, _: &str, _: &[u8]) -> Result<Zeroizing<Vec<u8>>, NotesError> {
            Err(NotesError::WalletCipher)
        }
    }

    fn add(doc: &mut Document, kind: Kind, title: &str, hidden: &[(&str, &str)]) -> String {
        let new = NewNote {
            kind,
            title: title.into(),
            visible: Default::default(),
            hidden: hidden
                .iter()
                .map(|(name, value)| (name.to_string(), Zeroizing::new(value.to_string())))
                .collect(),
        };
        doc.create(new, Some(&FakeSeal), NOW).unwrap()
    }

    /// A Private Key whose key was cleared afterwards: a Wallet Kind with nothing to open.
    fn add_cleared_key(doc: &mut Document, title: &str) {
        let id = add(
            doc,
            Kind::PrivateKey,
            title,
            &[(field::KEY, "fake-private-key-2")],
        );
        let clear = NoteEdit {
            hidden: [(field::KEY.to_string(), Zeroizing::new(String::new()))].into(),
            ..Default::default()
        };
        doc.update(&id, clear, Some(&FakeSeal), NOW).unwrap();
    }

    /// The Notes of [`fake_vault`] by title, and whether revealing that Note takes the Wallet
    /// Key: only a Seed Phrase or a Private Key that has a Hidden Field value.
    const NOTES: [(&str, bool); 8] = [
        ("Fake wallet", true),
        ("Fake key", true),
        ("Fake cleared key", false),
        ("Fake login", false),
        ("Fake API key", false),
        ("Fake text", false),
        ("Fake old text", false),
        ("Fake old wallet", true),
    ];

    /// One Note of every Kind, a Private Key whose key was cleared and, in Trash, a Text and a
    /// Seed Phrase.
    fn fake_vault() -> Document {
        let mut doc = Document::new(NOW);
        let words = [
            (field::WORDS, SEED_WORDS),
            (field::PASSPHRASE, "fake-test-passphrase"),
        ];
        add(&mut doc, Kind::SeedPhrase, "Fake wallet", &words);
        let key = [(field::KEY, "fake-private-key-1")];
        add(&mut doc, Kind::PrivateKey, "Fake key", &key);
        add_cleared_key(&mut doc, "Fake cleared key");
        add_ordinary_kinds(&mut doc);
        let old_text = [(field::BODY, "fake-trashed-body")];
        let in_trash = [
            add(&mut doc, Kind::Text, "Fake old text", &old_text),
            add(&mut doc, Kind::SeedPhrase, "Fake old wallet", &words),
        ];
        for id in in_trash {
            doc.trash(&id, NOW + 1).unwrap();
        }
        doc
    }

    /// A Login, an API Key and a Text, each with every Hidden Field filled in.
    fn add_ordinary_kinds(doc: &mut Document) {
        let password = [(field::PASSWORD, "fake-login-password-123")];
        add(doc, Kind::Login, "Fake login", &password);
        let key_and_secret = [
            (field::KEY, "fake-api-key-0000"),
            (field::SECRET, "fake-api-secret-0000"),
        ];
        add(doc, Kind::ApiKey, "Fake API key", &key_and_secret);
        add(doc, Kind::Text, "Fake text", &[(field::BODY, "fake-body")]);
    }

    fn number_of(all: &[Numbered], title: &str) -> String {
        match all.iter().find(|note| note.view.title == title) {
            Some(note) => note.number.to_string(),
            None => panic!("no Note is titled {title}"),
        }
    }

    /// The selection `run` makes for these options.
    fn selected<'a>(all: &'a [Numbered], options: &Options) -> Selection<'a> {
        match Selection::of(all, options.note) {
            Ok(selection) => selection,
            Err(none) => panic!("{}", none.message()),
        }
    }

    /// Makes the decision as `run` does for these arguments, with a stand-in for the derivation
    /// that counts its calls. Returns how many times the Wallet Key was derived, and whether
    /// one was handed on.
    fn derivations(all: &[Numbered], args: &[&str]) -> (usize, bool) {
        let options = options(args);
        let selection = selected(all, &options);
        let derived = Cell::new(0);
        let key = wallet_key_if_needed(&options, &selection, || {
            derived.set(derived.get() + 1);
            Ok(STAND_IN_KEY)
        });
        match key {
            Ok(None) => (derived.get(), false),
            Ok(Some(key)) => {
                assert_eq!(key, STAND_IN_KEY);
                (derived.get(), true)
            }
            Err(_) => panic!("{args:?}: the stand-in derivation never fails"),
        }
    }

    #[test]
    fn revealing_one_note_derives_the_wallet_key_only_for_a_wallet_kind_with_a_value() {
        let doc = fake_vault();
        let all = report::numbered(&doc);
        assert_eq!(all.len(), NOTES.len());
        for (title, takes_the_key) in NOTES {
            let number = number_of(&all, title);
            let joined = format!("--note={number}");
            let ways: [&[&str]; 2] = [
                &["f", "--note", number.as_str(), "--reveal"],
                &["--reveal", joined.as_str(), "f"],
            ];
            for args in ways {
                assert_eq!(
                    derivations(&all, args),
                    (usize::from(takes_the_key), takes_the_key),
                    "{title}: {args:?}"
                );
            }
        }
    }

    #[test]
    fn nothing_is_derived_without_reveal_whatever_is_selected() {
        let doc = fake_vault();
        let all = report::numbered(&doc);
        assert_eq!(derivations(&all, &["f"]), (0, false));
        for (title, _) in NOTES {
            let number = number_of(&all, title);
            let args = ["f", "--note", number.as_str()];
            assert_eq!(derivations(&all, &args), (0, false), "{title}");
        }
    }

    #[test]
    fn revealing_every_note_derives_once_and_only_if_a_wallet_kind_has_a_value() {
        let doc = fake_vault();
        let all = report::numbered(&doc);
        assert_eq!(derivations(&all, &["f", "--reveal"]), (1, true));

        // No Wallet Kind with a value anywhere: nothing to derive the key for.
        let mut plain = Document::new(NOW);
        add_ordinary_kinds(&mut plain);
        add_cleared_key(&mut plain, "Fake cleared key");
        let all = report::numbered(&plain);
        assert_eq!(all.len(), 4);
        assert_eq!(derivations(&all, &["f", "--reveal"]), (0, false));

        let empty = report::numbered(&Document::new(NOW));
        assert_eq!(derivations(&empty, &["f", "--reveal"]), (0, false));
    }

    #[test]
    fn a_failed_derivation_is_passed_on_and_one_that_is_not_needed_is_never_run() {
        let doc = fake_vault();
        let all = report::numbered(&doc);
        let reveal = |title: &str| {
            let number = number_of(&all, title);
            options(&["f", "--note", number.as_str(), "--reveal"])
        };
        let fails = || Err::<&str, _>(Failure::WrongCredential("fake failure".into()));

        let wallet = reveal("Fake wallet");
        let outcome = wallet_key_if_needed(&wallet, &selected(&all, &wallet), fails);
        assert!(matches!(
            outcome,
            Err(Failure::WrongCredential(message)) if message == "fake failure"
        ));

        // A Login takes no Wallet Key, so the derivation that would fail is not run at all.
        let login = reveal("Fake login");
        let outcome = wallet_key_if_needed(&login, &selected(&all, &login), fails);
        assert!(matches!(outcome, Ok(None)));
    }
}
