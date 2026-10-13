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
  --note <number>       Print only the Note with this number in the list. A Note keeps its
                        number for as long as the file stays the same.
  --reveal              Also print Hidden Fields: passwords, keys, Seed Phrase words and
                        Passphrases, and the text of Text Notes. With --note, only that
                        Note's; without it, every Note's. Only prints to a terminal: if the
                        output is redirected to a file or piped into another program, it
                        stops without printing anything.
  --allow-non-terminal  With --reveal, print even when the output is redirected or piped.
                        Whatever receives it then holds your secrets as plain text.
  --credential-stdin    Read the Master Password or Recovery Key from the first line of
                        stdin instead of asking (for scripts).
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

    // Only when a Note that is about to be revealed is a Wallet Kind: revealing a Login must
    // not bring the Wallet Key into memory.
    let wallet = if options.reveal && selection.needs_wallet_key() {
        // A fresh derivation from the same credential that opened the Vault (ADR-0004).
        let key =
            vault_format::unlock_wallet_key(&unlocked, secret.credential(), &Limits::PRODUCTION)
                .map_err(|e| failure(e, Some(&secret)))?;
        Some(WalletFields {
            key,
            vault_id: unlocked.vault_id(),
        })
    } else {
        None
    };
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
    use super::*;

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
}
