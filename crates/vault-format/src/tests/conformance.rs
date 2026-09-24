//! Format conformance: the byte layout in the crate docs (the source of truth for FORMAT.md and
//! the Emergency Reader) is checked from the outside, with the crypto crates used directly.
//!
//! Round-trip tests can't catch a change made the same way on the write and read sides. These
//! can: one test decrypts a file written by [`create`] by hand, and the other writes a file by
//! hand, pins its bytes, and requires [`open`] to read it. Nothing here calls the crate's own
//! AAD, padding or KDF helpers.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

use crate::{
    Credential, Error, KdfParams, Limits, PADDING_BLOCK, RecoveryKey, create, open,
    open_wallet_field, seal_wallet_field, unlock_wallet_key,
};

use super::hex;
use super::layout::{BODY_NONCE, HEADER_LEN, PASSWORD_SLOT, RECOVERY_SLOT, SLOT_LEN};

// A password whose NFC form differs from its NFD form ("é" composed vs decomposed) and from its
// NFKC form (U+FB01 "ﬁ" ligature, which only compatibility normalisation rewrites to "fi").
/// What the owner types, in NFD: "é" decomposed.
const TYPED_PASSWORD: &str = "correct-horse-te\u{301}st-\u{FB01}-password-1";
/// NFC of [`TYPED_PASSWORD`], written out by hand.
const NFC_PASSWORD: &str = "correct-horse-t\u{E9}st-\u{FB01}-password-1";
const NFKC_PASSWORD: &str = "correct-horse-t\u{E9}st-fi-password-1";

const MAGIC: &[u8; 8] = b"\x89ENOTE\r\n";
const SLOT_PASSWORD: u8 = 1;
const SLOT_RECOVERY: u8 = 2;
const KDF: KdfParams = KdfParams::TESTING;

// ---------------------------------------------------------------------------------------------
// The documented layout, rebuilt independently
// ---------------------------------------------------------------------------------------------

fn le32(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap().to_le_bytes()
}

/// KEK = Argon2id v0x13(input, salt, m, t, p) -> 32 bytes.
fn kek(input: &[u8], salt: &[u8]) -> [u8; 32] {
    let params = Params::new(KDF.m_kib, KDF.t, KDF.p, Some(32)).unwrap();
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(input, salt, &mut out)
        .unwrap();
    out
}

/// magic || format_version || flags || vault_id || kdf_id || m || t || p || slot_count.
fn fixed_header(vault_id: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&1u16.to_le_bytes()); // format_version
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(vault_id);
    out.push(1); // kdf_id: Argon2id v0x13
    out.extend_from_slice(&KDF.m_kib.to_le_bytes());
    out.extend_from_slice(&KDF.t.to_le_bytes());
    out.extend_from_slice(&KDF.p.to_le_bytes());
    out.push(2); // slot_count
    out
}

/// magic || format_version || vault_id || kdf_id || m || t || p || slot_type.
fn slot_aad(vault_id: &[u8], slot_type: u8) -> Vec<u8> {
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(&1u16.to_le_bytes());
    aad.extend_from_slice(vault_id);
    aad.push(1);
    aad.extend_from_slice(&KDF.m_kib.to_le_bytes());
    aad.extend_from_slice(&KDF.t.to_le_bytes());
    aad.extend_from_slice(&KDF.p.to_le_bytes());
    aad.push(slot_type);
    aad
}

/// b"enote-wallet-v1" || vault_id || len(note_id) u32 || note_id || len(field) u32 || field.
fn wallet_aad(vault_id: &[u8], note_id: &str, field: &str) -> Vec<u8> {
    let mut aad = b"enote-wallet-v1".to_vec();
    aad.extend_from_slice(vault_id);
    aad.extend_from_slice(&le32(note_id.len()));
    aad.extend_from_slice(note_id.as_bytes());
    aad.extend_from_slice(&le32(field.len()));
    aad.extend_from_slice(field.as_bytes());
    aad
}

