//! Known-answer tests run against the pinned crypto crates directly, so a dependency bump that
//! changes their output fails loudly.

use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};

use super::hex;

/// RFC 9106 section 5.3 (Argon2id, version 0x13).
#[test]
fn argon2id_matches_rfc9106_test_vector() {
    let params = ParamsBuilder::new()
        .m_cost(32)
        .t_cost(3)
        .p_cost(4)
        .data(AssociatedData::new(&[0x04; 12]).unwrap())
        .output_len(32)
        .build()
        .unwrap();
    let secret = [0x03u8; 8];
    let argon =
        Argon2::new_with_secret(&secret, Algorithm::Argon2id, Version::V0x13, params).unwrap();
    let mut tag = [0u8; 32];
    argon
        .hash_password_into(&[0x01; 32], &[0x02; 16], &mut tag)
        .unwrap();
    assert_eq!(
        tag.to_vec(),
        hex("0d 64 0d f5 8d 78 76 6c 08 c0 37 a3 4a 8b 53 c9
             d0 1e f0 45 2d 75 b6 5e b5 25 20 e9 6b 01 e6 59")
    );
}

/// draft-irtf-cfrg-xchacha-03, appendix A.3.1.
#[test]
fn xchacha20poly1305_matches_draft_irtf_cfrg_xchacha_a31() {
    let plaintext = hex(
        "4c616469657320616e642047656e746c656d656e206f662074686520636c6173
         73206f66202739393a204966204920636f756c64206f6666657220796f75206f
         6e6c79206f6e652074697020666f7220746865206675747572652c2073756e73
         637265656e20776f756c642062652069742e",
    );
    assert!(plaintext.starts_with(b"Ladies and Gentlemen of the class of '99"));
    let aad = hex("50515253c0c1c2c3c4c5c6c7");
    let key = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let nonce = hex("404142434445464748494a4b4c4d4e4f5051525354555657");
    let ciphertext = hex(
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb
         731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452
         2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9
         21f9664c97637da9768812f615c68b13b52e",
    );
    let tag = hex("c0875924c1c7987947deafd8780acf49");

    let key: [u8; 32] = key.try_into().unwrap();
    let nonce: [u8; 24] = nonce.try_into().unwrap();
    let cipher = XChaCha20Poly1305::new(&key.into());

    let sealed = cipher
        .encrypt(
            &nonce.into(),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .unwrap();
    let mut expected = ciphertext.clone();
    expected.extend_from_slice(&tag);
    assert_eq!(sealed, expected);

    let opened = cipher
        .decrypt(
            &nonce.into(),
            Payload {
                msg: &sealed,
                aad: &aad,
            },
        )
        .unwrap();
    assert_eq!(opened, plaintext);

    let mut forged = sealed.clone();
    forged[0] ^= 1;
    assert!(
        cipher
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: &forged,
                    aad: &aad
                }
            )
            .is_err()
    );
}
