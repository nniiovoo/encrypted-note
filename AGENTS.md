# encrypted-note

A personal app for storing secrets, encrypted and saved only on this computer.

## Rules

- Never put real secrets (passwords, keys, seed phrases) in code, tests, docs, `.scratch/` issues, or commits. Use obviously fake placeholders.
- Never add network access to the Vault core or WebView: no HTTP clients, updaters, telemetry, crash reporters or analytics (ADR-0001). The only approved exception is the optional Agent Companion helper (ADR-0009): a separately sandboxed XPC service that makes only the companion's fixed OpenAI request. Never widen it, give it Vault access, or add networking anywhere else.
- Keys and decrypted Notes live only in the Rust core; the WebView receives one Hidden Field at a time (ADR-0003). The Agent Companion helper may hold one separately created Agent Key, never a Vault value (ADR-0009).
- Never pass Vault content, Note metadata, Note keys, Wallet Kinds, the Master Password or the Recovery Key to the Agent Companion, and never forward helper output, provider errors, keys or tokens to the WebView (ADR-0009).
- Add a dependency only when it's clearly needed, pin its exact version, and say why in the PR/issue.

## Agent skills

### Issue tracker

Issues and PRDs are local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), recorded as a `Status:` line in each issue file. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
