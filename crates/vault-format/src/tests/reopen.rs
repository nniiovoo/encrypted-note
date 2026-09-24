//! `reopen`: decrypting a file with the Vault Key already held, to verify saves and Backups.

use crate::{Error, Limits, change_password, reopen, rotate_keys, seal};

use super::layout::{BODY_NONCE, HEADER_LEN, PASSWORD_SLOT, RECOVERY_SLOT, VAULT_ID};
use super::{OTHER_PASSWORD, TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

#[test]
fn a_sealed_file_reopens_with_the_held_keys() {
    let created = testing_vault(TEST_BODY);
    assert_eq!(
        reopen(&created.unlocked, &created.bytes)
            .unwrap()
            .as_slice(),
        TEST_BODY
    );

    let new_body = b"a later version of the fake vault document";
    let bytes = seal(&created.unlocked, new_body).unwrap();
    assert_eq!(
        reopen(&created.unlocked, &bytes).unwrap().as_slice(),
        new_body
    );
}

#[test]
fn another_vaults_file_is_refused() {
    let ours = testing_vault(TEST_BODY);
    let theirs = testing_vault(TEST_BODY);
    assert_eq!(
        reopen(&ours.unlocked, &theirs.bytes).unwrap_err(),
        Error::WrongCredential
    );
}

#[test]
fn a_tampered_body_or_body_nonce_is_damaged() {
    let created = testing_vault(TEST_BODY);
    for offset in [
        BODY_NONCE,
        HEADER_LEN - 1,
        HEADER_LEN,
        created.bytes.len() - 1,
    ] {
        let mut bytes = created.bytes.clone();
        bytes[offset] ^= 0x01;
        assert_eq!(
            reopen(&created.unlocked, &bytes).unwrap_err(),
            Error::Damaged,
            "offset {offset}"
        );
    }
}

#[test]
fn a_tampered_header_is_refused_without_decrypting() {
    let created = testing_vault(TEST_BODY);
    for offset in [VAULT_ID, PASSWORD_SLOT + 1, RECOVERY_SLOT + 1] {
        let mut bytes = created.bytes.clone();
        bytes[offset] ^= 0x01;
        assert_eq!(
            reopen(&created.unlocked, &bytes).unwrap_err(),
            Error::WrongCredential,
            "offset {offset}"
        );
    }
}

#[test]
fn structure_and_version_are_checked_first() {
    let created = testing_vault(TEST_BODY);
    assert!(matches!(
        reopen(&created.unlocked, &created.bytes[..100]).unwrap_err(),
        Error::Malformed(_)
    ));
    let mut newer = created.bytes.clone();
    newer[8] = 2;
    assert_eq!(
        reopen(&created.unlocked, &newer).unwrap_err(),
        Error::UnsupportedVersion(2)
    );
}

#[test]
fn a_file_from_before_a_password_change_or_key_rotation_is_refused() {
    let mut created = testing_vault(TEST_BODY);
    let before = created.bytes.clone();
    let (old, new) = (pw(TEST_PASSWORD), pw(OTHER_PASSWORD));

    change_password(
        &mut created.unlocked,
        password(&old),
        &new,
        &Limits::TESTING,
    )
    .unwrap();
    // The password slot differs now, so the older file is not the one we hold keys for.
    assert_eq!(
        reopen(&created.unlocked, &before).unwrap_err(),
        Error::WrongCredential
    );
    let after_change = seal(&created.unlocked, TEST_BODY).unwrap();
    assert!(reopen(&created.unlocked, &after_change).is_ok());

    let rotation = rotate_keys(&created.unlocked, password(&new), &old, &Limits::TESTING).unwrap();
    assert_eq!(
        reopen(&rotation.unlocked, &after_change).unwrap_err(),
        Error::WrongCredential
    );
    let after_rotation = seal(&rotation.unlocked, TEST_BODY).unwrap();
    assert!(reopen(&rotation.unlocked, &after_rotation).is_ok());
    assert_eq!(
        reopen(&created.unlocked, &after_rotation).unwrap_err(),
        Error::WrongCredential
    );
}
