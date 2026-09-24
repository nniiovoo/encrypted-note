//! Vault crypto and file format.
//!
//! This is the deepest module of encrypted-note: everything that turns a **Master Password** or
//! **Recovery Key** into keys, and keys into an encrypted **Vault** file, lives here. It does no
//! I/O. Callers hand it bytes and get bytes back.
//!
//! Decisions it implements: ADR-0004 (Wallet Key), ADR-0005 (credential slots, password change
//! vs Key Rotation). The byte layout below is the source of truth for FORMAT.md.
//!
//! # File layout (format version 1, all integers little-endian)
//!
//! ```text
//! header:
//!   magic            8   b"\x89ENOTE\r\n"
//!   format_version   2   = 1
//!   flags            2   = 0 (reject anything else)
//!   vault_id        16   random, fixed for the life of a Vault (kept across Key Rotation)
//!   kdf_id           1   = 1 (Argon2id, version 0x13)
//!   kdf_m_kib        4
//!   kdf_t            4
//!   kdf_p            4
//!   slot_count       1   = 2
//!   slots (x2):
//!     slot_type      1   1 = Master Password, 2 = Recovery Key (exactly one of each, in this order)
//!     salt          16   random, new whenever the slot is (re)written
//!     nonce         24   random, new whenever the slot is (re)written
//!     wrapped       80   XChaCha20-Poly1305(KEK, nonce, vault_key(32) || wallet_key(32)) incl. 16-byte tag
//!   body_nonce      24   random, new on every seal
//! body:
//!   XChaCha20-Poly1305(vault_key, body_nonce, padded_plaintext), associated data = all header bytes
//! padded_plaintext:
//!   len u32 || plaintext || zero bytes up to a multiple of 4096
//! ```
//!
//! * KEK = Argon2id(input, salt, m, t, p, 32 bytes). Input for the password slot is the password
//!   normalised to Unicode NFC, as UTF-8. Input for the recovery slot is the Recovery Key's 16
//!   raw random bytes.
//! * Slot associated data = magic || format_version || vault_id || kdf_id || m || t || p || slot_type.
//!   So editing any KDF parameter, the vault id, or swapping slots makes unwrapping fail.
//! * Wallet fields (ADR-0004) = nonce(24) || XChaCha20-Poly1305(wallet_key, nonce, value), with
//!   associated data = b"enote-wallet-v1" || vault_id || len(note_id) u32 || note_id || len(field) u32 || field.
//!
//! # Verifying what was written
//!
//! [`reopen`] decrypts a file with the Vault Key the app already holds (no Argon2id), after
//! checking that every header byte but the body nonce is the one it holds. The app runs it on
//! each save and each Backup it has just written, before trusting the file.
//!
//! # Security rules for implementers
//! * Every nonce comes from the OS random generator (`getrandom`). Never reuse, never count.
//! * Use the non-in-place AEAD APIs; never look at plaintext when authentication fails.
//! * Keys and plaintext live in `Zeroizing` / `secrecy` types. No `Debug` that prints secrets.
//! * KDF parameters read from a file are checked against [`Limits`] before any derivation runs.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Version};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use secrecy::{ExposeSecret, SecretString};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

/// `Debug` that prints only the type name, never the secret inside.
macro_rules! redacted_debug {
    ($($name:ident),+) => {$(
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(..)"))
            }
        }
    )+};
}

pub mod recovery_key;
pub use recovery_key::{RecoveryKey, RecoveryKeyError};

mod header;
use header::{Header, Slot, SlotType};

// Build-time guard: the AEAD cipher copies every key it is given (Vault Key, Wallet Key, KEK),
// and the Recovery Key checksum feeds its 16 bytes through SHA-256; both must wipe their copy on
// drop. This stops compiling if either `zeroize` feature is dropped.
const _: () = {
    fn wipes_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    let _ = wipes_on_drop::<XChaCha20Poly1305>;
    let _ = wipes_on_drop::<sha2::Sha256>;
};

/// First 8 bytes of every Vault file.
pub const MAGIC: [u8; 8] = *b"\x89ENOTE\r\n";
/// The only format version this code writes and reads.
pub const FORMAT_VERSION: u16 = 1;
/// Plaintext bodies are padded to a multiple of this many bytes, so the file size leaks less.
pub const PADDING_BLOCK: usize = 4096;

/// Argon2id cost parameters as stored in the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Lanes (parallelism).
    pub p: u32,
}

