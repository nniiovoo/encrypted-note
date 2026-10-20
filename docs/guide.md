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
