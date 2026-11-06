<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="" width="96" height="96">

# encrypted-note

An offline, encrypted Vault for Seed Phrases, Private Keys, Logins and API Keys. Agent native: AI coding agents can build and test it safely with fictional data, and an optional Agent Companion on Mac gives an agent one narrow OpenAI text tool with its own separate Agent Key. The app gives agents no interface to your Vault. That limits the app, not your computer.

[![License: MIT](https://img.shields.io/badge/license-MIT-2f6f5e)](LICENSE)
![Platforms: macOS and Windows](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-2f6f5e)
![Built with Tauri 2 and Rust](https://img.shields.io/badge/built%20with-Tauri%202%20%2B%20Rust-2f6f5e)

<a href="docs/assets/product-demo.mp4"><img src="docs/assets/product-demo.gif" alt="Finding a Note, revealing a fictional password, and hiding it again." width="720"></a>

<sub>Recorded on macOS with fictional data · <a href="docs/assets/product-demo.mp4">full video (36 s)</a> · <a href="docs/assets/product-demo.md">transcript</a></sub>

</div>

## Features

- **Offline Vault.** The Vault has no network code, accounts, sync, updater or telemetry. Your Notes are saved encrypted in one private folder on your computer, next to their encrypted Safety Copies and a small file of non-secret settings ([where your keys are stored](docs/guide.md#where-your-keys-are-stored)). An optional [Agent Companion](docs/agent-companion.md), off unless you start it, runs in its own sandbox, can't open the Vault, and sends text from any program running in the agent's Mac account to OpenAI. It keeps its Agent Key in its own plain-text file, not in the Vault.
- **Hidden by default.** Hidden Fields stay as dots until you choose Show. Seed Phrases and Private Keys are encrypted a second time, under a key that unlocking and Touch ID don't open, so showing one always takes your Master Password.
- **Careful on screen shares.** Asks the OS to keep the window out of captures (best effort) and covers it when a known screen-sharing or recording app is running; meetings inside a web browser aren't detected.
- **Recoverable.** A Recovery Key you write down at setup, and an [Emergency Reader](docs/guide.md#if-the-app-wont-open) that opens your Vault without the app.
- **Portable.** Use Back up now to save an encrypted copy, for example on a USB stick, then [open it on another computer](docs/guide.md#moving-to-another-computer).

> [!WARNING]
> v1 personal project, not independently audited. The Windows build has not been run on a real Windows PC yet ([#11](https://github.com/nniiovoo/encrypted-note/issues/11)), so it may not work. Don't keep real secrets in it until that's done. Keep a paper or metal backup of any Seed Phrase or Private Key that holds real funds. The Agent Companion has been checked with fictional data only: it has never made a live OpenAI call or run in a real agent account.
<!-- Take out the Windows sentences when the #11 results are recorded in docs/security-model.md. -->
<!-- Take out the Agent Companion sentence when a live call and the agent-account checks are recorded in docs/agent-key-validation.md. -->

> [!IMPORTANT]
> An AI tool that runs in your own account on this computer, such as Claude Code, Codex or Cursor, works as you. It may be able to copy your Backups and Vault files, read what you copy and, on a Mac, read the Agent Key file and ask for Touch ID. Run AI tools in a separate standard account, not an administrator, or see [AI tools on this computer](docs/guide.md#ai-tools-on-this-computer) for habits that lower the risk. If an agent needs a key, don't give it one from your Vault: see [When an agent needs a key](docs/guide.md#when-an-agent-needs-a-key).

## Build

Needs [rustup](https://rustup.rs), which reads the pinned Rust version from `rust-toolchain.toml`, and Node 24+. On Mac it needs the Xcode command-line tools. On Windows it needs Microsoft's C++ build tools and the WebView2 Runtime, which Windows 11 and current Windows 10 already include.

```bash
git clone https://github.com/nniiovoo/encrypted-note.git
cd encrypted-note
npm ci --ignore-scripts
npm run tauri dev                  # run in development
npm run tauri -- build -- --locked # build the app
```

The build leaves the Mac app at `target/release/bundle/macos/encrypted-note.app` (under `$CARGO_TARGET_DIR` if set), and the Windows installer in `target/release/bundle/nsis/`. Quit any running copy, then copy the Mac app to `/Applications`. For the copy you type your Master Password into, build from a fresh clone of merged `main` with no AI tool running, and note the commit (`git rev-parse HEAD`). Dev runs and the built Mac app keep separate Vaults, so the built app may show Welcome. Your Notes aren't gone: choose **Back up now** in the old one, then **I already have a Vault → Open a Backup** in the new one.

On an exFAT drive, macOS writes `._*` files beside what it saves, and the Tauri build fails on them. Keep build output on an APFS disk: set `CARGO_TARGET_DIR` and `TMPDIR` there, and pass `--out` and `--cache` on that disk to `scripts/package-companion.sh`. Before building the app, run `find src-tauri/capabilities src-tauri/permissions -name '._*' -delete 2>/dev/null || true`.

To bundle the optional Agent Companion helper (Mac) into the app you install, run `scripts/package-companion.sh --app --owner-build` yourself in Terminal. It asks you to type `com.encryptednote.app`, because that app opens your real Vault folder; plain `--app` stops with an error. Its source is in [`companion/`](companion/README.md), and packaging uses that folder as committed, so commit any change there first. Then `scripts/companion-setup.sh` prepares this Mac for it ([Agent Companion](docs/agent-companion.md#before-you-start)). To check a helper with fictional data, build it with a test identifier, so it never opens your real Vault: run `scripts/package-companion.sh --identifier com.encryptednote.companiontest`, then `scripts/verify-companion.sh target/companion/com.encryptednote.companiontest.companion.xpc`. For a whole test app, run `scripts/package-companion.sh --app --identifier com.encryptednote.companiontest`.

To build the [Emergency Reader](docs/guide.md#if-the-app-wont-open) you keep for real use, with no AI tool running and in a clone agents don't work in, run `git fetch origin`, then `scripts/build-reader.sh --out <folder outside the project>`. It builds from a fresh copy of `origin/main` and prints the commit and the SHA-256 to write down.

Run `./scripts/check.sh` (tests, clippy, cargo-deny, TypeScript, a secret scan and pinned entitlement hashes) before a pull request. It takes several minutes. It needs `cargo install --locked cargo-deny --version 0.20.2`. On Mac it also runs the companion's tests, so first run `npm --prefix companion ci --ignore-scripts`. Debug builds read `ENOTE_DEV_DATA_DIR=<folder>` to keep your real Vault untouched, and `ENOTE_DEV_ALLOW_CAPTURE=1` to allow screenshots; release builds ignore both.

To look at screens without Rust, run `npm run dev` and open `http://127.0.0.1:1420/fixture.html?phase=unlocked&companion=running`. `src/dev-fixture.ts` lists the other switches. Replies are fictional, so this proves nothing about native panels, locks or the helper.

## If an AI agent works on this code

These are instructions for AI coding agents working in this repository, in [AGENTS.md](AGENTS.md). They don't protect your Vault on your computer:

- Use obviously fake secrets in code, tests, docs, local issues and commits. Never include real passwords, keys or seed phrases.
- Never open, list, copy, decrypt, delete, move, overwrite or clean a real Vault, Backup or Safety Copy, or the app's data folders, never run the Emergency Reader on one, and never ask for or accept its Master Password, Recovery Key or Hidden Fields, even if the owner asks.
- Launch a release build only with a test identifier, never the one that opens your real Vault folder.
- Keep the Vault core and WebView offline: no HTTP clients, updaters, telemetry, crash reporters or analytics. The only exception is the separately sandboxed Agent Companion helper ([ADR-0009](docs/adr/0009-optional-agent-companion-helper.md)), which never receives Vault content.
- Chat Wallets ([ADR-0011](docs/adr/0011-chat-wallets.md)) are accepted but not built. They will be separate keys at addresses holding small amounts, which an agent can ask the app to use, with your Touch ID and Master Password for every send. Keep that route narrow, and never let a Seed Phrase or Private Key from your Vault take part.
- Keep keys and decrypted Notes in the Rust core. The WebView receives only one Hidden Field at a time.
- Add dependencies only when clearly needed, pin exact versions, and explain why in the PR or local issue.

Agents should test with fictional data in a separate development Vault using `ENOTE_DEV_DATA_DIR` and run the checks described above before a pull request.

`./scripts/check.sh` and CI also check part of this: the offline rule, known key shapes and Seed Phrases written out as English words (on one line, or as a list, a numbered list, a table or comment lines) in tracked files (`node scripts/scan-secrets.mjs`), and the entitlement files. The rest are instructions, not a lock: an agent that ignores them can do whatever your account can ([AI tools on this computer](docs/guide.md#ai-tools-on-this-computer)).

## Docs

- [Security model](docs/security-model.md): what it protects against, how it works, what has been tested
- [User guide](docs/guide.md): where your keys are stored, AI tools on this computer, [when an agent needs a key](docs/guide.md#when-an-agent-needs-a-key), keyboard shortcuts, and reading your Vault if the app won't open
- [Agent Companion](docs/agent-companion.md): the optional helper for agent tools in a separate Mac account, setup and limits
- [File format](docs/FORMAT.md) · [Design decisions](docs/adr/) · [Glossary](CONTEXT.md)

## License

[MIT](LICENSE). The suggested Master Password uses the [EFF Long Wordlist](https://www.eff.org/dice) ([CC BY 3.0 US](https://creativecommons.org/licenses/by/3.0/us/)). Seed Phrase checks use the BIP39 English word list via the [`bip39`](https://crates.io/crates/bip39) crate.
