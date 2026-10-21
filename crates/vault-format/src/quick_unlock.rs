//! The narrow Rust-to-Keychain custody boundary. No file-format or credential-slot changes.

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{Error, Limits, UnlockedVault, VaultKey, decrypt, header, reopen, unpad};

const PAYLOAD_MAGIC: &[u8; 16] = b"ENOTE-QUICK-v1\0\0";
/// Exact byte length accepted from the protected Keychain item.
pub const QUICK_UNLOCK_PAYLOAD_LEN: usize = 16 + 32 + 32;

/// Non-secret fingerprint of the Vault identity, KDF parameters and both credential slots.
/// The body nonce is excluded, so ordinary saves keep the same binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickUnlockBinding([u8; 32]);

impl QuickUnlockBinding {
    /// Non-secret settings representation; never carries any key material.
    pub fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Only the body-encryption key and its exact credential-header binding. Wiped on drop.
/// This Rust-only type deliberately has no Clone, Serialize, or raw-key accessor.
pub struct QuickUnlockKey {
    binding: QuickUnlockBinding,
    vault_key: VaultKey,
}

impl std::fmt::Debug for QuickUnlockKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QuickUnlockKey(..)")
    }
}

impl QuickUnlockKey {
    pub fn binding(&self) -> QuickUnlockBinding {
        self.binding
    }

    /// Supply a bounded temporary payload only to the Rust Keychain adapter. The adapter must
    /// use its protected, device-only item; this payload must never enter IPC, logs or a file.
    pub fn with_keychain_payload<T>(&self, write: impl FnOnce(&[u8]) -> T) -> T {
        let mut payload = Zeroizing::new([0u8; QUICK_UNLOCK_PAYLOAD_LEN]);
        payload[..16].copy_from_slice(PAYLOAD_MAGIC);
        payload[16..48].copy_from_slice(&self.binding.0);
        payload[48..].copy_from_slice(self.vault_key.0.as_ref());
        write(payload.as_ref())
    }

    /// Take ownership of the protected item's bytes so that all parse paths wipe the input.
    /// A valid payload still proves nothing about the current file until authenticated opening.
    pub fn from_keychain_payload(payload: Zeroizing<Vec<u8>>) -> Result<Self, Error> {
        if payload.len() != QUICK_UNLOCK_PAYLOAD_LEN || &payload[..16] != PAYLOAD_MAGIC {
            return Err(Error::Malformed("invalid Quick Unlock payload"));
        }
        let mut binding = [0u8; 32];
        binding.copy_from_slice(&payload[16..48]);
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&payload[48..]);
        Ok(Self {
            binding: QuickUnlockBinding(binding),
            vault_key: VaultKey(key),
        })
    }
}

fn binding(header: &header::Header) -> QuickUnlockBinding {
    // The nonce is fixed only for this fingerprint; every actual seal still draws a fresh one.
    QuickUnlockBinding(Sha256::digest(header.encode(&[0; 24])).into())
}

/// Validate the file structure and KDF limits, then read its non-secret registration binding.
pub fn quick_unlock_binding(bytes: &[u8], limits: &Limits) -> Result<QuickUnlockBinding, Error> {
    let file = header::parse(bytes)?;
    limits.check(file.header.kdf)?;
    Ok(binding(&file.header))
}

/// Enroll only an already authenticated, committed file matching the held Vault key and header.
/// The caller must obtain fresh native Master Password authorization before calling this.
/// No Wallet Key can be enrolled: UnlockedVault never holds one.
pub fn enroll_quick_unlock(
    unlocked: &UnlockedVault,
    committed_bytes: &[u8],
    limits: &Limits,
) -> Result<QuickUnlockKey, Error> {
    let binding = quick_unlock_binding(committed_bytes, limits)?;
    drop(reopen(unlocked, committed_bytes)?);
    Ok(QuickUnlockKey {
        binding,
        vault_key: VaultKey(Zeroizing::new(*unlocked.vault_key.0)),
    })
}

/// Open the current committed file with the protected item's actual Vault Key. Checks the same
/// structure, limits, authenticated header and body rules as credential opening. Never trusts
/// a biometric success boolean or Vault ID alone; never reconstructs or retains a Wallet Key.
pub fn open_with_quick_unlock(
    bytes: &[u8],
    key: QuickUnlockKey,
    limits: &Limits,
) -> Result<(UnlockedVault, Zeroizing<Vec<u8>>), Error> {
    let file = header::parse(bytes)?;
    limits.check(file.header.kdf)?;
    if binding(&file.header) != key.binding {
        return Err(Error::WrongCredential);
    }
    let padded = decrypt(
        &key.vault_key.0,
        &file.body_nonce,
        file.body,
        file.header_bytes,
    )
    .ok_or(Error::Damaged)?;
    let body = unpad(&padded).ok_or(Error::Damaged)?;
    Ok((
        UnlockedVault {
            header: file.header,
            vault_key: key.vault_key,
        },
        body,
    ))
}
