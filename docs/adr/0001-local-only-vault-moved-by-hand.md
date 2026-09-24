# Local-only Vault, moved between computers by hand

The app runs on Mac and Windows, and both open the same portable **Vault** file. We deliberately have no cloud service and no automatic sync: the owner copies the file themselves (for example on a USB drive), and that copy also serves as their backup. Sync would put the Vault on someone else's server or force us to write conflict-merging logic for two computers editing at once, and neither is worth it for a single-owner v1. As a result, if the owner edits on both computers between copies, the copy opened last wins. To soften that, opening an older **Backup** triggers a plain-language warning, and the replaced **Vault** is kept as a **Safety Copy**.

The app also has no network capability of any kind: no updater, no telemetry, no crash reporting. This is enforced by the OS where possible (Mac: no `com.apple.security.network.client` entitlement; WebView CSP `connect-src 'none'`; no Tauri HTTP plugin). So even a compromised dependency can't send the Vault anywhere through this app. The owner updates by rebuilding or reinstalling. Open risk: Apple forum reports (community, unconfirmed) say a sandboxed app using WKWebView, which Tauri uses on Mac, needs `network.client` just to render. An early spike must build a sandboxed Tauri app without that entitlement. If it fails, the app says "contains no network code" (enforced by CSP and a `cargo-deny` ban on HTTP crates) and never says "blocked by the OS".

## Considered Options

- **Separate vault per computer**: simpler, but the owner's secrets would be split across machines.
- **Auto-sync through a shared folder**: convenient, but needs conflict handling and invites cloud-synced folders (iCloud, OneDrive, Dropbox) back in.
