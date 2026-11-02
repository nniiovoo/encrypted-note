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
| Your Notes, including API Keys, Private Keys and Seed Phrases | `vault.enote` in the Vault folder | Yes. Seed Phrases and Private Keys are encrypted twice. |
| The keys that lock your Vault | Inside `vault.enote`, locked by your Master Password and by your Recovery Key | Yes. |
| Safety Copies | `safety/` in the Vault folder | Yes. Each opens with the Master Password and Recovery Key it had when saved. A Note you deleted or changed stays inside older copies until the app removes them. It keeps your 10 most recent saves, the first save of each day for 30 days, and a Vault replaced by a Backup or restored copy for at least 30 days. |
| Master Password | Not in any file | Only you know it. |
| Recovery Key | Not in any file | Only on the paper where you wrote it. |
| Touch ID (Mac, if you turn it on) | `quick-unlock.sealed` in the Vault folder | Yes. It holds a protected copy of the key that opens your Logins, API Keys and Text. It works only on this Mac, with a fingerprint saved on this Mac. It can't open the secret parts of Seed Phrases or Private Keys. |
| Settings | `settings.json` in the Vault folder | No. Choices, dates, the count of wrong password tries and, on a Mac, a check value for Touch ID; no Notes, titles or keys. |
| Backups | Wherever you saved them. The app suggests the name `encrypted-note-backup-YYYY-MM-DD-HHMM.enote`, with the date and time in UTC | Yes. Each opens with the Master Password and Recovery Key it had when saved. The app doesn't remember where. A Note you deleted or changed stays inside an old Backup for as long as you keep it. |
| Copied values | Clipboard for 30 or 60 seconds (Seed Phrases and Private Keys always 30), then cleared | No. Other apps could read it during that time. If the app crashes, it can stay until you copy something else. |
| Open Notes | App memory while Unlocked | Cleared on lock and quit, as far as the computer allows. The key that opens your Notes, unsaved typing and shown values are in memory too, and the computer may write memory to disk. |
| Agent Key (only if you set up the Agent Companion) | `credential.env` in the helper's folder: `~/Library/Containers/com.encryptednote.app.companion/Data/Library/Application Support/encrypted-note-companion/` | **No. Plain text.** Other macOS accounts can't read it, but other software running under your account may be able to. While the Agent Companion runs, the key is also in the helper's memory. The file you made for setup also holds the key until you delete it. Deleting a file doesn't cancel the key: cancel it at OpenAI. |
| Agent token (only while the Agent Companion runs) | `/Users/Shared/encrypted-note-companion/agent-token` | No. Made only for the agent account. Removed when the Agent Companion stops normally; each Start makes a new one. Not your Agent Key. |

Nothing is kept in Apple Keychain or on any server.

Time Machine or other full-computer backups may also copy these folders. Your Vault stays encrypted there, but old copies open with the old Master Password and Recovery Key, and the Agent Key file stays plain text.

**What to keep safe:** the paper with your Recovery Key, somewhere physical and apart from your Backups (never photograph it or save it on a computer), and a Backup made with Back up now. Nothing else in the folder needs copying.

**If you lose the paper:** if you still know your Master Password, you can make a new Recovery Key. In Settings, go to Change Master Password and choose "Someone may know it, or I lost my paper". You also choose a new Master Password there, and then write down the new Recovery Key. Touch ID turns off, and you can turn it on again. Then choose Back up now. Your older Backups and Safety Copies still open only with the old password and the old key, and anyone who finds the old paper could open those old copies.

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

**Using a password manager.** Switching to another app closes the password box, so copy your Master Password before you start, then paste it in the box that opens (Cmd+V on Mac, Ctrl+V on Windows). A password you paste from another app gets none of this app's clipboard protection: other apps and clipboard history can read it, and this app can't clear it. When you finish, copy something else to replace it on the clipboard. This doesn't remove it from a clipboard history app. Pasting into these boxes has not been tested by hand yet.

On a supported Mac, open Settings → Touch ID → Turn on Touch ID. Type your Master Password, then touch the sensor. This uses your Mac's security chip and a small encrypted file. It needs no account, server or Apple Developer membership. If Touch ID isn't available on this Mac, use your Master Password.

After every app restart, use your Master Password before Touch ID. Enter it again at least every seven days; Touch ID never moves that deadline. Changing your Master Password, using your Recovery Key, Key Rotation and installing a Backup or Safety Copy turn Touch ID off, and you have to turn Touch ID on again. Wallet actions continue to require their own fresh Master Password. Choosing Forgot it? Use your Recovery Key turns Touch ID off as soon as the box opens, even if you cancel. Starting to change your Master Password does the same. A missed finger, a lockout, a closed lid or a disconnected Touch ID keyboard leaves Touch ID on. Changing the fingers saved on this Mac turns it off.

