# User guide

## Where your keys are stored

Your Vault is saved in one folder in your user account on this computer (table below). In the app: Settings → Where your keys are stored.

| Computer | Vault folder |
|---|---|
| Mac, the built app | `~/Library/Containers/com.encryptednote.app/Data/Library/Application Support/com.encryptednote.app/Vault.noindex` |
| Mac, a release build that isn't sandboxed | `~/Library/Application Support/com.encryptednote.app/Vault.noindex` |
| Mac, a test run (`npm run tauri dev`) | `~/Library/Application Support/com.encryptednote.app/Vault.dev.noindex`. A Vault from an older test run stays in `Vault.noindex` beside it. A Windows test run uses `Vault.dev.noindex` in the Windows folder. |
| Windows | `%LOCALAPPDATA%\com.encryptednote.app\Vault.noindex` |

| What | Where | Encrypted? |
|---|---|---|
| Your Notes, including API Keys, Private Keys and Seed Phrases | `vault.enote` in the Vault folder | Yes. Seed Phrases and Private Keys are encrypted twice. Both layers open with your Master Password or your Recovery Key. |
| The keys that lock your Vault | Inside `vault.enote`, locked by your Master Password and by your Recovery Key | Yes. |
| Safety Copies | `safety/` in the Vault folder | Yes. Each opens with the Master Password and Recovery Key it had when saved. A Note you deleted or changed stays inside older copies until the app removes them. It keeps your 10 most recent saves, the first save of each day for 30 days, and a Vault replaced by a Backup or restored copy for at least 30 days. |
| Master Password | Not in any file | Only you know it. |
| Recovery Key | Not in any file | Only on the paper where you wrote it. |
| Touch ID (Mac, if you turn it on) | `quick-unlock.sealed` in the Vault folder | Yes. It holds a protected copy of the key that opens your Logins, API Keys and Text. It works only on this Mac, with a fingerprint saved on this Mac. It can't open the secret parts of Seed Phrases or Private Keys. |
| Settings | `settings.json` in the Vault folder | No. Choices, dates, the count of wrong password tries, your Vault's id and a count of its changes (so the app can warn if an older copy opens), the agent's Mac user name if you set up agents storing keys and, on a Mac, a check value for Touch ID; no Notes, titles or keys. Other programs in your account can change this file, so the Backup reminder is a nudge, not proof that a Backup exists, and the older-copy warning catches mistakes, not someone who changes this file too. |
| Backups | Wherever you saved them. The app suggests the name `encrypted-note-backup-YYYY-MM-DD-HHMM.enote`, with the date and time in UTC | Yes. Each opens with the Master Password and Recovery Key it had when saved. The app doesn't remember where. A Note you deleted or changed stays inside an old Backup for as long as you keep it. |
| Copied values | Clipboard for 30 or 60 seconds (Seed Phrases and Private Keys always 30), then cleared | No. Other apps could read it during that time. If the app crashes, it can stay until you copy something else. |
| Open Notes | App memory while Unlocked | Cleared on lock and quit, as far as the computer allows. The key that opens your Notes, unsaved typing and shown values are in memory too, and the computer may write memory to disk. |
| Agent Key (only if you set up the Agent Companion) | `credential.env` in the helper's folder: `~/Library/Containers/com.encryptednote.app.companion/Data/Library/Application Support/encrypted-note-companion/` | **No. Plain text.** Other non-admin Mac accounts can't read it, but other software running under your account may be able to. While the Agent Companion runs, the key is also in the helper's memory. The file you made for setup also holds the key until you delete it. Deleting a file doesn't cancel the key: cancel it at OpenAI. |
| Agent token (only while the Agent Companion runs) | `/Users/Shared/encrypted-note-companion/agent-token` | No. Made only for the agent account. Removed when the Agent Companion stops normally; each Start makes a new one. Not your Agent Key. |
| Agents storing keys (only if you set it up) | `/Users/Shared/encrypted-note-keys/agent.sock`, only while a grant runs | Nothing is stored there: it's the app's socket, which only the agent account you named can use. The keys an agent asked to store are Notes in `vault.enote`, encrypted like any other. Removed when the grant ends normally. |

