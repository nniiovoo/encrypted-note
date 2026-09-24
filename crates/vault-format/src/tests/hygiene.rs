//! Key hygiene the type system can check. (The AEAD cipher and the SHA-256 hasher are checked at
//! build time in `lib.rs`.)

use zeroize::ZeroizeOnDrop;

/// Argon2id's working memory holds enough to recompute the KEK without paying the KDF cost
/// (the KEK is a hash of the last block of each lane), so it must be wiped too.
#[test]
fn the_kdf_working_memory_is_wiped_on_drop() {
    fn wipes<T: ZeroizeOnDrop>(_: &T) {}
    let params = argon2::Params::new(8, 1, 1, Some(32)).unwrap();
    let memory = crate::kdf_memory(&params).unwrap();
    wipes(&memory);
    assert_eq!(memory.len(), params.block_count());
}
