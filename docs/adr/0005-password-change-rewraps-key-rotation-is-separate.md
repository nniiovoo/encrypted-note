# Changing the Master Password re-wraps keys; Key Rotation is a separate step

Each way into the **Vault** (the **Master Password**, the **Recovery Key**) has its own slot in the header. The slot holds the Vault Key and the Wallet Key (ADR-0004), encrypted under a key derived from that credential with Argon2id. Changing the **Master Password** re-encrypts only the password slot, so it's instant and the **Recovery Kit** stays valid. Replacing the Vault Key would also mean re-encrypting the recovery slot, and that requires the **Recovery Key**, which the app deliberately never stores. So we don't rotate on every password change. We offer **Key Rotation** instead: new Vault Key, new Wallet Key and new **Recovery Key**, re-encrypting every **Note** and showing a new **Recovery Kit**. The change-password screen asks why the owner is changing it. "Someone may know my old password" also runs **Key Rotation**.

## Consequences

- Without **Key Rotation**, someone holding an old copy plus the old **Master Password** could decrypt a newer copy of the same **Vault** if they obtained it. The UI says so plainly.
- After **Key Rotation** the old **Recovery Kit** no longer opens this copy (or later copies), but it still opens older copies.

## Considered Options

- **Always rotate on password change, re-wrapping for recovery with an X25519 recovery public key**: one action for the owner, but asymmetric crypto adds surface (rejected for the same reason as in ADR-0004).
- **Ask for the Recovery Key during every password change**: rotation is always possible, but it's hostile to beginners.