To turn Touch ID off, use the same Settings section. This also deletes its encrypted file. If deleting the file fails, Touch ID stays off and the app asks you to try again. Old system backups or copied files may still exist and can still work on this Mac. Other apps running under your macOS account can copy this file and ask for Touch ID to open it, so only touch the sensor when you asked for it yourself.

These protected screens and Touch ID are not yet fully tested on real hardware, including keyboard, screen reader and capture use. See the [security model](security-model.md) and the [Quick Unlock acceptance record](quick-unlock-design.md#boundaries-and-acceptance-still-pending). Windows Hello comes later.

## Moving to another computer

Build a current encrypted-note there first. Choose Settings → Backups → Back up now. On the other computer choose "I already have a Vault → Open a Backup". To go back, on the Locked screen choose "Restore a saved copy (Backup or Safety Copy)". This computer's current Vault is kept as a Safety Copy.

Restoring an older copy also brings back the Master Password and Recovery Key it had. If those differ from the ones you use now, the ones you use now stop working, so keep the paper with the Recovery Key that matches that copy.

## Deleting the app

Deleting the Mac app leaves the Vault folder in place, so your Vault stays on this computer. If you set up the Agent Companion, its Agent Key file stays too: see [Stop, revoke, clean up](agent-companion.md#stop-revoke-clean-up).

On Windows, what the uninstaller does to the Vault folder has not been checked. Make a Backup first and keep it outside that folder. If the uninstaller offers to delete the app's data, that could remove your Vault and Safety Copies.

## If the app won't open

Your Notes are still in the Vault file, and the Emergency Reader reads it without the app.

**Where the files are.** One folder holds `vault.enote` (the Vault), a `safety` folder (the Safety Copies, each a complete Vault), `settings.json`, and `quick-unlock.sealed` if you use Touch ID. While the app still runs, Settings → Where your keys are stored shows that folder and lets you copy its path. The folder for each computer is in the table under [Where your keys are stored](#where-your-keys-are-stored).

If the Locked screen keeps saying encrypted-note couldn't read or write a file, or that the file is far larger than any Vault: quit the app and move `vault.enote` out of that folder. Keep it; don't delete it. Reopen the app: it offers your Safety Copies and "I already have a Vault → Open a Backup".

**Read them with the Emergency Reader.** It works on a Vault, a Backup or a Safety Copy. It asks for the Master Password without showing what you type (add `--recovery-key` to use the Recovery Key instead), prints to the terminal and writes nothing to disk. Your terminal keeps what it prints and may save it.

```bash
cargo run --locked -p enote-reader -- <file>                    # list every Note with a number; Hidden Fields stay as dots
cargo run --locked -p enote-reader -- <file> --note 3 --reveal  # only Note 3, with its Hidden Fields
cargo run --locked -p enote-reader -- <file> --reveal           # every Note, with its Hidden Fields
```

Give the reader a file, such as `vault.enote`, or a Safety Copy inside the `safety` folder. Don't give it the folder. Run these from a current copy of this project's source, with Rust installed. The first build may need internet, to download the exact versions listed in `Cargo.lock`. Keep the quotes, because the Mac path has a space in it. Don't write `~` inside quotes: the shell won't expand it.

**Do this once, now, while everything works:** run `cargo build --locked --release -p enote-reader`, then copy `target/release/enote-reader` and this guide next to your Backup. Replace it after updating the app. No Rust? [FORMAT.md section 11](FORMAT.md#11-reference-decryptor-python) has a Python decryptor.

Mac, the built app:

```bash
cargo run --locked -p enote-reader -- "$HOME/Library/Containers/com.encryptednote.app/Data/Library/Application Support/com.encryptednote.app/Vault.noindex/vault.enote"
```

Mac, a release build that isn't sandboxed (for a test run, use `Vault.dev.noindex` in this path):

```bash
cargo run --locked -p enote-reader -- "$HOME/Library/Application Support/com.encryptednote.app/Vault.noindex/vault.enote"
```

Windows, PowerShell (this line has not been run on a real Windows PC yet; `%LOCALAPPDATA%` from the table works only in Command Prompt and File Explorer):

```powershell
cargo run --locked -p enote-reader -- "$env:LOCALAPPDATA\com.encryptednote.app\Vault.noindex\vault.enote"
```

Keep `--note` and `--reveal` after the file, as shown above. `--reveal` prints only to a terminal, so nothing lands in a file by accident.

The terminal has no Capture Hiding or Sharing Guard. Before `--reveal`, stop any screen share, use `--note <number> --reveal` to show only the Note you need, and close the terminal window when you finish.

`No such file or directory` means the path is wrong, not that your Vault is gone.

The built Mac app's folder is protected from other apps, so macOS may ask before Terminal can read it. `Operation not permitted` means macOS said no. Run the command again and allow it when macOS asks. If you refused before and it doesn't ask again, change that choice for Terminal in System Settings → Privacy & Security. Or give the reader a Backup instead, because a Backup isn't in that protected folder. These macOS steps have not been checked by hand.
