# User guide

## Keyboard shortcuts

Cmd on Mac, Ctrl on Windows.

| Keys | Action |
|---|---|
| Cmd/Ctrl+L | Lock |
| Cmd/Ctrl+F | Search (arrow keys move through the list, Enter opens, Esc clears) |
| Cmd/Ctrl+N | New Note (then **1** to **5** pick its Kind) |
| Cmd/Ctrl+S or Cmd/Ctrl+Enter | Save |
| Cmd/Ctrl+Shift+C | Copy the open Note's first Hidden Field |

Copy feedback names the Note and field that reached the clipboard. The shortcut still targets the open Note while search highlights another Note. Lock and Sharing Guard remove the source title from feedback.

## Opening the Vault and Wallet values

The app collects your Master Password and Recovery Key in native controls. New-password suggestion, strength feedback, confirmation and your Recovery Kit stay there too. Wallet values already stored in the Vault use native readouts after a fresh Master Password check; copying happens inside the Rust core. New or edited Hidden Field values still use the Note editor in the WebView.

On a supported Mac, open Settings → Touch ID → Enable Touch ID. Enter the current Vault's Master Password in the native panel, then complete the protected Touch ID read-back. This local route uses the Secure Enclave and an encrypted private file; it needs no account, server or Apple Developer membership/profile. If the hardware or its enrolled biometric protection is unavailable, use your Master Password.

After every app restart, use your Master Password before Touch ID. Enter it again at least every seven days; Touch ID never moves that deadline. Password/recovery changes, Key Rotation and installing a Backup or Safety Copy turn the registration off and require explicit enrollment again. Wallet actions continue to require their own fresh Master Password.

Disable Touch ID in the same Settings section to turn app policy off and remove its current encrypted registration. A cleanup error leaves it disabled and asks you to retry. Old system backups or copied registration files are not assured erased. Apps with the same user's file access are outside a Keychain application-access-group boundary; approve only biometric prompts you intentionally requested.

The native/Touch ID implementation is present in source; its physical hardware, accessibility and capture acceptance are still pending. See the [security model](security-model.md) and [Quick Unlock acceptance record](quick-unlock-design.md#boundaries-and-acceptance-still-pending). Windows Hello remains later.

## If the app won't open

Your Notes are still in the Vault file, and the Emergency Reader reads it without the app.

**Where the files are.** One folder holds `vault.enote` (the Vault) and a `safety` folder (the Safety Copies, each a complete Vault). While the app still runs, Settings → Backups shows that folder.

| Computer | Vault folder |
|---|---|
| Mac, the built app | `~/Library/Containers/com.encryptednote.app/Data/Library/Application Support/com.encryptednote.app/Vault.noindex` |
| Mac, a build that isn't sandboxed (such as `npm run tauri dev`) | `~/Library/Application Support/com.encryptednote.app/Vault.noindex` |
| Windows | `%LOCALAPPDATA%\com.encryptednote.app\Vault.noindex` |

The built Mac app is sandboxed, so its folder is inside the app's container, and macOS may ask before Terminal is allowed to read it.

**Read them with the Emergency Reader.** It works on a Vault, a Backup or a Safety Copy. It asks for the Master Password without showing what you type (add `--recovery-key` to use the Recovery Key instead), prints to the terminal and writes nothing to disk.

```bash
cargo run -p enote-reader -- <file>                    # list every Note with a number; Hidden Fields stay as dots
cargo run -p enote-reader -- <file> --note 3 --reveal  # only Note 3, with its Hidden Fields
cargo run -p enote-reader -- <file> --reveal           # every Note, with its Hidden Fields
```

Put the path in quotes: the Mac one has a space in it. `--reveal` prints only to a terminal, so nothing lands in a file by accident.
