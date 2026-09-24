//! Behaviour of the Master Password policy, through the public interface only.

use policy::{MIN_CHARS, PASSPHRASE_WORDS, REQUIRED_SCORE, assess, suggest_passphrase, wordlist};

// ---- assess: the strength gate ----

#[test]
fn empty_password_is_refused_with_score_zero_and_a_hint() {
    let a = assess("");
    assert!(!a.acceptable);
    assert_eq!(a.score, 0);
    assert!(!a.feedback.is_empty());
}

#[test]
fn short_password_is_refused_even_if_random_looking() {
    // 14 characters of noise: strong-looking but under the length floor.
    let a = assess("q7#Vz!m2Kx9@Lp");
    assert!(!a.acceptable);
    assert!(
        a.feedback[0].contains("15 characters"),
        "length hint should come first: {:?}",
        a.feedback
    );
}

#[test]
fn length_counts_characters_not_bytes() {
    // 14 multi-byte characters (well over 15 bytes) are still too short.
    let pw = "ÅßÇðéƒĝħîĵķłɱñ";
    assert_eq!(pw.chars().count(), 14);
    assert!(pw.len() > MIN_CHARS);
    let a = assess(pw);
    assert!(!a.acceptable);
    assert!(a.feedback[0].contains("15 characters"));
}

#[test]
fn common_passwords_are_refused() {
    for pw in ["password", "123456", "qwerty", "letmein", "iloveyou"] {
        let a = assess(pw);
        assert!(!a.acceptable, "{pw} should be refused");
        assert!(a.score < REQUIRED_SCORE);
    }
}

/// PRD module 5: "not in a common-password blocklist". Famous, widely published phrases are
/// in every cracking wordlist even though zxcvbn alone rates them "very unguessable".
#[test]
fn famous_passphrases_are_refused_even_though_long() {
    for pw in [
        "correcthorsebatterystaple",
        "correct horse battery staple",
        "thequickbrownfoxjumpsoverthelazydog",
    ] {
        assert!(pw.chars().count() >= MIN_CHARS);
        let a = assess(pw);
        assert!(!a.acceptable, "{pw:?} should be blocklisted");
        assert!(a.score < REQUIRED_SCORE, "{pw:?} got {a:?}");
        assert!(
            a.feedback.iter().any(|h| h.contains("well-known")),
            "{pw:?} should get the well-known hint: {:?}",
            a.feedback
        );
    }
}

#[test]
fn blocklist_ignores_case_spaces_and_separators() {
    for pw in [
        "Correct-Horse-Battery-Staple",
        "CORRECT_HORSE_BATTERY_STAPLE!",
        "The quick brown fox jumps over the lazy dog.",
    ] {
        assert!(!assess(pw).acceptable, "{pw:?} should be blocklisted");
    }
}

#[test]
fn common_passwords_joined_together_are_refused() {
    for pw in [
        "monkeydragonshadowmaster",
        "sunshine-princess-football",
        "Iloveyou Trustno1 Letmein",
    ] {
        assert!(pw.chars().count() >= MIN_CHARS);
        // zxcvbn's own password lists catch these; no list of ours needed.
        assert!(!assess(pw).acceptable, "{pw:?} should be refused");
    }
}

#[test]
fn blocklist_does_not_refuse_unrelated_long_passwords() {
    // Contains a common password as one part, but is not made only of well-known pieces.
    let a = assess("correct-horse-test-password-1-zebra-quilt");
    assert!(a.acceptable, "{a:?}");
}

/// The vault uses the NFC form of the Master Password, so the gate judges that form.
#[test]
fn length_is_judged_on_the_nfc_form() {
    let decomposed = "q7#Vz!m2Kx9@Le\u{301}"; // 15 scalars, NFC form has 14 characters
    let composed = "q7#Vz!m2Kx9@L\u{e9}"; // the same password, 14 scalars
    let d = assess(decomposed);
    assert!(!d.acceptable, "{d:?}");
    assert!(d.feedback[0].contains("15 characters"), "{:?}", d.feedback);
    assert_eq!(d.acceptable, assess(composed).acceptable);
}

#[test]
fn composed_and_decomposed_forms_of_a_strong_password_are_both_accepted() {
    let composed = "q7#Vz!m2Kx9@L\u{e9}Wq\u{f1}"; // 17 characters
    let decomposed = "q7#Vz!m2Kx9@Le\u{301}Wqn\u{303}";
    assert!(assess(composed).acceptable, "{:?}", assess(composed));
    assert!(assess(decomposed).acceptable, "{:?}", assess(decomposed));
}

