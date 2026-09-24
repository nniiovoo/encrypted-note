//! Tests for the public interface of vault-format.
//!
//! They live inside the crate only so that the `cfg(test)` constants [`KdfParams::TESTING`] and
//! [`Limits::TESTING`] are available. They use nothing but public items, except `padding.rs`
//! (inputs the private `unpad` gets that `open` can't produce) and `hygiene.rs` (the KDF working
//! memory is wiped).

use secrecy::SecretString;

use crate::{Created, Credential, KdfParams, Limits, create};

mod calibrate;
mod conformance;
mod hygiene;
mod kat;
mod keys;
mod limits;
mod padding;
mod recovery_key;
mod reopen;
mod roundtrip;
mod tamper;
mod wallet;

/// Byte offsets from the documented file layout (format version 1).
pub(crate) mod layout {
    pub const VERSION: usize = 8;
    pub const FLAGS: usize = 10;
    pub const VAULT_ID: usize = 12;
    pub const KDF_ID: usize = 28;
    pub const KDF_M: usize = 29;
    pub const KDF_T: usize = 33;
    pub const KDF_P: usize = 37;
    pub const SLOT_COUNT: usize = 41;
    pub const SLOT_LEN: usize = 1 + 16 + 24 + 80;
    pub const PASSWORD_SLOT: usize = 42;
    pub const RECOVERY_SLOT: usize = PASSWORD_SLOT + SLOT_LEN;
    pub const BODY_NONCE: usize = RECOVERY_SLOT + SLOT_LEN;
    pub const HEADER_LEN: usize = BODY_NONCE + 24;
    pub const TAG_LEN: usize = 16;
}

pub(crate) const TEST_PASSWORD: &str = "correct-horse-test-password-1";
pub(crate) const OTHER_PASSWORD: &str = "battery-staple-test-password-2";
pub(crate) const TEST_BODY: &[u8] = b"fake vault document for tests";

pub(crate) fn pw(s: &str) -> SecretString {
    SecretString::from(s)
}

pub(crate) fn testing_vault(body: &[u8]) -> Created {
    create(
        &pw(TEST_PASSWORD),
        KdfParams::TESTING,
        &Limits::TESTING,
        body,
    )
    .unwrap()
}

pub(crate) fn password(p: &SecretString) -> Credential<'_> {
    Credential::MasterPassword(p)
}

pub(crate) fn hex(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
