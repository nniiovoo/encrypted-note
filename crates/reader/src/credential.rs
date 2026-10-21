//! Reading the Master Password or Recovery Key: from the terminal without echo, or from the
//! first line of stdin with `--credential-stdin`.

use std::fs::File;
use std::io::{self, IsTerminal, Read};

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
        let input = stdin_file().map_err(|e| stdin_failure(what, e))?;
        if input.is_terminal() {
            return Err(Failure::UsageOrIo(
                "--credential-stdin needs a pipe or file. To type the credential without \
                 showing it, run again without --credential-stdin."
                    .into(),
            ));
        }
        read_line(input, what)?
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

/// Borrow only stdin's OS handle; reading `Stdin` itself would fill its shared buffer.
fn stdin_file() -> io::Result<File> {
    let input = io::stdin();
    #[cfg(unix)]
    let handle = {
        use std::os::fd::AsFd;
        input.as_fd().try_clone_to_owned()?
    };
    #[cfg(windows)]
    let handle = {
        use std::os::windows::io::AsHandle;
        input.as_handle().try_clone_to_owned()?
    };
    Ok(File::from(handle))
}

fn stdin_failure(what: &str, error: impl std::fmt::Display) -> Failure {
    Failure::UsageOrIo(format!("Couldn't read the {what} from stdin ({error})."))
}

/// One line without its line ending (`\n` or `\r\n`); the last line may lack one.
fn read_line(mut input: impl Read, what: &str) -> Result<Zeroizing<String>, Failure> {
    let mut bytes = Zeroizing::new([0u8; MAX_LINE + 2]);
    let mut read = 0;
    // Read directly into wiping storage, without buffering or reading past the first line.
    while read < bytes.len() {
        match input.read(&mut bytes[read..read + 1]) {
            Ok(0) => break,
            Ok(_) => {
                read += 1;
                if bytes[read - 1] == b'\n' {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(stdin_failure(what, e)),
        }
    }
    if read == 0 {
        return Err(Failure::UsageOrIo(format!(
            "Expected the {what} on the first line of stdin, but stdin was empty."
        )));
    }
    let line = &bytes[..read];
    let content = match line.strip_suffix(b"\n") {
        Some(content) => content.strip_suffix(b"\r").unwrap_or(content),
        None => line,
    };
    if content.len() > MAX_LINE {
        return Err(Failure::UsageOrIo(format!(
            "The {what} on stdin is longer than {MAX_LINE} bytes."
        )));
    }
    let content = std::str::from_utf8(content).map_err(|e| stdin_failure(what, e))?;
    Ok(Zeroizing::new(content.to_owned()))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line(input: impl Read) -> Zeroizing<String> {
        read_line(input, "Master Password").unwrap_or_else(|_| panic!("fake input should read"))
    }

    fn error(input: impl Read) -> String {
        match read_line(input, "Master Password") {
            Err(Failure::UsageOrIo(message)) => message,
            _ => panic!("fake input should fail as a usage or I/O error"),
        }
    }

    #[test]
    fn unicode_and_line_endings_are_preserved_without_reading_ahead() {
        for ending in ["", "\n", "\r\n"] {
            assert_eq!(
                &*line(format!("fake-密碼-🔐{ending}").as_bytes()),
                "fake-密碼-🔐"
            );
        }
        assert_eq!(&*line(&b"\n"[..]), "");
        assert_eq!(&*line(&b"\r\n"[..]), "");
        assert_eq!(&*line(&b"fake\r"[..]), "fake\r");
        let mut input = io::Cursor::new(b"fake-first\nfake-second\n");
        assert_eq!(&*line(&mut input), "fake-first");
        assert_eq!(input.position(), 11);
    }

    #[test]
    fn the_bound_counts_content_bytes_for_every_line_ending() {
        let exact = format!("{}x", "假".repeat(MAX_LINE / 3));
        assert_eq!(exact.len(), MAX_LINE);
        for ending in ["", "\n", "\r\n"] {
            assert_eq!(&*line(format!("{exact}{ending}").as_bytes()), &exact);
            assert!(error(format!("{exact}x{ending}").as_bytes()).contains("longer than 4096"));
        }
        let mut input = io::repeat(b'x').take(10_000);
        assert!(error(&mut input).contains("longer than 4096"));
        assert_eq!(input.limit(), 10_000 - (MAX_LINE + 2) as u64);
    }

    #[test]
    fn empty_input_and_invalid_utf8_are_errors() {
        assert!(error(io::empty()).contains("stdin was empty"));
        let message = error(&b"fake-\xff\n"[..]);
        assert!(message.contains("Couldn't read"));
        assert!(!message.contains("fake-"));
    }

    #[test]
    fn interrupted_reads_retry_and_later_errors_are_reported_without_the_input() {
        struct Interrupts<R> {
            input: R,
            interrupted: bool,
        }
        impl<R: Read> Read for Interrupts<R> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                assert_eq!(out.len(), 1);
                self.interrupted = !self.interrupted;
                if self.interrupted {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    self.input.read(out)
                }
            }
        }
        assert_eq!(
            &*line(Interrupts {
                input: &b"fake\n"[..],
                interrupted: false
            }),
            "fake"
        );

        struct FailsAfterByte(bool);
        impl Read for FailsAfterByte {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::other("fake read error"));
                }
                self.0 = true;
                out[0] = b'X';
                Ok(1)
            }
        }
        let message = error(FailsAfterByte(false));
        assert!(message.contains("fake read error"));
        assert!(!message.contains('X'));
    }
}