#[test]
fn long_but_guessable_passwords_are_refused() {
    for pw in [
        "passwordpassword",
        "aaaaaaaaaaaaaaaaaaaa",
        "abcdefghijklmnopqrstuvwxyz",
        "1234567890123456",
        "qwertyuiopasdfghjkl",
    ] {
        assert!(pw.chars().count() >= MIN_CHARS);
        let a = assess(pw);
        assert!(!a.acceptable, "{pw} should be refused");
        assert!(a.score < REQUIRED_SCORE);
        assert!(!a.feedback.is_empty(), "{pw} should get a hint");
    }
}

#[test]
fn long_unguessable_password_is_accepted_without_feedback() {
    let a = assess("correct-horse-test-password-1-zebra-quilt");
    assert!(a.acceptable, "{a:?}");
    assert_eq!(a.score, REQUIRED_SCORE);
    assert!(a.feedback.is_empty());
}

#[test]
fn score_is_reported_even_when_too_short() {
    let a = assess("x");
    assert!(a.score <= 4);
    assert!(!a.acceptable);
}

#[test]
fn score_never_exceeds_four() {
    for pw in ["", "a", "tvq-fake-test-value-ghx-plo-wmn-zzr-9912"] {
        assert!(assess(pw).score <= 4);
    }
}

#[test]
fn feedback_never_repeats_the_password() {
    let pw = "fakepassword";
    for hint in assess(pw).feedback {
        assert!(!hint.contains(pw), "hint leaks the password: {hint}");
    }
}

#[test]
fn feedback_has_no_duplicate_or_empty_hints() {
    for pw in ["", "abc", "passwordpassword", "q7#Vz!m2Kx9@Lp"] {
        let fb = assess(pw).feedback;
        for (i, h) in fb.iter().enumerate() {
            assert!(!h.trim().is_empty());
            assert!(!fb[..i].contains(h), "duplicate hint {h:?} in {fb:?}");
        }
    }
}

#[test]
fn very_long_input_does_not_panic() {
    let pw = "fake-long-input-".repeat(10_000);
    let _ = assess(&pw);
}

// ---- wordlist ----

#[test]
fn wordlist_has_the_7776_eff_words() {
    let w = wordlist();
    assert_eq!(w.len(), 7776);
    assert_eq!(w[0], "abacus");
    assert_eq!(w[7775], "zoom");
}

#[test]
fn wordlist_words_are_clean_lowercase_and_unique() {
    let w = wordlist();
    for word in w {
        assert!(!word.is_empty());
        assert!(
            word.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "unexpected word {word:?}"
        );
        assert!(!word.contains('\t') && !word.contains('\r'));
    }
    let mut sorted: Vec<_> = w.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), w.len());
}

#[test]
fn wordlist_is_parsed_once() {
    assert!(std::ptr::eq(wordlist(), wordlist()));
}

// ---- suggest_passphrase ----

#[test]
fn suggestion_has_six_words_from_the_list_joined_by_hyphens() {
    let list = wordlist();
    let s = suggest_passphrase();
    // EFF words may contain '-' themselves (e.g. "t-shirt"), so match greedily against the list.
    let words = split_into_list_words(&s, list).expect("every part should be a list word");
    assert_eq!(words, PASSPHRASE_WORDS);
}

#[test]
fn suggestions_always_pass_assess() {
    for _ in 0..50 {
        let s = suggest_passphrase();
        let a = assess(&s);
        assert!(a.acceptable, "suggestion refused: {a:?}");
    }
}

#[test]
fn consecutive_suggestions_differ() {
    let mut prev = suggest_passphrase();
    for _ in 0..20 {
        let next = suggest_passphrase();
        assert_ne!(*prev, *next);
        prev = next;
    }
}

#[test]
fn suggestions_use_many_different_words() {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for _ in 0..50 {
        let s = suggest_passphrase();
        for w in s.split('-') {
            seen.insert(w.to_owned());
        }
    }
    // 300 draws from 7,776 words: a working RNG gives far more than 200 distinct parts.
    assert!(seen.len() > 200, "only {} distinct words", seen.len());
}

/// Counts how many list words `s` splits into, allowing words that contain '-'.
fn split_into_list_words(s: &str, list: &[&str]) -> Option<usize> {
    fn go(rest: &str, list: &[&str]) -> Option<usize> {
        if rest.is_empty() {
            return Some(0);
        }
        for w in list {
            if let Some(after) = rest.strip_prefix(w) {
                if after.is_empty() {
                    return Some(1);
                }
                if let Some(n) = after.strip_prefix('-').and_then(|a| go(a, list)) {
                    return Some(n + 1);
                }
            }
        }
        None
    }
    go(s, list)
}
