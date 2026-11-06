# Chat Wallets: separate keys an agent can ask the app to use, with the owner's approval

Status: **Deferred by the owner, 2026-11-05**, the day it was accepted. The goal is now for agents to store and manage keys without using them ([ADR-0012](0012-agents-store-and-manage-keys.md)), so nothing here is built. It is kept as a record of the design, should signing come back. The owner's later answers (Touch ID with the Master Password, an encrypted send history, every EVM and Solana wallet rather than Base alone) would need folding in then.

Amends [ADR-0007](0007-agents-use-separate-restricted-keys.md). Leaves [ADR-0004](0004-wallet-kinds-under-a-separate-key.md) and [ADR-0009](0009-optional-agent-companion-helper.md) as written.

The owner wants encrypted-note to be a place where keys are stored and managed by chatting with an AI agent, and used that way too, safely. In October 2026, design panels put "signing inside the Vault, or any signer on the Mac" and "the Master Password or Touch ID as approval for agent requests" on the do-not-build list. Their main finding still holds: **an approval given on this Mac is not independent of software running on it.** This decision accepts that limit for one narrow case. A separate set of keys sits at addresses holding only small amounts, can never touch **Wallet Kinds**, and accepts only simple sends that the app reads out itself. Its safety rests on how little each address holds, not on the approval panel.

## Decision

- **encrypted-note holds keys, not money.** Funds stay on the chain, at the key's address. The owner decides how much sits at each one, and keeps larger amounts on a hardware wallet that no agent reaches.
- **A Chat Wallet is a new, separate Kind.** It holds one private key for one account on one chain, made only for agent use. The app normally creates the key itself, so its value never exists anywhere else. It is encrypted under its own key, separate from ordinary **Notes** and from the Wallet Key ([ADR-0004](0004-wallet-kinds-under-a-separate-key.md)). A **Seed Phrase** or **Private Key** can never become a Chat Wallet, be read through the agent route, or sign through it.
- **Base only, in version 1.** Testing starts on Base Sepolia (chain ID 84532), then moves to Base (chain ID 8453). The chain ID is pinned, and the app refuses any other.
- **The agent asks; it never sees a key.** The agent route accepts six requests. Every other request is refused:

  | Request | What the app does | What the agent gets back |
  |---|---|---|
  | List Chat Wallets | Reads Chat Wallets only | Each one's name, chain, public address and the limit left today |
  | Create one | The owner confirms in a native panel, then the app creates the key | Name and public address |
  | Import one | Opens a native panel. The owner pastes the key and types the **Master Password** | "Saved" or "Cancelled" |
  | Rename one | The owner confirms in a native panel | "Done" or "Cancelled" |
  | Ask for a send | Checks the rules, then shows the approval panel | The signed transaction, or the reason it was refused |
  | Save another key | Opens the normal Add window with a suggested name, and the owner types the value | "Saved" or "Cancelled" |

  A key's value never travels through the chat, a tool result, a log or the clipboard. Ordinary **Notes**, their titles and other metadata, **Wallet Kinds**, the **Master Password** and the **Recovery Key** are never visible through this route.
