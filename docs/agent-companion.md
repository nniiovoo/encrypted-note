# Agent Companion (optional)

> **Status:** implemented and checked with fictional data only. No real Agent Key, OpenAI account setting, macOS account, live request or ChatGPT connection has been used. Each step marked **Owner** is yours to do; nothing in the app does it for you. Design and limits: [ADR-0009](adr/0009-optional-agent-companion-helper.md).

The Agent Companion lets **one approved AI agent** use **one fixed OpenAI text tool** (`generate_text`, model `gpt-6-luna`) with a **separate Agent Key** you create only for it. It never sees your Vault.

| The helper can | The helper can't |
|---|---|
| Send text an approved agent gives it (at most 4,096 bytes) to OpenAI and return at most 512 tokens of text | Open your Vault, its Notes, their titles or keys, Wallet Kinds, your Master Password or Recovery Key |
| Run one request at a time: 6 a minute, 60 an hour, 100 per start | Use another model, URL, tool, file or provider |
| Run for one grant of at most an hour after you press Start | Start or restart by itself |

**Where your data goes.** Text an agent sends goes to OpenAI and can cost money. The answer goes back to the agent, and from there possibly to its own cloud provider. The Vault itself never connects to the internet.

**How the key is kept: read this before using a real key.** The helper stores the Agent Key in a plain-text file that only your macOS account can read, inside the helper's own sandbox container. While the helper is on, it also holds the key in Node.js memory. That is **not** encrypted Vault storage, and that memory can't be reliably erased. So the key must be new, narrow, short-lived and capped at OpenAI, and you revoke it **at OpenAI**: stopping the helper doesn't revoke anything.

## How it fits together

```
Your account                                     Agent account (standard, not admin)
┌───────────────────────────────────────┐        ┌───────────────────────────────┐
│ encrypted-note (Vault: offline)        │        │ AI agent (Codex, Claude Code…) │
│  Settings → Agent Companion            │        │   └ agent bridge (stdio MCP)   │
│      │ status/start/stop/setup (XPC)   │        │        │ token from handoff     │
│      ▼                                 │        │        │ folder                 │
│ Agent Companion helper (own sandbox)   │◄───────┼────────┘ 127.0.0.1:8789        │
│  └ pinned Node runtime + broker ───────┼──► api.openai.com/v1/responses           │
└───────────────────────────────────────┘        └───────────────────────────────┘
```

## Before you start

1. **A build that includes the helper (Mac only).** On a trusted build machine run `scripts/package-companion.sh --app`. It verifies the pinned companion commit, its policy hash and the Node.js release, then builds the app with the helper inside. An ordinary `tauri build` has no helper, and Settings says so.
2. **Owner: a standard agent account.** In System Settings → Users & Groups, add a new **Standard** user (for example short name `enote-agent`). Leave administrator off. Run every agent tool only in that account, and never put your own passwords, Apple Account or keys there.
3. **Owner: the handoff folder.** In Terminal, in your own account, replacing `enote-agent` with your agent account's short name:

   ```bash
   mkdir /Users/Shared/encrypted-note-companion
   ```
   ```bash
   chmod 700 /Users/Shared/encrypted-note-companion
   ```
   ```bash
   chmod +a "user:enote-agent allow search" /Users/Shared/encrypted-note-companion
   ```

   On each Start, the helper writes a fresh agent token there that only the agent account can read. It removes the token when it stops.
4. **Owner: the restricted OpenAI Agent Key.** At OpenAI, create a **new** key for a dedicated project with:
   - only the `api.responses.write` permission;
   - a model allowlist of `gpt-6-luna` only;
   - hosted tools turned off;
   - a short expiry (the approved trial is one hour);
   - a **hard** monthly spend limit (the approved trial is $1).

   Never reuse a key that's in your Vault. Save the key in a file containing exactly `OPENAI_AGENT_API_KEY=<key>` in your own account, then run `chmod 600` on it. Delete that file after setup: the helper keeps its own copy. The full provider checklist is in [Agent Key validation](agent-key-validation.md).
5. **Owner: the reviewed plan.** Copy `isolation/plan.example.json` from the pinned companion commit and fill in:
   - `ownerUid`: the output of `id -u`;
   - `agentUid`: the output of `id -u enote-agent`;
   - `bearerTokenFile`: `/Users/Shared/encrypted-note-companion/agent-token`.

   Leave `grantExpiresAt` as `null`, because the app issues each grant when you press Start.

## In the app

1. Unlock, open **Settings → Agent Companion**, and choose **Set up…**. Two file panels open: choose the plan first, then the Agent Key file. The app passes both files to the helper without reading them.
2. Check **Ready on this Mac**. Every item needs a ✓ before Start works:
   - pinned runtime and policy;
   - plan;
   - Agent Key file;
   - handoff folder;
   - standard agent account;
   - protected code.

   If the app is in `/Applications`, it accepts that folder's admin-group write permission only after confirming that the agent account isn't an administrator.
3. Choose **Start for one hour**, read the confirmation and confirm. The grant ends at the time shown, and also at once when:
   - you choose Stop;
   - the Vault locks;
   - the Mac sleeps or the screen locks;
   - you switch users;
   - you quit the app.

   If the helper stops unexpectedly, Settings says so and it never restarts by itself.

## In the agent account (local agents)

Install the companion at the same pinned commit (see `src-tauri/companion/companion.lock.json`) in a folder the agent account can read but not change. Then configure the agent's MCP client to start the stdio bridge with only the plan's path:

```text
command: /path/to/node
args:    /path/to/encrypted-note-agent-companion/isolation/agent-bridge.mjs /path/to/plan.json
```

The bridge checks that it's running as the plan's standard agent account. It reads only the token in the handoff folder and connects only to `127.0.0.1:8789`. The agent gets the `generate_text` tool, never the key.

## Private ChatGPT connection (optional)

ChatGPT can't reach this Mac's loopback or stdio directly. The private route is OpenAI's [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels): its local client runs **in the agent account** and starts the same agent bridge.
1. Add the tunnel as a custom MCP server in ChatGPT (Plugins → **+** → Add custom MCP server), following OpenAI's [connection guide](https://developers.openai.com/plugins/deploy/connect-chatgpt).
2. Use a ChatGPT account that has no other connectors to your own data. Never share the tunnel's identity with the inference key.
3. Make one test call with fictional text.

The app always shows this connection as **Not checked**. Count it as working only after you've seen an end-to-end call yourself, and seen calls refused after Stop or expiry.

## Stop, revoke, clean up

- **Stop** in Settings ends new requests at once. It can't undo a request already sent or billed.
- **Revoke** the Agent Key at OpenAI if anything looks wrong, and when you're done. That's the only real revocation.
- To remove the setup, delete `/Users/Shared/encrypted-note-companion` and the agent account, and revoke the key. The helper's container is `~/Library/Containers/<app identifier>.companion`.

## What has been checked

See [Agent Key validation → Integrated helper](agent-key-validation.md#integrated-helper-2026-10-26) for what passed with fictional data and what remains pending.