Nothing is kept in Apple Keychain or on any server.

Time Machine or other full-computer backups may also copy these folders. Your Vault stays encrypted there, but old copies open with the old Master Password and Recovery Key, and the Agent Key file stays plain text.

**What to keep safe:** the paper with your Recovery Key, somewhere physical and apart from your Backups (never photograph it or save it on a computer), and a Backup made with Back up now. Nothing else in the folder needs copying. The paper can't bring your Notes back by itself: it opens a Vault file or a Backup, so you need one of those too. Keep at least one Backup on a drive you unplug afterwards, not the drive that holds code, and never in a project folder, because `git clean -fdX` deletes `*.enote` files there. For a Seed Phrase or Private Key that holds real funds, also keep a paper or metal copy: the Vault shouldn't be your only one.

**If the Vault file goes missing, or an older one is put back.** If `vault.enote` is gone, Welcome offers the Safety Copies still in the folder. If there are none but `settings.json` shows a Vault was here, it says "Your Vault's files are missing" and offers **Open a Backup** first. A new Vault starts empty. If a Vault file was put back while the app was closed, for example from Time Machine, and it is older than one the app saved or opened here, the app says so after you unlock. If you didn't restore it yourself, look in Settings → Safety Copies, or open your newest Backup. Both checks need `settings.json`, so they catch mistakes, not someone who changes or deletes it as well.

**If you lose the paper:** if you still know your Master Password, you can make a new Recovery Key. In Settings, go to Change Master Password and choose "Someone may know it, or I lost my paper". You also choose a new Master Password there, and then write down the new Recovery Key. Touch ID turns off, and you can turn it on again. Then choose Back up now. Your older Backups and Safety Copies still open only with the old password and the old key, and anyone who finds the old paper could open those old copies. The app's message at the end says where they are.

## AI tools on this computer

encrypted-note gives AI agents no interface to your Vault, apart from one narrow, optional route: while you let it, an agent in a separate Mac account can ask the app to store a key you type into the app yourself, and manage only the keys it added, without ever seeing a value ([Agents storing keys](agent-keys.md); built, but checked with fictional data only). That limits the app, not your computer. Apart from that route and the optional [Agent Companion](agent-companion.md), it has no tool, server or file for them, and the Companion's one text tool can't open your Vault. That doesn't stop an AI tool that runs in your own account on this computer, such as Claude Code, Codex or Cursor. It works as you, so it may be able to:

- **Copy your Backups.** They stay encrypted, but whoever has a copy can keep guessing passwords on it for as long as they like, and it keeps opening with the Master Password and Recovery Key it was saved with.
- **Copy your Vault folder.** The copy is encrypted, like a Backup. For the built Mac app on macOS 14 or later, macOS should ask before Terminal or another app opens that folder. If you allow Terminal, every tool you run in it gets that access until Terminal quits, and an app with Full Disk Access is never asked. On macOS 13, and for a release build that isn't sandboxed or a test run (Vault in `~/Library/Application Support`), nothing asks.
- **Ask for Touch ID.** With the Vault folder, it can copy `quick-unlock.sealed` and ask macOS for Touch ID itself. If you touch the sensor then, it gets the key to your Logins, API Keys and Text.
- **Read what you copy** while it's on the clipboard (30 or 60 seconds).
- **Read your Agent Key.** The setup file `~/enote-agent-key.env` is plain text until you delete it, and the helper's copy may be readable too (table above). Deleting a file doesn't cancel the key; revoking it at OpenAI does.
- **See and click.** A tool you allow to record or control your screen ("computer use") can see what you show and press buttons in an Unlocked app. Show and Copy work only while the app's window is in front, which stops a press from the background but not a tool that brings the window forward first. Capture Hiding is best effort. On a Mac the Sharing Guard covers the window while the AI screen recorder Screenpipe runs, but other AI assistants aren't on its list, and it can't see a recorder started from a terminal.

