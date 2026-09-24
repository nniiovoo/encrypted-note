//! The header parser only lets whole padding blocks reach the private `unpad` through
//! [`crate::open`] (see `conformance.rs` for those cases), so inputs shorter than its length
//! field are checked here directly.

#[test]
fn unpad_rejects_input_too_short_for_a_length_field() {
    for len in 0..4 {
        assert!(crate::unpad(&vec![0; len]).is_none(), "{len} bytes");
    }
}
