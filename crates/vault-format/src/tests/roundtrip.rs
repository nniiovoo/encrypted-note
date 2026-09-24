use crate::{
    Credential, Error, FORMAT_VERSION, KdfParams, Limits, PADDING_BLOCK, RecoveryKey, create,
    inspect, open, seal, unlock_wallet_key,
};

use super::layout::{BODY_NONCE, HEADER_LEN, PASSWORD_SLOT, RECOVERY_SLOT, TAG_LEN};
use super::{OTHER_PASSWORD, TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

#[test]
fn create_then_open_with_either_credential_returns_the_body() {
    let created = testing_vault(TEST_BODY);
    let secret = pw(TEST_PASSWORD);
    for credential in [
        password(&secret),
        Credential::RecoveryKey(&created.recovery_key),
    ] {
        let (unlocked, body) = open(&created.bytes, credential, &Limits::TESTING).unwrap();
        assert_eq!(body.as_slice(), TEST_BODY);
        assert_eq!(unlocked.vault_id(), created.unlocked.vault_id());
    }
}

#[test]
fn recovery_key_typed_back_from_the_recovery_kit_opens_the_vault() {
    let created = testing_vault(TEST_BODY);
    let typed = created
        .recovery_key
        .expose_display()
        .to_lowercase()
        .replace('-', " ");
    let parsed = RecoveryKey::parse(&typed).unwrap();
    let (_, body) = open(
        &created.bytes,
        Credential::RecoveryKey(&parsed),
        &Limits::TESTING,
    )
    .unwrap();
    assert_eq!(body.as_slice(), TEST_BODY);
}

#[test]
fn a_wrong_password_or_recovery_key_is_a_wrong_credential() {
    let created = testing_vault(TEST_BODY);
    let other_password = pw(OTHER_PASSWORD);
    let other_key = RecoveryKey::generate().unwrap();
    for wrong in [
        password(&other_password),
        Credential::RecoveryKey(&other_key),
    ] {
        let err = open(&created.bytes, wrong, &Limits::TESTING).unwrap_err();
        assert_eq!(err, Error::WrongCredential);
        let err = unlock_wallet_key(&created.unlocked, wrong, &Limits::TESTING).unwrap_err();
        assert_eq!(err, Error::WrongCredential);
    }
}

#[test]
fn composed_and_decomposed_forms_of_a_password_both_unlock() {
    let composed = pw("caf\u{e9}-correct-horse-test-password");
    let decomposed = pw("cafe\u{301}-correct-horse-test-password");
    let created = create(&composed, KdfParams::TESTING, &Limits::TESTING, TEST_BODY).unwrap();
    assert!(open(&created.bytes, password(&decomposed), &Limits::TESTING).is_ok());

    let created = create(&decomposed, KdfParams::TESTING, &Limits::TESTING, TEST_BODY).unwrap();
    assert!(open(&created.bytes, password(&composed), &Limits::TESTING).is_ok());
}

#[test]
fn seal_writes_a_new_body_that_opens_with_both_credentials() {
    let created = testing_vault(TEST_BODY);
    let new_body = b"a later version of the fake vault document";
    let bytes = seal(&created.unlocked, new_body).unwrap();
    let secret = pw(TEST_PASSWORD);
    for credential in [
        password(&secret),
        Credential::RecoveryKey(&created.recovery_key),
    ] {
        let (_, body) = open(&bytes, credential, &Limits::TESTING).unwrap();
        assert_eq!(body.as_slice(), new_body);
    }
}

#[test]
fn seal_keeps_the_slots_and_draws_a_fresh_body_nonce_every_time() {
    let created = testing_vault(TEST_BODY);
    let first = seal(&created.unlocked, TEST_BODY).unwrap();
    let second = seal(&created.unlocked, TEST_BODY).unwrap();

    assert_eq!(first[..BODY_NONCE], created.bytes[..BODY_NONCE]);
    assert_eq!(first[..BODY_NONCE], second[..BODY_NONCE]);
    let nonces = [
        &created.bytes[BODY_NONCE..HEADER_LEN],
        &first[BODY_NONCE..HEADER_LEN],
        &second[BODY_NONCE..HEADER_LEN],
    ];
    assert_ne!(nonces[0], nonces[1]);
    assert_ne!(nonces[1], nonces[2]);
    assert_ne!(nonces[0], nonces[2]);
    assert_ne!(first[HEADER_LEN..], second[HEADER_LEN..]);
}

#[test]
fn every_create_draws_new_ids_salts_and_nonces() {
    let a = testing_vault(TEST_BODY);
    let b = testing_vault(TEST_BODY);
    assert_ne!(a.unlocked.vault_id(), b.unlocked.vault_id());
    assert_ne!(a.recovery_key.expose_bytes(), b.recovery_key.expose_bytes());
    // Whole slots (salt, nonce, wrapped keys) differ.
    assert_ne!(
        a.bytes[PASSWORD_SLOT..RECOVERY_SLOT],
        b.bytes[PASSWORD_SLOT..RECOVERY_SLOT]
    );
    assert_ne!(
        a.bytes[RECOVERY_SLOT..BODY_NONCE],
        b.bytes[RECOVERY_SLOT..BODY_NONCE]
    );
}

#[test]
fn body_is_padded_to_a_multiple_of_the_padding_block() {
    // The padded plaintext is len(u32) || body || zeros, so 4092 body bytes fill one block exactly.
    for (body_len, blocks) in [
        (0, 1),
        (1, 1),
        (PADDING_BLOCK - 4, 1),
        (PADDING_BLOCK - 3, 2),
        (2 * PADDING_BLOCK, 3),
    ] {
        let body = vec![0x5a; body_len];
        let created = testing_vault(&body);
        assert_eq!(
            created.bytes.len(),
            HEADER_LEN + blocks * PADDING_BLOCK + TAG_LEN,
            "body_len={body_len}"
        );
        let (_, opened) = open(
            &created.bytes,
            password(&pw(TEST_PASSWORD)),
            &Limits::TESTING,
        )
        .unwrap();
        assert_eq!(opened.as_slice(), body.as_slice());
    }
}

#[test]
fn unlocked_vault_reports_the_same_facts_as_inspect() {
    let created = testing_vault(TEST_BODY);
    let info = inspect(&created.bytes, &Limits::TESTING).unwrap();
    assert_eq!(info.format_version, FORMAT_VERSION);
    assert_eq!(info.vault_id, created.unlocked.vault_id());
    assert_eq!(info.kdf, KdfParams::TESTING);
    assert_eq!(created.unlocked.kdf(), KdfParams::TESTING);

    let (unlocked, _) = open(
        &created.bytes,
        password(&pw(TEST_PASSWORD)),
        &Limits::TESTING,
    )
    .unwrap();
    assert_eq!(unlocked.vault_id(), info.vault_id);
    assert_eq!(unlocked.kdf(), info.kdf);
}

#[test]
fn debug_output_never_shows_keys() {
    let created = testing_vault(TEST_BODY);
    let shown = format!("{created:?}");
    assert!(!shown.contains(created.recovery_key.expose_display()));
    // `Created`'s own Debug must go through the redacted impls (raw key bytes would pass the
    // check above).
    assert!(shown.contains("RecoveryKey(..)"));
    assert!(shown.contains("UnlockedVault(..)"));
}
