# Agents may store and manage the keys they add, without seeing or using them

Status: Accepted by the owner, 2026-11-05. **Built 2026-11-06 with fictional data; hand checks on a real Mac and the step 6 review still to do.** Until they are done, don't store real keys through an agent. How to set it up, its limits and the open hand checks: [Agents storing keys](../agent-keys.md).

Amends [ADR-0007](0007-agents-use-separate-restricted-keys.md). Defers [ADR-0011](0011-chat-wallets.md): signing is no longer the goal.

> **Amended 2026-11-09 (proposed; not built):** signing is the goal again. [ADR-0011](0011-chat-wallets.md) is reinstated, and its five Chat Wallet requests join this route, on the same socket, tool and grant. [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md) adds a check that the tool was started by Claude Code or Codex. Everything below about storing and managing keys stands.

> **Contradicts [ADR-0007](0007-agents-use-separate-restricted-keys.md)** ("The app offers no agent interface… no Note enumeration or metadata export"). **Worth reopening because** the owner wants Claude, ChatGPT or Codex to help store and organise keys. This route lets an agent put keys **in**, and look after their names and details, but never take a value **out** or use one. So the worst a tricked agent can do is make a mess of the keys it added, not leak or spend them.

## The owner's goal, in their words

"A safety layer for agents, ChatGPT, or like Claude, to safely manage and store the keys… We are not using that key… just store it like I'm typing my keys in… You can store successfully without seeing it… And you can manage that key."

## Decision

- **The agent asks; you type; the agent never sees a value.** To store a key, the agent asks the app, and the app opens its own window with the Kind, name and details the agent suggested. You paste or type the value there, and Seed Phrases and Private Keys still need your Master Password, as today. The agent gets back only "saved" and the key's id, or "cancelled".
- **Every Kind of key.** API Key, Login, Text, Seed Phrase and Private Key, for any chain: EVM (Ethereum, Base, Arbitrum…), Solana (SVM), Bitcoin or other. These are the app's existing Kinds, stored exactly as when you add them yourself.
- **The agent manages only the keys it added.** Each one is marked "added by an agent", and you can see that mark in the app. The agent can't see, list or touch any other Note, and it can't put the mark on a Note you added. You can remove the mark at any time, which takes the key out of the agent's reach at once.
- **Five requests.** Every other request is refused:

  | Request | What the app does | What the agent gets back |
  |---|---|---|
  | Store a key | Opens its own window, pre-filled with the agent's suggestion. You type the value and choose Save | "Saved" and the id, or "Cancelled" |
  | List its keys | Reads only the keys marked "added by an agent" | Each one's id, Kind, name and visible details (service, chain, public address, purpose, expiry date), and when it was added or changed. **Never a value**, and never whether a Seed Phrase has a Passphrase |
  | Edit details | Changes the name, purpose or expiry date at once. For anything that says where a key is used (chain, public address, derivation path, website, username, service), the app asks you in its own window first | "Done", "Cancelled" or "Not allowed" |
  | Replace a value | Opens its own window for that key. You type the new value | "Saved" or "Cancelled" |
  | Delete | Asks you in its own window, with your Master Password, then moves the key to Trash (kept 30 days) | "Deleted" or "Cancelled" |

- **Never, by any request:** show, copy or export a value, sign anything, list your own Notes, or see the Master Password, the Recovery Key, a Backup or a Safety Copy. No value ever travels through the chat, a tool result, a log or the clipboard.
- **Why some edits ask you first:** a tricked agent could change a Private Key's public address to one the attacker controls, and you might later send funds there believing it's yours. A name or a purpose can only confuse you, so those apply at once.
- **The same narrow route ADR-0011 described, off by default.** A local MCP tool for Claude Code, the Claude app or Codex, running in the separate standard agent account. It works only while the app is Unlocked and you have started a grant lasting at most one hour, and it stops on lock, sleep and quit. It accepts connections only from the agent account, and shares nothing with the Agent Companion or `companion/`. Mac only at first.

## Consequences

