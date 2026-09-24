//! Master Password policy (PRD: Implementation Decisions, module 5).
//!
//! A copied Vault file can be attacked offline with no rate limit, so the gate is strict:
//! * at least 15 characters, counted on the NFC form the Vault uses,
//! * not a famous phrase from `data/famous_phrases.txt` (matched ignoring case, spaces and
//!   punctuation). Common passwords need no list of our own: zxcvbn ships one, and
//! * zxcvbn estimates at least 10^16 guesses. That is stricter than its score 4 (10^10), which
//!   let four top-100 passwords glued together ("monkeydragonshadowmaster", about 10^12) through.
//!
//! No composition rules (no "must contain a symbol"), per NIST SP 800-63B-4. Feedback is plain
//! English suitable for a beginner, never blaming.
//!
//! Suggestions are 6 words from the EFF long word list (7,776 words, about 77.5 bits), chosen
//! with the OS random generator using rejection sampling (no modulo bias), joined with '-'.
//! A suggestion always passes [`assess`].

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

pub const MIN_CHARS: usize = 15;
pub const PASSPHRASE_WORDS: usize = 6;
pub const REQUIRED_SCORE: u8 = 4;
/// log10 of the zxcvbn guess estimate a Master Password needs.
pub const MIN_GUESSES_LOG10: f64 = 16.0;

/// The EFF long word list, shipped in `data/eff_large_wordlist.txt`
/// (SHA-256 addd35536511597a02fa0a9ff1e5284677b8883b83e986e43f15a3db996b903e).
const EFF_LIST: &str = include_str!("../data/eff_large_wordlist.txt");
const FAMOUS_PHRASES: &str = include_str!("../data/famous_phrases.txt");

const HINT_LENGTH: &str = "Use at least 15 characters. A few random words work well.";
const HINT_WELL_KNOWN: &str = "This is a well-known phrase that attackers try early. \
     Choose something of your own, or use a suggested passphrase.";
const HINT_GUESSABLE: &str =
    "This could still be guessed. Add another uncommon word or two, or use a suggested passphrase.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assessment {
    pub acceptable: bool,
    /// zxcvbn score 0-4 (0 for an empty or famous password).
    pub score: u8,
    /// Zero or more short plain-English hints, most important first.
    pub feedback: Vec<String>,
}

pub fn assess(password: &str) -> Assessment {
    let nfc: Zeroizing<String> = Zeroizing::new(password.nfc().collect());
    let long_enough = nfc.chars().count() >= MIN_CHARS;
    let famous = is_famous(&nfc);
    let entropy = zxcvbn::zxcvbn(&nfc, &[]);
    let score = if famous {
        0
    } else {
        u8::from(entropy.score()).min(REQUIRED_SCORE)
    };
    let acceptable = long_enough && !famous && entropy.guesses_log10() >= MIN_GUESSES_LOG10;

    let mut feedback = Vec::new();
    if !acceptable {
        if !long_enough {
            push_unique(&mut feedback, HINT_LENGTH);
        }
        if famous {
            push_unique(&mut feedback, HINT_WELL_KNOWN);
        } else if let Some(fb) = entropy.feedback() {
            if let Some(warning) = fb.warning() {
                push_unique(&mut feedback, &warning.to_string());
            }
            for suggestion in fb.suggestions() {
                push_unique(&mut feedback, &suggestion.to_string());
            }
        }
        if feedback.is_empty() {
            push_unique(&mut feedback, HINT_GUESSABLE);
        }
    }
    Assessment {
        acceptable,
        score,
        feedback,
    }
}

fn push_unique(feedback: &mut Vec<String>, hint: &str) {
    if !hint.trim().is_empty() && !feedback.iter().any(|h| h == hint) {
        feedback.push(hint.to_owned());
    }
}

/// Lower-case letters and digits only.
fn fold(text: &str) -> Zeroizing<String> {
    Zeroizing::new(
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect(),
    )
}

fn is_famous(password: &str) -> bool {
    static PHRASES: OnceLock<Vec<String>> = OnceLock::new();
    let phrases = PHRASES.get_or_init(|| {
        FAMOUS_PHRASES
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| fold(l).to_string())
            .collect()
    });
    let folded = fold(password);
    phrases.contains(&*folded)
}

/// The 7,776 words, parsed once (second column of the file).
pub fn wordlist() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        EFF_LIST
            .lines()
            .filter_map(|l| l.split('\t').nth(1))
            .map(str::trim)
            .collect()
    })
}

pub fn suggest_passphrase() -> Zeroizing<String> {
    // Re-draw on the astronomically unlikely refusal (e.g. the same word six times).
    loop {
        let words = wordlist();
        let mut out = Zeroizing::new(String::with_capacity(PASSPHRASE_WORDS * 10));
        for i in 0..PASSPHRASE_WORDS {
            if i > 0 {
                out.push('-');
            }
            out.push_str(words[random_index(words.len())]);
        }
        if assess(&out).acceptable {
            return out;
        }
    }
}

/// A uniform index in `0..n` (n < 2^32) from the OS random generator, by rejection sampling.
fn random_index(n: usize) -> usize {
    let n = n as u64;
    let limit = (1u64 << 32) / n * n;
    loop {
        // No safe fallback for a broken OS random generator: a guessable suggestion is worse
        // than none.
        #[allow(clippy::expect_used)]
        let x = u64::from(getrandom::u32().expect("the OS random number generator is unavailable"));
        if x < limit {
            return (x % n) as usize;
        }
    }
}
