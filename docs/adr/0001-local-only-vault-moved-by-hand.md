# Local-only Vault, moved between computers by hand

The app runs on Mac and Windows, and both open the same portable **Vault** file. We deliberately have no cloud service and no automatic sync: the owner copies the file themselves (for example on a USB drive), and that copy also serves as their backup. Sync would put the Vault on someone else's server or force us to write conflict-merging logic for two computers editing at once, and neither is worth it for a single-owner v1. As a result, if the owner edits on both computers between copies, the copy opened last wins. To soften that, opening an older **Backup** triggers a plain-language warning, and the replaced **Vault** is kept as a **Safety Copy**.

The app also has no network code of any kind: no updater, no telemetry, no crash reporting, no HTTP client in Rust or TypeScript (`cargo-deny` bans networking crates; the WebView CSP allows only Tauri IPC). So even a compromised dependency has no ready path to send the Vault anywhere. The owner updates by rebuilding or reinstalling. We hoped macOS could enforce this too, by shipping sandboxed without `com.apple.security.network.client`, but a spike (2026-09-24, macOS 26.4.1) showed WKWebView renders nothing without it. Release builds are therefore sandboxed **with** `network.client`. That still gives the Vault folder macOS's container protection from other apps, which an unsandboxed build (with full network access anyway) wouldn't have. The app says it "never connects to the internet", which is true by construction, and never claims the OS blocks it.

## Considered Options

- **Separate vault per computer**: simpler, but the owner's secrets would be split across machines.
- **Auto-sync through a shared folder**: convenient, but needs conflict handling and invites cloud-synced folders (iCloud, OneDrive, Dropbox) back in.
