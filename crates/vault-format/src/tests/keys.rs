//! Changing the Master Password and Key Rotation (ADR-0005).

use crate::{
    Credential, Error, Limits, RecoveryKey, change_password, open, open_wallet_field, rotate_keys,
    seal, seal_wallet_field, unlock_wallet_key,
};

use super::{OTHER_PASSWORD, TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

const NOTE_ID: &str = "note-0001";
const FIELD: &str = "words";
const FAKE_WORDS: &[u8] = b"fake fake fake fake fake fake fake fake fake fake fake fake";

#[test]
fn change_password_keeps_the_recovery_key_and_kills_the_old_password() {
    let mut created = testing_vault(TEST_BODY);
    let old = pw(TEST_PASSWORD);
    let new = pw(OTHER_PASSWORD);
    change_password(
        &mut created.unlocked,
        password(&old),
        &new,
        &Limits::TESTING,
    )
    .unwrap();
    let bytes = seal(&created.unlocked, TEST_BODY).unwrap();

    for credential in [
        password(&new),
        Credential::RecoveryKey(&created.recovery_key),
    ] {
        let (_, body) = open(&bytes, credential, &Limits::TESTING).unwrap();
        assert_eq!(body.as_slice(), TEST_BODY);
    }
    assert_eq!(
        open(&bytes, password(&old), &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );

    // The in-memory vault follows too.
    assert!(unlock_wallet_key(&created.unlocked, password(&new), &Limits::TESTING).is_ok());
    assert_eq!(
        unlock_wallet_key(&created.unlocked, password(&old), &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
}

#[test]
fn change_password_keeps_the_vault_id_kdf_and_wallet_key() {
    let mut created = testing_vault(TEST_BODY);
    let vault_id = created.unlocked.vault_id();
    let old_key = unlock_wallet_key(
        &created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &Limits::TESTING,
    )
    .unwrap();
    let sealed = seal_wallet_field(&old_key, &vault_id, NOTE_ID, FIELD, FAKE_WORDS).unwrap();

    let new = pw(OTHER_PASSWORD);
    change_password(
        &mut created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &new,
        &Limits::TESTING,
    )
    .unwrap();
    assert_eq!(created.unlocked.vault_id(), vault_id);
    assert_eq!(created.unlocked.kdf(), crate::KdfParams::TESTING);

    let new_key = unlock_wallet_key(&created.unlocked, password(&new), &Limits::TESTING).unwrap();
    let opened = open_wallet_field(&new_key, &vault_id, NOTE_ID, FIELD, &sealed).unwrap();
    assert_eq!(opened.as_slice(), FAKE_WORDS);
}

#[test]
fn change_password_after_recovery_uses_the_recovery_key_as_proof() {
    let created = testing_vault(TEST_BODY);
    let (mut unlocked, _) = open(
        &created.bytes,
        Credential::RecoveryKey(&created.recovery_key),
        &Limits::TESTING,
    )
    .unwrap();
    let new = pw(OTHER_PASSWORD);
    change_password(
        &mut unlocked,
        Credential::RecoveryKey(&created.recovery_key),
        &new,
        &Limits::TESTING,
    )
    .unwrap();
    let bytes = seal(&unlocked, TEST_BODY).unwrap();
    assert!(open(&bytes, password(&new), &Limits::TESTING).is_ok());
}

#[test]
fn change_password_writes_a_fresh_password_slot_even_for_the_same_password() {
    let mut created = testing_vault(TEST_BODY);
    let same = pw(TEST_PASSWORD);
    let before = seal(&created.unlocked, TEST_BODY).unwrap();
    change_password(
        &mut created.unlocked,
        password(&same),
        &same,
        &Limits::TESTING,
    )
    .unwrap();
    let after = seal(&created.unlocked, TEST_BODY).unwrap();
    use super::layout::{BODY_NONCE, PASSWORD_SLOT, RECOVERY_SLOT};
    assert_ne!(
        before[PASSWORD_SLOT..RECOVERY_SLOT],
        after[PASSWORD_SLOT..RECOVERY_SLOT]
    );
    assert_eq!(
        before[RECOVERY_SLOT..BODY_NONCE],
        after[RECOVERY_SLOT..BODY_NONCE]
    );
}

#[test]
fn change_password_with_a_wrong_current_credential_changes_nothing() {
    let mut created = testing_vault(TEST_BODY);
    let before = seal(&created.unlocked, TEST_BODY).unwrap();
    let err = change_password(
        &mut created.unlocked,
        password(&pw("not-the-test-password")),
        &pw(OTHER_PASSWORD),
        &Limits::TESTING,
    )
    .unwrap_err();
    assert_eq!(err, Error::WrongCredential);
    let after = seal(&created.unlocked, TEST_BODY).unwrap();
    use super::layout::BODY_NONCE;
    assert_eq!(before[..BODY_NONCE], after[..BODY_NONCE]);
}

#[test]
fn rotate_keys_replaces_both_credentials_and_both_keys() {
    let created = testing_vault(TEST_BODY);
    let old_password = pw(TEST_PASSWORD);
    let new_password = pw(OTHER_PASSWORD);
    let rotation = rotate_keys(
        &created.unlocked,
        password(&old_password),
        &new_password,
        &Limits::TESTING,
    )
    .unwrap();
    let bytes = seal(&rotation.unlocked, TEST_BODY).unwrap();

    for old in [
        password(&old_password),
        Credential::RecoveryKey(&created.recovery_key),
    ] {
        let err = open(&bytes, old, &Limits::TESTING).unwrap_err();
        assert_eq!(err, Error::WrongCredential);
    }
    for new in [
        password(&new_password),
        Credential::RecoveryKey(&rotation.recovery_key),
    ] {
        let (_, body) = open(&bytes, new, &Limits::TESTING).unwrap();
        assert_eq!(body.as_slice(), TEST_BODY);
    }
    assert_ne!(
        rotation.recovery_key.expose_bytes(),
        created.recovery_key.expose_bytes()
    );

    // Vault id and KDF parameters are kept.
    assert_eq!(rotation.unlocked.vault_id(), created.unlocked.vault_id());
    assert_eq!(rotation.unlocked.kdf(), created.unlocked.kdf());
}

#[test]
fn rotate_keys_hands_over_old_and_new_wallet_keys_that_differ() {
    let created = testing_vault(TEST_BODY);
    let vault_id = created.unlocked.vault_id();
    let before = unlock_wallet_key(
        &created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &Limits::TESTING,
    )
    .unwrap();
    let sealed_old = seal_wallet_field(&before, &vault_id, NOTE_ID, FIELD, FAKE_WORDS).unwrap();

    let new_password = pw(OTHER_PASSWORD);
    let rotation = rotate_keys(
        &created.unlocked,
        Credential::RecoveryKey(&created.recovery_key),
        &new_password,
        &Limits::TESTING,
    )
    .unwrap();

    // The old wallet key is the one fields were sealed under.
    let opened = open_wallet_field(
        &rotation.old_wallet_key,
        &vault_id,
        NOTE_ID,
        FIELD,
        &sealed_old,
    )
    .unwrap();
    assert_eq!(opened.as_slice(), FAKE_WORDS);
    // The new wallet key is different.
    assert_eq!(
        open_wallet_field(
            &rotation.new_wallet_key,
            &vault_id,
            NOTE_ID,
            FIELD,
            &sealed_old
        )
        .unwrap_err(),
        Error::Damaged
    );
    // Re-encrypted fields open with the new key, which both new credentials unlock.
    let sealed_new =
        seal_wallet_field(&rotation.new_wallet_key, &vault_id, NOTE_ID, FIELD, &opened).unwrap();
    for credential in [
        password(&new_password),
        Credential::RecoveryKey(&rotation.recovery_key),
    ] {
        let key = unlock_wallet_key(&rotation.unlocked, credential, &Limits::TESTING).unwrap();
        assert_eq!(
            open_wallet_field(&key, &vault_id, NOTE_ID, FIELD, &sealed_new)
                .unwrap()
                .as_slice(),
            FAKE_WORDS
        );
        assert!(open_wallet_field(&key, &vault_id, NOTE_ID, FIELD, &sealed_old).is_err());
    }
}

#[test]
fn rotate_keys_with_a_wrong_credential_fails() {
    let created = testing_vault(TEST_BODY);
    let other = RecoveryKey::generate().unwrap();
    let err = rotate_keys(
        &created.unlocked,
        Credential::RecoveryKey(&other),
        &pw(OTHER_PASSWORD),
        &Limits::TESTING,
    )
    .unwrap_err();
    assert_eq!(err, Error::WrongCredential);
}
