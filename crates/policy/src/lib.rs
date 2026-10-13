//! Master Password policy (PRD: Implementation Decisions, module 5).
//!
//! A copied Vault file can be attacked offline with no rate limit, so the gate is strict:
//! * at least 15 characters, counted on the NFC form the Vault uses,
//! * not built on a famous phrase from `data/famous_phrases.txt` (matched ignoring case, spaces,
//!   punctuation and look-alike digits such as "c0rrect h0rse"). A famous phrase counts for
//!   nothing wherever it appears and however often, so at least 15 other letters and digits must
//!   remain. Common passwords need no list of our own: zxcvbn ships one, and
//! * at least 10^16 guesses by zxcvbn's analysis. That is stricter than its score 4 (10^10),
//!   which let four top-100 passwords glued together ("monkeydragonshadowmaster", about 10^12)
//!   through. Two things about zxcvbn's own number are corrected, because each alone reaches
//!   the bar for passwords an attacker tries early:
//!   * it adds 10^4 guesses for every token beyond the first, so anything it splits into five
//!     tokens scores 10^16 whatever the tokens are. The estimate used here is the tokens'
//!     guesses multiplied together (times the orderings), without that addition;
//!   * it treats each space or hyphen as a token of its own, which rated "1 2 3 4 5 6 7 8 9 0"
//!     at about 10^19. So the password is judged as typed and again with its separators taken
//!     out, and both readings must pass.
//!
//! No composition rules (no "must contain a symbol"), per NIST SP 800-63B-4. Feedback is plain
//! English suitable for a beginner, never blaming.
//!
//! Suggestions are 6 words from the EFF long word list (7,776 words, about 77.5 bits), chosen
//! with the OS random generator using rejection sampling (no modulo bias), joined with '-'.
//! A suggestion always passes [`assess`].

#![forbid(unsafe_code)]

use std::collections::HashMap;
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
    /// 0-4 for the weaker of the two readings; 4 means it reaches the bar (0 for an empty or
    /// famous password).
    pub score: u8,
    /// Zero or more short plain-English hints, most important first.
    pub feedback: Vec<String>,
}

