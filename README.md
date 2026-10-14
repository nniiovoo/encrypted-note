# encrypted-note

encrypted-note is a local-only desktop app for storing Seed Phrases, Private Keys, Logins, API Keys, and private text notes in one encrypted Vault on macOS and Windows.

- **Local by design.** Your Vault stays on your computer. The app has no network code, accounts, syncing, updater, telemetry, or analytics.
- **Recovery you control.** Choose your Master Password or use the app's six-word suggestion. Write down the Recovery Key shown once during setup and keep it in a paper Recovery Kit.
- **Private by default.** Hidden Fields stay concealed until you choose Show and hide again when you open another Note. Logins, API Keys, and Text use a 30-second reveal. Seed Phrases and Private Keys require the Master Password for viewing or editing and offer hold-to-show or a 20-second timed reveal.
- **Screen-sharing safeguards.** Capture Hiding is best-effort and asks the OS to exclude the app window. The Sharing Guard covers the window when known screen-sharing or recording apps are detected.
- **Portable backups.** The Vault is one encrypted file. Create a Backup to open it on another computer; transfers are manual and encrypted-note does not sync data.

Vault keys and decrypted Notes remain in the Rust core. The WebView receives only the Hidden Field you choose to reveal.

> **Status: v1, personal project, not independently audited.** Keep an offline paper or metal backup of any Seed Phrase that holds real funds. This app is a convenient extra copy, not your only one.

## Product demo

[![Recorded macOS walkthrough: find a Note, reveal a fictional password, and hide it again.](docs/assets/product-demo.gif)](docs/assets/product-demo.mp4)

[Watch or download the full demo](docs/assets/product-demo.mp4) · 36 seconds · English captions

Recorded on macOS with fictional sample data. Capture Hiding was disabled for this development recording. [Transcript and recording notes](docs/assets/product-demo.md).

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

## Keyboard shortcuts

Cmd on Mac, Ctrl on Windows: **Cmd/Ctrl+L** locks · **Cmd/Ctrl+F** searches (then arrow keys move through the list, Enter opens, Esc clears) · **Cmd/Ctrl+N** starts a new Note (then **1** to **5** pick its Kind) · **Cmd/Ctrl+S** or **Cmd/Ctrl+Enter** saves · **Cmd/Ctrl+Shift+C** copies the open Note's first Hidden Field.

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

## If the app won't open

Your Notes are still in the Vault file, and the Emergency Reader reads it without the app.

**Where the files are.** One folder holds `vault.enote` (the Vault) and a `safety` folder (the Safety Copies, each a complete Vault). While the app still runs, Settings → Backups shows that folder.

| Computer | Vault folder |
|---|---|
| Mac, the built app | `~/Library/Containers/com.encryptednote.app/Data/Library/Application Support/com.encryptednote.app/Vault.noindex` |
| Mac, a build that isn't sandboxed (such as `npm run tauri dev`) | `~/Library/Application Support/com.encryptednote.app/Vault.noindex` |
| Windows | `%LOCALAPPDATA%\com.encryptednote.app\Vault.noindex` |

The built Mac app is sandboxed, so its folder is inside the app's container, and macOS may ask before Terminal is allowed to read it.

**Read them with the Emergency Reader.** It works on a Vault, a Backup or a Safety Copy. It asks for the Master Password without showing what you type (add `--recovery-key` to use the Recovery Key instead), prints to the terminal and writes nothing to disk.

```bash
cargo run -p enote-reader -- <file>                    # list every Note with a number; Hidden Fields stay as dots
cargo run -p enote-reader -- <file> --note 3 --reveal  # only Note 3, with its Hidden Fields
cargo run -p enote-reader -- <file> --reveal           # every Note, with its Hidden Fields
```

Put the path in quotes: the Mac one has a space in it. `--reveal` prints only to a terminal, so nothing lands in a file by accident.

## Tested so far

- **Automated:** 491 tests, including tamper tests over every byte of a Vault file and the RFC 9106 and XChaCha20-Poly1305 reference vectors.
- **By hand on macOS 26:** the main flows, clipboard privacy markers, the Sharing Guard with real apps, and a Backup read back by the Emergency Reader. Screen-capture results so far: [docs/capture-test.md](docs/capture-test.md).
- **Windows:** the adapters compile but have **not yet been run on a real Windows PC**.

## Credits

- The passphrase generator uses the [EFF Long Wordlist](https://www.eff.org/dice) by the Electronic Frontier Foundation, licensed [CC BY 3.0 US](https://creativecommons.org/licenses/by/3.0/us/).
- Seed Phrase checks use the BIP39 English word list via the [`bip39`](https://crates.io/crates/bip39) crate.

## License

[MIT](LICENSE)
