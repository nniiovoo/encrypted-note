# encrypted-note

A calm, local-only desktop vault for **Seed Phrases, Private Keys, Logins, API Keys and private Text**, for Mac and Windows.

- **Stays on your computer.** One encrypted file. The app has no network code at all: no sync, no account, no updater, no telemetry.
- **Beginner-friendly.** A suggested 6-word Master Password, a Recovery Kit you write on paper, plain-English messages.
- **Hidden until you choose.** Secret parts stay as dots until you click Show and hide again after 30 s. Seed Phrases and Private Keys need your Master Password again and show only while you hold the button.
- **Screen-share aware.** The window asks the OS to keep it out of screenshots and screen shares, and a *Sharing Guard* covers the whole window while apps like Zoom, Teams, Discord, OBS or QuickTime are running.

> **Status: v1, personal project, not independently audited.** Keep an offline paper or metal backup of any Seed Phrase that holds real funds. This app is a convenient extra copy, not your only one.

## What it protects against, and what it can't

| Protects | Doesn't protect |
|---|---|
| A stolen laptop or USB stick (Argon2id + XChaCha20-Poly1305, strong password required) | Malware already running on your computer (keyloggers, memory readers) |
| Someone at your unlocked computer (Auto-lock, re-entering the password for wallets, hold-to-show) | A phone camera pointed at your screen |
| Accidentally showing secrets while screen sharing (layered, see below) | A hacked operating system, or being forced to unlock |
| Clipboard history and Universal/cloud clipboard (private clipboard, cleared after 30 s) | Old copies of your Vault: they still open with the old password |

**Screen-capture hiding is best-effort on Mac.** Apple offers no API that guarantees it. Windows' exclusion is much stronger but still not a security boundary. That's why the app layers it with hidden-by-default fields and the Sharing Guard. See [ADR-0002](docs/adr/0002-screen-capture-hiding-is-layered-and-best-effort-on-mac.md).

## How it works

- **Tauri v2.** React/TypeScript screens run in an untrusted webview. A Rust core holds every key and decrypted Note, and hands the screens only the one field you're showing ([ADR-0003](docs/adr/0003-tauri-with-secrets-kept-in-rust.md)).
- **Encryption.** Argon2id is calibrated on your machine (≥ 64 MiB). A random Vault Key and a separate Wallet Key are wrapped by your Master Password and by your Recovery Key ([ADR-0004](docs/adr/0004-wallet-kinds-under-a-separate-key.md), [ADR-0005](docs/adr/0005-password-change-rewraps-key-rotation-is-separate.md)). The file format is documented byte by byte in [docs/FORMAT.md](docs/FORMAT.md).
- **Backups.** A Backup is just a copy of the Vault file you save wherever you like (e.g. an exFAT USB stick) and open on your other computer. There's no sync by design ([ADR-0001](docs/adr/0001-local-only-vault-moved-by-hand.md)).
- **Emergency Reader.** `crates/reader` is a small command-line tool that reads a Vault or Backup even if the app stops working.
- **Vocabulary.** [CONTEXT.md](CONTEXT.md) defines Vault, Note, Kind, Hidden Field, Recovery Kit and the rest.

## Build and run

Requirements: Rust 1.89+, Node 24+, and on Mac Xcode's command-line tools.

```bash
npm ci --ignore-scripts
npm run tauri dev                   # development
npx tauri build                     # release app bundle
cargo run -p enote-reader -- <file> # Emergency Reader
./scripts/check.sh                  # tests, clippy, cargo-deny (network ban), TypeScript
```

For experiments, debug builds read `ENOTE_DEV_DATA_DIR=/some/folder` so your real Vault is never touched. `ENOTE_DEV_ALLOW_CAPTURE=1` turns screen-capture hiding off, for screenshots while developing. Release builds ignore both.

## Tested so far

- **Automated:** 320 tests, including tamper tests over every byte of a Vault file and the RFC 9106 and XChaCha20-Poly1305 reference vectors.
- **By hand on macOS 26:** the main flows, clipboard privacy markers, the Sharing Guard with real apps, and a Backup read back by the Emergency Reader.
- **Windows:** the adapters compile but have **not yet been run on a real Windows PC**.

## Credits

- The passphrase generator uses the [EFF Long Wordlist](https://www.eff.org/dice) by the Electronic Frontier Foundation, licensed [CC BY 3.0 US](https://creativecommons.org/licenses/by/3.0/us/).
- Seed Phrase checks use the BIP39 English word list via the [`bip39`](https://crates.io/crates/bip39) crate.

## License

[MIT](LICENSE)