impl KdfParams {
    /// Starting point for calibration: 512 MiB, t=3, p=4.
    pub const DEFAULT: KdfParams = KdfParams {
        m_kib: 512 * 1024,
        t: 3,
        p: 4,
    };
    /// Tiny parameters for fast tests. Only valid under [`Limits::TESTING`].
    #[cfg(any(test, feature = "insecure-test-params"))]
    pub const TESTING: KdfParams = KdfParams {
        m_kib: 8,
        t: 1,
        p: 1,
    };
}

/// Floor and ceiling for KDF parameters. The floor stops a doctored file from downgrading
/// protection; the ceiling stops a doctored file from freezing the app.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min: KdfParams,
    pub max: KdfParams,
}

impl Limits {
    /// Used by the app and the Emergency Reader: m >= 64 MiB, t >= 3, p >= 1; m <= 2 GiB, t <= 20, p <= 16.
    pub const PRODUCTION: Limits = Limits {
        min: KdfParams {
            m_kib: 64 * 1024,
            t: 3,
            p: 1,
        },
        max: KdfParams {
            m_kib: 2 * 1024 * 1024,
            t: 20,
            p: 16,
        },
    };
    /// Tiny floor for tests only.
    #[cfg(any(test, feature = "insecure-test-params"))]
    pub const TESTING: Limits = Limits {
        min: KdfParams::TESTING,
        max: Limits::PRODUCTION.max,
    };

    /// `Err(Error::KdfOutOfLimits)` if any parameter is below `min` or above `max`
    /// (also if `m_kib < 8 * p`, which Argon2 forbids).
    pub fn check(&self, params: KdfParams) -> Result<(), Error> {
        let KdfParams { m_kib, t, p } = params;
        let (min, max) = (self.min, self.max);
        let in_range = (min.m_kib..=max.m_kib).contains(&m_kib)
            && (min.t..=max.t).contains(&t)
            && (min.p..=max.p).contains(&p)
            && u64::from(m_kib) >= 8 * u64::from(p);
        in_range.then_some(()).ok_or(Error::KdfOutOfLimits)
    }
}