pub fn assess(password: &str) -> Assessment {
    let nfc: Zeroizing<String> = Zeroizing::new(password.nfc().collect());
    let long_enough = nfc.chars().count() >= MIN_CHARS;
    let famous = is_famous(&nfc);
    // A famous phrase is refused whatever zxcvbn makes of it, so its estimate is not run.
    let weakest = (!famous).then(|| weakest_estimate(&nfc, long_enough));
    let (entropy, guesses_log10) = match &weakest {
        Some((entropy, guesses_log10)) => (Some(entropy), *guesses_log10),
        None => (None, 0.0),
    };
    let score = if famous { 0 } else { score_for(guesses_log10) };
    let acceptable = long_enough && !famous && guesses_log10 >= MIN_GUESSES_LOG10;

    let mut feedback = Vec::new();
    if !acceptable {
        if !long_enough {
            push_unique(&mut feedback, HINT_LENGTH);
        }
        if famous {
            push_unique(&mut feedback, HINT_WELL_KNOWN);
        } else if let Some(fb) = entropy.and_then(|e| e.feedback()) {
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

/// The weaker of two readings of the password, as typed and without its separators: zxcvbn's
/// analysis of that reading, and log10 of the guesses it comes to (see [`guesses_log10`]).
///
/// zxcvbn can take seconds on a long password full of symbols and this runs as the owner types,
/// so the second reading is made only when it can change the answer: the password passes as
/// typed, and taking the separators out leaves something different to judge. Neither reading is
/// cut shorter than zxcvbn's own 100 characters: a cut can land inside a repeat and raise the
/// estimate.
fn weakest_estimate(nfc: &str, long_enough: bool) -> (zxcvbn::Entropy, f64) {
    let typed = zxcvbn::zxcvbn(nfc, &[]);
    let typed_guesses = guesses_log10(&typed);
    if !long_enough || typed_guesses < MIN_GUESSES_LOG10 {
        return (typed, typed_guesses);
    }
    let bare = without_separators(nfc);
    if bare.is_empty() || bare.as_str() == nfc {
        return (typed, typed_guesses);
    }
    let stripped = zxcvbn::zxcvbn(&bare, &[]);
    let stripped_guesses = guesses_log10(&stripped);
    if stripped_guesses < typed_guesses {
        (stripped, stripped_guesses)
    } else {
        (typed, typed_guesses)
    }
}

/// log10 of: the guesses of each token zxcvbn found, multiplied together, times the number of
/// orderings. This is zxcvbn's own estimate without the 10^4 it adds per extra token, which
/// makes any five-token password worth 10^16 ("monkeydragonshadowmasterqwerty"). Never more
/// than zxcvbn's figure.
fn guesses_log10(entropy: &zxcvbn::Entropy) -> f64 {
    let tokens = entropy.sequence();
    let orderings: f64 = (1..=tokens.len()).map(|n| (n as f64).log10()).sum();
    let each: f64 = tokens
        .iter()
        .map(|token| (token.guesses.unwrap_or(1).max(1) as f64).log10())
        .sum();
    (orderings + each).min(entropy.guesses_log10())
}

/// 0-4 for the strength meter, in even steps up to the bar: 4 means the estimate reaches
/// 10^16. (zxcvbn's own scale gives 4 from 10^10, which would show full marks for passwords
/// the gate refuses.)
fn score_for(guesses_log10: f64) -> u8 {
    [0.25, 0.5, 0.75, 1.0]
        .iter()
        .filter(|part| guesses_log10 >= **part * MIN_GUESSES_LOG10)
        .count() as u8
}

/// The password without what only separates its parts, which costs a guesser nothing:
/// * spaces,
/// * any digit or symbol used three times or more ("one-two-three-four", "a*b*c*d",
///   "one1two1three1four"),
/// * every symbol, when there are no letters at all (a phone number, a date).
///
/// A symbol or digit used once or twice stays, and so do capitals: they can be part of what
/// makes a password hard to guess, and a random password rarely repeats one three times.
fn without_separators(text: &str) -> Zeroizing<String> {
    let has_letters = text.chars().any(char::is_alphabetic);
    // Counted once up front: a long paste must not cost a pass over the text per character.
    let mut uses: HashMap<char, usize> = HashMap::new();
    for c in text.chars().filter(|c| !c.is_alphabetic()) {
        *uses.entry(c).or_default() += 1;
    }
    let used_often = |c: char| uses.get(&c).is_some_and(|n| *n >= 3);
    Zeroizing::new(
        text.chars()
            .filter(|&c| {
                if c.is_whitespace() {
                    false
                } else if c.is_alphabetic() {
                    true
                } else if c.is_numeric() {
                    !(has_letters && used_often(c))
                } else {
                    has_letters && !used_often(c)
                }
            })
            .collect(),
    )
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

/// Like [`fold`], with the usual look-alikes turned back into letters first ("c0rrect h0r$e").
/// '1' stands for 'i' or 'l', so both letters become 'i'.
fn fold_leet(text: &str) -> Zeroizing<String> {
    Zeroizing::new(
        text.chars()
            .flat_map(char::to_lowercase)
            .map(|c| match c {
                '0' => 'o',
                '1' | 'l' => 'i',
                '3' => 'e',
                '4' | '@' => 'a',
                '5' | '$' => 's',
                '7' => 't',
                other => other,
            })
            .filter(|c| c.is_alphanumeric())
            .collect(),
    )
}

/// Both folds are tried, on the list and on the password: '@' and '$' can be look-alikes or
/// separators.
fn is_famous(password: &str) -> bool {
    static PHRASES: OnceLock<[Vec<String>; 2]> = OnceLock::new();
    let [plain, leet] = PHRASES.get_or_init(|| {
        let listed = |folded: fn(&str) -> Zeroizing<String>| {
            FAMOUS_PHRASES
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .map(|l| folded(l).to_string())
                .collect()
        };
        [listed(fold), listed(fold_leet)]
    });
    built_on_phrases(&fold(password), plain) || built_on_phrases(&fold_leet(password), leet)
}

/// True when `folded` contains a phrase and fewer than [`MIN_CHARS`] characters outside every
/// occurrence of every phrase: a famous phrase counts for nothing, so the rest must be long
/// enough on its own.
fn built_on_phrases(folded: &str, phrases: &[String]) -> bool {
    let mut covered = vec![false; folded.len()];
    let mut found = false;
    for phrase in phrases.iter().filter(|phrase| !phrase.is_empty()) {
        // Occurrences can overlap ("to be or not to be or not to be"), so the search goes on
        // one character after the start of each, not after its end.
        let step = phrase.chars().next().map_or(1, char::len_utf8);
        let mut from = 0;
        while let Some(at) = folded[from..].find(phrase.as_str()).map(|i| from + i) {
            covered[at..at + phrase.len()].fill(true);
            found = true;
            from = at + step;
        }
    }
    found
        && folded
            .char_indices()
            .filter(|(at, _)| !covered[*at])
            .count()
            < MIN_CHARS
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