/// len u32 || plaintext || zero bytes up to a multiple of 4096.
fn padded(plaintext: &[u8]) -> Vec<u8> {
    let mut out = le32(plaintext.len()).to_vec();
    out.extend_from_slice(plaintext);
    out.resize(out.len().next_multiple_of(4096), 0);
    out
}

fn aead_seal(key: &[u8], nonce: &[u8], msg: &[u8], aad: &[u8]) -> Vec<u8> {
    XChaCha20Poly1305::new(key.try_into().unwrap())
        .encrypt(nonce.try_into().unwrap(), Payload { msg, aad })
        .unwrap()
}

fn aead_open(key: &[u8], nonce: &[u8], msg: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    XChaCha20Poly1305::new(key.try_into().unwrap())
        .decrypt(nonce.try_into().unwrap(), Payload { msg, aad })
        .ok()
}

/// Obviously fake fixed bytes: `start, start + 1, ...`.
fn pattern<const N: usize>(start: u8) -> [u8; N] {
    std::array::from_fn(|i| start.wrapping_add(u8::try_from(i).unwrap()))
}

// Fixed inputs of the hand-written known-answer Vault.
fn kat_vault_id() -> [u8; 16] {
    pattern(0x10)
}
fn kat_vault_key() -> [u8; 32] {
    pattern(0x20)
}
fn kat_wallet_key() -> [u8; 32] {
    pattern(0x40)
}
fn kat_recovery_bytes() -> [u8; 16] {
    pattern(0x60)
}
/// Display form of [`kat_recovery_bytes`], computed outside this crate.
const KAT_RECOVERY_DISPLAY: &str = "C1GP-4RV4-CNK6-ET39-D9NP-RVBE-DW5V";
const KAT_BODY: &[u8] = b"fake known-answer vault body";
/// SHA-256 of the hand-written known-answer Vault file for [`KAT_BODY`].
const KAT_FILE_SHA256: &str = "0fa453ecc8afbf46f0cf8a7ba140a2b09e10cf3c79324149edbc9f41c7ab756c";

/// A complete Vault file written by hand from the documented layout, with `padded_plaintext`
/// as the body (so tests can also write bodies the crate itself would never produce).
fn hand_written_vault(padded_plaintext: &[u8]) -> Vec<u8> {
    let vault_id = kat_vault_id();
    let mut keys = kat_vault_key().to_vec();
    keys.extend_from_slice(&kat_wallet_key());

    let mut file = fixed_header(&vault_id);
    let recovery_bytes = kat_recovery_bytes();
    let slots: [(u8, &[u8], u8); 2] = [
        (SLOT_PASSWORD, NFC_PASSWORD.as_bytes(), 0x80),
        (SLOT_RECOVERY, &recovery_bytes, 0xa0),
    ];
    for (slot_type, input, seed) in slots {
        let salt: [u8; 16] = pattern(seed);
        let nonce: [u8; 24] = pattern(seed.wrapping_add(0x10));
        file.push(slot_type);
        file.extend_from_slice(&salt);
        file.extend_from_slice(&nonce);
        let wrapped = aead_seal(
            &kek(input, &salt),
            &nonce,
            &keys,
            &slot_aad(&vault_id, slot_type),
        );
        assert_eq!(wrapped.len(), 80);
        file.extend_from_slice(&wrapped);
    }
    let body_nonce: [u8; 24] = pattern(0xc0);
    file.extend_from_slice(&body_nonce);
    assert_eq!(file.len(), HEADER_LEN);
    let body = aead_seal(&kat_vault_key(), &body_nonce, padded_plaintext, &file);
    file.extend_from_slice(&body);
    file
}

// ---------------------------------------------------------------------------------------------
// Writer side: what `create` / `seal_wallet_field` produce matches the documented layout
// ---------------------------------------------------------------------------------------------