/// Everything that can go wrong. Messages are for logs/tests; the UI maps variants to its own wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a Vault file, truncated, or a structural field has an impossible value.
    Malformed(&'static str),
    /// A Vault written by a newer (or unknown) format version.
    UnsupportedVersion(u16),
    /// KDF parameters outside [`Limits`].
    KdfOutOfLimits,
    /// The Master Password or Recovery Key did not unwrap its slot.
    WrongCredential,
    /// The slot unwrapped but the body (or a wallet field) failed authentication: the file was
    /// damaged or tampered with.
    Damaged,
    /// The OS random generator failed.
    Randomness,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

/// 32-byte key that encrypts the body. Wiped on drop.
pub struct VaultKey(Zeroizing<[u8; 32]>);
/// 32-byte key that encrypts Wallet Kind Hidden Fields (ADR-0004). Wiped on drop.
pub struct WalletKey(Zeroizing<[u8; 32]>);

redacted_debug!(VaultKey, WalletKey, UnlockedVault);

/// A way into the Vault.
#[derive(Clone, Copy)]
pub enum Credential<'a> {
    MasterPassword(&'a SecretString),
    RecoveryKey(&'a RecoveryKey),
}

/// Non-secret facts readable from a header without any credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderInfo {
    pub format_version: u16,
    pub vault_id: [u8; 16],
    pub kdf: KdfParams,
}

/// An open Vault: the parsed header plus the Vault Key. Holds **no** Wallet Key.
pub struct UnlockedVault {
    header: Header,
    vault_key: VaultKey,
}

impl UnlockedVault {
    pub fn vault_id(&self) -> [u8; 16] {
        self.header.vault_id
    }
    pub fn kdf(&self) -> KdfParams {
        self.header.kdf
    }
}

/// Result of [`create`].
#[derive(Debug)]
pub struct Created {
    /// The complete Vault file.
    pub bytes: Vec<u8>,
    /// Show to the owner once, for their Recovery Kit. Never store it.
    pub recovery_key: RecoveryKey,
    pub unlocked: UnlockedVault,
}

/// Result of [`rotate_keys`] (ADR-0005). The caller re-encrypts wallet fields from
/// `old_wallet_key` to `new_wallet_key`, then [`seal`]s with `unlocked`.
#[derive(Debug)]
pub struct Rotation {
    pub unlocked: UnlockedVault,
    pub recovery_key: RecoveryKey,
    pub old_wallet_key: WalletKey,
    pub new_wallet_key: WalletKey,
}

/// Create a brand-new Vault: random vault id, Vault Key, Wallet Key and Recovery Key; both slots
/// written with `params` (checked against `limits`); `body` sealed.
pub fn create(
    master_password: &SecretString,
    params: KdfParams,
    limits: &Limits,
    body: &[u8],
) -> Result<Created, Error> {
    limits.check(params)?;
    let (unlocked, _, recovery_key) = fresh_keys(master_password, random([0; 16])?, params)?;
    let bytes = seal(&unlocked, body)?;
    Ok(Created {
        bytes,
        recovery_key,
        unlocked,
    })
}

/// Read the non-secret header facts (for previews and FORMAT checks). Validates structure,
/// version and limits but derives nothing.
pub fn inspect(bytes: &[u8], limits: &Limits) -> Result<HeaderInfo, Error> {
    let file = header::parse(bytes)?;
    limits.check(file.header.kdf)?;
    Ok(HeaderInfo {
        format_version: FORMAT_VERSION,
        vault_id: file.header.vault_id,
        kdf: file.header.kdf,
    })
}

/// Open a Vault with either credential. Returns the unlocked Vault (Vault Key only; the Wallet
/// Key unwrapped alongside it is wiped before returning) and the decrypted body plaintext.
///
/// Error order: structure/version -> limits -> `WrongCredential` (slot fails) -> `Damaged` (body fails).
pub fn open(
    bytes: &[u8],
    credential: Credential<'_>,
    limits: &Limits,
) -> Result<(UnlockedVault, Zeroizing<Vec<u8>>), Error> {
    let file = header::parse(bytes)?;
    // The Wallet Key is dropped (and wiped) at the end of this statement.
    let (vault_key, _) = unwrap_slot(&file.header, credential, limits)?;

    let padded = decrypt(&vault_key.0, &file.body_nonce, file.body, file.header_bytes)
        .ok_or(Error::Damaged)?;
    let body = unpad(&padded).ok_or(Error::Damaged)?;
    Ok((
        UnlockedVault {
            header: file.header,
            vault_key,
        },
        body,
    ))
}

/// Decrypt a Vault file with the Vault Key already held in `unlocked`: no credential, no
/// Argon2id. The app uses it to check a file it has just written (a save, a Backup) before
/// trusting it.
///
/// Error order: structure/version (as in [`open`]) -> `WrongCredential` unless every header
/// byte except the body nonce equals `unlocked`'s header (same vault id, KDF parameters and
/// both slots, so a file from another Vault, or from before a password change or Key Rotation,
/// is refused) -> `Damaged` (body fails). KDF limits are not checked: nothing is derived, and
/// the parameters must equal the ones already held.
pub fn reopen(unlocked: &UnlockedVault, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let file = header::parse(bytes)?;
    let ours = unlocked.header.encode(&file.body_nonce);
    if ours != file.header_bytes {
        return Err(Error::WrongCredential);
    }
    let padded = decrypt(
        &unlocked.vault_key.0,
        &file.body_nonce,
        file.body,
        file.header_bytes,
    )
    .ok_or(Error::Damaged)?;
    unpad(&padded).ok_or(Error::Damaged)
}

/// Produce a new Vault file for `body`: same slots, fresh body nonce, padded plaintext.
pub fn seal(unlocked: &UnlockedVault, body: &[u8]) -> Result<Vec<u8>, Error> {
    let padded = pad(body)?;
    let body_nonce = random([0; 24])?;
    let mut bytes = unlocked.header.encode(&body_nonce);
    bytes.extend(encrypt(
        &unlocked.vault_key.0,
        &body_nonce,
        &padded,
        &bytes,
    )?);
    Ok(bytes)
}

/// Re-derive the Wallet Key from a credential (fresh Argon2id run) using the slots held in
/// `unlocked`. Used for every show/copy/create/edit of a Wallet Kind (ADR-0004).
pub fn unlock_wallet_key(
    unlocked: &UnlockedVault,
    credential: Credential<'_>,
    limits: &Limits,
) -> Result<WalletKey, Error> {
    unwrap_slot(&unlocked.header, credential, limits).map(|(_, wallet_key)| wallet_key)
}

/// Re-wrap only the Master Password slot under `new_password` (new salt and nonce). The
/// Recovery Key slot and both keys stay the same. `current` proves the owner may do this and
/// supplies the Wallet Key needed to rebuild the slot. Persist by calling [`seal`] afterwards.
pub fn change_password(
    unlocked: &mut UnlockedVault,
    current: Credential<'_>,
    new_password: &SecretString,
    limits: &Limits,
) -> Result<(), Error> {
    let header = &unlocked.header;
    let (_, wallet_key) = unwrap_slot(header, current, limits)?;
    let slot = wrap_slot(
        Credential::MasterPassword(new_password),
        &header.vault_id,
        header.kdf,
        (&unlocked.vault_key, &wallet_key),
    )?;
    unlocked.header.slots[SlotType::MasterPassword.index()] = slot;
    Ok(())
}

/// Key Rotation (ADR-0005): new Vault Key, Wallet Key and Recovery Key; both slots rewritten
/// (the password slot under `new_password`); vault id and KDF parameters kept.
pub fn rotate_keys(
    unlocked: &UnlockedVault,
    current: Credential<'_>,
    new_password: &SecretString,
    limits: &Limits,
) -> Result<Rotation, Error> {
    let (_, old_wallet_key) = unwrap_slot(&unlocked.header, current, limits)?;
    let Header { vault_id, kdf, .. } = unlocked.header;
    let (unlocked, new_wallet_key, recovery_key) = fresh_keys(new_password, vault_id, kdf)?;
    Ok(Rotation {
        unlocked,
        recovery_key,
        old_wallet_key,
        new_wallet_key,
    })
}

/// Encrypt one Wallet Kind Hidden Field value (layout in the module docs).
pub fn seal_wallet_field(
    wallet_key: &WalletKey,
    vault_id: &[u8; 16],
    note_id: &str,
    field: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, Error> {
    let aad = wallet_field_aad(vault_id, note_id, field)
        .ok_or(Error::Malformed("note id or field too long"))?;
    let nonce = random([0u8; 24])?;
    let mut out = nonce.to_vec();
    out.extend(encrypt(&wallet_key.0, &nonce, plaintext, &aad)?);
    Ok(out)
}

/// Decrypt one Wallet Kind Hidden Field value. `Err(Damaged)` if the key, ids or bytes don't match.
pub fn open_wallet_field(
    wallet_key: &WalletKey,
    vault_id: &[u8; 16],
    note_id: &str,
    field: &str,
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let aad = wallet_field_aad(vault_id, note_id, field).ok_or(Error::Damaged)?;
    let (nonce, ciphertext) = sealed.split_first_chunk::<24>().ok_or(Error::Damaged)?;
    decrypt(&wallet_key.0, nonce, ciphertext, &aad).ok_or(Error::Damaged)
}

/// Pick KDF parameters that take 0.5-1 s on this machine, within `limits`.
///
/// `measure` runs one derivation with the given parameters and returns how long it took; it is
/// injected so tests can use a fake timer. Cheapest first, so a slow or memory-starved machine
/// spends seconds here, not minutes: start at the memory floor with `DEFAULT.t` and `DEFAULT.p`
/// (clamped into `limits`). While a run takes < 0.5 s, double memory up to `DEFAULT.m_kib`
/// (never above `limits.max.m_kib`); a doubling that takes > 1 s is not kept. Then, while a run
/// takes < 0.5 s and `t < limits.max.t`, add one iteration; if that pushes it over 1 s, step back
/// one. Returns parameters that pass `limits.check`.
pub fn calibrate(limits: &Limits, mut measure: impl FnMut(KdfParams) -> Duration) -> KdfParams {
    const TOO_SLOW: Duration = Duration::from_secs(1);
    const TOO_FAST: Duration = Duration::from_millis(500);
    let (min, max) = (limits.min, limits.max);
    // `max(lo).min(hi)` rather than `clamp`, which panics on inverted limits.
    let p = KdfParams::DEFAULT.p.max(min.p).min(max.p);
    let m_floor = min.m_kib.max(p.saturating_mul(8)).min(max.m_kib);
    let m_ceiling = KdfParams::DEFAULT.m_kib.min(max.m_kib).max(m_floor);
    let mut params = KdfParams {
        m_kib: m_floor,
        t: KdfParams::DEFAULT.t.max(min.t).min(max.t),
        p,
    };

    let mut took = measure(params);
    while took < TOO_FAST && params.m_kib < m_ceiling {
        let bigger = KdfParams {
            m_kib: params.m_kib.saturating_mul(2).min(m_ceiling),
            ..params
        };
        let bigger_took = measure(bigger);
        if bigger_took > TOO_SLOW {
            break;
        }
        (params, took) = (bigger, bigger_took);
    }
    while took < TOO_FAST && params.t < max.t {
        params.t += 1;
        took = measure(params);
        if took > TOO_SLOW {
            params.t -= 1;
            break;
        }
    }
    params
}

/// Time one real Argon2id derivation with `params` (helper for [`calibrate`] in the app).
pub fn measure_kdf(params: KdfParams) -> Duration {
    let started = Instant::now();
    let _ = derive_kek(b"calibration-input", &[0u8; 16], params);
    started.elapsed()
}

/// Fill `buf` (a plain array, or a `Zeroizing` one for keys) from the OS random generator.
fn random<T: AsMut<[u8]>>(mut buf: T) -> Result<T, Error> {
    getrandom::fill(buf.as_mut()).map_err(|_| Error::Randomness)?;
    Ok(buf)
}

/// New Vault Key, Wallet Key and Recovery Key, with both slots written (create and Key Rotation).
fn fresh_keys(
    password: &SecretString,
    vault_id: [u8; 16],
    kdf: KdfParams,
) -> Result<(UnlockedVault, WalletKey, RecoveryKey), Error> {
    let vault_key = VaultKey(random(Zeroizing::new([0; 32]))?);
    let wallet_key = WalletKey(random(Zeroizing::new([0; 32]))?);
    let recovery_key = RecoveryKey::generate()?;
    let keys = (&vault_key, &wallet_key);
    let slots = [
        wrap_slot(Credential::MasterPassword(password), &vault_id, kdf, keys)?,
        wrap_slot(Credential::RecoveryKey(&recovery_key), &vault_id, kdf, keys)?,
    ];
    let header = Header {
        vault_id,
        kdf,
        slots,
    };
    let unlocked = UnlockedVault { header, vault_key };
    Ok((unlocked, wallet_key, recovery_key))
}

/// The bytes a credential feeds into Argon2id (the Master Password normalised to Unicode NFC, as
/// UTF-8; the Recovery Key's 16 bytes), and the slot it opens.
fn credential_input(credential: Credential<'_>) -> (SlotType, Zeroizing<Vec<u8>>) {
    match credential {
        Credential::MasterPassword(password) => {
            let raw = password.expose_secret();
            // NFC grows UTF-8 by at most 3x; reserving that up front avoids reallocations that
            // would leave unwiped copies behind.
            let mut nfc = Zeroizing::new(String::with_capacity(raw.len().saturating_mul(3)));
            nfc.extend(raw.nfc());
            (
                SlotType::MasterPassword,
                Zeroizing::new(nfc.as_bytes().to_vec()),
            )
        }
        Credential::RecoveryKey(key) => (
            SlotType::RecoveryKey,
            Zeroizing::new(key.expose_bytes().to_vec()),
        ),
    }
}

/// KEK = Argon2id v0x13 (input, salt, m, t, p) -> 32 bytes. Callers check [`Limits`] first.
fn derive_kek(input: &[u8], salt: &[u8; 16], kdf: KdfParams) -> Result<Zeroizing<[u8; 32]>, Error> {
    let params = argon2::Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32))
        .map_err(|_| Error::KdfOutOfLimits)?;
    let mut memory = kdf_memory(&params)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut kek = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into_with_memory(input, salt, kek.as_mut(), memory.as_mut_slice())
        .map_err(|_| Error::KdfOutOfLimits)?;
    Ok(kek)
}

