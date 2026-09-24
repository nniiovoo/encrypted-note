# encrypted-note

A personal app for storing secrets, encrypted and saved only on this computer.

## Rules

- Never put real secrets (passwords, keys, seed phrases) in code, tests, docs, `.scratch/` issues, or commits. Use obviously fake placeholders.
- Never add network access: no HTTP clients, updaters, telemetry, crash reporters or analytics (ADR-0001).
- Keys and decrypted Notes live only in the Rust core; the WebView receives one Hidden Field at a time (ADR-0003).
- Add a dependency only when it's clearly needed, pin its exact version, and say why in the PR/issue.

## Agent skills

### Issue tracker

Issues and PRDs are local markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), recorded as a `Status:` line in each issue file. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
