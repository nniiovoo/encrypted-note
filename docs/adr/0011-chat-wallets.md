# Chat Wallets: separate keys an agent can ask the app to use, with the owner's approval

Status: Accepted by the owner 2026-11-05, deferred the same day, and **reinstated 2026-11-09** with the owner's later answers folded in (see [Reinstated 2026-11-09](#reinstated-2026-11-09)). The reinstatement stands once the owner merges it. **Not built.** No real funds go to a Chat Wallet until the drill and the security review in the build order are done.

Amends [ADR-0007](0007-agents-use-separate-restricted-keys.md) and [ADR-0012](0012-agents-store-and-manage-keys.md). Agents reach it only through the route built for ADR-0012, with the program check of [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md). Leaves [ADR-0004](0004-wallet-kinds-under-a-separate-key.md) and [ADR-0009](0009-optional-agent-companion-helper.md) as written.

The owner wants encrypted-note to be a place where keys are stored and managed by chatting with an AI agent, and used that way too, safely. In October 2026, design panels put "signing inside the Vault, or any signer on the Mac" and "the Master Password or Touch ID as approval for agent requests" on the do-not-build list. Their main finding still holds: **an approval given on this Mac is not independent of software running on it.** This decision accepts that limit for one narrow case. A separate set of keys sits at addresses holding only small amounts, can never touch **Wallet Kinds**, and accepts only simple sends that the app reads out itself. Its safety rests on how little each address holds, not on the approval panel.

## The owner's choice, in their words

"Now I need it fully be used by Claude and Codex, not others." Asked which keys agents must use, the owner chose wallet keys, to sign sends (this decision), and API keys, to call services ([ADR-0014](0014-agent-key-operations-through-the-companion.md)).

## Decision

- **encrypted-note holds keys, not money.** Funds stay on the chain, at the key's address. The owner decides how much sits at each one, and keeps larger amounts on a hardware wallet that no agent reaches.
- **A Chat Wallet is a new, separate Kind.** It holds one private key for one account on one chain, made only for agent use. The app normally creates the key itself, so its value never exists anywhere else. It is encrypted under its own key, separate from ordinary **Notes** and from the Wallet Key ([ADR-0004](0004-wallet-kinds-under-a-separate-key.md)). A **Seed Phrase** or **Private Key**, including an **Agent-added Note**, can never become a Chat Wallet, be read through the agent route, or sign through it.
- **EVM chains and Solana.** Each Chat Wallet is made for one chain, chosen when it is created: any EVM chain, named by its chain ID, or Solana.
  - **Testing first:** it starts on Base Sepolia (chain ID 84532) and Solana devnet.
  - **EVM:** the chain ID is inside every signed transaction, so the app refuses any other chain.
  - **Solana:** a transaction doesn't say which cluster it is for, so the app can't tell devnet from mainnet. A test Chat Wallet's key is a real key on every cluster, and must never hold real funds.
- **The agent asks; it never sees a key.** Five Chat Wallet requests join the five of [ADR-0012](0012-agents-store-and-manage-keys.md) on the same route. Every other request is refused:

  | Request | What the app does | What the agent gets back |
  |---|---|---|
  | List Chat Wallets | Reads only the Chat Wallets the current grant covers | Each one's name, chain, public address and the limit left today |
  | Create one | The owner confirms in a native panel, then the app creates the key | Name and public address |
  | Import one | Opens a native panel. The owner pastes the key and types the **Master Password** | "Saved" or "Cancelled" |
  | Rename one | The owner confirms in a native panel | "Done" or "Cancelled" |
  | Ask for a send | Checks the rules, then shows the approval panel | The signed transaction, or the reason it was refused |

  Storing any other key is [ADR-0012](0012-agents-store-and-manage-keys.md)'s "Store a key". A key's value never travels through the chat, a tool result, a log or the clipboard. Ordinary **Notes**, their titles and other metadata, **Wallet Kinds**, the **Master Password** and the **Recovery Key** are never visible through this route.
