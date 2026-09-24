//! Any edit to a Vault file must be detected, with the error class documented for its region.

use crate::{Credential, Error, KdfParams, Limits, inspect, open};

use super::layout::*;
use super::{TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

/// Same floor as `Limits::TESTING` but a low memory ceiling, so a flipped memory byte that lands
/// on tens of MiB is rejected by the limits instead of running a slow unoptimised derivation.
const FLIP_LIMITS: Limits = Limits {
    min: Limits::TESTING.min,
    max: KdfParams {
        m_kib: 1024,
        t: 20,
        p: 16,
    },
};

#[derive(Clone, Copy, Debug)]
enum Using {
    MasterPassword,
    RecoveryKey,
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// The documented error for an edit at `index`, given the edited file.
fn expected_error(index: usize, edited: &[u8], using: Using) -> Error {
    let own_slot = match using {
        Using::MasterPassword => PASSWORD_SLOT,
        Using::RecoveryKey => RECOVERY_SLOT,
    };
    match index {
        0..VERSION => Error::Malformed(""),
        VERSION..FLAGS => {
            Error::UnsupportedVersion(u16::from_le_bytes([edited[VERSION], edited[VERSION + 1]]))
        }
        FLAGS..VAULT_ID => Error::Malformed(""),
        VAULT_ID..KDF_ID => Error::WrongCredential,
        KDF_ID => Error::Malformed(""),
        KDF_M..SLOT_COUNT => {
            let params = KdfParams {
                m_kib: read_u32(edited, KDF_M),
                t: read_u32(edited, KDF_T),
                p: read_u32(edited, KDF_P),
            };
            match FLIP_LIMITS.check(params) {
                Err(_) => Error::KdfOutOfLimits,
                Ok(()) => Error::WrongCredential,
            }
        }
        SLOT_COUNT => Error::Malformed(""),
        i if i == PASSWORD_SLOT || i == RECOVERY_SLOT => Error::Malformed(""),
        i if (own_slot..own_slot + SLOT_LEN).contains(&i) => Error::WrongCredential,
        // The other slot, the body nonce and the body are all authenticated with the body.
        _ => Error::Damaged,
    }
}

fn same_class(actual: &Error, expected: &Error) -> bool {
    match (actual, expected) {
        (Error::Malformed(_), Error::Malformed(_)) => true,
        (a, e) => a == e,
    }
}

fn flip_every_byte(using: Using, mask: u8) {
    let created = testing_vault(TEST_BODY);
    let password_secret = pw(TEST_PASSWORD);
    let credential = match using {
        Using::MasterPassword => password(&password_secret),
        Using::RecoveryKey => Credential::RecoveryKey(&created.recovery_key),
    };
    assert!(open(&created.bytes, credential, &FLIP_LIMITS).is_ok());

    let mut edited = created.bytes.clone();
    for index in 0..edited.len() {
        edited[index] ^= mask;
        let result = open(&edited, credential, &FLIP_LIMITS);
        let expected = expected_error(index, &edited, using);
        match result {
            Ok(_) => panic!("flipping byte {index} with {mask:#04x} still opened ({using:?})"),
            Err(actual) => assert!(
                same_class(&actual, &expected),
                "byte {index} ({using:?}, mask {mask:#04x}): got {actual:?}, expected {expected:?}"
            ),
        }
        edited[index] ^= mask;
    }
}

#[test]
fn flipping_any_byte_fails_with_its_region_error_using_the_master_password() {
    flip_every_byte(Using::MasterPassword, 0xff);
}

#[test]
fn flipping_any_byte_fails_with_its_region_error_using_the_recovery_key() {
    flip_every_byte(Using::RecoveryKey, 0xff);
}

#[test]
fn flipping_the_lowest_bit_of_any_byte_fails_with_its_region_error() {
    flip_every_byte(Using::MasterPassword, 0x01);
}

#[test]
fn every_truncation_is_malformed() {
    let created = testing_vault(TEST_BODY);
    let secret = pw(TEST_PASSWORD);
    for len in 0..created.bytes.len() {
        let err = open(&created.bytes[..len], password(&secret), &Limits::TESTING).unwrap_err();
        assert!(matches!(err, Error::Malformed(_)), "len {len}: {err:?}");
        let err = inspect(&created.bytes[..len], &Limits::TESTING).unwrap_err();
        assert!(
            matches!(err, Error::Malformed(_)),
            "inspect len {len}: {err:?}"
        );
    }
}

#[test]
fn extra_bytes_are_detected() {
    let created = testing_vault(TEST_BODY);
    let secret = pw(TEST_PASSWORD);

    let mut one_more = created.bytes.clone();
    one_more.push(0);
    assert!(matches!(
        open(&one_more, password(&secret), &Limits::TESTING),
        Err(Error::Malformed(_))
    ));

    // A whole extra padding block keeps the structure valid but fails authentication.
    let mut one_block_more = created.bytes.clone();
    one_block_more.extend_from_slice(&[0u8; crate::PADDING_BLOCK]);
    assert_eq!(
        open(&one_block_more, password(&secret), &Limits::TESTING).unwrap_err(),
        Error::Damaged
    );
}

#[test]
fn a_newer_format_version_is_reported_as_unsupported() {
    let created = testing_vault(TEST_BODY);
    let mut edited = created.bytes.clone();
    edited[VERSION..FLAGS].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        inspect(&edited, &Limits::TESTING).unwrap_err(),
        Error::UnsupportedVersion(2)
    );
    assert_eq!(
        open(&edited, password(&pw(TEST_PASSWORD)), &Limits::TESTING).unwrap_err(),
        Error::UnsupportedVersion(2)
    );
}

#[test]
fn swapping_the_slots_is_detected() {
    let created = testing_vault(TEST_BODY);
    let secret = pw(TEST_PASSWORD);

    // Swapped whole slots (type bytes included): the order is fixed, so this is malformed.
    let mut swapped = created.bytes.clone();
    let (first, second) = swapped[PASSWORD_SLOT..BODY_NONCE].split_at_mut(SLOT_LEN);
    first.swap_with_slice(second);
    assert!(matches!(
        open(&swapped, password(&secret), &Limits::TESTING),
        Err(Error::Malformed(_))
    ));

    // Swapped slot contents under the original type bytes: the slot type is bound into the slot.
    let mut swapped = created.bytes.clone();
    let (first, second) = swapped[PASSWORD_SLOT + 1..BODY_NONCE].split_at_mut(SLOT_LEN);
    first[..SLOT_LEN - 1].swap_with_slice(&mut second[..SLOT_LEN - 1]);
    assert_eq!(
        open(&swapped, password(&secret), &Limits::TESTING).unwrap_err(),
        Error::WrongCredential
    );
    assert_eq!(
        open(
            &swapped,
            Credential::RecoveryKey(&created.recovery_key),
            &Limits::TESTING
        )
        .unwrap_err(),
        Error::WrongCredential
    );
}

#[test]
fn a_body_from_another_seal_of_the_same_vault_is_rejected() {
    let created = testing_vault(TEST_BODY);
    let later = crate::seal(&created.unlocked, b"later fake document").unwrap();
    let mut spliced = created.bytes[..HEADER_LEN].to_vec();
    spliced.extend_from_slice(&later[HEADER_LEN..]);
    assert_eq!(
        open(&spliced, password(&pw(TEST_PASSWORD)), &Limits::TESTING).unwrap_err(),
        Error::Damaged
    );
}

#[test]
fn garbage_input_never_panics() {
    let secret = pw(TEST_PASSWORD);
    let mut inputs: Vec<Vec<u8>> =
        vec![Vec::new(), vec![0], vec![0xff; 5000], crate::MAGIC.to_vec()];
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    for len in [10, 41, 42, 307, 308, 309, 4420, 9000] {
        let mut bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed.to_le_bytes()[0]
            })
            .collect();
        inputs.push(bytes.clone());
        if bytes.len() >= 10 {
            bytes[..8].copy_from_slice(&crate::MAGIC);
            bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
            inputs.push(bytes);
        }
    }
    for input in inputs {
        assert!(open(&input, password(&secret), &Limits::TESTING).is_err());
        assert!(inspect(&input, &Limits::TESTING).is_err());
    }
}
