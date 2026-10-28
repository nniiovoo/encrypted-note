# Wallet Kinds are encrypted under a separate key that unlocking doesn't open

Showing a **Seed Phrase** or **Private Key** requires the **Master Password** again. If the whole **Vault** were already decrypted in memory, that prompt would be only a UI check in front of exposed data. So the **Hidden Fields** of **Wallet Kinds** are encrypted a second time under a separate random Wallet Key, and a normal unlock does not open it. Only the **Master Password** (a fresh Argon2id derivation, about 0.5 s) or the **Recovery Key** can unwrap the Wallet Key. It is held just long enough to show, add or edit one **Note** and is then wiped. Someone at an **Unlocked** computer, or anything reading the app's memory while it's merely **Unlocked**, can see Wallet Kind titles, chains and addresses, but not the words or keys.

## Consequences

- Adding or editing a Wallet Kind also asks for the **Master Password**, because it needs the Wallet Key to encrypt. Moving one to **Trash**, deleting it for good and emptying a **Trash** that holds one ask for it too: not because a key is needed, but so that nobody at an **Unlocked** computer can remove a wallet either.
- "Anything reading the app's memory" is as true as wiping can make it (ADR-0003). Actual Master Password entry and stored Wallet readouts now use native controls; new/edited Hidden Field input still passes through the WebView. Rust-owned heap buffers are wiped as freed, while native/OS copies, JavaScript strings and the stack remain best effort. A reader who catches a Master Password or a transient Wallet Key in native memory can still decrypt Wallet fields. The separate key protects an ordinary Unlocked document; it does not promise protection from software that can read the native process at will.
- The Wallet Key sits in the same credential slots as the Vault Key (ADR-0005), so the **Master Password** or the **Recovery Key** yields both. On an ordinary unlock the app wipes the Wallet Key right away and keeps only the Vault Key. **Key Rotation** replaces both.
- Mac **Quick Unlock** enrolls only the Vault Key. It cannot release the Wallet Key or reuse an earlier Wallet approval: every Wallet action still requires a fresh native Master Password check. Stored Wallet values are shown in native controls and copied in Rust. Windows Hello and any separate biometric Wallet authorization remain outside this implementation.

## Considered Options

- **Re-prompt as a UI check only**: simpler, but defeated by anyone with the unlocked session or a memory read.
- **Asymmetric Wallet Key (X25519)**: would let new Seed Phrases be added without the password. We rejected it as more crypto surface for a beginner-maintained codebase.
