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
/// These tests read the reader's output through a pipe, and `--reveal` refuses to print into
/// anything but a terminal unless it is told to.
const REVEAL: [&str; 2] = ["--reveal", "--allow-non-terminal"];

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

    /// The number the list gives the Note with this title: its heading is
    /// `<number>. <Kind>: <title>`, starting at the left edge.
    fn number_of(&self, title: &str) -> String {
        let heading = self
            .stdout
            .lines()
            .find(|line| !line.starts_with(' ') && line.ends_with(title))
            .unwrap_or_else(|| panic!("no heading for {title:?}:\n{}", self.stdout));
        let (number, _) = heading.split_once(". ").expect("a numbered heading");
        assert!(
            number.parse::<usize>().is_ok_and(|n| n >= 1),
            "{heading:?} doesn't start with a number from 1 up"
        );
        number.to_string()
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

/// The built reader, set to run in `dir` with `dir` as its HOME and temp folder too.
fn reader(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_enote-reader"));
    command
        .current_dir(dir)
        .env("HOME", dir)
        .env("TMPDIR", dir)
        .env("TMP", dir)
        .env("TEMP", dir);
    command
}

/// Write `file_bytes` (if any) into a fresh directory as `test.vault`, run the reader on it with
/// `stdin`, and check that the directory is unchanged afterwards. The reader's output is a pipe.
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

    let mut child = reader(&dir)
        .args(args)
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
    let run = run("reveal", &text_and_login().bytes, &REVEAL, PASSWORD);
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
        &[
            "--reveal",
            "--allow-non-terminal",
            "--credential-stdin",
            "test.vault",
        ],
        format!("{PASSWORD}\n").as_bytes(),
    );
    assert_exit(&run, 0);
    assert!(run.stdout.contains(LOGIN_PASSWORD));
}

// --- Revealing when the output is not a terminal ---

/// The same answer for a good Vault with the right password, with a wrong one, and with no
/// file at all: the reader stops before it looks at either.
#[test]
fn reveal_into_a_pipe_is_refused_before_the_file_or_credential_is_read() {
    let vault = text_and_login();
    for (test, bytes, credential) in [
        ("reveal-piped", Some(&vault.bytes[..]), PASSWORD),
        ("reveal-piped-wrong", Some(&vault.bytes[..]), WRONG_PASSWORD),
        ("reveal-piped-no-file", None, PASSWORD),
    ] {
        let run = run_with_stdin(
            test,
            bytes,
            &["test.vault", "--credential-stdin", "--reveal"],
            format!("{credential}\n").as_bytes(),
        );
        assert_exit(&run, 3);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(
            run.stderr.contains("not going to a terminal"),
            "{test}: {}",
            run.stderr
        );
        // It says how to do it anyway.
        assert!(
            run.stderr.contains("--allow-non-terminal"),
            "{test}: {}",
            run.stderr
        );
    }
}

