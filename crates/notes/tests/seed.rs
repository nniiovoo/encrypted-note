use notes::seed::{self, ALLOWED_WORD_COUNTS, SeedCheck};

const VALID_12: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const VALID_12_B: &str =
    "legal winner thank year wave sausage worth useful legal winner thank yellow";
const VALID_24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

fn ok() -> SeedCheck {
    SeedCheck {
        unknown_words: vec![],
        checksum_ok: true,
    }
}

#[test]
fn allowed_word_counts_are_12_and_24() {
    assert_eq!(ALLOWED_WORD_COUNTS, [12, 24]);
}

#[test]
fn wordlist_is_the_2048_bip39_english_words_in_order() {
    let words = seed::wordlist();
    assert_eq!(words.len(), 2048);
    assert_eq!(words[0], "abandon");
    assert_eq!(words[3], "about");
    assert_eq!(words[2047], "zoo");
}

#[test]
fn official_vectors_pass_the_checksum() {
    assert_eq!(seed::check(VALID_12), ok());
    assert_eq!(seed::check(VALID_12_B), ok());
    assert_eq!(seed::check(VALID_24), ok());
}

#[test]
fn wrong_last_word_fails_the_checksum() {
    let bad_12 = VALID_12.replace("about", "abandon");
    let bad_24 = VALID_24.replace("art", "abandon");
    for phrase in [bad_12.as_str(), bad_24.as_str()] {
        assert_eq!(
            seed::check(phrase),
            SeedCheck {
                unknown_words: vec![],
                checksum_ok: false
            }
        );
    }
}

#[test]
fn check_ignores_case_and_extra_whitespace() {
    let messy = format!("  {}  \n", VALID_12_B.to_uppercase().replace(' ', "\t  "));
    assert_eq!(seed::check(&messy), ok());
}

#[test]
fn unknown_words_are_reported_by_position() {
    let phrase = VALID_12
        .replacen("abandon", "notaword", 1)
        .replace("about", "abot");
    assert_eq!(
        seed::check(&phrase),
        SeedCheck {
            unknown_words: vec![0, 11],
            checksum_ok: false
        }
    );
}

#[test]
fn counts_other_than_12_or_24_never_pass_the_checksum() {
    // 15 known words, not an allowed length.
    let fifteen = ["abandon"; 15].join(" ");
    assert!(!seed::check(&fifteen).checksum_ok);
    // More than 24 known words must stop at the 33-byte checksum buffer, not index past it.
    // "zoo" (index 2047) sets every bit, so an unbounded write would actually happen.
    let twenty_five = ["zoo"; 25].join(" ");
    assert!(!seed::check(&twenty_five).checksum_ok);
    let eleven = ["abandon"; 11].join(" ");
    assert_eq!(
        seed::check(&eleven),
        SeedCheck {
            unknown_words: vec![],
            checksum_ok: false
        }
    );
    assert_eq!(
        seed::check(""),
        SeedCheck {
            unknown_words: vec![],
            checksum_ok: false
        }
    );
}

#[test]
fn normalize_lowercases_and_single_spaces() {
    assert_eq!(
        seed::normalize("  Legal\tWINNER \n thank  "),
        "legal winner thank"
    );
    assert_eq!(seed::normalize("   "), "");
}

/// The checksum agrees with the bip39 crate for every possible last word, at both lengths.
#[test]
fn checksum_agrees_with_bip39_for_every_last_word() {
    let prefixes = [
        vec!["legal"; 11],
        "zoo wrong abandon ability able about above absent absorb abstract absurd abuse access \
         accident account accuse achieve acid acoustic acquire across act action"
            .split(' ')
            .collect::<Vec<_>>(),
    ];
    for prefix in prefixes {
        let mut passing = 0;
        for last in seed::wordlist() {
            let phrase = format!("{} {last}", prefix.join(" "));
            let expected =
                bip39::Mnemonic::parse_in_normalized(bip39::Language::English, &phrase).is_ok();
            assert_eq!(seed::check(&phrase).checksum_ok, expected, "{phrase}");
            passing += usize::from(expected);
        }
        // 4 checksum bits for 12 words, 8 for 24.
        assert_eq!(passing, if prefix.len() == 11 { 128 } else { 8 });
    }
}
