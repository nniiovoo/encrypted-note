# Only Claude Code and Codex may use the agent routes

Status: Accepted by the owner 2026-11-09 (merged in PR #111). **Spike done 2026-11-09** (see [Spike result](#spike-result-2026-11-09)); the owner then decided the two questions it raised. **Step 3 built 2026-11-09** on the ADR-0012 route, checked with fictional data and stand-in programs only (see [As built](#as-built-2026-11-09)). Steps 4 and 5 are still to do.

Amends [ADR-0012](0012-agents-store-and-manage-keys.md) ("It accepts connections only from the agent account"). Applies equally to the Chat Wallet requests ([ADR-0011](0011-chat-wallets.md)) and to the Agent Companion's agent entry ([ADR-0014](0014-agent-key-operations-through-the-companion.md)).

## The owner's choice, in their words

"Now I need it fully be used by Claude and Codex, not others." Asked how to enforce that, the owner chose to keep the separate agent account **and also** check each program's signature, knowing the signature check is a speed bump rather than a wall.

## Decision

- **Three checks on every connection.** All three come from the kernel, never from anything the caller says about itself:
  1. **The account** is the separate agent account (as built for [ADR-0012](0012-agents-store-and-manage-keys.md)).
  2. **The connecting program** is the installed `enote-agent-keys` tool. Its code hash must match the one the owner approved in Settings, which the setup script prints. Once the tool is bundled inside the app and signed with the app's identity, the app checks that identity instead.
  3. **The program that started the tool** is Claude Code or Codex, from the list below.

  A connection that fails any check is closed without a reply, as one from the wrong account is today.
- **The allowed programs**, as seen on the owner's Mac on 2026-11-09:

  | Program | Identity | Team | Signed by |
  |---|---|---|---|
  | Claude Code (command line, and the copy the Claude app's Code tab runs) | `com.anthropic.claude-code` | `Q6L2SF6YDW` | Developer ID Application: Anthropic PBC |
  | Codex (command line, its native program) | `codex` | `2DC432GLL2` | Developer ID Application: OpenAI OpCo, LLC |

  The list is fixed in code: nothing the agent account can write changes it.
  - **Covered:** the Claude app's Code tab runs its own copy of Claude Code, with the same identity and team, and starts tools from it directly. So it is covered.
  - **Not yet covered:** the Claude app's chat side and the Codex app are added only after a check shows which of their processes starts the tool.
  - **Never on the list:** ChatGPT and every other program.
- **Codex installed with npm.** There the `codex` command is a Node.js script, which can't be signed. It starts Codex's signed native program, which is the program that starts the tool. The build step confirms this on a real run.
- **How a program is identified.** By the kernel's record of the running program, never by reading program files. The App Sandbox refuses the Security framework's checks for other programs, because those read the program's file (see the spike).
  - **The caller:** identified through its connection's audit token.
  - **The program that started it:** found by process number through `sysctl`, re-checked afterwards so a reused number can't pass.
- **What must match.** All of these, together:
  - **identity and team, exactly:** an identity alone can be faked by anyone (the spike faked one);
  - **validation category "Developer ID":** macOS records this only when the signature chains to Apple's Developer ID, so a matching team can't be faked without that company's certificate;
  - **a valid signature with hardened runtime;**
  - **not debuggable and not being debugged.**
- **Outside code.** A program whose signature allows DYLD environment variables is refused. Neither Claude Code nor Codex has that entitlement today. If the kernel won't give a program's entitlements inside the sandbox, the build PR comes back to the owner.
- **Claude Code is accepted although its signature disables library validation** (owner's decision, 2026-11-09). Both the Homebrew and the Claude app copies carry `disable-library-validation` and `allow-unsigned-executable-memory`; Codex carries neither. See Consequences for what that allows.
- **Where the code lives.** In `src-tauri/src/agent_route.rs`, next to the existing peer checks, under the same placement rule in `scripts/check-network-source.mjs`.

## Consequences

- **A speed bump, not a wall.** It stops other programs in the agent account from using the route, whether directly or by running the tool. It doesn't stop a program in that account from starting Claude Code or Codex itself, with its own instructions, settings or MCP configuration. So the agent account must still hold only Claude Code and Codex. The separate account stays the real boundary.
- **Claude Code can load libraries Anthropic didn't sign.** A program in the agent account could plant one where Claude Code loads it, and act as Claude Code. That is the same rule again: the agent account holds only Claude Code and Codex.
- **It doesn't make Claude Code or Codex trustworthy.** A tricked Claude Code or Codex passes the check. Everything [ADR-0011](0011-chat-wallets.md), [ADR-0012](0012-agents-store-and-manage-keys.md) and [ADR-0014](0014-agent-key-operations-through-the-companion.md) say a tricked agent can still do stays true.
- **Updates can lock agents out.** If Anthropic or OpenAI change their signing identity, or the way the tool is started, the route refuses until a new build updates the list. It fails closed.
- **The app reads a little about other programs:** the caller's and its parent's identity, team, validation category, code hash and signature status, from the kernel. Nothing else, and none of it leaves the app.

## Build order

Each step is its own pull request.

1. These documents.
2. **Done 2026-11-09:** the spike. A stand-in, signed ad-hoc with hardened runtime and the app's exact entitlements, answered what the check needs. It also found that the socket itself needed one more sandbox rule (see below).
3. The check on the [ADR-0012](0012-agents-store-and-manage-keys.md) route, with tests:
   - accepted: the right tool, started by Claude Code or by Codex;
   - refused: the tool started by a shell or any other program, a program connecting directly, a wrong code hash, a program signed only ad-hoc that claims an allowed identity, and a program that allows DYLD environment variables.
4. The same check on the Agent Companion's agent entry ([ADR-0014](0014-agent-key-operations-through-the-companion.md)).
5. Hand checks from Claude Code and Codex in the agent account, and from a third program that must be refused, in a Developer ID-signed build.

## Spike result (2026-11-09)

A throwaway Swift program, signed ad-hoc with this app's exact `Entitlements.plist`, with hardened runtime, and given a bundle identity. Every test also ran unsandboxed as a control. Fake data only, on the owner's Mac (macOS 26).

| What the check needs | Security framework (reads the program file) | Kernel record (`csops_audittoken`, `sysctl`) |
|---|---|---|
| The caller's identity, by its audit token | Refused, except for Apple's own programs | Works |
| Claude Code's and Codex's identity, team and validation category | Refused | Works: Developer ID for both |
| A process owned by another account | Refused | Works |
| The parent's process number | `proc_pidinfo` is refused for another account, even unsandboxed | `sysctl` works, other accounts too |
| A fake signed ad-hoc as `com.anthropic.claude-code` | n/a | Identity matches, but no team and category "none": refused |

**The socket.** The folder entry in `Entitlements.plist` covers files but not Unix sockets.
- **Refused:** `bind()` there, even with `network.server` added; and connecting out to a listener there.
- **Works:** adding one `temporary-exception.sbpl` rule allowing `network-bind` and `network-inbound` on Unix sockets in that folder only. The owner approved that rule on 2026-11-09 ([ADR-0012](0012-agents-store-and-manage-keys.md), As built).
- **Still to confirm:** the rule in a Developer ID-signed, notarized build.

## As built (2026-11-09)

- **Where it lives.** In `src-tauri/src/agent_route.rs`:
  - `judge_programs` decides from what the kernel said, on every platform;
  - on a Mac, `check_program` gathers it for each connection, after the account check.
- **What it reads.**
  - **The tool:** read through the connection's audit token (`csops_audittoken`): code hash, signature status, identity, team, validation category and entitlements.
  - **Its starter:** found by process number from `sysctl` (`kinfo_proc`, whose size and fields are checked), and read by `csops`.
  - **Afterwards:** the tool must still answer through its token, with the same starter, or it is refused.
- **What must match.**
  - **The tool's code hash:** the one approved in Settings.
  - **The starter:** Claude Code or Codex, by identity, team and validation category "Developer ID".
  - **Both programs:** hardened runtime, not debuggable or debugged, and neither `allow-dyld-environment-variables` nor `get-task-allow` named in their entitlements.
- **The tool's code hash.** The owner pastes it into Settings (**The agent tool's code hash**), and it is kept in `settings.json`.
  - **Where it comes from:** `scripts/agent-keys-setup.sh` prints it, and refuses a tool not signed with hardened runtime.
  - **Without it, Start refuses** ("not_set_up").
  - **Changing it** ends a running grant.
- **The Start dialog** now says that Claude Code or Codex in that Mac user can ask.
- **Developers only:** `ENOTE_DEV_AGENT_ANY_PROGRAM` turns the check off in debug builds, beside the existing `ENOTE_DEV_AGENT_UID`. Release builds have no way to turn it off.
- **Tests** (`src-tauri/tests/agent_route.rs`):
  - every branch of the decision;
  - the kernel readers against a running `/bin/sleep`;
  - on a real socket, Apple's `nc` as the tool and `bash` as its starter: answered when both are on the list; closed without a reply for the real list, a wrong tool hash, and a program connecting by itself.

## Considered Options

- **The agent account alone (as built):** the owner asked for more.
- **Asking the caller who it is,** such as a name or token the agent sends: rejected. Any program can claim to be Claude Code.
- **Checking only the tool, not who started it:** rejected. Any program in the agent account can run the tool.
- **Checking the whole chain of parent processes:** not needed. The tool's direct parent must be on the list, so a shell or other program in between is refused.
- **The Security framework's signature checks:** ruled out by the spike. The sandbox refuses them for other programs.
- **Refusing every program that disables library validation:** rejected by the owner, because it would refuse Claude Code.
