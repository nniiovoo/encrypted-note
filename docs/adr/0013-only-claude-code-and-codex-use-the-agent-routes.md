# Only Claude Code and Codex may use the agent routes

Status: Proposed 2026-11-09 from the owner's choices below; it stands once the owner merges it. **Not built.**

Amends [ADR-0012](0012-agents-store-and-manage-keys.md) ("It accepts connections only from the agent account"). Applies equally to the Chat Wallet requests ([ADR-0011](0011-chat-wallets.md)) and to the Agent Companion's agent entry ([ADR-0014](0014-agent-key-operations-through-the-companion.md)).

## The owner's choice, in their words

"Now I need it fully be used by Claude and Codex, not others." Asked how to enforce that, the owner chose to keep the separate agent account **and also** check each program's signature, knowing the signature check is a speed bump rather than a wall.

## Decision

- **Three checks on every connection.** All three come from the kernel and macOS, never from anything the caller says about itself:
  1. **The account** is the separate agent account (as built for [ADR-0012](0012-agents-store-and-manage-keys.md)).
  2. **The connecting program** is the installed `enote-agent-keys` tool. Its code hash must match the one the owner approved in Settings, which the setup script prints. Once the tool is bundled inside the app and signed with the app's identity, the app checks that signature instead.
  3. **The program that started the tool** is Claude Code or Codex, checked by its Apple code signature against the list below.

  A connection that fails any check is closed without a reply, as one from the wrong account is today.
- **The allowed programs**, as found on the owner's Mac on 2026-11-09:

  | Program | Signed by | Team ID | Identifier |
  |---|---|---|---|
  | Claude Code (command line) | Developer ID Application: Anthropic PBC | `Q6L2SF6YDW` | `com.anthropic.claude-code` |
  | Codex (command line, its native program) | Developer ID Application: OpenAI OpCo, LLC | `2DC432GLL2` | `codex` |

  The list is fixed in code: nothing the agent account can write changes it. The Claude app and the Codex app are added only after a check shows which of their processes starts the tool. ChatGPT and every other program are not on the list.
- **Codex installed with npm.** There the `codex` command is a Node.js script, which can't be signed. It starts Codex's signed native program, which is the program that starts the tool. The build step confirms this on a real run.
- **No fooling it with reused process numbers or injected code.** The connecting program is identified by the connection's audit token. The program that started it is identified by its process number, re-checked after the signature check so a reused number can't pass; the build PR shows how. A program whose signature lets outside code be loaded into it is refused: disabled library validation, allowed DYLD environment variables, or a debuggable signature.
- **Where the code lives.** In `src-tauri/src/agent_route.rs`, next to the existing peer checks, under the same placement rule in `scripts/check-network-source.mjs`.
- **A spike comes first.** It tests whether the sandboxed app may read another account's process information and code signatures. If it can't, the check isn't built that way, and this decision goes back to the owner.

## Consequences

- **A speed bump, not a wall.** It stops other programs in the agent account from using the route, whether directly or by running the tool. It doesn't stop a program in that account from starting Claude Code or Codex itself, with its own instructions, settings or MCP configuration. So the agent account must still hold only Claude Code and Codex. The separate account stays the real boundary.
- **It doesn't make Claude Code or Codex trustworthy.** A tricked Claude Code or Codex passes the check. Everything [ADR-0011](0011-chat-wallets.md), [ADR-0012](0012-agents-store-and-manage-keys.md) and [ADR-0014](0014-agent-key-operations-through-the-companion.md) say a tricked agent can still do stays true.
- **Updates can lock agents out.** If Anthropic or OpenAI change their signing identity, or the way the tool is started, the route refuses until a new build updates the list. It fails closed.
- **The app reads a little about other programs:** the connecting program's signature and code hash, and its parent's signature. Nothing else, and none of it leaves the app.

## Build order

Each step is its own pull request.

1. These documents.
2. The spike, in a signed, sandboxed test build (a test identifier only, never the owner's Vault): can the app read what the check needs?
3. The check on the [ADR-0012](0012-agents-store-and-manage-keys.md) route, with tests:
   - accepted: the right tool, started by Claude Code or by Codex;
   - refused: the tool started by a shell or any other program, a program connecting directly, a wrong code hash, and a program whose signature allows outside code.
4. The same check on the Agent Companion's agent entry ([ADR-0014](0014-agent-key-operations-through-the-companion.md)).
5. Hand checks from Claude Code and Codex in the agent account, and from a third program that must be refused.

## Considered Options

- **The agent account alone (as built):** the owner asked for more.
- **Asking the caller who it is,** such as a name or token the agent sends: rejected. Any program can claim to be Claude Code.
- **Checking only the tool, not who started it:** rejected. Any program in the agent account can run the tool.
- **Checking the whole chain of parent processes:** not needed. The tool's direct parent must be on the list, so a shell or other program in between is refused.
