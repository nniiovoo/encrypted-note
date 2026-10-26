# Security model

> **Status: v1, personal project, not independently audited.** Keep an offline paper or metal backup of any Seed Phrase that holds real funds. This app is a convenient extra copy, not your only one.

## What it protects against, and what it can't

| Protects | Doesn't protect |
|---|---|
| A stolen laptop or USB stick (Argon2id + XChaCha20-Poly1305, strong password required) | Malware already running on your computer (keyloggers, memory readers) |
| Someone at your unlocked computer (Auto-lock, re-entering the password for wallets, hold-to-show) | A phone camera pointed at your screen |
| Accidentally showing secrets while screen sharing (layered, see below) | A hacked operating system, or being forced to unlock |
| Clipboard history and Universal/cloud clipboard (private clipboard, cleared after 30 s) | Old copies of your Vault: they still open with the old password |

**Screen-capture hiding is best-effort on Mac.** Apple offers no API that guarantees it. Windows' exclusion is much stronger but still not a security boundary. That's why the app layers it with hidden-by-default fields and the Sharing Guard, which covers the window when known screen-sharing or recording apps are detected. See [ADR-0002](adr/0002-screen-capture-hiding-is-layered-and-best-effort-on-mac.md).

## How it works

- **Tauri v2.** React/TypeScript screens run in an untrusted webview. A Rust core holds every key and decrypted Note, and hands the screens only the one field you're showing ([ADR-0003](adr/0003-tauri-with-secrets-kept-in-rust.md)).
- **Encryption.** Argon2id is calibrated on your machine (≥ 64 MiB). A random Vault Key and a separate Wallet Key are wrapped by your Master Password and by your Recovery Key ([ADR-0004](adr/0004-wallet-kinds-under-a-separate-key.md), [ADR-0005](adr/0005-password-change-rewraps-key-rotation-is-separate.md)). The file format is documented byte by byte in [FORMAT.md](FORMAT.md).
- **Reveal limits.** Logins, API Keys and Text use a 30-second reveal. Seed Phrases and Private Keys need the Master Password to view or edit, with hold-to-show or a 20-second timed reveal. The limits the owner can stretch, and the ones that stay fixed, are in [ADR-0006](adr/0006-everyday-key-use-limits-the-owner-can-stretch.md).
- **Backups.** A Backup is just a copy of the Vault file you save wherever you like (e.g. an exFAT USB stick) and open on your other computer. There's no sync by design ([ADR-0001](adr/0001-local-only-vault-moved-by-hand.md)).
- **Emergency Reader.** `crates/reader` is a small command-line tool that reads a Vault or Backup even if the app stops working ([how to use it](guide.md#if-the-app-wont-open)).
- **Optional Agent Companion.** A separate helper (an XPC service with its own App Sandbox container) can let one approved agent use one fixed OpenAI text tool with a separately created Agent Key. It can't open the Vault's container, receives nothing from the Vault, and stops on Stop, lock, sleep, grant expiry and quit. Its Agent Key is kept in an owner-only plain-text file and Node.js memory, not encrypted Vault storage ([ADR-0009](adr/0009-optional-agent-companion-helper.md), [guide](agent-companion.md)).
- **Vocabulary.** [CONTEXT.md](../CONTEXT.md) defines Vault, Note, Kind, Hidden Field, Recovery Kit and the rest.

## Tested so far

- **Automated:** `./scripts/check.sh` runs the test suite, including tamper tests over every byte of a Vault file and the RFC 9106 and XChaCha20-Poly1305 reference vectors.
- **By hand on macOS 26:** the main flows, clipboard privacy markers, the Sharing Guard with real apps, and a Backup read back by the Emergency Reader. Screen-capture results so far: [capture-test.md](capture-test.md).
- **Windows:** the adapters compile but have **not yet been run on a real Windows PC**.
- **Agent Companion:** checked with fictional data only, including that the packaged helper can't read a fake Vault canary in the app's container ([results](agent-key-validation.md#integrated-helper-2026-10-26)). No real key, live request or ChatGPT connection yet; it's unavailable on Windows.