- **Only plain transfers.** A send moves the chain's own coin, or one listed token, to a listed recipient, one transfer per transaction:
  - **EVM:** an EIP-1559 transaction sending the coin, or calling `transfer` on a listed token contract.
  - **Solana:** a System Program transfer of SOL, or an SPL Token `TransferChecked` of a listed token.

  The app refuses contract calls, token approvals, message signing, typed-data signing (permits), more than one transfer, and anything it can't fully decode.
- **The app signs but never sends.** It returns the signed transaction, and the agent sends it to the network. The **Vault** core and the WebView stay offline ([ADR-0001](0001-local-only-vault-moved-by-hand.md)).
- **Limits the agent can't change.** The owner sets these for each Chat Wallet, in each asset's own units:
  - a per-send limit;
  - a daily limit;
  - a list of allowed recipients.

  They are stored inside the encrypted **Vault**, not in the plain settings file, and the app refuses any send that breaks them.
  - **Raising a limit or adding a recipient:** only in the app's own window, with the **Master Password**, and it takes effect 24 hours later.
  - **Lowering a limit, removing a recipient or pausing a Chat Wallet:** takes effect at once.
  - **Deleting a Chat Wallet, or showing or copying its key:** only in the app's own window.
- **Every send needs Touch ID and then the Master Password**, in a native panel, whatever the amount.
  - **What the panel shows:** what the app decoded from the transaction itself, never the agent's description: chain, from, to (the recipient's saved name and full address), amount, fee and the limit left today.
  - **Touch ID is an extra check**, never a replacement for the **Master Password**. A Mac without working Touch ID can't sign Chat Wallet sends.
- **A send history.** Every signed send (time, Chat Wallet, recipient, amount, fee) is recorded encrypted in the **Vault**. It is shown only in the app's own window, never to the agent.
- **A grant covers only the Chat Wallets you choose.** Starting a grant ([ADR-0012](0012-agents-store-and-manage-keys.md)) covers no Chat Wallet unless the owner ticks it in Start's own dialog. List shows only those. A request about any other Chat Wallet gets the same answer as one that doesn't exist.
- **The same route, off by default.** Chat Wallet requests use [ADR-0012](0012-agents-store-and-manage-keys.md)'s socket, tool, grant and agent-account check, plus [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md)'s check that the tool was started by Claude Code or Codex.
  - **When it works:** only while the app is **Unlocked** and the owner has started a grant, which lasts at most one hour.
  - **When it stops:** on Stop, on lock, on sleep and when the app quits.
  - **What it shares:** nothing with the **Agent Companion**: no code, process or entitlement, and nothing from `companion/`.
  - **Platform:** Mac only.
- **Other keys.**
  - **Storing a Login or API Key:** the agent can ask to store one ([ADR-0012](0012-agents-store-and-manage-keys.md)).
  - **Using an API key:** only through an **Agent Key** and the **Agent Companion** ([ADR-0014](0014-agent-key-operations-through-the-companion.md)), never an API Key stored in the **Vault**.

## Consequences

- **The worst case for each Chat Wallet is whatever sits at its address.** A tricked agent, a careless approval, or malware that has captured the **Master Password** and can reach the Touch ID prompt can empty it. The limits, the panel and the 24-hour wait make losses smaller, slower and less likely. They don't make them impossible.
- **What a tricked agent can still do:**
  - ask for sends within the limits to listed recipients, which go through if the owner approves;
  - flood the owner with requests;
  - learn the names, addresses and limits of the Chat Wallets a grant covers;
  - choose when, or whether, to send a signed transaction.

  Chat Wallet names should be chosen with that in mind.
- **The route grows from storing to signing**, in the process that holds the **Vault**'s keys. Each new request needs refusal tests for every shape outside the rules, and a security review before any real funds.
- **New signing code and dependencies in the Rust core.**
  - **EVM:** secp256k1, Keccak-256, RLP and EIP-1559 transaction encoding.
  - **Solana:** Ed25519, base58 and transaction-message encoding.

  The plan is small single-purpose crates, not whole-chain SDKs, each pinned and explained in its PR. Each signer is tested against published test vectors, and refuses every other transaction shape and chain.
