use notes::keys::{Chain, check_private_key};

const FAKE_HEX_KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

#[test]
fn chain_names_round_trip() {
    for chain in [Chain::Evm, Chain::Solana, Chain::Bitcoin, Chain::Other] {
        assert_eq!(Chain::parse(chain.as_str()), Some(chain));
        assert_eq!(Chain::parse(&chain.as_str().to_uppercase()), Some(chain));
    }
    assert_eq!(Chain::parse("not-a-chain"), None);
}

#[test]
fn evm_accepts_64_hex_with_or_without_0x() {
    assert_eq!(check_private_key(Chain::Evm, FAKE_HEX_KEY), None);
    assert_eq!(
        check_private_key(Chain::Evm, &format!("0x{}", FAKE_HEX_KEY.to_uppercase())),
        None
    );
    assert_eq!(
        check_private_key(Chain::Evm, &format!("  {FAKE_HEX_KEY} ")),
        None
    );
}

#[test]
fn evm_hints_on_wrong_length_or_non_hex() {
    assert!(check_private_key(Chain::Evm, &FAKE_HEX_KEY[1..]).is_some());
    assert!(check_private_key(Chain::Evm, &FAKE_HEX_KEY.replacen('1', "g", 1)).is_some());
    assert!(check_private_key(Chain::Evm, "").is_some());
}

#[test]
fn solana_accepts_base58_keypair_or_json_array() {
    assert_eq!(check_private_key(Chain::Solana, &"2".repeat(88)), None);
    assert_eq!(check_private_key(Chain::Solana, &"z".repeat(87)), None);
    let array = format!("[{}]", vec!["7"; 64].join(", "));
    assert_eq!(check_private_key(Chain::Solana, &array), None);
    let max = format!("[{}]", vec!["255"; 64].join(","));
    assert_eq!(check_private_key(Chain::Solana, &max), None);
}

#[test]
fn solana_hints_on_bad_input() {
    assert!(check_private_key(Chain::Solana, &"2".repeat(86)).is_some());
    // 0 and l are not base58.
    assert!(check_private_key(Chain::Solana, &"0".repeat(88)).is_some());
    let short = format!("[{}]", vec!["7"; 63].join(","));
    assert!(check_private_key(Chain::Solana, &short).is_some());
    let too_big = format!("[{}]", vec!["256"; 64].join(","));
    assert!(check_private_key(Chain::Solana, &too_big).is_some());
    assert!(check_private_key(Chain::Solana, "[not json").is_some());
}

#[test]
fn bitcoin_accepts_wif_or_hex() {
    assert_eq!(
        check_private_key(Chain::Bitcoin, &format!("5{}", "H".repeat(50))),
        None
    );
    assert_eq!(
        check_private_key(Chain::Bitcoin, &format!("K{}", "x".repeat(51))),
        None
    );
    assert_eq!(
        check_private_key(Chain::Bitcoin, &format!("c{}", "x".repeat(51))),
        None
    );
    assert_eq!(check_private_key(Chain::Bitcoin, FAKE_HEX_KEY), None);
}

#[test]
fn bitcoin_hints_on_bad_input() {
    assert!(check_private_key(Chain::Bitcoin, &format!("A{}", "H".repeat(50))).is_some());
    assert!(check_private_key(Chain::Bitcoin, &format!("5{}", "H".repeat(40))).is_some());
    assert!(check_private_key(Chain::Bitcoin, &format!("5{}", "0".repeat(50))).is_some());
}

#[test]
fn other_chain_is_never_checked() {
    assert_eq!(check_private_key(Chain::Other, "anything at all"), None);
    assert_eq!(check_private_key(Chain::Other, ""), None);
}
