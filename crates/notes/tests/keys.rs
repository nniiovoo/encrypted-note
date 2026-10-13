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
    let long = format!("[{}]", vec!["7"; 65].join(","));
    assert!(check_private_key(Chain::Solana, &long).is_some());
    for odd in ["-1", "1.0", "1e1", "01", "\"7\"", "", " "] {
        let mut items = vec!["7"; 64];
        items[10] = odd;
        let array = format!("[{}]", items.join(","));
        assert!(
            check_private_key(Chain::Solana, &array).is_some(),
            "{odd:?} is not a byte"
        );
    }
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

/// Fake PEM text: the dashes and labels of a real block around a body that is not a key.
const FAKE_PEM: &str =
    "-----BEGIN FAKE PRIVATE KEY-----\nZmFrZS1ub3QtYS1rZXk=\n-----END FAKE PRIVATE KEY-----\n";

#[test]
fn key_text_of_several_lines_or_in_a_pem_block_gets_no_hint_on_any_chain() {
    let one_line_pem = "-----BEGIN FAKE PRIVATE KEY-----ZmFrZQ==-----END FAKE PRIVATE KEY-----";
    for chain in [Chain::Evm, Chain::Solana, Chain::Bitcoin, Chain::Other] {
        assert_eq!(check_private_key(chain, FAKE_PEM), None, "{chain:?}");
        assert_eq!(
            check_private_key(chain, &FAKE_PEM.replace('\n', "\r\n")),
            None,
            "{chain:?}"
        );
        // Recognised by how it starts, whatever is around it.
        assert_eq!(check_private_key(chain, one_line_pem), None, "{chain:?}");
        assert_eq!(
            check_private_key(chain, &format!("  \n{FAKE_PEM}")),
            None,
            "{chain:?}"
        );
        // Not a PEM block, but more than one line: still another format.
        assert_eq!(
            check_private_key(chain, "fake line one\nfake line two"),
            None,
            "{chain:?}"
        );
        assert_eq!(
            check_private_key(chain, "fake line one\rfake line two"),
            None,
            "{chain:?}"
        );
    }
}

#[test]
fn a_line_break_only_around_a_one_line_key_does_not_switch_the_check_off() {
    // It is stored without them (see the document tests), so it is checked without them.
    assert_eq!(
        check_private_key(Chain::Evm, &format!("{FAKE_HEX_KEY}\n")),
        None
    );
    assert!(check_private_key(Chain::Evm, "fake-not-hex\n").is_some());
    assert!(check_private_key(Chain::Bitcoin, "\r\nfake-not-wif\r\n").is_some());
    // "BEGIN" somewhere else is not a PEM block.
    assert!(check_private_key(Chain::Evm, "fake -----BEGIN").is_some());
}

#[test]
fn any_chain_name_becomes_the_chain_to_check_and_unknown_names_are_not_checked() {
    for chain in [Chain::Evm, Chain::Solana, Chain::Bitcoin, Chain::Other] {
        assert_eq!(Chain::for_check(chain.as_str()), chain);
        assert_eq!(
            Chain::for_check(&format!(" {} ", chain.as_str().to_uppercase())),
            chain
        );
    }
    for unknown in ["", "   ", "polygon", "Ethereum", "not-a-chain", "evm2"] {
        assert_eq!(Chain::for_check(unknown), Chain::Other, "{unknown:?}");
        // Which means no hint, whatever the key looks like.
        assert_eq!(
            check_private_key(Chain::for_check(unknown), "anything at all"),
            None
        );
    }
}

#[test]
fn chains_have_search_words_and_other_has_none() {
    assert_eq!(Chain::Evm.search_words(), &["ethereum", "evm"]);
    assert_eq!(Chain::Solana.search_words(), &["solana"]);
    assert_eq!(Chain::Bitcoin.search_words(), &["bitcoin"]);
    assert!(Chain::Other.search_words().is_empty());
    // Search lowercases the query, so the words must be lowercase too.
    for chain in [Chain::Evm, Chain::Solana, Chain::Bitcoin] {
        for word in chain.search_words() {
            assert_eq!(*word, word.to_lowercase());
        }
    }
}
