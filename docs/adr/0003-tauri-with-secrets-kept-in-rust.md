# Tauri v2, with every secret kept in the Rust core

The app must run on Mac and Windows and support **Capture Hiding** on both. The owner writes TypeScript, and Rust is already installed. We chose Tauri v2: the screens are TypeScript in the system WebView, and a Rust core owns everything sensitive (key derivation, encryption, the **Vault** file, Keychain / Windows Hello). The WebView is treated as untrusted. It never holds the **Master Password**-derived key or the decrypted **Vault**. When the owner clicks Show, it receives exactly one **Hidden Field**, and it drops that field when the field hides again. Rust can wipe secrets from memory (`zeroize`); JavaScript strings can't be wiped.

## Considered Options

- **Electron**: all TypeScript, but secrets would live in JS memory that can't be wiped (Bitwarden desktop issue #6231: master password found in memory dumps), plus the largest attack surface and an 8-week Chromium upgrade treadmill.
- **Flutter**: one codebase and a better mobile path later, but a new language, and Capture Hiding needs native code on each platform.
- **Native SwiftUI + a separate Windows app**: strongest per-platform integration (the research favoured SwiftUI for a Mac-only app) but two apps to build.

## Consequences

- Stay on Tauri v2 (2.11.6+, which includes the GHSA-w28w-mhc8-qvjv IPC fix), not the v3 alpha, and expect a v3 migration later. Use a single webview.
- Turn on Capture Hiding with `"contentProtected": true` in the window config, so it's active from creation and needs no JS permission. Use Tauri's isolation pattern and a strict CSP (`connect-src 'none'`), and grant only the capabilities needed.
- Keep npm and crate dependencies minimal and pinned (`npm ci --ignore-scripts`, `cargo-deny`/`cargo-vet`). npm (Sept 2025, and a malicious `@bitwarden/cli` in Apr 2026) and crates.io (2025, Aug 2026) have both shipped malware that stole wallet keys or secrets.
- Seed Phrase words are typed into one `<input type="password">` per word. WKWebView and Chromium turn on Secure Event Input for password fields, but not for a `<textarea>`. Secret inputs disable autocorrect, spellcheck, text replacement and Writing Tools.
- Touch ID and Windows Hello have no official Tauri desktop plugin. Never use the community `tauri-plugin-biometry`: it uses `.userPresence`, which falls back to the Mac login password. For Mac **Quick Unlock** (v1.1), choose between (a) a data-protection Keychain item created from Rust with the `security-framework` crate (`BIOMETRY_CURRENT_SET`, `AccessibleWhenPasscodeSetThisDeviceOnly`, `use_protected_keychain`), which needs a provisioning profile (the owner has a paid Apple Developer team), and (b) a small Swift helper holding a CryptoKit Secure Enclave key whose blob lives in our own file.
- Early spike: build a **sandboxed** Mac Tauri app (the container then asks before other apps read the Vault folder) without `network.client`; see the open risk in ADR-0001.