/// Argon2id working memory that is wiped on drop. `hash_password_into` frees its own memory
/// unwiped, and the KEK can be recomputed cheaply from it (it is a hash of each lane's last
/// block). An allocation failure is an error, not an abort.
fn kdf_memory(params: &argon2::Params) -> Result<Zeroizing<Vec<argon2::Block>>, Error> {
    let count = params.block_count();
    let mut blocks = Vec::new();
    blocks
        .try_reserve_exact(count)
        .map_err(|_| Error::KdfOutOfLimits)?;
    blocks.resize(count, argon2::Block::new());
    Ok(Zeroizing::new(blocks))
}

/// XChaCha20-Poly1305 encryption; the result is ciphertext || tag.
fn encrypt(key: &[u8; 32], nonce: &[u8; 24], msg: &[u8], aad: &[u8]) -> Result<Vec<u8>, Error> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(nonce.into(), Payload { msg, aad })
        .map_err(|_| Error::Malformed("too large to encrypt"))
}

/// XChaCha20-Poly1305 decryption; `None` when authentication fails (nothing is decrypted then).
fn decrypt(key: &[u8; 32], nonce: &[u8; 24], msg: &[u8], aad: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(nonce.into(), Payload { msg, aad })
        .ok()
        .map(Zeroizing::new)
}