#[test]
fn a_created_vault_decrypts_by_hand_following_the_documented_layout() {
    let created = create(
        &SecretString::from(TYPED_PASSWORD),
        KDF,
        &Limits::TESTING,
        KAT_BODY,
    )
    .unwrap();
    let bytes = &created.bytes;
    let vault_id = created.unlocked.vault_id();
    assert_eq!(bytes[..PASSWORD_SLOT], fixed_header(&vault_id));

    // Each slot: slot_type(1) || salt(16) || nonce(24) || wrapped(80).
    let slot_fields = |at: usize| {
        let slot = &bytes[at..at + SLOT_LEN];
        (slot[0], &slot[1..17], &slot[17..41], &slot[41..])
    };

    // Password slot: KEK from the NFC password, as UTF-8.
    let (slot_type, salt, nonce, wrapped) = slot_fields(PASSWORD_SLOT);
    assert_eq!(slot_type, SLOT_PASSWORD);
    let aad = slot_aad(&vault_id, SLOT_PASSWORD);
    let keys = aead_open(&kek(NFC_PASSWORD.as_bytes(), salt), nonce, wrapped, &aad)
        .expect("password slot opens with a KEK from the NFC password");
    assert_eq!(keys.len(), 64);
    // Control: NFD (the raw typed bytes) and NFKC are not what the format feeds Argon2id.
    for other in [TYPED_PASSWORD, NFKC_PASSWORD] {
        assert!(aead_open(&kek(other.as_bytes(), salt), nonce, wrapped, &aad).is_none());
    }

    // Recovery slot: KEK from the Recovery Key's 16 raw bytes; same two keys inside.
    let (slot_type, salt, nonce, wrapped) = slot_fields(RECOVERY_SLOT);
    assert_eq!(slot_type, SLOT_RECOVERY);
    let recovery_kek = kek(created.recovery_key.expose_bytes(), salt);
    let recovery_keys = aead_open(
        &recovery_kek,
        nonce,
        wrapped,
        &slot_aad(&vault_id, SLOT_RECOVERY),
    )
    .expect("recovery slot opens with a KEK from the Recovery Key bytes");
    assert_eq!(recovery_keys, keys);
    let (vault_key, wallet_key) = keys.split_at(32);

    // Body: AAD = every header byte, body nonce included.
    let plaintext = aead_open(
        vault_key,
        &bytes[BODY_NONCE..HEADER_LEN],
        &bytes[HEADER_LEN..],
        &bytes[..HEADER_LEN],
    )
    .expect("body opens with the Vault Key and the header as AAD");
    assert_eq!(plaintext, padded(KAT_BODY));

    // Wallet field: nonce(24) || AEAD(wallet_key, nonce, value, wallet AAD with u32 LE lengths).
    let current = SecretString::from(NFC_PASSWORD);
    let crate_wallet_key = unlock_wallet_key(
        &created.unlocked,
        Credential::MasterPassword(&current),
        &Limits::TESTING,
    )
    .unwrap();
    let sealed = seal_wallet_field(
        &crate_wallet_key,
        &vault_id,
        "note-7",
        "seed",
        b"fake seed words",
    )
    .unwrap();
    let (nonce, ciphertext) = sealed.split_at(24);
    let aad = wallet_aad(&vault_id, "note-7", "seed");
    assert_eq!(
        aead_open(wallet_key, nonce, ciphertext, &aad).as_deref(),
        Some(&b"fake seed words"[..])
    );
}

// ---------------------------------------------------------------------------------------------
// Reader side: a pinned, hand-written file opens
// ---------------------------------------------------------------------------------------------

#[test]
fn the_hand_written_known_answer_vault_is_pinned() {
    let file = hand_written_vault(&padded(KAT_BODY));
    assert_eq!(Sha256::digest(&file).to_vec(), hex(KAT_FILE_SHA256));
}

