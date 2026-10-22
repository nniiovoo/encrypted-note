# Native credential custody

Status: ready-for-human — implementation present in this source tree; physical acceptance pending. Refs [#13](https://github.com/nniiovoo/encrypted-note/issues/13). Master Password and Recovery Key entry, new-password suggestion/strength/confirmation, Recovery Kit display/confirmation and stored Wallet readouts now use Rust-owned native controls. They are not WebView IPC values. Windows and Mac physical keyboard, accessibility and capture acceptance remain open.

## Native boundary

The WebView requests a named operation with non-credential arguments. Rust validates the current action, target Note, field and Kind, freezes its arguments and encrypted source state, and owns the native controls plus their callback. The [coordinator](../src-tauri/src/native_coordinator.rs) serializes native operations; [core jobs](../src-tauri/src/core/native.rs) derive credentials off the core mutex and commit only after rechecking the captured operation, phase, epoch, session generation, document counter, exact file bytes, deadline and Sharing Guard.

[AppKit controls](../src-tauri/src/native_credential/mac.rs) replace the existing capture-protected main window's content view while retaining the hidden WebView. The [Windows adapter](../src-tauri/src/native_credential/windows.rs) uses native single-line password EDIT controls in the same protected main window. Input, Recovery Kits and Wallet readouts do not use a separate browser window. Native controls clear their text when they close and return wiping Rust buffers; that cannot promise physical erasure of OS string/editor copies. Apple documents [secure text fields](https://developer.apple.com/documentation/appkit/nssecuretextfield) and Microsoft documents [ES_PASSWORD](https://learn.microsoft.com/en-us/windows/win32/controls/edit-control-styles).

Only the native callback can submit credential text to the frozen action. A request ID can cancel a request; it cannot authorize a different action. No credential string, generic authentication boolean, reusable approval grant or retained Wallet Key is returned to the WebView. Ordinary Hidden Field Show still returns its single authorized value under the existing reveal rules. Stored Wallet values use a native readout and return only completion through IPC; show-and-copy writes the clipboard in Rust.

## Implemented callers

| Operation | Native behavior | WebView result |
|---|---|---|
| Create Vault | Choose/confirm a strong Master Password, derive keys, show the Recovery Kit and confirm its final groups natively. No Vault file is saved before confirmation. | Completion/status |
| Unlock | Select Master Password or Recovery Key; parse/normalize and apply persistent credential delays in Rust. | Completion/status |
| Set new password after Recovery Key | Choose/confirm the replacement password natively; persist the candidate before leaving recovery state. | Completion/status |
| Wallet Show, accessible Show and Show-and-copy | Authenticate the exact Note/field, hold a transient Wallet Key, then show the value in native controls. Hold/timed rules, bounded extension and the shared disclosure allowance remain enforced. | Completion only |
| Whole-field copy | Rust decides whether the Note is a Wallet Kind; prompt natively when required and copy in Rust. | Completion only |
| Wallet create/edit/trash/delete/Empty Trash | Freeze the input and target set, request the fresh Master Password, derive the Wallet Key only for that action, recheck state, and wipe it afterward. | Existing non-secret action result |
| Backup or Safety Copy inspection | Collect the selected copy's credential natively and bind the exact source. Preserve preview expiry and explicit installation. | Preview metadata only |
| Password change or Key Rotation | Collect current/new password and confirmation. Rotation's new Recovery Kit stays native until confirmed; failed/cancelled candidates do not replace the committed Vault. | Completion/status |
| Loosen Auto-lock | Normalize the complete candidate, collect native Master Password only for a weaker rule, and persist before applying it. Tightening stays prompt-free. | Non-secret settings |
| Enable Quick Unlock | Fresh native Master Password proof of the current committed Vault precedes local Secure Enclave enrollment. | Availability/status only |

Raw password-generation/scoring IPC, password arguments, React password modals and Recovery Kit state have been removed from the app callers and registered command/capability routes. New-password controls call the existing policy internally. Native create calibration is owned by the operation lease. The terminal Emergency Reader retains its separate no-echo credential entry; it has no agent or biometric route.

## Cancellation, retention and limits

Native operations expire after two minutes. Escape/close, explicit cancellation/navigation/reload, owner Lock, timeout, sleep, screen lock, user switch, focus loss, replacement/password/rotation transitions, target mutation/removal or Sharing Guard arrival invalidates pending work. Rust rejects stale callbacks before failed-password accounting or any plaintext, clipboard or disk effect. A cancelled derivation cannot revive a held reveal or approve another action.

Native activity keeps the existing idle timer current. Wallet readouts are bound to the native operation epoch and current reveal identity, so an obsolete panel cannot hide or extend a newer field. Recovery Kit cancellation rolls back the uncommitted creation/rotation state. Plaintext values and transient Wallet Keys are dropped on every completion/error path; the app does not cache a previous Master Password approval for future Wallet actions.

Ordinary Note entry/checks, including newly entered Wallet Hidden Fields, remain in the WebView. Explicit ordinary Show values also enter JavaScript. A compromised renderer could read those inputs/outputs or draw a convincing fake credential prompt; native custody cannot prevent an owner typing into that imitation. The native panels remove the app's actual Master Password, Recovery Key, Recovery Kit and stored Wallet readout routes from renderer memory. They do not protect a compromised Rust/native process, keylogger, administrator or operating system. OS/native editor buffers and the stack remain best effort.

## Validation still required

Fake-data core/coordinator tests cover creation and Recovery Kit confirmation, both unlock kinds, recovery reset, Wallet mutations/readouts, both password-change modes, copy inspection, Auto-lock weakening/tightening and stale/expired/cancelled operations. Browser fixture checks cover non-credential IPC contracts and the migrated callers. The latest build/test report records execution; the presence of these tests is not evidence of physical native behavior.

Run the current local app on real macOS and Windows for complete create/recovery/Wallet/copy/rotation/settings flows, native keyboard behavior, VoiceOver/Narrator, held Space/button release, long/control-character readouts, lock/switch/sleep races and first-frame capture behavior. Inspect only sanitized outcomes and fake fixture markers when checking IPC/events/logs; real credential or Wallet contents must not enter reports. Existing capture results from an older UI do not validate the new native controls. Touch ID has its own [physical acceptance checklist](quick-unlock-design.md#boundaries-and-acceptance-still-pending).
