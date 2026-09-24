//! The Emergency Reader, tested through its command line.
//!
//! Each test writes a real Vault with the core crates (`vault-format` + `notes`) at the
//! PRODUCTION floor (the reader opens files with `Limits::PRODUCTION` only), runs the built
//! binary on it with the credential on stdin, and checks what it prints and its exit code.
//! Every run happens in a fresh directory that is also the child's working directory, HOME and
//! TMPDIR, and every run checks that the directory holds exactly the same files afterwards.
//!
//! All Note contents are obviously fake.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use notes::{Document, Kind, NewNote, NotesError, WalletCipher, field};
use secrecy::SecretString;
use vault_format::{Credential, KdfParams, Limits, RecoveryKey, UnlockedVault, WalletKey};
use zeroize::Zeroizing;

const PASSWORD: &str = "correct-horse-test-password-1";
const WRONG_PASSWORD: &str = "battery-staple-test-password-2";
/// The PRODUCTION floor.
const FLOOR: KdfParams = KdfParams {
    m_kib: 65536,
    t: 3,
    p: 1,
};
const NOW: i64 = 1_790_000_000;
const MASK: &str = "••••••";

// The Text + Login Vault.
const TEXT_TITLE: &str = "Test shopping list";
const TEXT_BODY: &str = "fake-text-body-line-1\nfake-text-body-line-2";
const LOGIN_TITLE: &str = "Test example login";
const WEBSITE: &str = "https://login.example.test";
const USERNAME: &str = "test-user@example.test";
const LOGIN_PASSWORD: &str = "fake-login-password-123";

// The Wallet Kinds + Trash Vault.
const SEED_TITLE: &str = "Test hardware wallet";
const WALLET_NAME: &str = "Test Wallet One";
/// The public BIP39 test vector: not anyone's wallet.
const SEED_WORDS: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const SEED_PASSPHRASE: &str = "fake-test-passphrase";
const KEY_TITLE: &str = "Test EVM account";
const CHAIN: &str = "evm";
const ADDRESS: &str = "0xfake-test-address-0001";
const PRIVATE_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const API_TITLE: &str = "Test API key";
const SERVICE: &str = "Example Test Service";
const API_KEY: &str = "fake-api-key-0000";
const TRASHED_TITLE: &str = "Old test note";
const TRASHED_BODY: &str = "fake-trashed-body";

// --- Fixtures ---

fn new_note(kind: Kind, title: &str, visible: &[(&str, &str)], hidden: &[(&str, &str)]) -> NewNote {
    NewNote {
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

/// Seals Wallet Kind fields the way the app does, with `vault_format`'s wallet-field functions.
struct TestWalletCipher<'a> {
    key: &'a WalletKey,
    vault_id: [u8; 16],
}

impl WalletCipher for TestWalletCipher<'_> {
    fn seal(&self, note_id: &str, field: &str, plaintext: &[u8]) -> Result<Vec<u8>, NotesError> {
        vault_format::seal_wallet_field(self.key, &self.vault_id, note_id, field, plaintext)
            .map_err(|_| NotesError::WalletCipher)
    }

    fn open(
        &self,
        note_id: &str,
        field: &str,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, NotesError> {
        vault_format::open_wallet_field(self.key, &self.vault_id, note_id, field, sealed)
            .map_err(|_| NotesError::WalletCipher)
    }
}

/// A Vault whose body is one Text Note and one Login, made with `vault_format::create`.
struct TextAndLogin {
    bytes: Vec<u8>,
    recovery_key: String,
}

fn text_and_login() -> &'static TextAndLogin {
    static VAULT: OnceLock<TextAndLogin> = OnceLock::new();
    VAULT.get_or_init(|| {
        let mut doc = Document::new(NOW);
        doc.create(
            new_note(Kind::Text, TEXT_TITLE, &[], &[(field::BODY, TEXT_BODY)]),
            None,
            NOW,
        )
        .unwrap();
        doc.create(
            new_note(
                Kind::Login,
                LOGIN_TITLE,
                &[(field::WEBSITE, WEBSITE), (field::USERNAME, USERNAME)],
                &[(field::PASSWORD, LOGIN_PASSWORD)],
            ),
            None,
            NOW,
        )
        .unwrap();
        let password = SecretString::from(PASSWORD);
        let created =
            vault_format::create(&password, FLOOR, &Limits::PRODUCTION, &doc.to_json()).unwrap();
        TextAndLogin {
            bytes: created.bytes,
            recovery_key: created.recovery_key.expose_display().to_string(),
        }
    })
}