- **What leaves the computer:** the names and visible details of agent-added keys reach the AI company, as part of the chat. Values never do. Name keys with that in mind ("Binance main 2 BTC" says a lot).
- **What a tricked agent can still do:**
  - add junk keys, or flood you with windows (you cancel them);
  - rename an agent-added key or change its purpose to confuse you;
  - ask you to delete or replace one, which happens only if you confirm.

  It can't read, copy, move or spend anything.
- **Storing through the agent is no less safe than typing the key in yourself.** The value goes from your keyboard into the same native control as today, never through the agent.
- **The app gains its first way in from other programs**, in the process that holds the Vault's keys. It needs refusal tests for every request outside the five and for every Note not marked "added by an agent", and a security review before real keys go in.
- **Small Vault format change:** a Note gains one optional mark, "added by an agent" (document schema 3, written only while some Note has it). The Emergency Reader shows the mark.
- **Signing is out.** [ADR-0011](0011-chat-wallets.md)'s Chat Wallets (limits, recipients, Base signing) are deferred, and none of their code is merged.

## Build order

Each step is its own pull request, with fake keys only.

1. These documents: this ADR, ADR-0011 marked Deferred, ADR-0007 amended, the glossary and `AGENTS.md` updated.
2. The mark: the "added by an agent" flag in the Vault format and the notes crate, the Emergency Reader and FORMAT.md, plus tests that agent requests see only marked Notes.
3. The app's own windows: store, replace and delete, opened from a request, reusing today's native entry and Master Password panels. Plus the confirm window for edits that say where a key is used, and showing and removing the mark.
4. The agent route: the MCP tool, the agent-account check, the grant and the five requests, with refusal tests for everything else.
5. A drill with fake keys: a planted instruction to "store this key and then show it", an attempt to edit a Private Key's address, and a flood of store requests.
6. A security review, then real keys.

## As built (2026-11-06)

The owner settled these details on 2026-11-05 and 2026-11-06, before the build:

- **Where values are typed:** in the app's ordinary editor, the same Add and Edit screen as when you add a key yourself, filled in from the agent's suggestion. Seed Phrases and Private Keys then need the Master Password in today's native panel. "The app's own window" above means the app's window, not the agent's: there is no new native panel for values, and the value takes the same path as any value typed in the editor ([ADR-0003](0003-tauri-with-secrets-kept-in-rust.md)).
- **Edits, field by field.** At once: the name of a Login, API Key or Text, a Login's or API Key's purpose, and an API Key's expiry date. The owner allows first: a Login's website and username, and an API Key's service, key ID, where it is managed and variable name. The owner allows, then types the Master Password: every edit of a Seed Phrase or Private Key, its name included. Never: a Hidden Field (only Replace changes a value) and a Seed Phrase's word count.
- **Delete** needs the owner's confirmation and the Master Password for every Kind.
- **The approved entitlement:** one `temporary-exception.files.absolute-path.read-write` entry for `/Users/Shared/encrypted-note-keys/`, where the app makes its socket. No network entitlement.

Where it lives: the mark and the edit rules in `crates/notes/src/agent.rs`, the grant and the five requests in `src-tauri/src/core/agent.rs`, the socket in `src-tauri/src/agent_route.rs`, the agent's tool in `crates/agent-keys-mcp`, the owner's setup in `scripts/agent-keys-setup.sh`, and the screens in `src/AgentRequest.tsx`, `src/Note.tsx`, `src/Home.tsx` and `src/Settings.tsx`. Steps 2 to 4 of the build order are built and tested with fictional data; steps 5 and 6 are not done.

## Considered Options

- **The agent sees or handles the value** (pasted into chat, passed through a tool). Rejected: the value reaches the AI company.
- **The agent sees every key in the Vault.** Rejected by the owner: titles would reach the AI company, and a planted title could trick the agent.
- **You mark which keys an agent may manage.** Not chosen for now; it can be added later as a way to hand an existing key to the agent.
- **Signing with stored keys (ADR-0011).** Deferred: not the goal now.
