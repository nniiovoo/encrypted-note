//! KDF parameter floor and ceiling.

use crate::{Credential, Error, KdfParams, Limits, create, inspect, open, unlock_wallet_key};

use super::layout::{KDF_M, KDF_P, KDF_T};
use super::{TEST_BODY, TEST_PASSWORD, password, pw, testing_vault};

#[test]
fn production_accepts_its_own_bounds_and_the_default() {
    let limits = Limits::PRODUCTION;
    assert_eq!(limits.check(limits.min), Ok(()));
    assert_eq!(limits.check(limits.max), Ok(()));
    assert_eq!(limits.check(KdfParams::DEFAULT), Ok(()));
}

#[test]
fn production_rejects_each_parameter_just_outside_its_bounds() {
    let limits = Limits::PRODUCTION;
    let min = limits.min;
    let max = limits.max;
    let base = KdfParams::DEFAULT;
    for params in [
        KdfParams {
            m_kib: min.m_kib - 1,
            ..base
        },
        KdfParams {
            t: min.t - 1,
            ..base
        },
        KdfParams {
            p: min.p - 1,
            ..base
        },
        KdfParams {
            m_kib: max.m_kib + 1,
            ..base
        },
        KdfParams {
            t: max.t + 1,
            ..base
        },
        KdfParams {
            p: max.p + 1,
            ..base
        },
        KdfParams {
            m_kib: u32::MAX,
            t: u32::MAX,
            p: u32::MAX,
        },
        KdfParams {
            m_kib: 0,
            t: 0,
            p: 0,
        },
    ] {
        assert_eq!(
            limits.check(params),
            Err(Error::KdfOutOfLimits),
            "{params:?}"
        );
    }
}

#[test]
fn memory_below_eight_kib_per_lane_is_rejected() {
    let limits = Limits::TESTING;
    assert_eq!(
        limits.check(KdfParams {
            m_kib: 16,
            t: 1,
            p: 2
        }),
        Ok(())
    );
    assert_eq!(
        limits.check(KdfParams {
            m_kib: 15,
            t: 1,
            p: 2
        }),
        Err(Error::KdfOutOfLimits)
    );
    assert_eq!(
        limits.check(KdfParams {
            m_kib: 8,
            t: 1,
            p: 4
        }),
        Err(Error::KdfOutOfLimits)
    );
}

#[test]
fn production_rejects_a_file_written_with_testing_params() {
    let created = testing_vault(TEST_BODY);
    assert_eq!(
        inspect(&created.bytes, &Limits::PRODUCTION).unwrap_err(),
        Error::KdfOutOfLimits
    );
    let secret = pw(TEST_PASSWORD);
    for credential in [
        password(&secret),
        Credential::RecoveryKey(&created.recovery_key),
    ] {
        let err = open(&created.bytes, credential, &Limits::PRODUCTION).unwrap_err();
        assert_eq!(err, Error::KdfOutOfLimits);
        let err =
            unlock_wallet_key(&created.unlocked, credential, &Limits::PRODUCTION).unwrap_err();
        assert_eq!(err, Error::KdfOutOfLimits);
    }
}

#[test]
fn create_refuses_params_outside_the_limits() {
    let err = create(
        &pw(TEST_PASSWORD),
        KdfParams::TESTING,
        &Limits::PRODUCTION,
        TEST_BODY,
    )
    .unwrap_err();
    assert_eq!(err, Error::KdfOutOfLimits);
    let err = create(
        &pw(TEST_PASSWORD),
        KdfParams {
            t: 21,
            ..KdfParams::TESTING
        },
        &Limits::TESTING,
        TEST_BODY,
    )
    .unwrap_err();
    assert_eq!(err, Error::KdfOutOfLimits);
}

#[test]
fn a_doctored_floor_or_ceiling_is_rejected_before_any_derivation_runs() {
    // If a derivation ran with the ceiling values the test would hang or run out of memory.
    let created = testing_vault(TEST_BODY);
    for (offset, value) in [
        (KDF_M, 3 * 1024 * 1024u32),
        (KDF_M, u32::MAX),
        (KDF_T, 21),
        (KDF_T, u32::MAX),
        (KDF_P, 17),
        (KDF_M, 7),
        (KDF_T, 0),
        (KDF_P, 0),
    ] {
        let mut edited = created.bytes.clone();
        edited[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert_eq!(
            inspect(&edited, &Limits::TESTING).unwrap_err(),
            Error::KdfOutOfLimits
        );
        assert_eq!(
            open(&edited, password(&pw(TEST_PASSWORD)), &Limits::TESTING).unwrap_err(),
            Error::KdfOutOfLimits
        );
    }
}
