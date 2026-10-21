# An agent gateway waits for one concrete API need

Status: Deferred; the owner has not selected a first service or operation. Refs [issue #28](https://github.com/nniiovoo/encrypted-note/issues/28).

The owner approved separate restricted Agent Keys and no owner-Vault access ([ADR-0007](0007-agents-use-separate-restricted-keys.md)). That boundary does not approve a remote gateway. No API, network service, hosting account, repository or deployment has been chosen.

## Decision gate

Before proposing implementation, record one actual owner task and check the selected service's current official connector documentation. Decide whether an existing connector satisfies it; a gateway needs a concrete remaining need. Record the checked source and date instead of assuming a connector exists or is safe.

If a gateway is still necessary, obtain a reviewed decision for all of these together:

- One fixed read-only operation on named resources, with narrowly validated parameters and destination.
- Authentication enforced outside agent control; provider-enforced scope, expiry, revocation and required hard caps. Agent Keys cannot administer credentials or raise their own limits.
- The exact allowed output fields and cloud-model destinations. No owner-Vault content, metadata, Master Password, Recovery Key, Wallet Kind or Emergency Reader access.
- A separate explicitly chosen repository, deployment, account and operational scope. Network code never enters encrypted-note ([ADR-0001](0001-local-only-vault-moved-by-hand.md)); [ADR-0003](0003-tauri-with-secrets-kept-in-rust.md) stays in force.

Only then create focused implementation issues for the approved operation and its tests. A key hidden from an agent still permits misuse of every allowed operation; a domain allowlist alone cannot restrict actions at that domain.

## Current outcome

This records a gate, not an approved pilot. Service selection, connector verification, the operation and all deployment/account decisions remain pending. It creates no listener, endpoint, OAuth setup, credential handling or generic gateway framework. Issue #28 remains open until its concrete API decision is made.
