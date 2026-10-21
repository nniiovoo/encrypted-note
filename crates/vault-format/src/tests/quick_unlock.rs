//! Fake-data checks for the only Rust Keychain custody route.

use zeroize::Zeroizing;

use crate::{
    Error, Limits, QUICK_UNLOCK_PAYLOAD_LEN, QuickUnlockKey, change_password, enroll_quick_unlock,
    open_with_quick_unlock, quick_unlock_binding, rotate_keys, seal,
};

use super::layout::{BODY_NONCE, HEADER_LEN, PASSWORD_SLOT, RECOVERY_SLOT};
use super::{OTHER_PASSWORD, TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

fn read_item(key: &QuickUnlockKey) -> QuickUnlockKey {
    key.with_keychain_payload(|payload| {
        assert_eq!(payload.len(), QUICK_UNLOCK_PAYLOAD_LEN);
        QuickUnlockKey::from_keychain_payload(Zeroizing::new(payload.to_vec())).unwrap()
    })
}

#[test]
fn the_protected_payload_opens_a_current_file_and_later_ordinary_save() {
    let created = testing_vault(TEST_BODY);
    let key = enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::TESTING).unwrap();
    let (opened, body) =
        open_with_quick_unlock(&created.bytes, read_item(&key), &Limits::TESTING).unwrap();
    assert_eq!(body.as_slice(), TEST_BODY);
    assert_eq!(opened.vault_id(), created.unlocked.vault_id());
    let later = seal(&opened, b"later fake document").unwrap();
    assert_ne!(
        &created.bytes[BODY_NONCE..HEADER_LEN],
        &later[BODY_NONCE..HEADER_LEN]
    );
    assert_eq!(
        quick_unlock_binding(&later, &Limits::TESTING).unwrap(),
        key.binding()
    );
    let (_, body) = open_with_quick_unlock(&later, read_item(&key), &Limits::TESTING).unwrap();
    assert_eq!(body.as_slice(), b"later fake document");
}

#[test]
fn enrollment_refuses_another_vault_old_slots_or_damaged_committed_file() {
    let mut created = testing_vault(TEST_BODY);
    let other = testing_vault(TEST_BODY);
    assert_eq!(
        enroll_quick_unlock(&created.unlocked, &other.bytes, &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
    let mut damaged = created.bytes.clone();
    damaged[HEADER_LEN] ^= 1;
    assert_eq!(
        enroll_quick_unlock(&created.unlocked, &damaged, &Limits::TESTING).unwrap_err(),
        Error::Damaged
    );
    change_password(
        &mut created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &pw(OTHER_PASSWORD),
        &Limits::TESTING,
    )
    .unwrap();
    assert_eq!(
        enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
}

#[test]
fn password_changes_rotation_and_either_slot_change_invalidate_the_binding() {
    let mut created = testing_vault(TEST_BODY);
    let key = enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::TESTING).unwrap();
    for offset in [PASSWORD_SLOT + 1, RECOVERY_SLOT + 1] {
        let mut altered = created.bytes.clone();
        altered[offset] ^= 1;
        assert_eq!(
            open_with_quick_unlock(&altered, read_item(&key), &Limits::TESTING).unwrap_err(),
            Error::WrongCredential
        );
    }
    change_password(
        &mut created.unlocked,
        password(&pw(TEST_PASSWORD)),
        &pw(OTHER_PASSWORD),
        &Limits::TESTING,
    )
    .unwrap();
    let changed = seal(&created.unlocked, TEST_BODY).unwrap();
    assert_eq!(
        open_with_quick_unlock(&changed, read_item(&key), &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
    let rotation = rotate_keys(
        &created.unlocked,
        password(&pw(OTHER_PASSWORD)),
        &pw(TEST_PASSWORD),
        &Limits::TESTING,
    )
    .unwrap();
    assert_eq!(rotation.unlocked.vault_id(), created.unlocked.vault_id());
    let rotated = seal(&rotation.unlocked, TEST_BODY).unwrap();
    assert_eq!(
        open_with_quick_unlock(&rotated, read_item(&key), &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
}

#[test]
fn current_body_and_nonce_must_authenticate_and_the_actual_key_must_match() {
    let created = testing_vault(TEST_BODY);
    let key = enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::TESTING).unwrap();
    for offset in [BODY_NONCE, HEADER_LEN, created.bytes.len() - 1] {
        let mut altered = created.bytes.clone();
        altered[offset] ^= 1;
        assert_eq!(
            open_with_quick_unlock(&altered, read_item(&key), &Limits::TESTING).unwrap_err(),
            Error::Damaged
        );
    }
    let wrong_key = key.with_keychain_payload(|payload| {
        let mut payload = Zeroizing::new(payload.to_vec());
        payload[QUICK_UNLOCK_PAYLOAD_LEN - 1] ^= 1;
        QuickUnlockKey::from_keychain_payload(payload).unwrap()
    });
    assert_eq!(
        open_with_quick_unlock(&created.bytes, wrong_key, &Limits::TESTING).unwrap_err(),
        Error::Damaged
    );
}

#[test]
fn malformed_payloads_and_files_are_bounded_and_limits_are_still_checked() {
    for len in [
        0,
        15,
        16,
        QUICK_UNLOCK_PAYLOAD_LEN - 1,
        QUICK_UNLOCK_PAYLOAD_LEN,
        QUICK_UNLOCK_PAYLOAD_LEN + 1,
    ] {
        assert!(matches!(
            QuickUnlockKey::from_keychain_payload(Zeroizing::new(vec![0; len])),
            Err(Error::Malformed(_))
        ));
    }
    let created = testing_vault(TEST_BODY);
    let key = enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::TESTING).unwrap();
    assert_eq!(
        enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::PRODUCTION).unwrap_err(),
        Error::KdfOutOfLimits
    );
    assert_eq!(
        open_with_quick_unlock(&created.bytes, read_item(&key), &Limits::PRODUCTION).unwrap_err(),
        Error::KdfOutOfLimits
    );
    assert!(matches!(
        open_with_quick_unlock(&created.bytes[..100], read_item(&key), &Limits::TESTING),
        Err(Error::Malformed(_))
    ));
    let mut newer = created.bytes.clone();
    newer[8] = 2;
    assert_eq!(
        open_with_quick_unlock(&newer, read_item(&key), &Limits::TESTING).unwrap_err(),
        Error::UnsupportedVersion(2)
    );
}

#[test]
fn secret_objects_debug_output_is_redacted() {
    let created = testing_vault(TEST_BODY);
    let key = enroll_quick_unlock(&created.unlocked, &created.bytes, &Limits::TESTING).unwrap();
    assert_eq!(format!("{key:?}"), "QuickUnlockKey(..)");
    key.with_keychain_payload(|payload| {
        assert_eq!(&payload[48..], created.unlocked.vault_key.0.as_ref());
    });
}