/// A Vault with a Seed Phrase, a Private Key, an API Key and a Text Note in Trash. Keeps the
/// unlocked Vault and Wallet Key so other bodies can be sealed into it without new derivations.
struct WalletVault {
    bytes: Vec<u8>,
    recovery_key: String,
    unlocked: UnlockedVault,
    wallet_key: WalletKey,
}

fn wallet_vault() -> &'static WalletVault {
    static VAULT: OnceLock<WalletVault> = OnceLock::new();
    VAULT.get_or_init(|| {
        let password = SecretString::from(PASSWORD);
        let created = vault_format::create(&password, FLOOR, &Limits::PRODUCTION, b"").unwrap();
        let wallet_key = vault_format::unlock_wallet_key(
            &created.unlocked,
            Credential::MasterPassword(&password),
            &Limits::PRODUCTION,
        )
        .unwrap();
        let cipher = TestWalletCipher {
            key: &wallet_key,
            vault_id: created.unlocked.vault_id(),
        };

        let mut doc = Document::new(NOW);
        doc.create(
            new_note(
                Kind::SeedPhrase,
                SEED_TITLE,
                &[(field::WALLET_NAME, WALLET_NAME)],
                &[
                    (field::WORDS, SEED_WORDS),
                    (field::PASSPHRASE, SEED_PASSPHRASE),
                ],
            ),
            Some(&cipher),
            NOW,
        )
        .unwrap();
        doc.create(
            new_note(
                Kind::PrivateKey,
                KEY_TITLE,
                &[(field::CHAIN, CHAIN), (field::ADDRESS, ADDRESS)],
                &[(field::KEY, PRIVATE_KEY)],
            ),
            Some(&cipher),
            NOW,
        )
        .unwrap();
        doc.create(
            new_note(
                Kind::ApiKey,
                API_TITLE,
                &[(field::SERVICE, SERVICE)],
                &[(field::KEY, API_KEY)],
            ),
            None,
            NOW,
        )
        .unwrap();
        let trashed = doc
            .create(
                new_note(
                    Kind::Text,
                    TRASHED_TITLE,
                    &[],
                    &[(field::BODY, TRASHED_BODY)],
                ),
                None,
                NOW,
            )
            .unwrap();
        doc.trash(&trashed, NOW + 60).unwrap();

        let bytes = vault_format::seal(&created.unlocked, &doc.to_json()).unwrap();
        WalletVault {
            bytes,
            recovery_key: created.recovery_key.expose_display().to_string(),
            unlocked: created.unlocked,
            wallet_key,
        }
    })
}

// --- Running the binary ---

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn line_with(&self, needle: &str) -> &str {
        self.stdout
            .lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no output line contains {needle:?}:\n{}", self.stdout))
    }

    /// The first line printing the field `label` (`<indent><label>: ...`).
    fn field_line(&self, label: &str) -> &str {
        let prefix = format!("{label}:");
        self.stdout
            .lines()
            .find(|line| line.starts_with(' ') && line.trim_start().starts_with(&prefix))
            .unwrap_or_else(|| panic!("no field line for {label:?}:\n{}", self.stdout))
    }
}

/// Every path under `dir`, with each file's contents.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.insert(path.clone(), None);
                pending.push(path);
            } else {
                let contents = std::fs::read(&path).unwrap();
                found.insert(path, Some(contents));
            }
        }
    }
    found
}