/// `enote-reader vault --reveal > notes.txt` must not leave the secrets in `notes.txt`.
#[test]
fn reveal_redirected_to_a_file_leaves_it_empty_unless_allowed() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("enote-reader-tests")
        .join(format!("reveal-redirected-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.vault"), &text_and_login().bytes).unwrap();

    let redirect = |extra: &[&str]| {
        let out = dir.join("notes.txt");
        let mut child = reader(&dir)
            .args(["test.vault", "--credential-stdin", "--reveal"])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut child_stdin = child.stdin.take().unwrap();
        // The reader may exit before reading stdin, so a failed write is fine.
        let _ = child_stdin.write_all(format!("{PASSWORD}\n").as_bytes());
        drop(child_stdin);
        let output = child.wait_with_output().unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        (output.status.code(), written)
    };

    let (code, written) = redirect(&[]);
    assert_eq!(code, Some(3));
    assert!(written.is_empty(), "nothing may reach the file");

    let (code, written) = redirect(&["--allow-non-terminal"]);
    assert_eq!(code, Some(0));
    assert!(written.contains(LOGIN_PASSWORD));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn control_and_format_characters_in_notes_are_printed_as_replacement_characters() {
    // An escape sequence that would clear the screen, a right-to-left override that would show
    // the rest of the line backwards, and two characters that take no space.
    const ESCAPE: char = '\u{1b}';
    const RIGHT_TO_LEFT_OVERRIDE: char = '\u{202e}';
    const ZERO_WIDTH_SPACE: char = '\u{200b}';
    const BYTE_ORDER_MARK: char = '\u{feff}';
    let title = format!("Fake{ESCAPE}[2J title");
    let website = format!("https://{RIGHT_TO_LEFT_OVERRIDE}tset.elpmaxe.etis");
    let username = format!("fake{ZERO_WIDTH_SPACE}user");
    let password = format!("fake{BYTE_ORDER_MARK}pass{RIGHT_TO_LEFT_OVERRIDE}word");

    let mut doc = Document::new(NOW);
    doc.create(
        new_note(
            Kind::Login,
            &title,
            &[(field::WEBSITE, &website), (field::USERNAME, &username)],
            &[(field::PASSWORD, &password)],
        ),
        None,
        NOW,
    )
    .unwrap();
    let bytes = vault_format::seal(&wallet_vault().unlocked, &doc.to_json()).unwrap();

    let run = run("hostile-characters", &bytes, &REVEAL, PASSWORD);
    assert_exit(&run, 0);
    for hostile in [
        ESCAPE,
        RIGHT_TO_LEFT_OVERRIDE,
        ZERO_WIDTH_SPACE,
        BYTE_ORDER_MARK,
    ] {
        assert!(
            !run.stdout.contains(hostile),
            "U+{:04X} was printed",
            hostile as u32
        );
    }
    assert!(run.stdout.contains("Fake\u{fffd}[2J title"));
    assert!(
        run.field_line("Website")
            .contains("https://\u{fffd}tset.elpmaxe.etis")
    );
    assert!(run.field_line("Username").contains("fake\u{fffd}user"));
    assert!(
        run.field_line("Password")
            .contains("fake\u{fffd}pass\u{fffd}word")
    );
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
        ("wallet-reveal-password", &REVEAL[..], PASSWORD),
        (
            "wallet-reveal-recovery",
            &["--recovery-key", "--reveal", "--allow-non-terminal"],
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

// --- Numbers, and printing one Note ---

const WALLET_VAULT_TITLES: [&str; 4] = [SEED_TITLE, KEY_TITLE, API_TITLE, TRASHED_TITLE];
/// Something only `--reveal` prints, for each Note of the wallet Vault.
const WALLET_VAULT_HIDDEN: [(&str, &[&str]); 4] = [
    (SEED_TITLE, &["abandon", "about", SEED_PASSPHRASE]),
    (KEY_TITLE, &[PRIVATE_KEY]),
    (API_TITLE, &[API_KEY]),
    (TRASHED_TITLE, &[TRASHED_BODY]),
];

#[test]
fn the_list_numbers_every_note_from_1_with_trash_last_and_the_same_way_each_time() {
    let vault = wallet_vault();
    let first = run("numbered", &vault.bytes, &[], PASSWORD);
    assert_exit(&first, 0);
    let mut numbers = WALLET_VAULT_TITLES.map(|title| first.number_of(title));
    // The Note in Trash is printed last.
    assert_eq!(numbers[3], "4");
    assert!(
        first
            .line_with(TRASHED_TITLE)
            .starts_with("4. [In Trash] Text: ")
    );
    numbers.sort();
    assert_eq!(numbers, ["1", "2", "3", "4"]);
    // It says how to show one Note's Hidden Fields without showing them all.
    assert!(first.stdout.contains("--note <its number> --reveal"));

    // The same file, read again and with the other credential: the same numbers.
    let again = run(
        "numbered-again",
        &vault.bytes,
        &["--recovery-key"],
        &vault.recovery_key,
    );
    assert_exit(&again, 0);
    for title in WALLET_VAULT_TITLES {
        assert_eq!(again.number_of(title), first.number_of(title), "{title}");
    }
}

#[test]
fn note_prints_only_that_note_with_its_hidden_fields_masked() {
    let vault = wallet_vault();
    let list = run("note-list", &vault.bytes, &[], PASSWORD);
    for title in WALLET_VAULT_TITLES {
        let number = list.number_of(title);
        let one = run(
            &format!("note-masked-{number}"),
            &vault.bytes,
            &["--note", &number],
            PASSWORD,
        );
        assert_exit(&one, 0);
        // The Note's heading is the first line: no count of Notes, nothing before it.
        let heading = one.stdout.lines().next().unwrap();
        assert!(heading.starts_with(&format!("{number}. ")), "{heading}");
        assert!(heading.ends_with(title), "{heading}");
        assert_eq!(heading, list.line_with(title));
        for other in WALLET_VAULT_TITLES.iter().filter(|t| **t != title) {
            assert!(
                !one.stdout.contains(other),
                "--note {number} printed {other}"
            );
        }
        assert!(one.stdout.contains(MASK));
        for (_, hidden) in WALLET_VAULT_HIDDEN {
            for value in hidden {
                assert!(
                    !one.stdout.contains(value),
                    "--note {number} printed {value}"
                );
            }
        }
        assert!(
            one.stdout.contains(&format!("--note {number} --reveal")),
            "{}",
            one.stdout
        );
    }
    // The visible fields of the chosen Note are there, and only its own.
    let key = run(
        "note-masked-visible",
        &vault.bytes,
        &[&format!("--note={}", list.number_of(KEY_TITLE))],
        PASSWORD,
    );
    assert_exit(&key, 0);
    assert!(key.field_line("Chain").contains(CHAIN));
    assert!(key.field_line("Address").contains(ADDRESS));
    assert!(key.field_line("Key").contains(MASK));
    assert!(!key.stdout.contains(WALLET_NAME));
    assert!(!key.stdout.contains(SERVICE));
}

#[test]
fn note_with_reveal_reveals_only_that_note() {
    let vault = wallet_vault();
    let list = run("note-reveal-list", &vault.bytes, &[], PASSWORD);
    for (title, shown) in WALLET_VAULT_HIDDEN {
        let number = list.number_of(title);
        let one = run(
            &format!("note-reveal-{number}"),
            &vault.bytes,
            &["--note", &number, "--reveal", "--allow-non-terminal"],
            PASSWORD,
        );
        assert_exit(&one, 0);
        assert_eq!(one.stdout.lines().next().unwrap(), list.line_with(title));
        assert!(!one.stdout.contains(MASK));
        for value in shown {
            assert!(one.stdout.contains(value), "--note {number}: no {value}");
        }
        for (other_title, hidden) in WALLET_VAULT_HIDDEN {
            if other_title == title {
                continue;
            }
            assert!(!one.stdout.contains(other_title));
            for value in hidden {
                assert!(
                    !one.stdout.contains(value),
                    "--note {number} revealed {value}"
                );
            }
        }
    }

    // A Wallet Kind opens with the Recovery Key too.
    let number = list.number_of(SEED_TITLE);
    let seed = run(
        "note-reveal-recovery",
        &vault.bytes,
        &[
            "--recovery-key",
            "--reveal",
            "--allow-non-terminal",
            "--note",
            &number,
        ],
        &vault.recovery_key,
    );
    assert_exit(&seed, 0);
    assert!(seed.field_line("Passphrase").contains(SEED_PASSPHRASE));
    let printed: Vec<&str> = seed
        .stdout
        .split_whitespace()
        .filter(|w| *w == "abandon" || *w == "about")
        .collect();
    assert_eq!(printed, SEED_WORDS.split(' ').collect::<Vec<_>>());
    assert!(!seed.stdout.contains(PRIVATE_KEY));

    // Revealing one Note into a pipe is refused like revealing them all.
    let piped = run(
        "note-reveal-piped",
        &vault.bytes,
        &["--note", &number, "--reveal"],
        PASSWORD,
    );
    assert_exit(&piped, 3);
    assert!(piped.stdout.is_empty());
    assert!(piped.stderr.contains("--allow-non-terminal"));
}

/// The other Notes are not opened: a Wallet Kind field that can't be decrypted spoils a reveal
/// of everything (exit 2), but not the reveal of a Login beside it.
#[test]
fn revealing_one_note_leaves_the_hidden_fields_of_the_others_alone() {
    let vault = wallet_vault();
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
        NOW + 1,
    )
    .unwrap();
    let bytes = vault_format::seal(&vault.unlocked, &doc.to_json()).unwrap();

    // The Login was changed last, so it is Note 1 and the Private Key is Note 2.
    let login = run(
        "note-beside-damage",
        &bytes,
        &["--note", "1", "--reveal", "--allow-non-terminal"],
        PASSWORD,
    );
    assert_exit(&login, 0);
    assert!(login.line_with(LOGIN_TITLE).starts_with("1. Login: "));
    assert!(login.field_line("Password").contains(LOGIN_PASSWORD));
    assert!(!login.stdout.contains(KEY_TITLE));
    assert!(!login.stdout.contains("damaged"));
    assert!(login.stderr.to_lowercase().contains("warning"));
    assert!(!login.stderr.contains("damaged"));

    let key = run(
        "note-is-damaged",
        &bytes,
        &["--note", "2", "--reveal", "--allow-non-terminal"],
        PASSWORD,
    );
    assert_exit(&key, 2);
    assert!(key.line_with(KEY_TITLE).starts_with("2. Private Key: "));
    assert!(key.field_line("Key").contains("damaged"));
    assert!(!key.stdout.contains(LOGIN_TITLE));
    assert!(!key.stdout.contains(LOGIN_PASSWORD));
}

#[test]
fn a_note_number_that_matches_nothing_exits_4_and_prints_no_notes() {
    let vault = wallet_vault();
    for (test, flags) in [
        ("no-such-note", &["--note", "5"][..]),
        ("no-such-note-far", &["--note=4000"]),
        (
            "no-such-note-reveal",
            &["--note", "5", "--reveal", "--allow-non-terminal"],
        ),
    ] {
        let run = run(test, &vault.bytes, flags, PASSWORD);
        assert_exit(&run, 4);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(
            run.stderr.contains("There is no Note"),
            "{test}: {}",
            run.stderr
        );
        // It says which numbers there are, and nothing else about the Notes.
        assert!(run.stderr.contains("1 to 4"), "{test}: {}", run.stderr);
        for title in WALLET_VAULT_TITLES {
            assert!(!run.stderr.contains(title), "{test}");
        }
    }

    // The file is opened before the number can be judged, so a wrong credential comes first.
    let wrong = run(
        "no-such-note-wrong-password",
        &vault.bytes,
        &["--note", "5"],
        WRONG_PASSWORD,
    );
    assert_exit(&wrong, 1);

    let empty = Document::new(NOW);
    let bytes = vault_format::seal(&vault.unlocked, &empty.to_json()).unwrap();
    let none = run("no-such-note-empty", &bytes, &["--note", "1"], PASSWORD);
    assert_exit(&none, 4);
    assert!(none.stdout.is_empty());
    assert!(none.stderr.contains("no Notes"), "{}", none.stderr);
}

// --- The fields added in document schema 2 ---

const KEY_ID: &str = "FAKEKEYID0001";
const API_PURPOSE: &str = "Nightly fake uploads";
const API_URL: &str = "https://console.example.test/keys";
const EXPIRES: &str = "2027-01-31";
const ENV_VAR: &str = "FAKE_DEPLOY_KEY";
const API_SECRET: &str = "fake-api-secret-0000";
const LOGIN_PURPOSE: &str = "Shared with the fake team";
const SEED_ADDRESS: &str = "0xfake-seed-address-0001";
const DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";

/// A Vault whose Notes use every field schema 2 added, sealed into the wallet Vault's keys.
fn schema_2_vault() -> Vec<u8> {
    let vault = wallet_vault();
    let cipher = TestWalletCipher {
        key: &vault.wallet_key,
        vault_id: vault.unlocked.vault_id(),
    };
    let mut doc = Document::new(NOW);
    doc.create(
        new_note(
            Kind::ApiKey,
            API_TITLE,
            &[
                (field::SERVICE, SERVICE),
                (field::KEY_ID, KEY_ID),
                (field::PURPOSE, API_PURPOSE),
                (field::URL, API_URL),
                (field::EXPIRES, EXPIRES),
                (field::ENV_VAR, ENV_VAR),
            ],
            &[(field::KEY, API_KEY), (field::SECRET, API_SECRET)],
        ),
        None,
        NOW,
    )
    .unwrap();
    doc.create(
        new_note(
            Kind::Login,
            LOGIN_TITLE,
            &[
                (field::WEBSITE, WEBSITE),
                (field::USERNAME, USERNAME),
                (field::PURPOSE, LOGIN_PURPOSE),
            ],
            &[(field::PASSWORD, LOGIN_PASSWORD)],
        ),
        None,
        NOW,
    )
    .unwrap();
    doc.create(
        new_note(
            Kind::SeedPhrase,
            SEED_TITLE,
            &[
                (field::WALLET_NAME, WALLET_NAME),
                (field::ADDRESS, SEED_ADDRESS),
                (field::DERIVATION_PATH, DERIVATION_PATH),
            ],
            &[(field::WORDS, SEED_WORDS)],
        ),
        Some(&cipher),
        NOW,
    )
    .unwrap();
    assert_eq!(doc.schema_version(), 2);
    vault_format::seal(&vault.unlocked, &doc.to_json()).unwrap()
}

#[test]
fn a_schema_2_vault_is_read_and_its_new_fields_are_printed_under_their_names() {
    let bytes = schema_2_vault();
    let masked = run("schema-2-masked", &bytes, &[], PASSWORD);
    assert_exit(&masked, 0);
    for (label, value) in [
        ("Key ID", KEY_ID),
        ("Managed at", API_URL),
        ("Expires", EXPIRES),
        ("Environment variable", ENV_VAR),
        ("Derivation path", DERIVATION_PATH),
        ("Address", SEED_ADDRESS),
    ] {
        assert_eq!(
            masked.field_line(label).trim(),
            format!("{label}: {value}"),
            "{label}"
        );
    }
    // Both Notes with a purpose print it.
    for purpose in [API_PURPOSE, LOGIN_PURPOSE] {
        assert_eq!(
            masked.line_with(purpose).trim(),
            format!("Purpose: {purpose}")
        );
    }
    // The second Hidden Field of an API Key is hidden like the first.
    assert!(masked.field_line("Secret").contains(MASK));
    assert!(!masked.stdout.contains(API_SECRET));
    assert!(!masked.stdout.contains(API_KEY));

    let number = masked.number_of(API_TITLE);
    let revealed = run(
        "schema-2-reveal",
        &bytes,
        &["--note", &number, "--reveal", "--allow-non-terminal"],
        PASSWORD,
    );
    assert_exit(&revealed, 0);
    assert_eq!(revealed.field_line("Key").trim(), format!("Key: {API_KEY}"));
    assert_eq!(
        revealed.field_line("Secret").trim(),
        format!("Secret: {API_SECRET}")
    );
    assert!(!revealed.stdout.contains(LOGIN_PASSWORD));
}

#[test]
fn a_vault_from_a_newer_document_schema_is_called_newer_not_damaged() {
    let body = br#"{"schema_version":3,"changed_at":1790000000,"change_counter":0,"notes":[]}"#;
    let bytes = vault_format::seal(&wallet_vault().unlocked, body).unwrap();
    for (test, flags) in [("schema-3", &[][..]), ("schema-3-note", &["--note", "1"])] {
        let run = run(test, &bytes, flags, PASSWORD);
        assert_exit(&run, 2);
        assert!(run.stdout.is_empty(), "{test}");
        assert!(run.stderr.contains("schema 3"), "{test}: {}", run.stderr);
        assert!(run.stderr.contains("newer"), "{test}: {}", run.stderr);
        assert!(!run.stderr.contains("damaged"), "{test}: {}", run.stderr);
    }
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

#[test]
fn stdin_credentials_are_bounded_and_never_printed() {
    for (length, code) in [(4096, 1), (4097, 3)] {
        for (suffix, ending) in [("eof", ""), ("lf", "\n"), ("crlf", "\r\n")] {
            let fake = "x".repeat(length);
            let run = run_with_stdin(
                &format!("credential-bound-{length}-{suffix}"),
                Some(&text_and_login().bytes),
                &["test.vault", "--credential-stdin"],
                format!("{fake}{ending}").as_bytes(),
            );
            assert_exit(&run, code);
            assert!(run.stdout.is_empty());
            assert!(!run.stderr.contains(&fake));
            if code == 3 {
                assert!(run.stderr.contains("longer than 4096"));
            }
        }
    }
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

    let run = run("wallet-damaged", &bytes, &REVEAL, PASSWORD);
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
fn sparse_files_at_the_size_boundary_are_checked_before_credentials() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("enote-reader-size-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fake-sparse.vault");
    let file = std::fs::File::create(&path).unwrap();
    for (length, code, message) in [
        (64 * 1024 * 1024, 2, "damaged"),
        (64 * 1024 * 1024 + 1, 3, "larger than 64 MiB"),
    ] {
        file.set_len(length).unwrap();
        let output = reader(&dir)
            .args(["fake-sparse.vault", "--credential-stdin"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr).unwrap().contains(message));
        assert_eq!(file.metadata().unwrap().len(), length);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }
    drop(file);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_directory_is_rejected_before_credentials() {
    let run = run_with_stdin("directory-input", None, &[".", "--credential-stdin"], b"");
    assert_exit(&run, 3);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("not an ordinary file"));
}

#[cfg(unix)]
#[test]
fn a_special_file_is_rejected_before_credentials() {
    let run = run_with_stdin(
        "device-input",
        None,
        &["/dev/null", "--credential-stdin"],
        b"",
    );
    assert_exit(&run, 3);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("not an ordinary file"));
}

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
    let cases: [(&str, &[&str]); 8] = [
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
        // --note needs a number from 1 up, once. These stop before the file is opened.
        (
            "note-without-number",
            &["test.vault", "--credential-stdin", "--note"],
        ),
        (
            "note-zero",
            &["test.vault", "--credential-stdin", "--note", "0"],
        ),
        (
            "note-not-a-number",
            &["test.vault", "--credential-stdin", "--note", "two"],
        ),
        (
            "note-twice",
            &[
                "test.vault",
                "--credential-stdin",
                "--note",
                "1",
                "--note=2",
            ],
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
    assert!(run.stdout.contains("--allow-non-terminal"));
    assert!(run.stdout.contains("--recovery-key"));
    assert!(run.stdout.contains("--note <number>"));
    // Both ways of writing it that the reader accepts are shown.
    assert!(run.stdout.contains("--note=<number>"));
    // Every exit code is listed, the one for a number no Note has included.
    assert!(run.stdout.contains("4 no Note has the number"));
}
