# Quick Unlock on Mac: proposed implementation

Status: ready-for-human — design only. Refs [#12](https://github.com/nniiovoo/encrypted-note/issues/12); Quick Unlock is not implemented. Windows Hello stays later.

## Selected architecture

Choose ADR-0003's **Rust-owned data-protection Keychain item**, accessed through a future narrowly scoped `security-framework` adapter. Store only the Vault Key and its binding to this computer's committed Vault, never the Master Password, Recovery Key, Wallet Key or decrypted Notes. A successful protected item read must supply the actual key to Rust; a fingerprint-success boolean cannot unlock anything. There is no new Vault credential slot or file-format change.

Require `kSecUseDataProtectionKeychain = true`, non-synchronizable storage, `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly` and `biometryCurrentSet` access control. Never use `.userPresence`, device-passcode fallback, `biometryAny`, an OR policy, a community plugin or an unprotected fallback item. Unsupported attributes fail closed. Apple documents the [data-protection query](https://developer.apple.com/documentation/security/ksecusedataprotectionkeychain), [device-only accessibility](https://developer.apple.com/documentation/security/ksecattraccessiblewhenpasscodesetthisdeviceonly), and [current-enrollment invalidation](https://developer.apple.com/documentation/security/secaccesscontrolcreateflags/biometrycurrentset). These API requirements were reviewed on 2026-10-21; their behavior in our signed app remains untested.

The Keychain item binds the key to Vault identity and the exact credential slots/KDF header, excluding the body nonce that changes on a normal save. Validate that binding and authenticated body decryption against the current file before opening it. Vault ID alone is insufficient: Key Rotation keeps it. `vault-format` currently has no key-custody import/export API; add only the Rust-side operation needed to enroll/open with this bound Vault Key, never a serializable IPC key getter. Preserve `open`/`reopen` validation and malformed-file limits.

## Lifecycle and failure rules

| Event | Proposed rule |
|---|---|
| Enable, initially off | After a fresh native Master Password check of the current, committed Vault, explicitly enroll its Vault Key. Verify a biometry-gated read succeeds before showing enabled. Cancellation or any write/read failure leaves the feature off and the Vault unchanged. |
| Process start or crash/relaunch | Require the Master Password before any Quick Unlock, even if an enrolled item survives. Never trust a persisted enabled flag or timestamp as authorization. |
| Seven-day rule | Rust keeps a process-local deadline from the last successful native Master Password verification of the current Vault. Count sleep with the continuous clock already used by `platform::continuous_ms`; expiry, a missing clock or inconsistent timing requires the Master Password. Touch ID never extends the deadline. |
| Normal lock, sleep, screen lock or user switch | Keep the enrolled item but wipe in-memory Vault state and cancel/invalidate pending native authentication. A later unlock requires a new protected item read, subject to startup/seven-day rules. |
| Touch ID request | Fresh native authentication context and protected item read, with reuse disabled; no separate cached success. Capture the Vault/action generation, wait without holding the core mutex, then recheck current file binding and generation before starting the existing session. |
| Cancel, failed match, lockout, missing Touch ID or Keychain error | Stay Locked, wipe all returned key/buffers, publish only a sanitized error and offer native Master Password entry. Do not fall back to the Mac login password or clear existing password-attempt backoff. |
| Finger enrollment changes | Treat the invalidated item as unavailable; require the Master Password and explicit re-enrollment. Never silently recreate it with a weaker constraint. |
| Master Password change, recovery reset, Backup/Safety Copy installation | Invalidate registration and process authorization, even for the same Vault ID; native Master Password plus explicit enrollment is required for the committed result. Inspecting a copy alone never enrolls it. |
| Key Rotation | Suspend registration as soon as rotation starts. Cancelled/failed rotation never enrolls pending keys. After Recovery Kit confirmation and durable save, remove the old registration and require explicit enrollment for the new keys. |
| Disable or stale item | Mark unavailable before deleting the exact app-owned item. Failed deletion leaves it unusable through app policy and requires a sanitized cleanup warning; never broaden a Keychain query. Re-enrollment must not reuse a stale binding. |

A valid current-Vault Master Password check may refresh the seven-day deadline; a Recovery Key, a credential for a different copy, or Touch ID may not. Opening a Vault with Quick Unlock follows existing session startup, Sharing Guard reset, clipboard and Trash rules. The UI receives availability/outcome only. Wallet Kind actions still require a fresh Master Password and transient Wallet Key under ADR-0004; no biometric Wallet item is part of this first slice.

Use a fresh [LAContext](https://developer.apple.com/documentation/localauthentication/lacontext) per request and invalidate it on completion/cancel/lock. Key release must remain controlled by the Keychain ACL. Native callback success arriving after a lock, replacement, generation change or user cancellation must be discarded.

## Implementation sequence and acceptance

1. Complete native credential custody for enrollment and fallback ([#13](https://github.com/nniiovoo/encrypted-note/issues/13)); agree the exact action/generation checks before adding a Keychain command. Preserve current password/recovery behavior until the complete route exists.
2. Implement fake-adapter lifecycle tests in `src-tauri/tests/core.rs` and bounded key-custody tests in `crates/vault-format`: startup/seven-day gate, clock/sleep, key mismatch, cancellation, stale callbacks, ordinary save versus credential-slot changes, rotation rollback/commit and failed item cleanup. Keep all fixture keys obviously fake.
3. Add the Mac adapter and availability/enable/disable/unlock callers in `platform.rs`, `commands.rs`, `core.rs`, `src/api.ts`, `Start.tsx` and `Settings.tsx`; update command registration, build manifest and capabilities together. Any new dependency must have a verified exact version and stated need in that implementation PR. Do not add it in this design PR.
4. Test a provisioned, distribution-like signed sandboxed app on actual Touch ID hardware: enable/cancel/retry, no login-password fallback, enrollment changes, missing/locked Keychain, app/OS restart, seven-day boundary, lock/sleep/switch races, password/recovery/Backup/rotation transitions, disable/delete failure and signed-app update. Record sanitized versions/outcomes, no actual keys or private account details.

The current configuration is ad-hoc signed (`signingIdentity: "-"`) and has no Keychain access-group entitlement/profile configuration. Apple's [TN3137](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains) requires provisioning-authorized code-signing entitlements for data-protection Keychain access. An owner-prepared signing identity, bundle/access-group configuration, matching profile and physical Touch ID setup are prerequisites. No provisioning, Keychain or enrollment operation is authorized by this document.

All implementation and hardware acceptance remain open. Quick Unlock adds another local copy of the Vault Key behind OS access controls; it does not protect a compromised Rust process, administrator or OS, and never grants agents access to the owner's Vault.