On Windows nothing asks first: any program in your account can copy the Vault folder (`%LOCALAPPDATA%\com.encryptednote.app\Vault.noindex`), your Backups and the clipboard. Touch ID and the Agent Companion are Mac only. This hasn't been checked on a real Windows PC yet ([#11](https://github.com/nniiovoo/encrypted-note/issues/11)).

This project's [AGENTS.md](../AGENTS.md) tells agents never to open, copy, delete or overwrite your Vault or Backups, but that's an instruction, not a lock.

**Safest: run AI tools in a separate standard account, not an administrator,** as the [Agent Companion setup](agent-companion.md#before-you-start) does on a Mac. Never sign that account in to your Apple Account or put your passwords or keys there. On a Mac it can't open your Library, Desktop, Documents or Downloads folders, so it can't reach your Vault folder or the helper's Agent Key file, and the app saves Backups so that only your account can open them. It can still see the top of your home folder and read your Public folder, `/Users/Shared` and any file you saved at the top of your home folder yourself. That's why the [setup](agent-companion.md#before-you-start) makes `~/enote-agent-key.env` readable only by you, with `scripts/companion-setup.sh` for the fake key and `write-agent-key.mjs` for the real one: don't make that file another way. A Backup on a USB stick or exFAT drive has no owner, so any account can read it. None of this, and whether the clipboard, screen and typing stay apart between accounts, has been tested on a real Mac yet ([checks](agent-account-validation.md)).

**If you run them in your own account:**

- Mac: keep Touch ID off while AI tools run in your account (Settings → Touch ID). Turn it off before you start one, because turning it off later doesn't stop a copy a tool already made. If you leave it on, touch the sensor only right after you asked encrypted-note to unlock.
- Lock or quit encrypted-note before you let an AI tool control your screen, and put encrypted-note on the tool's blocked or denied apps list if it has one.
- Don't copy a secret while an AI tool is working. If you just copied one, wait until the clipboard clears (30 or 60 seconds) before you give the tool a task. If encrypted-note quit unexpectedly, copy a harmless word to replace the secret.
- Keep Backups on a USB stick you unplug. Don't leave a copy anywhere else on this computer, on a drive that stays plugged in, or in a cloud folder.
- Mac: say no when macOS asks whether Terminal or an AI app may use data from other apps, unless you're running the [Emergency Reader](#if-the-app-wont-open) yourself, and don't give either Full Disk Access. macOS should keep a yes only until that app quits, so quit it (Cmd+Q) to undo one. To remove Full Disk Access, turn it off in System Settings → Privacy & Security → Full Disk Access.
- Mac: delete `~/enote-agent-key.env` once the agent's first call through the Agent Companion has returned text, and empty the Trash. If a tool may have read it or the helper's copy, revoke the key at OpenAI.
- Turn on the tool's sandbox and keep it asking before it runs commands (in Claude Code the sandbox stays off until you run `/sandbox`). Avoid modes that skip these, such as "full access" or "bypass permissions". If it can block files, block the Vault folder, the helper's folder `~/Library/Containers/com.encryptednote.app.companion`, `~/enote-agent-key.env` and `*.enote`.
- Never paste a Seed Phrase, key, Master Password or Recovery Key into an AI chat. If an agent needs a key, see [When an agent needs a key](#when-an-agent-needs-a-key).

**If you touched the sensor for an unlock you didn't start:** treat your Logins, API Keys and Text as known to that tool, and change those passwords and keys where you use them. Then, in Settings, go to Change Master Password and choose "Someone may know it, or I lost my paper". That also replaces the key Touch ID gave out; a normal password change keeps it. Then choose Back up now. Older Backups and Safety Copies still open with the old key. Touch ID can't open the secret parts of Seed Phrases or Private Keys.

These lower the risk without removing it: a tool's file rules can miss a script the tool writes for itself. The macOS steps here have not been checked by hand.

**The Agent Companion needs your Vault Unlocked.** Its grant ends whenever the Vault locks. By default Auto-lock locks after 5 minutes without activity in the app's window, and work in Terminal or another app counts as idle unless you turn on Settings → Auto-lock → Count my activity anywhere on this computer. So keeping the agent's tool running means keeping your Vault Unlocked while the agent works.

## When an agent needs a key

Keys in your Vault are for you. The app gives agents no way to use them, and an Agent Key is never a value stored in the Vault ([ADR-0007](adr/0007-agents-use-separate-restricted-keys.md)). The [Agent Companion](agent-companion.md) has its own Agent Key for its one tool.

**If you want an agent to put a key in your Vault for you,** for example while it sets something up, see [Agents storing keys](agent-keys.md): the agent asks, the app opens its editor, and you type the value there. The agent can then manage that key's name and details, but it never sees the value and can't use it. It isn't a way to give an agent a key to use. That route is built but has been checked with fictional data only, so don't use it for real keys yet.

When an agent needs a key for anything else, such as GitHub or a cloud service:

1. Your main key stays in the Vault.
2. At the service's website, make a new key for this one job, with the fewest permissions, the shortest expiry and any spending cap the service offers. Don't give it permission to make other keys.
3. Keep it in a private file outside any repository, in the account where the agent runs. Never put it in a project's `.env` file, on a shared drive or an exFAT drive (exFAT has no owner-only permissions), or in a chat. On a Mac, copy the new key at the service, run this, then copy something else so the key leaves the clipboard. It leaves nothing in shell history:

   ```bash
   (
     umask 077
     key_file=$(mktemp "$HOME/agent-job-key.XXXXXX") || exit 1
     printf 'SERVICE_API_KEY=%s\n' "$(pbpaste)" > "$key_file" && printf 'Saved key file: %s\n' "$key_file"
   )
   ```

   This creates a fresh private file with a different name each time; it never overwrites an earlier key file or follows an existing link. Use the file name it prints when setting up your agent tool.

   If the agent runs in a separate account, how to get the key into that account is still open ([Order and account use](agent-companion.md#order-and-account-use)).

4. Treat the key as already seen by the agent and by its AI company.
5. When you're done, cancel it at the service, then delete the file and empty the Trash.
6. Never copy a Vault value for this. **Copy as NAME=value** on an API Key copies your own key: use it for your own programs, not an agent's.

A key pasted into an AI chat is copied into that tool's transcripts on this computer and sent to its company, which may keep it for a long time, depending on your account's settings. Deleting the chat doesn't undo that. Only cancelling the key at the service does.

To keep track of a key you made for an agent, you can save an API Key Note with the key left empty: the service, where it is managed (where you cancel it) and the day it expires. Never type the key itself into it.

If agents run in your own Mac account, a private file doesn't keep the key from them, so narrow permissions and a short expiry are what limit the damage. In a separate standard account, the key file is readable by every tool in that account, but not by tools in your own account ([AI tools on this computer](#ai-tools-on-this-computer)); that hasn't been tested yet.

Many services don't offer every limit ADR-0007 asks for, most often a hard spending cap, so a key you make by hand for an agent is a risk you accept. From each service's own documentation on 2 November 2026 (nothing was tried): GitHub fine-grained tokens and Cloudflare API tokens can be narrowed and can expire, but have no spending or usage cap of their own; a Stripe restricted key has no expiry you can set when you make it; AWS budgets update a few times a day, so spending can pass one before it acts; Anthropic API keys can expire, and a workspace spend limit exists, but not on the Default Workspace. Check again before you rely on this, because services change.

## Keyboard shortcuts

Cmd on Mac, Ctrl on Windows.

| Keys | Action |
|---|---|
| Cmd/Ctrl+L | Lock |
| Cmd/Ctrl+F | Search (arrow keys move through the list, Enter opens, Esc clears) |
| Cmd/Ctrl+N | New Note (then **1** to **5** pick its Kind) |
| Cmd/Ctrl+S or Cmd/Ctrl+Enter | Save |
| Cmd/Ctrl+Shift+C | Copy the open Note's first Hidden Field |
| Esc | Leave the Editor (asks first if anything changed) or the Kind picker |

Copy feedback names the Note and field that reached the clipboard. The shortcut still targets the open Note while search highlights another Note. Lock and Sharing Guard remove the source title from feedback.

## Opening the Vault and Wallet values

You type your Master Password and Recovery Key on a separate screen inside the app window, made with your computer's own controls. The normal app page can't read what you type there, but this doesn't stop malware already on your computer. Your Recovery Key is not in any file: the app shows it once, and the only copy is the one you write on paper. To show a stored Seed Phrase or Private Key you type your Master Password again, and it appears on that same screen. New or edited values, including new Seed Phrases and Private Keys, are typed in the normal Note editor in the app's window, so they don't get this extra protection.

**Checking what you typed.** Password boxes show dots. Nothing is shown unless you ask, and each option starts off every time:

- Recovery Key box, Mac only: tick **Show what I type**. The box keeps its dots, and a separate line shows the characters. It turns off after 1 minute with no typing. Tick it again to see the key. On Mac and Windows the box also counts the characters and checks for typing mistakes, which shows nothing secret.
- New and Confirm Master Password boxes, Mac only: click the box you want to check, then press and hold **Hold to show what I typed**. A line above the text names the box. If that box is empty, or you use the keyboard to hold, it shows the box you typed in last. It lasts about 10 seconds at most, and it can't be used until one of the two boxes has text.
- Note editor: press **Show** for a Login password, or for an API Key's key or secret. The value appears on a line under the box. It hides after 30 seconds, or when you leave the window. Seed Phrases and Private Keys never have this button.

The first two can't be used while a known screen-sharing or recording app is running, even if you dismissed the Sharing Guard. A line on the screen says so. Quit that app, then open the screen again. All of them put the text on your screen, and keeping the window out of captures is best-effort, so stop any screen share first. These options have not been tested by hand yet.

**Suggest a password** shows the new password in plain text on the same screen, on Mac and Windows, so you can write it down. It does this even while a known screen-sharing or recording app is running, if you dismissed the Sharing Guard, so stop any screen share first.

One box is different: when the app gives you a new Recovery Key, the box where you type its last two groups shows what you type, because the key is on screen right above it. You can't copy or cut from that box, and right-clicking it does nothing. You can still paste into it with Cmd+V on Mac or Ctrl+V on Windows.

**Using a password manager.** Switching to another app closes the password box, so copy your Master Password before you start, then paste it in the box that opens (Cmd+V on Mac, Ctrl+V on Windows). Don't do this while an AI tool runs in your account: it can read the clipboard. A password you paste from another app gets none of this app's clipboard protection: other apps and clipboard history can read it, and this app can't clear it. When you finish, copy something else to replace it on the clipboard. This doesn't remove it from a clipboard history app. Pasting into these boxes has not been tested by hand yet.

On a supported Mac, open Settings → Touch ID → Turn on Touch ID. Type your Master Password, then touch the sensor. This uses your Mac's security chip and a small encrypted file. It needs no account, server or Apple Developer membership. If Touch ID isn't available on this Mac, use your Master Password.

After every app restart, use your Master Password before Touch ID. Enter it again at least every seven days; Touch ID never moves that deadline. Changing your Master Password, using your Recovery Key, Key Rotation and installing a Backup or Safety Copy turn Touch ID off, and you have to turn Touch ID on again. Wallet actions continue to require their own fresh Master Password. Choosing Forgot it? Use your Recovery Key turns Touch ID off as soon as the box opens, even if you cancel. Starting to change your Master Password does the same. A missed finger, a lockout, a closed lid or a disconnected Touch ID keyboard leaves Touch ID on. Changing the fingers saved on this Mac turns it off. So does a Vault file replaced while the app was closed by another Vault, or by a copy from before a change of Master Password or Recovery Key: the Lock screen and Settings then say so. A copy with the same Master Password and Recovery Key keeps Touch ID on, and the older-copy warning after unlock covers it.

To turn Touch ID off, use the same Settings section. This also deletes its encrypted file. If deleting the file fails, Touch ID stays off and the app asks you to try again. Old system backups or copied files may still exist and can still work on this Mac. Other apps running under your macOS account can copy this file and ask for Touch ID to open it, so only touch the sensor when you asked for it yourself.

These protected screens and Touch ID are not yet fully tested on real hardware, including keyboard, screen reader and capture use. See the [security model](security-model.md) and the [Quick Unlock acceptance record](quick-unlock-design.md#boundaries-and-acceptance-still-pending). Windows Hello comes later.

## Moving to another computer

Build a current encrypted-note there first. Choose Settings → Backups → Back up now. On the other computer choose "I already have a Vault → Open a Backup". To go back, on the Locked screen choose "Restore a saved copy (Backup or Safety Copy)". This computer's current Vault is kept as a Safety Copy.

Restoring an older copy also brings back the Master Password and Recovery Key it had. If those differ from the ones you use now, the ones you use now stop working, so keep the paper with the Recovery Key that matches that copy.

## Deleting the app

Deleting the Mac app leaves the Vault folder in place, so your Vault stays on this computer. If you set up the Agent Companion, its Agent Key file stays too: see [Stop, revoke, clean up](agent-companion.md#stop-revoke-clean-up). If you set up agents storing keys, its folder and the agent's tool stay: see [Stop and clean up](agent-keys.md#stop-and-clean-up).

On Windows, what the uninstaller does to the Vault folder has not been checked. Make a Backup first and keep it outside that folder. If the uninstaller offers to delete the app's data, that could remove your Vault and Safety Copies.

## If the app won't open

Your Notes are still in the Vault file, and the Emergency Reader reads it without the app.

**Keep AI tools out of this.** Quit AI coding tools first, and use the plain Terminal app, not a terminal panel inside an editor or AI app. Never give an AI tool your Master Password, your Recovery Key or anything the Reader prints, not even the list.

**Where the files are.** One folder holds `vault.enote` (the Vault), a `safety` folder (the Safety Copies, each a complete Vault), `settings.json`, and `quick-unlock.sealed` if you use Touch ID. While the app still runs, Settings → Where your keys are stored shows that folder and lets you copy its path. The folder for each computer is in the table under [Where your keys are stored](#where-your-keys-are-stored).

If the Locked screen keeps saying encrypted-note couldn't read or write a file, or that the file is far larger than any Vault: quit the app and move `vault.enote` out of that folder. Keep it; don't delete it. Reopen the app: it offers your Safety Copies and "I already have a Vault → Open a Backup".

**Best: a saved Reader, made now while everything works.** On a Mac, with no AI tool running, open the plain Terminal app in a clone of this project that agents don't work in, and run:

```bash
git fetch origin
scripts/build-reader.sh --out <folder>
```

It needs Rust and Node.js 24 or newer, like the app build. Choose a folder outside the project, for example next to your Backups; the script refuses one inside it, or one that already holds a Reader. It builds the Reader from a fresh copy of `origin/main` as of that `git fetch`, not from your working folder, and prints the commit and the SHA-256 of the `enote-reader` it saved. Write both on paper, and copy this guide next to the Reader. Before you use it, run `shasum -a 256 enote-reader` in that folder and check that the result matches your paper. Another build of the same commit may give a different SHA-256, so check the copy you saved. Make a new one after updating the app, because a newer Vault may need a newer Reader. `--commit` builds an older commit on `main`, and `--offline` builds only from what Cargo has already downloaded. The script hasn't been tried on Windows.

Why not just `cargo run`? It compiles whatever the project folder holds at the moment you type your Master Password, including unfinished or unreviewed changes an AI coding tool made there. A saved Reader built from merged `main` doesn't change afterwards.

**Read them with the Emergency Reader.** It works on a Vault, a Backup or a Safety Copy. It asks for the Master Password without showing what you type (add `--recovery-key` to use the Recovery Key instead), prints to the terminal and writes nothing to disk. Your terminal keeps what it prints and may save it. In the folder where you saved the Reader:

```bash
./enote-reader <file>                    # list every Note with a number; titles, usernames, websites and wallet addresses show, Hidden Fields stay as dots
./enote-reader <file> --note 3 --reveal  # only Note 3, with its Hidden Fields
./enote-reader <file> --reveal           # every Note, with its Hidden Fields
```

Give the reader a file, such as `vault.enote`, or a Safety Copy inside the `safety` folder. Don't give it the folder. Keep the quotes, because the Mac path has a space in it. Don't write `~` inside quotes: the shell won't expand it.

Mac, the built app:

```bash
./enote-reader "$HOME/Library/Containers/com.encryptednote.app/Data/Library/Application Support/com.encryptednote.app/Vault.noindex/vault.enote"
```

Mac, a release build that isn't sandboxed (for a test run, use `Vault.dev.noindex` in this path):

```bash
./enote-reader "$HOME/Library/Application Support/com.encryptednote.app/Vault.noindex/vault.enote"
```

**No saved Reader?** With Rust installed, put `cargo run --locked -p enote-reader --` where these lines have `./enote-reader`. Run it in a fresh clone of `main` (see the [README's Build section](../README.md#build)), not in a folder AI tools work in. The first build may need internet, to download the exact versions listed in `Cargo.lock`. No Rust? [FORMAT.md section 11](FORMAT.md#11-reference-decryptor-python) has a Python decryptor.

Windows, PowerShell, from a fresh clone of `main` (this line has not been run on a real Windows PC yet; `%LOCALAPPDATA%` from the table works only in Command Prompt and File Explorer):

```powershell
cargo run --locked -p enote-reader -- "$env:LOCALAPPDATA\com.encryptednote.app\Vault.noindex\vault.enote"
```

Keep `--note` and `--reveal` after the file, as shown above. The Reader prints Notes only to a terminal, the list as well as `--reveal`: if its output goes to a file or another program, it prints nothing, so nothing lands there by accident. It can't tell you from a program that runs the Reader in a terminal of its own, so it doesn't keep AI tools out. If it finds a sign that an AI tool runs the terminal (the `AI_AGENT` or `CLAUDECODE` variable), it warns you first, but still runs. `--allow-non-terminal` and `--credential-stdin` are for scripts you write yourself: never use them to hand your Notes or your Master Password to an AI tool.

The terminal has no Capture Hiding or Sharing Guard. Before `--reveal`, stop any screen share, use `--note <number> --reveal` to show only the Note you need, and quit Terminal (Cmd+Q) when you finish.

`No such file or directory` means the path is wrong, not that your Vault is gone.

The built Mac app's folder is protected from other apps, so macOS may ask before Terminal can read it. Easiest: give the reader a Backup instead, because a Backup isn't in that protected folder. To read the Vault folder itself, first quit any AI tool running in Terminal, then allow it when macOS asks. `Operation not permitted` means macOS said no: quit Terminal, open it again and run the command again. If it still doesn't ask, look for Terminal in System Settings → Privacy & Security. macOS should keep a yes only until Terminal quits, and every tool you run in Terminal meanwhile gets it too ([AI tools on this computer](#ai-tools-on-this-computer)), so quit Terminal with Cmd+Q when you finish. These macOS steps have not been checked by hand.