/// Encrypt `vault_key || wallet_key` into a fresh slot (new salt and nonce) for `credential`.
fn wrap_slot(
    credential: Credential<'_>,
    vault_id: &[u8; 16],
    kdf: KdfParams,
    (vault_key, wallet_key): (&VaultKey, &WalletKey),
) -> Result<Slot, Error> {
    let (slot_type, input) = credential_input(credential);
    let salt = random([0; 16])?;
    let nonce = random([0; 24])?;
    let kek = derive_kek(&input, &salt, kdf)?;
    let mut keys = Zeroizing::new([0u8; 64]);
    keys[..32].copy_from_slice(vault_key.0.as_ref());
    keys[32..].copy_from_slice(wallet_key.0.as_ref());
    let aad = header::slot_aad(vault_id, kdf, slot_type);
    let wrapped = encrypt(&kek, &nonce, keys.as_ref(), &aad)?
        .try_into()
        .map_err(|_| Error::Malformed("wrapped keys have the wrong length"))?;
    Ok(Slot {
        salt,
        nonce,
        wrapped,
    })
}

/// Check the header's KDF parameters against `limits`, then derive the credential's KEK and
/// unwrap its slot into (Vault Key, Wallet Key).
fn unwrap_slot(
    header: &Header,
    credential: Credential<'_>,
    limits: &Limits,
) -> Result<(VaultKey, WalletKey), Error> {
    limits.check(header.kdf)?;
    let (slot_type, input) = credential_input(credential);
    let slot = &header.slots[slot_type.index()];
    let kek = derive_kek(&input, &slot.salt, header.kdf)?;
    let aad = header::slot_aad(&header.vault_id, header.kdf, slot_type);
    let keys = decrypt(&kek, &slot.nonce, &slot.wrapped, &aad).ok_or(Error::WrongCredential)?;
    let ([vault_key, wallet_key], []) = keys.as_chunks::<32>() else {
        return Err(Error::WrongCredential);
    };
    Ok((
        VaultKey(Zeroizing::new(*vault_key)),
        WalletKey(Zeroizing::new(*wallet_key)),
    ))
}