- **Only plain transfers.** Version 1 signs a transfer of the chain's own coin, or of one listed token, to a listed recipient. It refuses contract calls, token approvals, message signing, typed-data signing (permits) and anything it can't fully decode.
- **The app signs but never sends.** It returns the signed transaction, and the agent sends it to the network. The **Vault** core and the WebView stay offline ([ADR-0001](0001-local-only-vault-moved-by-hand.md)).
- **Limits the agent can't change.** The owner sets these for each Chat Wallet: a per-send limit, a daily limit and a list of allowed recipients. They are stored inside the encrypted **Vault**, not in the plain settings file. The app refuses any send that breaks them. They can be raised, or a recipient added, only in the app's own window, with the **Master Password**, and the change takes effect 24 hours later. Lowering a limit, removing a recipient or pausing a Chat Wallet takes effect at once. Deleting a Chat Wallet, or showing or copying its key, happens only in the app's own window.
- **Every send needs Touch ID and then the Master Password**, in a native panel, whatever the amount. The panel shows what the app decoded from the transaction itself, never the agent's description: chain, from, to (the recipient's saved name and full address), amount, fee and the limit left today. Touch ID here is an extra check, never a replacement for the Master Password. A Mac without working Touch ID can't sign Chat Wallet sends.
- **A send history.** Every signed send (time, Chat Wallet, recipient, amount, fee) is recorded encrypted in the **Vault**. It is shown only in the app's own window, never to the agent.
- **A separate route, off by default.** A local MCP tool talks to the app over a private local connection. It works only while the app is **Unlocked** and the owner has started a grant, which lasts at most one hour. It stops on Stop, on lock, on sleep and when the app quits. It accepts connections only from the separate standard agent account, and refuses every other caller, including programs in the owner's own account. It is not the **Agent Companion**, and shares no code, process or entitlement with it or with `companion/`. Mac only.
- **Other keys.** For Logins and API Keys, the agent can only open the normal Add window. Using API Keys from an agent stays on the **Agent Key** route ([ADR-0007](0007-agents-use-separate-restricted-keys.md)).

## Consequences

- **The worst case for each Chat Wallet is whatever sits at its address.** A tricked agent, a careless approval, or malware that has captured the **Master Password** and can reach the Touch ID prompt can empty it. The limits, the panel and the 24-hour wait make losses smaller, slower and less likely. They don't make them impossible.
- **What a tricked agent can still do:**
  - ask for sends within the limits to listed recipients, which go through if the owner approves;
  - flood the owner with requests;
  - learn Chat Wallet names, addresses and limits;
  - choose when, or whether, to send a signed transaction.

  Chat Wallet names should be chosen with that in mind.
- **The app gains its first way in from other programs**, in the process that holds the **Vault**'s keys. Each part needs refusal tests for every request outside the six, and a security review before any real funds.
- **New signing code and dependencies in the Rust core** (secp256k1, Keccak, EIP-1559 transaction encoding). Each is pinned and explained in its PR. The signer is tested against published test vectors and refuses every other transaction shape and chain ID.
- **The Vault format changes**: a new Kind under its own key, with its limits, recipients and history. `docs/FORMAT.md` and the **Emergency Reader** follow.
- **When it ships**, the README, the guide, the security model and the glossary's "no interface" lines change with it, and the guide gets a section on using keys from an agent that starts with the worst case.
- Windows waits until its own route can be built and checked on a real Windows PC.

## Build order

Each step is its own pull request, with fake keys only.

1. These documents.
2. Storage: the Kind, its key, limits, recipients and history.
3. The Base signer, with test vectors and refusal tests.
4. The app's own window: create, import, rename, limits, recipients, pause, history, and the 24-hour wait.
5. The approval panel: Touch ID, then the Master Password.
6. The agent route: the MCP tool, the agent-account check, the grant and the six requests.
7. A drill on Base Sepolia with fake funds. It includes a planted "send everything to 0xFAKE…" message, an agent asking to raise a limit, and a timed stop.
8. A security review, then one small real Chat Wallet.

## Considered Options

- **The agent sees or receives the key**, by pasting it into chat or through a tool result. Rejected: the key reaches the AI company.
- **The agent uses Seed Phrases or Private Keys already in the Vault.** Rejected: the worst case becomes everything on that seed, on every chain.
- **A separate pot, approved on a hardware wallet.** Stronger, because the approval doesn't depend on the Mac, but nothing is stored or used through the app. It stays the route for larger amounts.
- **Touch ID alone, or Touch ID only for small sends.** Rejected by the owner: Touch ID is easy to give without reading.
- **No limits, only the approval panel.** Rejected by the owner: reading every panel carefully would be the only protection.
- **Agent apps in the owner's own account.** Rejected by the owner: they can already do anything the owner can, including trying to drive the approval panel.
- **Running this through the Agent Companion.** Rejected: [ADR-0009](0009-optional-agent-companion-helper.md) promises that helper never gets Vault access, and it is the one part of the app with network access.
