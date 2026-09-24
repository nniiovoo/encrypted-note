//! Wallet Kind Hidden Fields under the Wallet Key (ADR-0004).

use crate::Credential;
use crate::{Error, Limits, WalletKey, open_wallet_field, seal_wallet_field, unlock_wallet_key};

use super::{TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

const NOTE_ID: &str = "note-0001";
const FIELD: &str = "key";
const FAKE_KEY: &[u8] = b"0xFAKE-private-key-for-tests-only";

fn wallet_key_and_vault_id() -> (WalletKey, [u8; 16]) {
    let created = testing_vault(TEST_BODY);
    let key = unlock_wallet_key(
        &created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &Limits::TESTING,
    )
    .unwrap();
    (key, created.unlocked.vault_id())
}

#[test]
fn wallet_field_round_trips() {
    let (key, vault_id) = wallet_key_and_vault_id();
    for value in [FAKE_KEY, b""] {
        let sealed = seal_wallet_field(&key, &vault_id, NOTE_ID, FIELD, value).unwrap();
        assert_eq!(sealed.len(), 24 + value.len() + 16);
        let opened = open_wallet_field(&key, &vault_id, NOTE_ID, FIELD, &sealed).unwrap();
        assert_eq!(opened.as_slice(), value);
    }
}

#[test]
fn both_credentials_unlock_the_same_wallet_key() {
    let created = testing_vault(TEST_BODY);
    let vault_id = created.unlocked.vault_id();
    let by_password = unlock_wallet_key(
        &created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &Limits::TESTING,
    )
    .unwrap();
    let by_recovery = unlock_wallet_key(
        &created.unlocked,
        Credential::RecoveryKey(&created.recovery_key),
        &Limits::TESTING,
    )
    .unwrap();
    let sealed = seal_wallet_field(&by_password, &vault_id, NOTE_ID, FIELD, FAKE_KEY).unwrap();
    assert_eq!(
        open_wallet_field(&by_recovery, &vault_id, NOTE_ID, FIELD, &sealed)
            .unwrap()
            .as_slice(),
        FAKE_KEY
    );
}

#[test]
fn wallet_field_fails_with_a_wrong_key_vault_id_note_id_or_field() {
    let (key, vault_id) = wallet_key_and_vault_id();
    let (other_key, _) = wallet_key_and_vault_id();
    let mut other_id = vault_id;
    other_id[0] ^= 1;
    let sealed = seal_wallet_field(&key, &vault_id, "ab", "c", FAKE_KEY).unwrap();
    for (key, vault_id, note_id, field) in [
        (&other_key, &vault_id, "ab", "c"),
        (&key, &other_id, "ab", "c"),
        (&key, &vault_id, "ab", "d"),
        (&key, &vault_id, "ac", "c"),
        // Length prefixes keep ("ab", "c") and ("a", "bc") apart.
        (&key, &vault_id, "a", "bc"),
    ] {
        assert_eq!(
            open_wallet_field(key, vault_id, note_id, field, &sealed).unwrap_err(),
            Error::Damaged,
            "{note_id} {field}"
        );
    }
}

#[test]
fn any_edit_or_truncation_of_a_wallet_field_is_damaged() {
    let (key, vault_id) = wallet_key_and_vault_id();
    let sealed = seal_wallet_field(&key, &vault_id, NOTE_ID, FIELD, FAKE_KEY).unwrap();
    for index in 0..sealed.len() {
        let mut edited = sealed.clone();
        edited[index] ^= 0x80;
        assert_eq!(
            open_wallet_field(&key, &vault_id, NOTE_ID, FIELD, &edited).unwrap_err(),
            Error::Damaged
        );
    }
    for len in 0..sealed.len() {
        assert_eq!(
            open_wallet_field(&key, &vault_id, NOTE_ID, FIELD, &sealed[..len]).unwrap_err(),
            Error::Damaged,
            "len {len}"
        );
    }
}

#[test]
fn sealing_twice_uses_fresh_nonces() {
    let (key, vault_id) = wallet_key_and_vault_id();
    let a = seal_wallet_field(&key, &vault_id, NOTE_ID, FIELD, FAKE_KEY).unwrap();
    let b = seal_wallet_field(&key, &vault_id, NOTE_ID, FIELD, FAKE_KEY).unwrap();
    assert_ne!(a[..24], b[..24]);
}

#[test]
fn wallet_key_debug_hides_the_key() {
    let (key, _) = wallet_key_and_vault_id();
    assert_eq!(format!("{key:?}"), "WalletKey(..)");
}
