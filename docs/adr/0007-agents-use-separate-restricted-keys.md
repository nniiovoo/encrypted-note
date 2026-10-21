# Agents use separate restricted Agent Keys; the owner Vault stays out of reach

Status: Accepted by the owner, 2026-10-21 ([issue #25](https://github.com/nniiovoo/encrypted-note/issues/25)).

The owner has approved a separate **Agent Key** boundary. AI agents receive no access to the owner's **Vault**, its **Notes** or their metadata. An **Agent Key** is a new credential created at a service only for agent work, never a copy of an owner's existing key and never a value stored in the **Vault**. This decision approves the boundary, not a provider operation or a pilot.

## Decision

- The app offers no agent interface: no Vault bridge, MCP server, socket, pipe, helper, URL scheme, Note enumeration or metadata export. Agents must not receive Vault files, **Backups**, **Safety Copies**, revealed fields, or access through the app's window or IPC. The **Emergency Reader** is not an agent integration.
- The **Master Password**, **Recovery Key** and **Wallet Kinds** remain outside all agent work. Independent owner approval cannot make them, or any other Vault content, an agent data source.
- [ADR-0001](0001-local-only-vault-moved-by-hand.md) and [ADR-0003](0003-tauri-with-secrets-kept-in-rust.md) stand: encrypted-note adds no network access; its keys and decrypted Notes remain in the Rust core. Any future service integration is separate from this app and cannot call into it.
- A future design must keep raw Agent Keys out of agent-visible prompts, chat, tool results and logs. This ADR chooses no provider, key holder, sandbox or gateway. Hiding a key does not limit its authority: an agent can misuse every operation its provider scope permits, up to the enforced limits.

## Before any future pilot

Only exact, owner-selected service operations on named resources may be considered. Each allowed action needs an owner-reviewed record covering all of the following:

| Boundary | Required record and enforcement |
| --- | --- |
| Operation | The exact operation and resources, with narrow permissions enforced by the provider; no arbitrary command, destination or expansion of scope. |
| Expiry | A fixed expiry enforced by the provider, after which the key cannot be used. |
| Revocation | The provider-side disable/delete procedure under the owner's control; stopping a local tool alone is insufficient. |
| Caps | Hard provider-enforced spending and usage caps appropriate to the action; a budget warning or agent promise is insufficient. |
| Results | The fields the action may return and the destinations that may receive them, including any cloud model provider. Return only those fields; never keys or anything from the Vault, including Note titles and other metadata. |

Provider support must be checked against current primary documentation and verified before a pilot. Do not assume every provider offers these controls. If narrow permissions, expiry, revocation or the required hard caps cannot be enforced, that action gets no pilot. No Agent Key may create credentials, expand its own permissions, raise caps or extend its expiry.

The owner must approve the action's scope and limits through an account and session the agent cannot control. Credential administration, permission or limit changes, destructive operations and financial actions cannot receive approval from the agent itself; they require independent owner approval and a separate reviewed design before inclusion in any pilot. A model's confirmation, tool annotation or click in an agent-controlled session is not independent approval. No approval overrides the Vault exclusions above.

Allowed service results may leave this computer through an external agent or its model provider. That must be explicit in the action's record; the app's local-only promise does not extend to those external results. Expiry, revocation and caps limit future use; they do not undo prior disclosures or actions.

## Consequences

This is a documentation-only decision. It adds no command, dependency, file-format change, credential handling, account setup or provider call. A future pilot needs its own reviewed implementation and boundary checks with fake keys before real credentials are considered. The absence of an agent interface is not a defense against malware already able to read the owner's files or session ([ADR-0003](0003-tauri-with-secrets-kept-in-rust.md) and [ADR-0004](0004-wallet-kinds-under-a-separate-key.md)).

## Considered Options

- **Give agents owner keys, Vault content or Note metadata through a local or remote bridge:** rejected; it breaks the owner-Vault boundary.
- **Use the Emergency Reader or sign with Wallet Kinds for an agent:** rejected; recovery and wallet access stay owner-only.
- **Choose a sandbox, broker or hosted gateway now:** deferred; this decision establishes requirements without adopting an unverified implementation.
