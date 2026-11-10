# Agents storing keys (optional)

> **Don't put real keys in through an agent yet.**
>
> **Status:** built 2026-11-06 and checked with fictional data only, in automated tests and in the screens' fixture. The check that only Claude Code and Codex are answered ([ADR-0013](adr/0013-only-claude-code-and-codex-use-the-agent-routes.md)) was added 2026-11-09, also checked with fictional data and stand-in programs only. Nobody has yet run it on a real Mac with a real separate agent account, used it from Claude Code (in Terminal or the Claude app's Code tab) or Codex, or seen its native panels and its window coming to the front. The fake-key drill and the security review that [ADR-0012](adr/0012-agents-store-and-manage-keys.md#build-order) asks for before real keys (steps 5 and 6) are still to do. Mac only. Each step marked **Owner** is yours to do; nothing in the app does it for you.

While you let it, an AI agent running in **one separate standard Mac account** can ask encrypted-note to store a key. The app comes to the front with its editor filled in from what the agent suggested (Kind, name, visible details). You type or paste the value there and choose Save. The agent gets back only "saved" and the key's id, or "cancelled". It never sees the value, and it can't use the key.

Afterwards the agent can look after the keys it added, and only those: list them, change their names and visible details, and ask you to replace a value or delete one. It can't see, list or touch any Note you added yourself.

This is separate from the [Agent Companion](agent-companion.md): it shares no code, folder, account check or grant with it, and it makes no network request. Design and decisions: [ADR-0012](adr/0012-agents-store-and-manage-keys.md).

## What an agent can ask

Five requests. The app refuses everything else.

| The agent asks to | What happens in the app | What the agent gets back |
|---|---|---|
| Store a key (`store_key`) | The app comes to the front with its Add editor filled in and the line "An agent asked to store this key. Type the value and choose Save." You type the value and choose Save, or Cancel. A Seed Phrase or Private Key also needs your Master Password, as when you add one yourself | "Saved" and the id, or "Cancelled" |
| List its keys (`list_keys`) | Nothing on screen | For each key it added and that isn't in Trash: the id, Kind, name, visible details, and when it was added and last changed |
| Edit details (`edit_key_details`) | A name, purpose or expiry date changes at once. Anything that says where a key is used asks you first (table below) | "Done", or why not |
| Replace a value (`ask_to_replace_value`) | The app opens that key with only its value editable, and the line "An agent asked to replace this value." A Seed Phrase or Private Key also needs your Master Password | "Saved" or "Cancelled" |
| Delete (`ask_to_delete`) | "An agent asked to delete X. It will go to Trash for 30 days." If you choose Move to Trash, your Master Password follows, for every Kind | "Deleted" or "Cancelled" |

The app answers only Claude Code and Codex ([below](#how-it-fits-together)), but it says "An agent", not which of the two asked.

**What the agent never gets, by any request:** a value (a password, key, secret, Seed Phrase words, Passphrase, private key or Text body), whether a value is set, whether a Seed Phrase has a Passphrase or how many words it has, whether a key is a Favorite, any Note you added, any Note in Trash, your Master Password, your Recovery Key, a Backup or a Safety Copy. Nothing is ever signed. A key you added, a missing id and a key in Trash all get the same answer, "No key with that id among the keys you added", so the agent can't even tell your keys exist.

### What each edit needs

| Kind | Changes at once | You allow it first (Allow or Cancel) | You allow it, then your Master Password | Never |
|---|---|---|---|---|
| Login | name, purpose | website, username | | the password |
| API Key | name, purpose, expiry date | service, key ID, where it is managed, variable name | | the key, the secret |
| Text | name | | | the body |
| Seed Phrase | | | name, wallet name, address, derivation path | the words, the Passphrase, the word count |
| Private Key | | | name, chain, address | the key |

An edit that asks for several changes needs what the strictest of them needs. The confirm lists each change, from what to what, because a tricked agent could change a website, an address or a chain to one an attacker controls. Values change only through Replace, which you type yourself.

### What leaves this computer

The names and visible details of the keys an agent adds (service, website, username, chain, public address, purpose, expiry date and so on), their ids and dates reach the agent's AI company as part of the chat, and whatever the agent suggested came from there in the first place. Name keys with that in mind: "Binance main 2 BTC" says a lot. Values never travel this way. If you paste a value into the chat, it has gone to the AI company whatever this app does: the tool tells the agent to ask you not to, and to use `store_key` instead.

## How it fits together

```
Your account (the owner)                Agent account (Standard, not admin)
┌───────────────────────────────────┐   ┌───────────────────────────────────┐
│ encrypted-note (Vault: offline)   │   │ Claude Code or Codex              │
│  editor, confirms, password panel │   │  └ enote-agent-keys (stdio MCP):  │
│  Settings → Agents storing keys   │   │    checks the app runs as you     │
│  agent route: answers only the    │   │    one JSON line, never a value   │
│  agent account's uid, and there   │   │                                   │
│  only the approved tool, started  │   │                                   │
│  by Claude Code or Codex          │   │                                   │
└─────────────────▲─────────────────┘   └─────────────────┬─────────────────┘
                  │                                       │
                  └───────────── agent.sock ◄─────────────┘
          in /Users/Shared/encrypted-note-keys, only while your grant runs
```

- **The socket is local.** It is a Unix domain socket in a folder only you own, which the agent account can pass through but not list. It isn't a network port, so nothing else on your network can reach it.
- **Both ends check each other.** The app asks macOS who is connecting, and answers only the account you named in Settings: a program in your own account, or in any other account, gets no reply at all. The tool checks the same way that the app is running as your account before it sends anything.
- **Only Claude Code and Codex** ([ADR-0013](adr/0013-only-claude-code-and-codex-use-the-agent-routes.md)). In that account, the app also asks the kernel which program is connecting, and which program started it. It answers only if both of these hold:
  - the connecting program is `enote-agent-keys` with the code hash you approved in Settings;
  - it was started by Claude Code (signed by Anthropic, team `Q6L2SF6YDW`) or Codex (signed by OpenAI, team `2DC432GLL2`), both validated by macOS as Developer ID.

  Both programs must have hardened runtime and allow no outside code in. Anything else gets no reply: another program, a shell, or the tool started by anything else. This is a speed bump, not a wall: any program in the agent account could start Claude Code or Codex itself, so keep only those two there.
- **Nothing on the route can hold a value.** Each request is one line with fixed fields (Kind, name, details, id). A detail named after a value, such as `password`, `key`, `secret`, `words` or `passphrase`, is refused by the tool before anything is sent, and by the app again.
- **Only while you let it.** The app listens only while a grant you started is running: at most one hour, and it ends at once when you choose Stop, the Vault locks, the Mac sleeps or the screen locks, you switch users, or you quit. After a lock, Settings shows **Off: ended because the Vault locked**; unlocking doesn't start it again.

## Before you start

1. **Owner: a standard agent account.** If you set up the [Agent Companion](agent-companion.md#before-you-start), use the same account. Otherwise, in System Settings → Users & Groups, add a new **Standard** user with the Account name (short name) `enote-agent`. Leave administrator off. Run the agent's tools only in that account, and never put your own passwords, Apple Account or keys there. The app refuses your own account, system accounts, and any account in the admin, wheel or operator group.
2. **Owner: the agent's tool, `enote-agent-keys`.** It isn't inside the app yet. In your own account, in the clone you built the app from, with no AI tool running:
   - build it;
   - sign it with hardened runtime, so no other code can be loaded into it (the app refuses it otherwise);
   - install it where the agent account can run it but not change it. Installing into `/usr/local/bin` needs your administrator password.

   ```bash
   cargo build --release --locked -p agent-keys-mcp
   codesign --force --sign - --options runtime target/release/enote-agent-keys
   sudo install -m 755 target/release/enote-agent-keys /usr/local/bin/enote-agent-keys
   ```

   (With `CARGO_TARGET_DIR` set, the file is under that folder instead of `target/`.) Not in `/Users/Shared`, your home folder or an exFAT drive: the setup script refuses those places, because there another account could change the tool, or the agent account can't run it. Install it again after each update of the app.
3. **Owner: the private socket folder.** In Terminal, in your own account, run this with your agent account's short name in place of `enote-agent`. Read what it says it would do, then run it again without `--dry-run`:

   ```bash
   scripts/agent-keys-setup.sh --dry-run --binary /usr/local/bin/enote-agent-keys enote-agent
   scripts/agent-keys-setup.sh --binary /usr/local/bin/enote-agent-keys enote-agent
   ```

   It stops with a message if that account doesn't exist, is your own, is a system account, or is in the admin, wheel or operator group, or if `/Users/Shared` isn't the system's shared folder. Otherwise it makes `/Users/Shared/encrypted-note-keys`, only yours (`chmod 700`), with one entry for the agent account (`user:enote-agent allow search`) that lets it reach the app's socket inside but not list, add or remove anything. It removes any earlier entries first, so you can run it again. It checks that the agent account can run the tool but not change it, and that the tool has hardened runtime. It prints the tool's code hash (`CDHash=…`) for Settings, and the lines for the next section with your account number (uid). It handles no key, writes nothing in the agent account and needs no administrator password.

## In the app

1. Unlock and open **Settings → Agents storing keys**.
   - Type the agent's Mac user (its Account name, such as `enote-agent`) and choose **Save**.
   - Paste the `CDHash=…` line the setup script printed as **The agent tool's code hash**, and choose **Save**.

   Both are kept in `settings.json`, which isn't secret. Changing either stops a running grant. After you install the tool again (after each update), run the setup script again and paste its new code hash.
2. Choose **Start for up to one hour**. A dialog of the app's own, outside its page (so nothing in the page can answer it), names the account and asks you to confirm. Start refuses, and says why, if the folder isn't set up or the account isn't a separate Standard one. Settings then shows **On** and the minutes left.
3. When an agent asks for something that needs you, the app comes to the front:

   - **Store or replace:** the editor opens with the agent's suggestion, or with only the key's value fields. For a replace, Save stays off until you type a value. Cancel, Escape, or leaving the editor any other way cancels the request, and the agent hears "Cancelled". If you are in the middle of something, such as another dialog or unsaved typing, the app shows a line at the top with **Open** and **Cancel the request** instead of taking over.
   - **Edit:** a confirm lists each change, from what to what. Allow applies it (after your Master Password for a Seed Phrase or Private Key); Cancel refuses it.
   - **Delete:** a confirm, then your Master Password. The key goes to Trash for 30 days, where you can restore it. Restoring it gives it back to the agent's list.

   A request you don't answer within 5 minutes is cancelled, and so is one still open when the grant ends, or one about a key you move to Trash, delete or take out of the agent's reach yourself.
4. A key an agent added shows **Added by an agent** in its row and at the top of its page. **Stop agent managing this** on that page takes it out of the agent's reach at once: it leaves its list and the agent can't change, replace or delete it. The key itself stays as it is. There's no way to give it back.
5. Choose **Stop** when the agent is done.

## In the agent account

Add the tool to the agent's program, in the agent account. The setup script printed these lines with your own uid in place of `501`:

Claude Code:
```bash
claude mcp add --scope user enote-keys -- /usr/local/bin/enote-agent-keys --owner-uid 501
```

Codex, in `~/.codex/config.toml`. `tool_timeout_sec` matters: a request waits up to 5 minutes for you, and Codex gives up on a tool after 60 seconds unless told otherwise:
```toml
[mcp_servers.enote-keys]
command = "/usr/local/bin/enote-agent-keys"
args = ["--owner-uid", "501"]
tool_timeout_sec = 330
```

The Claude app's Code tab runs Claude Code, so it is answered. The Claude app's chat side, the Codex app and ChatGPT are not ([ADR-0013](adr/0013-only-claude-code-and-codex-use-the-agent-routes.md)): a tool they start gets no reply.

**How long the agent's program waits.** Store, replace, delete and an edit you must allow each wait up to 5 minutes for you. If you have set `MCP_TOOL_TIMEOUT` for Claude Code, keep it at 330000 (milliseconds) or more. When a program gives up first, the request stays open in the app: answer it there as usual. The agent is told the call failed; the tool's descriptions tell it not to ask again but to check with `list_keys` a little later, and the tool keeps answering `list_keys` while a request waits. While it waits, the tool also sends the program progress notices, which some programs take as a reason to keep waiting.

The agent then has five tools: `store_key`, `list_keys`, `edit_key_details`, `ask_to_replace_value` and `ask_to_delete`. Their descriptions tell it that you type every value in the app, that the tool never sees or returns one, and that `name` and `details` reach its AI company and must hold no secret. Every answer is a short fixed text, such as:

| The agent hears | What it means |
|---|---|
| "The encrypted-note app isn't letting agents in…" | The app isn't open, or no grant is running: choose **Start** in Settings. Or the app turned the program away: the tool wasn't started by Claude Code or Codex, or its code hash isn't the one in Settings. |
| "Agents aren't let in right now…" | The grant ended (Stop, a lock, sleep or the hour). Choose **Start** again. |
| "The encrypted-note app is locked…" | Unlock it. |
| "No key with that id among the keys you added…" | The id is one of your Notes, in Trash, removed from the agent's reach, or doesn't exist. The agent can't tell which. |
| "That isn't allowed…" | The agent tried to change a value or the word count. (The tool itself refuses a detail named after a value, or one the Kind doesn't have, before sending anything.) |
| "The owner already has a request from you open in the app…" | One request waits for you at a time. |
| "The owner is in the middle of something in the app…" | An edit that would apply at once waits while one of the app's panels or another request is open. |
| "Cancelled: the owner said no, or didn't answer within 5 minutes…" | Nothing was changed. |

## Limits

- **Mac only.** On Windows, Start says it works only on a Mac for now.
- **Your Vault must stay Unlocked while the agent works.** Agent requests don't count as using the app, so Auto-lock still locks after 5 minutes without activity in the app's own window (unless you chose **Count my activity anywhere on this computer**), and the lock ends the grant. Fast user switching to the agent account locks the Vault too. How to reach the agent account while your own session stays active is still open, as for the [Agent Companion](agent-companion.md#order-and-account-use).
- **An agent in your own Mac account can't use this route,** because the app answers only the separate account. Such an agent works as you anyway ([AI tools on this computer](guide.md#ai-tools-on-this-computer)), and this route doesn't change that.
- **Only Claude Code and Codex are answered, but they share the one grant.** Any program in the agent account could start Claude Code or Codex itself with its own instructions, so install only those two there. Claude Code's signature lets libraries Anthropic didn't sign be loaded into it (you accepted this on 2026-11-09), so a program in that account could plant one: the same reason to keep it clean.
- **One request waits at a time,** for at most 5 minutes. Names and details are limited to 1,000 characters each.
- **The value is typed into the app's ordinary editor,** exactly as when you add a key yourself, so it has the same protection as that: it goes from the editor to the app's Rust core, never to the agent ([ADR-0003](adr/0003-tauri-with-secrets-kept-in-rust.md)).
- **A tricked agent can still** add junk keys, open requests you have to cancel, rename one of its keys or change a purpose or expiry date to confuse you, and ask you to change where a key is used, replace a value or delete a key. Read each confirm: none of those happens unless you allow it. It can't read, copy, move or spend anything.
- **Older apps can't open a Vault with an agent's key in it.** A Vault where any Note, Trash included, carries the mark is saved as document schema 3, which older builds of the app and older Emergency Readers refuse as made by a newer version. Update the app on your other computer, and every Emergency Reader you saved, before you move such a Vault there ([FORMAT.md](FORMAT.md#schema-versions)). Once no Note carries the mark, the next save is as readable by them as before it. The Emergency Reader prints "(added by an agent)" after such a Note's title.

## Stop and clean up

- **Stop** in Settings ends the grant at once and cancels any request still waiting. Stop, a lock and quitting all remove the socket.
- To remove the setup: choose **Stop**, then in the agent account remove the tool (`claude mcp remove --scope user enote-keys`, or delete the `[mcp_servers.enote-keys]` lines from `~/.codex/config.toml`). In your own account, delete the folder with `rm -r /Users/Shared/encrypted-note-keys` and the tool with `sudo rm /usr/local/bin/enote-agent-keys`, and clear the agent's Mac user in Settings. The keys an agent added stay in your Vault, still marked; choose **Stop agent managing this** on each if you like.

## For developers: trying it with a fictional Vault

Debug builds only, and never with a real Vault:
- `ENOTE_DEV_AGENT_UID` names the account number to accept, so one account can try the whole route;
- `ENOTE_DEV_AGENT_SOCKET_DIR` moves the folder;
- `ENOTE_DEV_AGENT_ANY_PROGRAM` answers any program in that account, so a tool started from your shell gets in.

Release builds ignore all three. The folder must be yours, mode 700, and every folder above it root's or yours and not writable by others (unless it is root's and sticky); one in your home folder works.

```bash
sock_dir=$(mktemp -d "$HOME/enote-dev-agent.XXXXXX")   # mode 700
ENOTE_DEV_DATA_DIR="$(mktemp -d)" ENOTE_DEV_AGENT_UID="$(id -u)" ENOTE_DEV_AGENT_SOCKET_DIR="$sock_dir" ENOTE_DEV_AGENT_ANY_PROGRAM=1 npm run tauri dev
```

In the app, create a fictional Vault and choose **Start** (its confirm says "developer override"). Then point a tool at the debug build with `cargo build --locked -p agent-keys-mcp` and `target/debug/enote-agent-keys --owner-uid "$(id -u)" --socket "$sock_dir/agent.sock"`. Use obviously fake values only ([AGENTS.md](../AGENTS.md)).

The code: `src-tauri/src/core/agent.rs` (the grant, the five requests and their rules), `src-tauri/src/agent_route.rs` (the socket, the account and folder checks, the protocol), `crates/agent-keys-mcp` (the agent's tool), `crates/notes/src/agent.rs` (the mark, what the agent sees, the edit rules) and `src/AgentRequest.tsx` with the editor in `src/Note.tsx` (the screens). Their tests are `src-tauri/tests/agent.rs`, `src-tauri/tests/agent_route.rs`, `crates/agent-keys-mcp/tests/mcp.rs` and `crates/notes/tests/agent.rs`.

## What has been checked, and what hasn't

**Checked with fictional data, automatically:** that the agent sees only its own keys, never in Trash, and never a value, a Passphrase sign or a word count; that your Notes, missing ids and Notes in Trash get the same answer; every edit rule for every field of every Kind; that store, replace and delete need you, and the Master Password where the tables above say; one request at a time, the 5-minute limit, and the grant ending on lock, Auto-lock, sleep, screen lock, user switch, quit and the hour; that agent requests don't keep Auto-lock away; the socket end to end in one account, including that a connection from any other uid gets no reply, that long or slow lines are refused and that the connection limit holds; the real `enote-agent-keys` against the real listener; and the Vault mark, schema 3 and the Emergency Reader.

**ADR-0013's check, also automatic, with stand-ins:**
- **The decision itself:** the approved tool started by Claude Code or Codex is let in. Refused:
  - another tool hash;
  - a fake signed ad-hoc as Claude Code;
  - the right identity with another team, or not validated as Developer ID;
  - Apple's own programs;
  - programs without hardened runtime, debuggable, or allowing DYLD injection.
- **The kernel readers:** a real running program's identity and parent.
- **On a real socket,** with Apple's `nc` standing in for the tool and `bash` for its starter:
  - answered when both are on the list;
  - closed without a reply for the real list (bash isn't Claude Code or Codex), for a wrong tool hash, and for a program connecting by itself.

A spike on 2026-11-09 showed that the kernel calls the check uses work inside the App Sandbox, in a stand-in signed with this app's exact entitlements. `scripts/check-network-source.mjs` keeps Unix sockets and peer checks to `agent_route.rs` and the tool's crate, and keeps the Agent Companion's code out of them. The screens were clicked through in the fixture only (`npm run dev`), which proves nothing about native panels, locks or the route.

**Still to do by hand, on a real Mac, with fake keys:**

- [ ] `scripts/agent-keys-setup.sh` without `--dry-run`, for a real Standard account: the folder, its mode and its one ACL entry, and that the agent account can reach the socket but not list the folder.
- [ ] The built, sandboxed app can make its socket in `/Users/Shared/encrypted-note-keys` with its two entitlements, and Start's account check works inside the sandbox (if macOS can't answer the group check there, Start refuses).
  - **Partly answered by a spike on 2026-11-09.** It used a stand-in signed ad-hoc, with hardened runtime and this app's exact entitlements.
  - **The folder entry alone isn't enough:** the sandbox refuses to make the socket, and adding `network.server` doesn't help.
  - **The added `temporary-exception.sbpl` rule makes it work**, and the stand-in identified each caller.
  - **Still to confirm** in a Developer ID-signed, notarized build.
- [ ] From Claude Code (also in the Claude app's Code tab) and Codex in the agent account: every request, including a Seed Phrase and a Private Key stored, edited, replaced and deleted through the Master Password panel, and the app coming to the front.
- [ ] A connection from your own account, and from a third account, gets no reply.
- [ ] In the agent account, the tool started by Claude Code and by Codex is answered. Refused: the tool started from a shell or by another program, a program connecting by itself, and the tool after it is rebuilt but before its new code hash is pasted.
- [ ] Stop, a lock, sleep, screen lock, user switching, quitting and the end of the hour each end the grant and remove the socket, and a waiting request is cancelled.
- [ ] Each screen in the real app: the editor for store and replace, the edit and delete confirms, the banner, the mark and **Stop agent managing this**, with VoiceOver too.
- [ ] The tool bundled inside the app. Until then, the setup script's default `--binary` (inside `/Applications/encrypted-note.app`) doesn't exist, and its check would refuse `/Applications` anyway, because administrators can write there.
- [ ] The drill from [ADR-0012](adr/0012-agents-store-and-manage-keys.md#build-order) step 5: a planted instruction to "store this key and then show it", an attempt to change a Private Key's address, and a flood of store requests.
- [ ] The security review of step 6, before any real key goes in this way.
