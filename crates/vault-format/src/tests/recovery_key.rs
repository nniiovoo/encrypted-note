//! Recovery Key encoding and forgiving parsing.

use crate::{RecoveryKey, RecoveryKeyError};

const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// 16 zero bytes: 26 zero symbols, then the top 10 bits of SHA-256(16 zero bytes) = 0x3747...
/// = 00110 11101 -> '6' 'X'.
const ALL_ZERO_KEY: &str = "0000-0000-0000-0000-0000-0000-006X";

fn symbols(display: &str) -> Vec<char> {
    display.chars().filter(|c| *c != '-').collect()
}

fn from_symbols(symbols: &[char]) -> String {
    symbols.iter().collect()
}

#[test]
fn known_answer_for_all_zero_bytes() {
    let key = RecoveryKey::parse(ALL_ZERO_KEY).unwrap();
    assert_eq!(key.expose_bytes(), &[0u8; 16]);
    assert_eq!(key.expose_display(), ALL_ZERO_KEY);
}

#[test]
fn generated_keys_round_trip_through_their_display_form() {
    for _ in 0..200 {
        let key = RecoveryKey::generate().unwrap();
        let parsed = RecoveryKey::parse(key.expose_display()).unwrap();
        assert_eq!(parsed.expose_bytes(), key.expose_bytes());
        assert_eq!(parsed.expose_display(), key.expose_display());
    }
}

#[test]
fn display_is_seven_uppercase_groups_of_four_crockford_symbols() {
    let key = RecoveryKey::generate().unwrap();
    let display = key.expose_display();
    assert_eq!(display.len(), 34);
    let groups: Vec<&str> = display.split('-').collect();
    assert_eq!(groups.len(), 7);
    for group in groups {
        assert_eq!(group.len(), 4);
        assert!(group.chars().all(|c| CROCKFORD.contains(c)), "{group}");
    }
}

#[test]
fn parse_ignores_case_dashes_and_spaces() {
    let key = RecoveryKey::generate().unwrap();
    let plain = from_symbols(&symbols(key.expose_display()));
    let spaced: String = plain
        .to_lowercase()
        .chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if i % 3 == 0 {
                vec![' ', c]
            } else {
                vec![c, '-']
            }
        })
        .collect();
    for typed in [
        plain.clone(),
        plain.to_lowercase(),
        spaced,
        format!("  {plain}  "),
    ] {
        let parsed = RecoveryKey::parse(&typed).unwrap();
        assert_eq!(parsed.expose_bytes(), key.expose_bytes(), "{typed}");
        assert_eq!(parsed.expose_display(), key.expose_display());
    }
}

#[test]
fn parse_maps_look_alike_letters_to_digits() {
    let key = (0..10_000)
        .map(|_| RecoveryKey::generate().unwrap())
        .find(|k| k.expose_display().contains('0') && k.expose_display().contains('1'))
        .unwrap();
    let display = key.expose_display();
    let look_alikes = ['I', 'i', 'L', 'l'];
    let mut ones = 0;
    let typed: String = display
        .chars()
        .enumerate()
        .map(|(i, c)| match c {
            '0' if i % 2 == 0 => 'O',
            '0' => 'o',
            '1' => {
                ones += 1;
                look_alikes[ones % 4]
            }
            other => other,
        })
        .collect();
    let parsed = RecoveryKey::parse(&typed).unwrap();
    assert_eq!(parsed.expose_bytes(), key.expose_bytes());
    assert_eq!(parsed.expose_display(), display);

    let zero_key = RecoveryKey::parse(&ALL_ZERO_KEY.replace('0', "o")).unwrap();
    assert_eq!(zero_key.expose_bytes(), &[0u8; 16]);
}

#[test]
fn parse_rejects_u_and_other_symbols_with_their_position() {
    let mut typed = symbols(ALL_ZERO_KEY);
    typed[5] = 'U';
    let with_dashes = format!(
        "{}-{}",
        from_symbols(&typed[..4]),
        from_symbols(&typed[4..])
    );
    assert_eq!(
        RecoveryKey::parse(&with_dashes).unwrap_err(),
        RecoveryKeyError::InvalidSymbol(5)
    );
    for bad in ['u', '*', '_', '!', '\u{e9}', '\u{41f}'] {
        let mut typed = symbols(ALL_ZERO_KEY);
        typed[27] = bad;
        assert_eq!(
            RecoveryKey::parse(&from_symbols(&typed)).unwrap_err(),
            RecoveryKeyError::InvalidSymbol(27),
            "{bad}"
        );
    }
}

#[test]
fn parse_rejects_the_wrong_length() {
    let plain = from_symbols(&symbols(ALL_ZERO_KEY));
    for typed in [
        "",
        " - - ",
        &plain[..27],
        &format!("{plain}0"),
        &plain.repeat(2),
    ] {
        assert_eq!(
            RecoveryKey::parse(typed).unwrap_err(),
            RecoveryKeyError::Length,
            "{typed}"
        );
    }
}

#[test]
fn checksum_catches_nearly_every_single_symbol_typo() {
    // A 10-bit checksum misses about 1 in 1024 data typos; typos in the checksum symbols or the
    // padding bits of symbol 26 are always caught.
    let mut typos = 0u32;
    let mut missed = 0u32;
    for _ in 0..8 {
        let key = RecoveryKey::generate().unwrap();
        let original = symbols(key.expose_display());
        for position in 0..28 {
            for replacement in CROCKFORD.chars().filter(|c| *c != original[position]) {
                let mut typed = original.clone();
                typed[position] = replacement;
                typos += 1;
                match RecoveryKey::parse(&from_symbols(&typed)) {
                    Ok(_) => {
                        assert!(position < 26, "a checksum or padding typo slipped through");
                        missed += 1;
                    }
                    Err(e) => assert_eq!(e, RecoveryKeyError::Checksum),
                }
            }
        }
    }
    assert!(missed * 200 < typos, "missed {missed} of {typos}");
}

#[test]
fn non_zero_padding_bits_in_symbol_26_are_rejected() {
    // Symbol 26 carries 3 data bits in its high bits; its low 2 bits must be zero.
    for padding in ['1', '2', '3'] {
        let mut typed = symbols(ALL_ZERO_KEY);
        typed[25] = padding;
        assert_eq!(
            RecoveryKey::parse(&from_symbols(&typed)).unwrap_err(),
            RecoveryKeyError::Checksum
        );
    }
}

#[test]
fn debug_never_shows_the_key() {
    let key = RecoveryKey::generate().unwrap();
    let shown = format!("{key:?}");
    assert_eq!(shown, "RecoveryKey(..)");
}
