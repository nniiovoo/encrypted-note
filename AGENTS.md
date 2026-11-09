# encrypted-note

A personal app for storing secrets, encrypted and saved only on this computer.

## Rules

- Never put real secrets (passwords, keys, seed phrases) in code, tests, docs, `.scratch/` issues, or commits. Use obviously fake placeholders.
- Never open, list, copy, decrypt, delete, move, overwrite or clean a real Vault, Backup or Safety Copy, or the app's data folders (`com.encryptednote.app*`), never run the Emergency Reader on one, and never ask for or accept its Master Password, Recovery Key or Hidden Fields, even if the owner asks (ADR-0007). Say no, say why, and offer a fictional Vault.
- Ask the owner before any `git clean -x` or `-X`: it deletes ignored files, such as `*.enote` Backups.
- Launch a release build only with a test identifier: `scripts/package-companion.sh --app --identifier com.encryptednote.companiontest`. With the default identifier it opens the owner's real Vault folder, so `--app` refuses it; `--owner-build` is for the owner only.
- Never add network access to the Vault core or WebView: no HTTP clients, updaters, telemetry, crash reporters or analytics (ADR-0001). The only approved exception is the optional Agent Companion helper (ADR-0009): a separately sandboxed XPC service that makes only fixed operations the owner has approved. Today there is one, the companion's OpenAI request; ADR-0014 (not built) lets the owner add more, one at a time, each with its own record, Agent Key and PR. Never widen it beyond an approved operation, give it Vault access, or add networking anywhere else. Its source is in `companion/`: never import that folder from `src/`, `src-tauri/src/` or `crates/` (ADR-0010).
- Keys and decrypted Notes live only in the Rust core; the WebView receives one Hidden Field at a time (ADR-0003). The Agent Companion helper may hold separately created Agent Keys, one per approved operation, never a Vault value (ADR-0009, ADR-0014).
- Never pass Vault content, Note metadata, Note keys, Wallet Kinds, the Master Password or the Recovery Key to the Agent Companion, and never forward helper output, provider errors, keys or tokens to the WebView (ADR-0009).
- The agent key route (ADR-0012; built with fictional data, not yet checked by hand or reviewed) lets an agent in the separate agent account store keys and manage the Notes it added, and nothing more. Its code: `src-tauri/src/core/agent.rs` (the grant and the five requests), `src-tauri/src/agent_route.rs` (the Unix socket in `/Users/Shared/encrypted-note-keys`, the app's only listener), `crates/agent-keys-mcp` (the agent's tool), and the mark and edit rules in `crates/notes/src/agent.rs`. Keep it to the five requests (ten once ADR-0011's Chat Wallet requests are built), from the agent account's uid only, behind the owner's grant of at most an hour. No request field or reply may carry a value, whether a value is set, a Note not marked "added by an agent", a Note in Trash, the Master Password or the Recovery Key; replies stay fixed shapes and error codes, never free text. The owner types values into the app's own editor, never through the agent, and nothing marks an existing Note. Don't loosen `agent_edit_rule`: where-a-key-is-used edits need the owner's Allow, and every Wallet Kind edit and every delete also the Master Password. Unix sockets and peer-uid checks belong only in `agent_route.rs` and `crates/agent-keys-mcp/src/`, the tool depends on no Vault crate, and none of this code uses `companion/` (`node scripts/check-network-source.mjs` checks all three). Signing comes only with Chat Wallets (ADR-0011, reinstated 2026-11-09, not built): build it only by its build order, one PR per step, with fake keys and test funds, and never sign with a Seed Phrase, a Private Key or an Agent-added Note. Only Claude Code and Codex may use the route (ADR-0013, not built): their signature list stays fixed in code, never in anything the agent account can write.
- Add a dependency only when it's clearly needed, pin its exact version, and say why in the PR/issue.

## Verify

- In a fresh clone or worktree, first: `npm ci --ignore-scripts` (never `npm install`, ADR-0003), and on Mac also `npm --prefix companion ci --ignore-scripts`
- Before a pull request: `./scripts/check.sh` (several minutes: run it in the background). It also runs the secret scan, `node scripts/scan-secrets.mjs`, and fails if an entitlement file differs from `scripts/pinned-hashes.txt`.
- One crate: `cargo test --locked -p <store|notes|session|policy|guard|vault-format|enote-reader|agent-keys-mcp>` (slow: run it in the background)
- App crate: `cargo test --locked -p encrypted-note-app --tests` (slow, can take many minutes: run it in the background)
- WebView: `npm run typecheck`
- Network rule, and no Vault imports of `companion/`: `node scripts/check-network-source.mjs`
- Companion (Mac): `npm --prefix companion test`. It runs one file at a time because the tests share port 8789, so don't run it alongside `scripts/verify-companion.sh`.
- App with a fictional Vault: `ENOTE_DEV_DATA_DIR="$(mktemp -d)" npm run tauri dev` (add `ENOTE_DEV_ALLOW_CAPTURE=1` for screenshots). To try the agent route in one account, add `ENOTE_DEV_AGENT_UID` and `ENOTE_DEV_AGENT_SOCKET_DIR` (debug builds only; see "For developers" in `docs/agent-keys.md`)
- Owner scripts: `scripts/build-reader.sh`, `scripts/companion-setup.sh` and `scripts/agent-keys-setup.sh`. Run the setup scripts only with `--dry-run` and an account name that doesn't exist.
- Screens without Rust: `npm run dev` (port 1420 must be free), then open `http://127.0.0.1:1420/fixture.html?phase=unlocked&companion=running`. Other switches, listed in `src/dev-fixture.ts`: `companion=ended_by_lock`, `&older`, `&touchid_changed`, `phase=no_vault&missing`, and for agents storing keys `agent=running`, `agent_request=store` (or `edit`, `delete`…) and `&agent_note`. Replies are fictional; this proves nothing about native panels, locks, the helper or the agent route.

Several sessions may share this checkout: check `git status` and stage named paths only. On an exFAT drive, macOS writes `._*` files that break the Tauri build: set `CARGO_TARGET_DIR` and `TMPDIR` to an APFS disk, pass `--out` and `--cache` on that disk to `scripts/package-companion.sh`, and first run `find src-tauri/capabilities src-tauri/permissions -name '._*' -delete 2>/dev/null || true`.

## Agent skills

### Issue tracker

New issues and PRDs are local markdown files under `.scratch/<feature>/`, which git ignores, so a fresh clone or worktree has none. Issues the owner has published are on GitHub (`gh issue list`). Read both; post to GitHub only when the owner asks. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), recorded as a `Status:` line in each issue file. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
