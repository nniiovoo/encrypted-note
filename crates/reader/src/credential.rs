//! Reading the Master Password or Recovery Key: from the terminal without echo, or from the
//! first line of stdin with `--credential-stdin`.

use std::io::{self, BufRead};

use secrecy::SecretString;
use vault_format::{Credential, RecoveryKey, RecoveryKeyError};
use zeroize::Zeroizing;

use crate::{Failure, Options};

/// Longest credential line accepted on stdin, in bytes (a bound on untrusted input).
const MAX_LINE: usize = 4096;

/// The credential the owner gave, held in wiping types.
pub enum Secret {
    MasterPassword(SecretString),
    RecoveryKey(RecoveryKey),
}

impl Secret {
    pub fn credential(&self) -> Credential<'_> {
        match self {
            Secret::MasterPassword(password) => Credential::MasterPassword(password),
            Secret::RecoveryKey(key) => Credential::RecoveryKey(key),
        }
    }

    /// How error messages name it.
    pub fn name(&self) -> &'static str {
        match self {
            Secret::MasterPassword(_) => "password",
            Secret::RecoveryKey(_) => "Recovery Key",
        }
    }
}

/// A Recovery Key that can't be parsed is a wrong credential.
pub fn read(options: &Options) -> Result<Secret, Failure> {
    let what = if options.recovery_key {
        "Recovery Key"
    } else {
        "Master Password"
    };
    let line = if options.credential_stdin {
        read_line(io::stdin().lock(), what)?
    } else {
        rpassword::prompt_password(format!("{what}: "))
            .map(Zeroizing::new)
            .map_err(|e| {
                Failure::UsageOrIo(format!(
                    "Couldn't read the {what} from the terminal ({e}). \
                     To give it on stdin instead, add --credential-stdin."
                ))
            })?
    };

    if options.recovery_key {
        RecoveryKey::parse(&line)
            .map(Secret::RecoveryKey)
            .map_err(recovery_key_failure)
    } else {
        // An exact-size copy, so building the secret box leaves no unwiped buffer behind.
        Ok(Secret::MasterPassword(SecretString::from(line.as_str())))
    }
}

/// One line without its line ending (`\n` or `\r\n`); the last line may lack one.
fn read_line(input: impl BufRead, what: &str) -> Result<Zeroizing<String>, Failure> {
    let limit = MAX_LINE + 2;
    // Reserved up front so reading never reallocates (which would leave unwiped copies).
    let mut line = Zeroizing::new(String::with_capacity(limit));
    let read = input
        .take(limit as u64)
        .read_line(&mut line)
        .map_err(|e| Failure::UsageOrIo(format!("Couldn't read the {what} from stdin ({e}).")))?;
    if read == 0 {
        return Err(Failure::UsageOrIo(format!(
            "Expected the {what} on the first line of stdin, but stdin was empty."
        )));
    }
    let content_len = match line.strip_suffix('\n') {
        Some(content) => content.strip_suffix('\r').unwrap_or(content).len(),
        None if line.len() >= limit => {
            return Err(Failure::UsageOrIo(format!(
                "The {what} on stdin is longer than {MAX_LINE} bytes."
            )));
        }
        None => line.len(),
    };
    line.truncate(content_len);
    Ok(line)
}

fn recovery_key_failure(error: RecoveryKeyError) -> Failure {
    let problem = match error {
        RecoveryKeyError::Length => "That isn't a whole Recovery Key: it has 28 letters and \
             digits, in 7 groups of 4 like XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX."
            .to_string(),
        RecoveryKeyError::InvalidSymbol(position) => format!(
            "That Recovery Key has a character that can't be in one (character {}, not \
             counting spaces and dashes).",
            position + 1
        ),
        RecoveryKeyError::Checksum => "That Recovery Key has a typo.".to_string(),
    };
    Failure::WrongCredential(format!(
        "{problem} Check it against your Recovery Kit and try again."
    ))
}