#[test]
fn a_hand_written_vault_opens_with_both_credentials() {
    let file = hand_written_vault(&padded(KAT_BODY));

    let typed = SecretString::from(TYPED_PASSWORD);
    let (unlocked, body) =
        open(&file, Credential::MasterPassword(&typed), &Limits::TESTING).unwrap();
    assert_eq!(body.as_slice(), KAT_BODY);
    assert_eq!(unlocked.vault_id(), kat_vault_id());
    assert_eq!(unlocked.kdf(), KDF);

    let recovery = RecoveryKey::parse(KAT_RECOVERY_DISPLAY).unwrap();
    assert_eq!(recovery.expose_bytes(), &kat_recovery_bytes());
    let (_, body) = open(&file, Credential::RecoveryKey(&recovery), &Limits::TESTING).unwrap();
    assert_eq!(body.as_slice(), KAT_BODY);

    // Compatibility normalisation (NFKC) is not applied: "ﬁ" stays a ligature.
    let nfkc = SecretString::from(NFKC_PASSWORD);
    assert_eq!(
        open(&file, Credential::MasterPassword(&nfkc), &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
}

#[test]
fn a_hand_written_wallet_field_opens_with_the_unwrapped_wallet_key() {
    let file = hand_written_vault(&padded(KAT_BODY));
    let recovery = RecoveryKey::parse(KAT_RECOVERY_DISPLAY).unwrap();
    let credential = Credential::RecoveryKey(&recovery);
    let (unlocked, _) = open(&file, credential, &Limits::TESTING).unwrap();
    let wallet_key = unlock_wallet_key(&unlocked, credential, &Limits::TESTING).unwrap();

    let vault_id = kat_vault_id();
    let nonce: [u8; 24] = pattern(0xe0);
    let mut sealed = nonce.to_vec();
    sealed.extend(aead_seal(
        &kat_wallet_key(),
        &nonce,
        b"fake private key",
        &wallet_aad(&vault_id, "note-1", "private key"),
    ));
    let opened = open_wallet_field(&wallet_key, &vault_id, "note-1", "private key", &sealed);
    assert_eq!(opened.unwrap().as_slice(), b"fake private key");
}

// ---------------------------------------------------------------------------------------------
// Padding: authenticated but inconsistent bodies are Damaged, never a panic
// ---------------------------------------------------------------------------------------------

fn open_body(padded_plaintext: &[u8]) -> Result<Vec<u8>, Error> {
    let file = hand_written_vault(padded_plaintext);
    let typed = SecretString::from(TYPED_PASSWORD);
    open(&file, Credential::MasterPassword(&typed), &Limits::TESTING).map(|(_, body)| body.to_vec())
}

/// One padding block holding `len` as the length field and `body` after it.
fn block_with(len: u32, body: &[u8], blocks: usize) -> Vec<u8> {
    let mut out = len.to_le_bytes().to_vec();
    out.extend_from_slice(body);
    out.resize(blocks * PADDING_BLOCK, 0);
    out
}

#[test]
fn a_body_that_exactly_fills_its_padding_blocks_opens() {
    let largest = vec![0x61; PADDING_BLOCK - 4];
    assert_eq!(open_body(&padded(&largest)).unwrap(), largest);
    assert_eq!(padded(&largest).len(), PADDING_BLOCK);
    assert_eq!(open_body(&padded(b"")).unwrap(), b"");
}

#[test]
fn a_length_field_that_does_not_fit_the_padded_body_is_damaged() {
    // u32::MAX, one past the plaintext, and a whole block.
    for len in [u32::MAX, PADDING_BLOCK as u32 - 3, PADDING_BLOCK as u32] {
        assert_eq!(
            open_body(&block_with(len, b"", 1)),
            Err(Error::Damaged),
            "len {len}"
        );
    }
}

#[test]
fn a_non_zero_byte_in_the_padding_is_damaged() {
    let mut body = block_with(5, b"fake!", 1);
    *body.last_mut().unwrap() = 1;
    assert_eq!(open_body(&body), Err(Error::Damaged));
    let mut body = block_with(5, b"fake!", 1);
    body[4 + 5] = 0x80;
    assert_eq!(open_body(&body), Err(Error::Damaged));
}

#[test]
fn padding_longer_than_the_next_multiple_of_the_block_is_damaged() {
    // len 5 fits in one block, so a two-block body is not the one valid encoding.
    assert_eq!(open_body(&block_with(5, b"fake!", 2)), Err(Error::Damaged));
    // ... while a body that really needs two blocks is fine.
    let big = vec![0x62; PADDING_BLOCK - 3];
    assert_eq!(open_body(&padded(&big)).unwrap(), big);
}