/// Write `file_bytes` (if any) into a fresh directory as `test.vault`, run the reader on it with
/// `stdin`, and check that the directory is unchanged afterwards.
fn run_with_stdin(test: &str, file_bytes: Option<&[u8]>, args: &[&str], stdin: &[u8]) -> Run {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("enote-reader-tests")
        .join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(bytes) = file_bytes {
        std::fs::write(dir.join("test.vault"), bytes).unwrap();
    }
    let before = snapshot(&dir);

    let mut child = Command::new(env!("CARGO_BIN_EXE_enote-reader"))
        .args(args)
        .current_dir(&dir)
        .env("HOME", &dir)
        .env("TMPDIR", &dir)
        .env("TMP", &dir)
        .env("TEMP", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child_stdin = child.stdin.take().unwrap();
    // The reader may exit before reading stdin (usage errors), so a failed write is fine.
    let _ = child_stdin.write_all(stdin);
    drop(child_stdin);
    let output = child.wait_with_output().unwrap();

    assert!(
        before == snapshot(&dir),
        "the Emergency Reader must not create, remove or change files"
    );
    std::fs::remove_dir_all(&dir).unwrap();

    Run {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// Run the reader on `bytes` with `extra` flags and `credential` on stdin.
fn run(test: &str, bytes: &[u8], extra: &[&str], credential: &str) -> Run {
    let mut args = vec!["test.vault", "--credential-stdin"];
    args.extend_from_slice(extra);
    run_with_stdin(
        test,
        Some(bytes),
        &args,
        format!("{credential}\n").as_bytes(),
    )
}

fn assert_exit(run: &Run, code: i32) {
    assert_eq!(
        run.code,
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );
}

// --- Printing Notes ---

#[test]
fn prints_titles_and_visible_fields_with_hidden_fields_masked_and_warns_on_stderr() {
    let run = run("masked", &text_and_login().bytes, &[], PASSWORD);
    assert_exit(&run, 0);

    assert!(run.line_with(TEXT_TITLE).contains("Text"));
    assert!(run.line_with(LOGIN_TITLE).contains("Login"));
    assert!(run.stdout.contains(WEBSITE));
    assert!(run.stdout.contains(USERNAME));

    assert!(!run.stdout.contains(LOGIN_PASSWORD));
    assert!(!run.stdout.contains("fake-text-body"));
    assert!(run.field_line("Password").contains(MASK));
    assert!(run.field_line("Body").contains(MASK));

    // Warns not to redirect the output to a file.
    let warning = run
        .stderr
        .lines()
        .find(|line| line.contains("file"))
        .expect("a warning about files on stderr");
    assert!(warning.to_lowercase().contains("redirect"));
    assert!(!run.stdout.to_lowercase().contains("redirect"));
}

#[test]
fn reveal_prints_hidden_fields() {
    let run = run("reveal", &text_and_login().bytes, &["--reveal"], PASSWORD);
    assert_exit(&run, 0);

    assert!(run.stdout.contains(TEXT_TITLE));
    assert!(run.stdout.contains(LOGIN_TITLE));
    assert!(run.field_line("Password").contains(LOGIN_PASSWORD));
    assert!(run.stdout.contains("fake-text-body-line-1"));
    assert!(run.stdout.contains("fake-text-body-line-2"));
    assert!(!run.stdout.contains(MASK));
}

#[test]
fn flags_may_come_before_the_file() {
    let run = run_with_stdin(
        "flags-first",
        Some(&text_and_login().bytes),
        &["--reveal", "--credential-stdin", "test.vault"],
        format!("{PASSWORD}\n").as_bytes(),
    );
    assert_exit(&run, 0);
    assert!(run.stdout.contains(LOGIN_PASSWORD));
}

#[test]
fn credential_on_stdin_may_end_without_a_newline_or_with_crlf() {
    for (test, stdin) in [
        ("no-newline", PASSWORD.to_string()),
        ("crlf", format!("{PASSWORD}\r\n")),
    ] {
        let run = run_with_stdin(
            test,
            Some(&text_and_login().bytes),
            &["test.vault", "--credential-stdin"],
            stdin.as_bytes(),
        );
        assert_exit(&run, 0);
        assert!(run.stdout.contains(LOGIN_TITLE));
    }
}

#[test]
fn wallet_kinds_and_trash_are_printed_with_hidden_fields_masked() {
    let run = run("wallet-masked", &wallet_vault().bytes, &[], PASSWORD);
    assert_exit(&run, 0);

    assert!(run.line_with(SEED_TITLE).contains("Seed Phrase"));
    assert!(run.line_with(KEY_TITLE).contains("Private Key"));
    assert!(run.line_with(API_TITLE).contains("API Key"));
    for shown in [WALLET_NAME, CHAIN, ADDRESS, SERVICE] {
        assert!(run.stdout.contains(shown), "{shown} should be printed");
    }
    assert!(run.field_line("Word count").contains("12"));

    for secret in [
        "abandon",
        SEED_PASSPHRASE,
        PRIVATE_KEY,
        API_KEY,
        TRASHED_BODY,
    ] {
        assert!(!run.stdout.contains(secret), "{secret} should be hidden");
    }
    assert!(run.field_line("Words").contains(MASK));
    assert!(run.field_line("Passphrase").contains(MASK));

    // Notes in Trash are printed and marked.
    assert!(run.line_with(TRASHED_TITLE).contains("Trash"));
    for title in [SEED_TITLE, KEY_TITLE, API_TITLE] {
        assert!(!run.line_with(title).contains("Trash"));
    }
}

#[test]
fn reveal_opens_wallet_kind_fields_with_either_credential() {
    let vault = wallet_vault();
    for (test, flags, credential) in [
        ("wallet-reveal-password", &["--reveal"][..], PASSWORD),
        (
            "wallet-reveal-recovery",
            &["--recovery-key", "--reveal"],
            &vault.recovery_key,
        ),
    ] {
        let run = run(test, &vault.bytes, flags, credential);
        assert_exit(&run, 0);
        assert!(run.field_line("Passphrase").contains(SEED_PASSPHRASE));
        assert!(run.stdout.contains(PRIVATE_KEY));
        assert!(run.stdout.contains(API_KEY));
        assert!(run.stdout.contains(TRASHED_BODY));
        // Every word is printed, in order.
        let printed: Vec<&str> = run
            .stdout
            .split_whitespace()
            .filter(|w| *w == "abandon" || *w == "about")
            .collect();
        assert_eq!(printed, SEED_WORDS.split(' ').collect::<Vec<_>>());
        assert!(!run.stdout.contains(MASK));
    }
}

#[test]
fn an_empty_vault_says_it_has_no_notes() {
    let empty = Document::new(NOW);
    let bytes = vault_format::seal(&wallet_vault().unlocked, &empty.to_json()).unwrap();
    let run = run("empty", &bytes, &[], PASSWORD);
    assert_exit(&run, 0);
    assert!(run.stdout.contains("no Notes"));
}

// --- Credentials ---

#[test]
fn recovery_key_opens_the_vault_also_in_lowercase_without_dashes() {
    let vault = text_and_login();
    for (test, typed) in [
        ("recovery", vault.recovery_key.clone()),
        (
            "recovery-typed",
            vault.recovery_key.replace('-', " ").to_lowercase(),
        ),
    ] {
        let run = run(test, &vault.bytes, &["--recovery-key"], &typed);
        assert_exit(&run, 0);
        assert!(run.stdout.contains(TEXT_TITLE));
        assert!(run.stdout.contains(LOGIN_TITLE));
        assert!(!run.stdout.contains(LOGIN_PASSWORD));
    }
}

/// A wrong Master Password, the Recovery Key given as a Master Password, and another Vault's
/// Recovery Key.
#[test]
fn a_credential_that_does_not_unlock_the_vault_exits_1_and_prints_no_notes() {
    let vault = text_and_login();
    let other = RecoveryKey::generate().unwrap();
    for (test, flags, credential) in [
        ("wrong-password", &[][..], WRONG_PASSWORD),
        ("recovery-as-password", &[], &vault.recovery_key),
        (
            "other-recovery-key",
            &["--recovery-key"],
            other.expose_display(),
        ),
    ] {
        let run = run(test, &vault.bytes, flags, credential);
        assert_exit(&run, 1);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(run.stderr.contains("didn't unlock this Vault"), "{test}");
    }
}

#[test]
fn a_recovery_key_with_a_typo_or_the_wrong_length_exits_1() {
    let vault = text_and_login();
    // Change one symbol of the first group to a different valid symbol.
    let mut typo: Vec<char> = vault.recovery_key.chars().collect();
    typo[0] = if typo[0] == 'A' { 'B' } else { 'A' };
    let typo: String = typo.into_iter().collect();
    for (test, typed) in [
        ("recovery-typo", typo.as_str()),
        ("recovery-length", "ABCD-EFGH"),
    ] {
        let run = run(test, &vault.bytes, &["--recovery-key"], typed);
        assert_exit(&run, 1);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(run.stderr.contains("Recovery Key"), "{test}");
    }
}

#[test]
fn empty_stdin_is_a_usage_error() {
    let run = run_with_stdin(
        "empty-stdin",
        Some(&text_and_login().bytes),
        &["test.vault", "--credential-stdin"],
        b"",
    );
    assert_exit(&run, 3);
    assert!(run.stdout.is_empty());
}

// --- Damaged and unsupported files ---

#[test]
fn a_damaged_or_unsupported_file_exits_2_and_prints_nothing() {
    let mut damaged_body = text_and_login().bytes.clone();
    *damaged_body.last_mut().unwrap() ^= 0x01;
    let mut newer_version = text_and_login().bytes.clone();
    // format_version is the u16 after the 8-byte magic.
    newer_version[8..10].copy_from_slice(&2u16.to_le_bytes());
    // Tiny parameters are fine for vault-format's test limits, but the reader never uses those.
    let tiny = KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    };
    let tiny_limits = Limits {
        min: tiny,
        max: Limits::PRODUCTION.max,
    };
    let password = SecretString::from(PASSWORD);
    let valid_body = Document::new(NOW).to_json();
    let below_floor = vault_format::create(&password, tiny, &tiny_limits, &valid_body).unwrap();
    let not_a_document =
        vault_format::seal(&wallet_vault().unlocked, b"{\"not\": \"a document\"}").unwrap();

    for (test, bytes, stderr) in [
        ("damaged-body", &damaged_body[..], "damaged"),
        (
            "not-a-vault",
            b"just some fake text, not a Vault",
            "damaged",
        ),
        ("newer-version", &newer_version, "version 2"),
        ("below-floor", &below_floor.bytes, "damaged"),
        ("not-a-document", &not_a_document, "damaged"),
    ] {
        let run = run(test, bytes, &[], PASSWORD);
        assert_exit(&run, 2);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(run.stderr.contains(stderr), "{test}: {}", run.stderr);
    }
}

#[test]
fn a_wallet_field_that_fails_to_decrypt_is_marked_and_exits_2() {
    let vault = wallet_vault();
    // Sealed under the right Wallet Key but bound to another vault id, so it can't be opened.
    let wrong = TestWalletCipher {
        key: &vault.wallet_key,
        vault_id: [0xee; 16],
    };
    let mut doc = Document::new(NOW);
    doc.create(
        new_note(
            Kind::PrivateKey,
            KEY_TITLE,
            &[(field::CHAIN, CHAIN), (field::ADDRESS, ADDRESS)],
            &[(field::KEY, PRIVATE_KEY)],
        ),
        Some(&wrong),
        NOW,
    )
    .unwrap();
    doc.create(
        new_note(
            Kind::Login,
            LOGIN_TITLE,
            &[(field::WEBSITE, WEBSITE), (field::USERNAME, USERNAME)],
            &[(field::PASSWORD, LOGIN_PASSWORD)],
        ),
        None,
        NOW,
    )
    .unwrap();
    let bytes = vault_format::seal(&vault.unlocked, &doc.to_json()).unwrap();

    let run = run("wallet-damaged", &bytes, &["--reveal"], PASSWORD);
    assert_exit(&run, 2);
    // Everything else is still printed.
    assert!(run.stdout.contains(KEY_TITLE));
    assert!(run.stdout.contains(ADDRESS));
    assert!(run.field_line("Password").contains(LOGIN_PASSWORD));
    assert!(!run.stdout.contains(PRIVATE_KEY));
    assert!(run.field_line("Key").contains("damaged"));
    assert!(run.stderr.contains("damaged"));
}

// --- Usage and I/O errors ---

#[test]
fn a_missing_file_exits_3() {
    let run = run_with_stdin(
        "missing-file",
        None,
        &["test.vault", "--credential-stdin"],
        format!("{PASSWORD}\n").as_bytes(),
    );
    assert_exit(&run, 3);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("test.vault"));
}

#[test]
fn usage_errors_exit_3() {
    let cases: [(&str, &[&str]); 4] = [
        ("no-args", &[]),
        ("only-flags", &["--reveal", "--credential-stdin"]),
        (
            "unknown-flag",
            &["test.vault", "--credential-stdin", "--show-all"],
        ),
        (
            "two-files",
            &["test.vault", "other.vault", "--credential-stdin"],
        ),
    ];
    for (test, args) in cases {
        let run = run_with_stdin(
            test,
            Some(&text_and_login().bytes),
            args,
            format!("{PASSWORD}\n").as_bytes(),
        );
        assert_exit(&run, 3);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(run.stderr.contains("Usage"), "{test}: {}", run.stderr);
    }
}

#[test]
fn help_prints_usage_and_exits_0() {
    let run = run_with_stdin("help", None, &["--help"], b"");
    assert_exit(&run, 0);
    assert!(run.stdout.contains("Usage"));
    assert!(run.stdout.contains("--reveal"));
    assert!(run.stdout.contains("--recovery-key"));
}
