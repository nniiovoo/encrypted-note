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

- **Tauri v2.** React/TypeScript screens run in an untrusted WebView. Rust/native code owns keys and decrypted Notes. Ordinary Show returns one authorized Hidden Field; stored Wallet readouts remain native ([ADR-0003](adr/0003-tauri-with-secrets-kept-in-rust.md)). New/edited Hidden Field inputs still pass through JavaScript, so a compromised renderer can observe them.
- **Native credential entry.** Actual Master Password and Recovery Key entry, password suggestion/strength/confirmation and Recovery Kit display stay in native controls. Rust binds the exact action and checks cancellation, source state and expiry before committing. No credential or reusable approval is returned through IPC. Native/OS copies and the stack remain best effort; a fake renderer prompt or compromised native process remains a risk. See [native custody](native-credential-design.md).
- **Encryption.** Argon2id is calibrated on your machine (≥ 64 MiB). A random Vault Key and a separate Wallet Key are wrapped by your Master Password and by your Recovery Key ([ADR-0004](adr/0004-wallet-kinds-under-a-separate-key.md), [ADR-0005](adr/0005-password-change-rewraps-key-rotation-is-separate.md)). The file format is documented byte by byte in [FORMAT.md](FORMAT.md).
- **Reveal limits.** Logins, API Keys and Text use a 30-second reveal. Seed Phrases and Private Keys need the Master Password to view or edit, with hold-to-show or a 20-second timed reveal. The limits the owner can stretch, and the ones that stay fixed, are in [ADR-0006](adr/0006-everyday-key-use-limits-the-owner-can-stretch.md).
- **Quick Unlock on Mac.** The source implementation uses a biometric-gated Secure Enclave key to wrap only the Vault Key in an encrypted private local file. No account, server, Apple Developer profile or persistent Keychain item is part of this route. Native Master Password verification is required after app restart and every seven days; Touch ID does not extend that deadline, release a Wallet Key or erase password backoff. This file uses filesystem access controls rather than Keychain app access-group isolation: another process with the same user's file access may copy it and request its own protected operation outside the app's process policy. Deleting the current file does not erase backup copies. [Custody details and physical acceptance](quick-unlock-design.md).
- **Backups.** A Backup is just a copy of the Vault file you save wherever you like (e.g. an exFAT USB stick) and open on your other computer. There's no sync by design ([ADR-0001](adr/0001-local-only-vault-moved-by-hand.md)).
- **Emergency Reader.** `crates/reader` is a small command-line tool that reads a Vault or Backup even if the app stops working ([how to use it](guide.md#if-the-app-wont-open)).
- **Vocabulary.** [CONTEXT.md](../CONTEXT.md) defines Vault, Note, Kind, Hidden Field, Recovery Kit and the rest.

## Tested so far

- **Automated:** `./scripts/check.sh` runs the test suite, including tamper tests over every byte of a Vault file and the RFC 9106 and XChaCha20-Poly1305 reference vectors.
- **Earlier UI, by hand on macOS 26:** the main flows, clipboard privacy markers, the Sharing Guard with real apps, and a Backup read back by the Emergency Reader. Those records do not validate the new native credential/readout controls or local Touch ID custody. Screen-capture evidence and remaining matrix: [capture-test.md](capture-test.md).
- **New native flows:** fake-data core/coordinator, custody/lifecycle and private-file checks exist; build/test reports record their execution separately. Actual Touch ID enrollment/unlock, biometric ACL behavior, restart/sleep/switch races, native accessibility and first-frame capture acceptance remain pending.
- **Windows:** native runtime behavior has **not yet been tested on a real Windows PC**. Compilation checks cannot establish it; Windows Hello remains later.