- **The Vault format changes:** a new Kind under its own key, with its limits, recipients and history. `docs/FORMAT.md` and the **Emergency Reader** follow.
- **When it ships**, the README, the guide, the security model and the glossary's "no interface" lines change with it, and the guide gets a section on using keys from an agent that starts with the worst case.
- Windows waits until its own route can be built and checked on a real Windows PC.

## Build order

Each step is its own pull request, with fake keys and test funds only.

1. **These documents:**
   - this ADR reinstated, with [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md) and [ADR-0014](0014-agent-key-operations-through-the-companion.md);
   - notes in ADR-0007, ADR-0008, ADR-0009 and ADR-0012;
   - the glossary and `AGENTS.md`.
2. **The ground it stands on:**
   - [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md)'s program check on the existing route;
   - the hand checks [ADR-0012](0012-agents-store-and-manage-keys.md) still needs, above all that a signed, sandboxed build can open its socket.
3. **Storage:** the Kind, its key, limits, recipients and history.
4. **The EVM signer,** with test vectors and refusal tests.
5. **The Solana signer,** the same way.
6. **The app's own window:** create, import, rename, limits, recipients, pause, history, and the 24-hour wait.
7. **The approval panel:** Touch ID, then the **Master Password**.
8. **The five requests on the route,** and the grant's choice of Chat Wallets.
9. **A drill on Base Sepolia and Solana devnet, with test funds.** It includes:
   - a planted "send everything to 0xFAKE…" message;
   - an agent asking to raise a limit;
   - a program other than Claude Code or Codex trying the route;
   - a timed stop.
10. **A security review,** then one small real Chat Wallet.

## Reinstated 2026-11-09

The owner deferred this decision on 2026-11-05, because the goal had become storing keys without using them ([ADR-0012](0012-agents-store-and-manage-keys.md)). On 2026-11-09 the owner asked for keys to be used by Claude Code and Codex. What changed from the 2026-11-05 version:

- **Chains:** Base only became any EVM chain and Solana, as the owner had answered after accepting.
- **Route:** a separate route became [ADR-0012](0012-agents-store-and-manage-keys.md)'s, plus [ADR-0013](0013-only-claude-code-and-codex-use-the-agent-routes.md)'s program check. One way in to review, not two.
- **Requests:** six became five. "Save another key" is now [ADR-0012](0012-agents-store-and-manage-keys.md)'s "Store a key".
- **New:** a grant covers only the Chat Wallets the owner ticks.
- **Kept from the owner's answers:** Touch ID and then the **Master Password** for every send, and the encrypted send history.
- **Kept as decided:** the limits, the 24-hour wait, plain transfers only, and an app that signs but never sends.

## Considered Options

- **The agent sees or receives the key**, by pasting it into chat or through a tool result. Rejected: the key reaches the AI company.
- **The agent uses Seed Phrases or Private Keys already in the Vault.** Rejected: the worst case becomes everything on that seed, on every chain.
- **A separate pot, approved on a hardware wallet.** Stronger, because the approval doesn't depend on the Mac, but nothing is stored or used through the app. It stays the route for larger amounts.
- **Touch ID alone, or Touch ID only for small sends.** Rejected by the owner: Touch ID is easy to give without reading.
- **No limits, only the approval panel.** Rejected by the owner: reading every panel carefully would be the only protection.
- **Agent apps in the owner's own account.** Rejected by the owner: they can already do anything the owner can, including trying to drive the approval panel.
- **Running this through the Agent Companion.** Rejected: [ADR-0009](0009-optional-agent-companion-helper.md) promises that helper never gets Vault access, and it is the one part of the app with network access.
- **Base only.** Replaced by the owner's later answer: every EVM chain and Solana.
- **A route of its own, apart from ADR-0012's.** Replaced now that ADR-0012's route is built: a second way into the process that holds the Vault's keys would double what needs review.
