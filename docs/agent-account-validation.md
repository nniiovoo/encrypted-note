# Separate macOS account validation

Status: ready-for-human — not run. Refs [#26](https://github.com/nniiovoo/encrypted-note/issues/26).

This is a future owner-scoped test of the proposed separation between an owner account and a standard account for local agents. It makes no claim that the accounts are isolated on a particular Mac. The accepted boundary is separate restricted Agent Keys and absolutely no agent access to the owner's Vault, Notes, Master Password, Recovery Key, Backups or Safety Copies; [#25](https://github.com/nniiovoo/encrypted-note/issues/25) records that decision. The app keeps decrypted Notes in Rust ([ADR-0003](adr/0003-tauri-with-secrets-kept-in-rust.md)); it does not provide an agent interface.

## Before a run

The owner must explicitly scope concrete operations in a private operational record: test accounts, fake fixture locations, tools, permissions, allowed changes, cleanup and stop conditions. Approval of the boundary or this document does not authorize creating accounts, changing home permissions, migrating apps or histories, installing tools, changing Accessibility/Screen Recording grants, modifying security plugins, or inspecting a real Vault. Those actions need their own owner-scoped work.

Use only owner-prepared fake fixtures and an isolated fake-data Vault. The test fixture should sit behind the same filesystem boundary intended for the owner's material, without probing that material itself. Do not use the owner's actual Vault folder, Keychain, histories, account tokens or service credentials as a target. If a required tool or permission is unavailable, leave the check pending; do not grant or install it to finish this record.

Prepare one owner-side text file containing an obviously fake marker such as `FAKE-OWNER-ONLY-NOT-A-CREDENTIAL`, one agent-side marker, and a separate fake-data app session. Keep fixture paths and account names private. Put fixtures on each filesystem that the design depends on, including any external/shared volume; one protected home-folder test cannot establish a shared volume's permissions.

## Checks

Run the denied-access checks through the agent's actual approved tools as well as the interactive account. Record expected and observed outcomes separately. Stop before any test reaches real data or needs an unapproved privilege or permission change.

| Boundary | Fake-data procedure | Expected observation / limit | Result |
|---|---|---|---|
| Account identity | Inspect the test account's role and the identity used by the agent process | Agent executes as the distinct standard user, without admin membership | pending |
| Privilege | Check whether the standard user can obtain admin authorization without owner credentials; never enter owner credentials | Authorization is refused or requests separate administrator approval; a prompt is not a passed privilege escalation | pending |
| Owner fixture read | Try to list and read only the designated owner-side fake fixture | Denied from the Agent Account and its tools | pending |
| Owner fixture write | Try to modify only that same fake fixture; owner later compares its marker | Denied and marker unchanged | pending |
| Agent workspace | Read and change the agent-side fake fixture | Allowed only within the approved workspace; confirms the agent tool actually ran | pending |
| Shared/external locations | Repeat the read/write checks for designated fake fixtures on every planned volume | Record actual behavior per volume; a readable/writable fixture means that location does not supply the proposed boundary | pending |
| Clipboard | Copy an owner-side fake marker, switch users, and try to paste in the Agent Account; repeat in the reverse direction | Record whether the marker crosses either direction; any crossing disproves clipboard separation for that setup | pending |
| Screen capture | With an already-approved capture tool running in the Agent Account, switch to an owner-side fake-data session and inspect only fake test pixels | Record whether owner-session pixels are visible, including tool version and permission state; unavailable tool/grant stays pending | pending |
| Input observation | With an already-approved input-observation tool in the Agent Account, type only a fake marker in the owner-side fake-data session | Record whether keystrokes cross the account boundary; do not install a keylogger or broaden permissions for this check | pending |
| App user switch | Unlock only the fake-data Vault, switch accounts, then return | The app is Locked and no Hidden Field remains shown | pending |
| Host services and mounts | Review only the owner-approved fake test endpoint and mount inventory; test its intended authentication without real tokens | No unauthenticated route from the Agent Account to the owner-side fixture; record untested routes as limits | pending |

A failed row blocks claiming that boundary works. A clipboard, capture or input failure requires an owner decision or a different execution boundary, followed by a rerun; it is not fixed by documenting the risk. An untested route cannot be counted as denied.

## Sanitized record

Keep private logs and operational details out of GitHub and this repository. Publish only fake fixture labels, dates, exact OS/tool versions, result summaries and follow-up issue links. Never include account names, home paths, real values, directory inventories, terminal transcripts, Note titles or raw captures.

```text
Run date / reviewed app commit / exact macOS version:
Private operational scope approved on (date only):
Agent tool and exact version / permission state:
Fixture labels and filesystem types (no paths):
Check / expected outcome / observed outcome / pass, fail or pending:
Sanitized failure issue / unresolved limitation:
Owner confirmed fake-fixture cleanup (or pending):
```

No results have been recorded here. A standard account is not a defense against an administrator, a compromised OS, or an agent still running as the owner. These checks cover only the tested account, filesystems, tools and permission state. Recheck after those change. This document does not authorize a real-key pilot.