/// Size of the padded plaintext for a `body_len`-byte body; `None` on overflow.
fn padded_len(body_len: usize) -> Option<usize> {
    body_len
        .checked_add(4)?
        .checked_next_multiple_of(PADDING_BLOCK)
}

/// `len u32 || body || zeros` up to a multiple of [`PADDING_BLOCK`].
fn pad(body: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
    let (Ok(len), Some(total)) = (u32::try_from(body.len()), padded_len(body.len())) else {
        return Err(Error::Malformed("body too large"));
    };
    let mut padded = Zeroizing::new(Vec::with_capacity(total));
    padded.extend_from_slice(&len.to_le_bytes());
    padded.extend_from_slice(body);
    padded.resize(total, 0);
    Ok(padded)
}

/// Inverse of [`pad`]; `None` if the (authenticated) length or padding is inconsistent, or the
/// padding is longer than [`pad`] writes (so each body has exactly one valid encoding).
fn unpad(padded: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    let (len, rest) = padded.split_first_chunk::<4>()?;
    let len = usize::try_from(u32::from_le_bytes(*len)).ok()?;
    if padded.len() != padded_len(len)? {
        return None;
    }
    let (body, padding) = rest.split_at_checked(len)?;
    padding
        .iter()
        .all(|b| *b == 0)
        .then(|| Zeroizing::new(body.to_vec()))
}

/// b"enote-wallet-v1" || vault_id || len(note_id) u32 || note_id || len(field) u32 || field.
fn wallet_field_aad(vault_id: &[u8; 16], note_id: &str, field: &str) -> Option<Vec<u8>> {
    let mut aad = b"enote-wallet-v1".to_vec();
    aad.extend_from_slice(vault_id);
    for part in [note_id, field] {
        aad.extend_from_slice(&u32::try_from(part.len()).ok()?.to_le_bytes());
        aad.extend_from_slice(part.as_bytes());
    }
    Some(aad)
}

#[cfg(test)]
mod tests;
