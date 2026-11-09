# Agents use API keys through the Agent Companion, one fixed operation at a time

Status: Proposed 2026-11-09 from the owner's choices below; it stands once the owner merges it. **Not built, and no new operation is chosen yet.**

Amends [ADR-0009](0009-optional-agent-companion-helper.md) ("make only the fixed `generate_text` request") and [ADR-0008](0008-agent-gateway-awaits-a-concrete-api.md). Every requirement of [ADR-0007](0007-agents-use-separate-restricted-keys.md) stands, and each new operation must meet it.

> **Contradicts [ADR-0009](0009-optional-agent-companion-helper.md)** (the helper may "make only the fixed `generate_text` request") and the `AGENTS.md` rule "Never widen it". **Worth reopening because** the owner wants Claude Code and Codex to call services with API keys. [ADR-0007](0007-agents-use-separate-restricted-keys.md) already says how that can be done safely: a separate **Agent Key** with limits the provider enforces, kept out of chat. The Agent Companion is the one part of the project built for exactly that. It runs outside the Vault's process, in its own sandbox, and already holds an Agent Key without the agent ever seeing it.

## The owner's choice, in their words

"Now I need it fully be used by Claude and Codex, not others." Asked which keys agents must use, the owner chose API keys, to call services (this decision), and wallet keys, to sign sends ([ADR-0011](0011-chat-wallets.md)).

## Decision

- **The agent asks the helper; it never holds the key.** The agent calls a tool. The Agent Companion makes the request with the Agent Key and returns only the result fields that operation's record allows. The key never appears in the chat, a tool result or a log.
- **Only Agent Keys, never the owner's own** ([ADR-0007](0007-agents-use-separate-restricted-keys.md)). Each operation uses a key the owner creates at the service just for it, with permissions, expiry, revocation and hard caps that the provider enforces. An API Key stored as a **Note** is never used, copied or offered to the helper. [ADR-0009](0009-optional-agent-companion-helper.md)'s list of what never reaches the helper stands.
- **One fixed operation at a time.** Each new operation needs:
  - its own record, the table of [ADR-0007](0007-agents-use-separate-restricted-keys.md): operation and resources, expiry, revocation, caps, results and where they go;
  - the owner's approval of that record;
  - its own pull request.
- **The helper builds every request in its own code.** The agent chooses no URL, method, header, host or body template. There is no general HTTP proxy and no following of redirects. The helper's policy file lists every operation and destination, and the app checks it against a pinned hash, as it does today.
- **If the provider can't enforce the limits, there is no operation** ([ADR-0007](0007-agents-use-separate-restricted-keys.md)). Narrow permissions, expiry, revocation and hard caps are checked against the provider's current documentation before any pilot.
- **Keys stay with the helper.** The owner sets up each Agent Key through Settings, as the first one is set up today. It is kept in the helper's own container, with the custody limits of [ADR-0009](0009-optional-agent-companion-helper.md): an owner-only file there, and held in memory while the helper runs. The **Vault** never holds or reads an Agent Key.
- **Only Claude Code and Codex, in the agent account.** The helper's agent entry gets [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md)'s program check before any new operation goes live. Today the agent account's bridge reaches the helper on loopback TCP (`127.0.0.1:8789`). If the check can't be made reliably there, the bridge moves to a Unix socket in an owner-made folder; that step's PR decides which.
- **One grant, covering the operations you choose.** The existing grant (at most one hour, ended by Stop, any lock, sleep and quit) covers only the operations the owner ticks when starting it.

## Consequences

- **What leaves the computer:** what the agent supplies goes to that service. The result returns to the agent and may reach its AI company. Vault content never does.
- **A hidden key keeps its full authority.** An agent can make every call the operation and the key's provider scope allow, up to the caps ([ADR-0007](0007-agents-use-separate-restricted-keys.md)). Caps and expiry make a loss smaller, not impossible.
- **The helper's network reach grows** with each operation, which adds a destination. Each one updates the helper's policy and its pinned hash, and gets its own review. The Vault core and the WebView stay offline ([ADR-0001](0001-local-only-vault-moved-by-hand.md)).
- **The first new operation isn't chosen.** It needs a service whose provider enforces narrow permissions, expiry, revocation and hard caps. The owner picks it, and its record is checked against the provider's current documentation before anything is built.

## Build order

Each step is its own pull request, with fake keys only until the pilot.

1. **These documents.**
2. **[ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md)'s check on the helper's agent entry,** or the move to a Unix socket.
3. **The first operation's record:** the owner picks the service and the operation, and the record is written and checked against the provider's current documentation.
4. **The operation in the helper,** with fictional tests and refusal tests: other URLs, methods, hosts and fields.
5. **A pilot with a new, hard-capped Agent Key,** then a review before wider use.

## Considered Options

- **Give the agent the Agent Key itself:** rejected. Raw keys stay out of anything the agent sees ([ADR-0007](0007-agents-use-separate-restricted-keys.md)), and through the chat a key reaches the AI company.
- **Let the Vault app make the call with an API Key stored as a Note:** rejected. That would put network code in the process that holds the Vault's keys ([ADR-0001](0001-local-only-vault-moved-by-hand.md), [ADR-0003](0003-tauri-with-secrets-kept-in-rust.md)), and the owner's own keys must never be used by agents.
- **A new, separate broker program:** not chosen. It would repeat the companion's sandbox, grant and setup, leaving two helpers to review instead of one.
- **A general HTTP proxy with a list of allowed hosts:** rejected. The agent could still choose any operation the key allows on those hosts.
