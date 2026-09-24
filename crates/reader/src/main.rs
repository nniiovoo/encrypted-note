//! Emergency Reader (CONTEXT.md): prints a Vault or Backup's Notes to the terminal and writes
//! nothing to disk. Flags and exit codes are in [`USAGE`] and docs/FORMAT.md section 13.

mod credential;
mod report;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use notes::{Document, NotesError, WalletCipher};
use vault_format::{Error, Limits, WalletKey};
use zeroize::Zeroizing;

use credential::Secret;

const USAGE: &str = "\
Usage: enote-reader <vault-or-backup-file> [--recovery-key] [--reveal] [--credential-stdin]

Emergency Reader: prints the Notes in a Vault or Backup, for when the encrypted-note app
no longer works. It asks for the Master Password without showing what you type, and it
writes nothing to disk.

Options:
  --recovery-key      Ask for the Recovery Key (from your Recovery Kit) instead.
  --reveal            Also print Hidden Fields: passwords, keys, Seed Phrase words and
                      Passphrases, and the text of Text Notes.
  --credential-stdin  Read the Master Password or Recovery Key from the first line of stdin
                      instead of asking (for scripts).
  -h, --help          Show this help.

Exit codes: 0 ok, 1 wrong credential, 2 damaged or unsupported file, 3 usage or I/O error.
";

const DAMAGED: &str = "This Vault file is damaged. Restore a Safety Copy or Backup.";

/// Why the reader stopped (exit code 1, 2 or 3), with a message for stderr that never includes
/// credentials or Note contents.
enum Failure {
    WrongCredential(String),
    Damaged(String),
    UsageOrIo(String),
}

struct Options {
    file: PathBuf,
    recovery_key: bool,
    reveal: bool,
    credential_stdin: bool,
}

fn main() -> ExitCode {
    let (code, message) = match run() {
        Ok(()) => return ExitCode::SUCCESS,
        Err(Failure::WrongCredential(message)) => (1, message),
        Err(Failure::Damaged(message)) => (2, message),
        Err(Failure::UsageOrIo(message)) => (3, message),
    };
    // Nothing more can be done if stderr itself is gone.
    let _ = writeln!(io::stderr(), "{message}");
    ExitCode::from(code)
}

fn run() -> Result<(), Failure> {
    let Some(options) = parse(std::env::args_os().skip(1))? else {
        return print(USAGE);
    };
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
            _ => DAMAGED.to_string(),
        })
    })?;
    drop(body);

    let wallet = if options.reveal && report::needs_wallet_key(&document) {
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

    let (text, undecryptable) = report::render(&document, options.reveal, wallet.as_ref());
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
/// file named like one).
fn parse(args: impl Iterator<Item = OsString>) -> Result<Option<Options>, Failure> {
    let mut file = None;
    let (mut recovery_key, mut reveal, mut credential_stdin, mut help, mut flags_ended) =
        (false, false, false, false, false);
    for arg in args {
        if !flags_ended && arg.len() > 1 && arg.as_encoded_bytes().starts_with(b"-") {
            match arg.to_str() {
                Some("--") => flags_ended = true,
                Some("--recovery-key") => recovery_key = true,
                Some("--reveal") => reveal = true,
                Some("--credential-stdin") => credential_stdin = true,
                Some("-h" | "--help") => help = true,
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
        reveal,
        credential_stdin,
    }))
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
        // The limits were already checked, so this is the key derivation failing to get memory.
        (Error::KdfOutOfLimits, Some(_)) => Failure::UsageOrIo(
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
