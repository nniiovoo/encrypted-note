//! Private Key format hints. **Warnings only**: they never block saving (PRD user story 40).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chain {
    /// Ethereum and EVM chains: 64 hex chars, optional `0x`.
    Evm,
    /// Solana: base58 of a 64-byte keypair (87-88 chars), or the `[n, n, ...]` JSON array of 64
    /// numbers 0-255 from a CLI keypair file.
    Solana,
    /// Bitcoin: WIF (base58, 51-52 chars starting 5/K/L/9/c) or 64 hex chars.
    Bitcoin,
    /// Anything else: no check.
    Other,
}

impl Chain {
    /// Stored as the visible `chain` field value.
    pub fn as_str(self) -> &'static str {
        match self {
            Chain::Evm => "evm",
            Chain::Solana => "solana",
            Chain::Bitcoin => "bitcoin",
            Chain::Other => "other",
        }
    }
    pub fn parse(s: &str) -> Option<Chain> {
        let s = s.trim();
        [Chain::Evm, Chain::Solana, Chain::Bitcoin, Chain::Other]
            .into_iter()
            .find(|c| c.as_str().eq_ignore_ascii_case(s))
    }

    /// The chain a key is checked as. The stored `chain` is free text, so any name
    /// [`Chain::parse`] doesn't know is checked as [`Chain::Other`]: no check at all.
    pub fn for_check(name: &str) -> Chain {
        Chain::parse(name).unwrap_or(Chain::Other)
    }

    /// The chain's name in lowercase words, for search: the stored value is the short form
    /// (`evm`), which nobody types when looking for an Ethereum key. `Other` has none.
    pub fn search_words(self) -> &'static [&'static str] {
        match self {
            Chain::Evm => &["ethereum", "evm"],
            Chain::Solana => &["solana"],
            Chain::Bitcoin => &["bitcoin"],
            Chain::Other => &[],
        }
    }
}

/// `None` if the key looks right for the chain, else a short plain-English hint such as
/// "Ethereum private keys are usually 64 hex characters (0-9, a-f), optionally starting with 0x."
///
/// Never a hint for key text in another format ([`is_another_format`]): the chain's hint
/// describes a one-line key and would only mislead there.
pub fn check_private_key(chain: Chain, key: &str) -> Option<&'static str> {
    let key = key.trim();
    if is_another_format(key) {
        return None;
    }
    let (ok, hint) = match chain {
        Chain::Evm => {
            let hex = key
                .strip_prefix("0x")
                .or_else(|| key.strip_prefix("0X"))
                .unwrap_or(key);
            (
                is_hex_64(hex),
                "Ethereum private keys are usually 64 hex characters (0-9, a-f), optionally starting with 0x.",
            )
        }
        Chain::Solana => (
            (is_base58(key) && (87..=88).contains(&key.len())) || parse_solana_keypair(key),
            "Solana private keys are usually 87-88 base58 characters, or a list of 64 numbers from 0 to 255 like [12, 34, ...] from a keypair file.",
        ),
        Chain::Bitcoin => (
            (is_base58(key)
                && (51..=52).contains(&key.len())
                && key.starts_with(['5', 'K', 'L', '9', 'c']))
                || is_hex_64(key),
            "Bitcoin private keys are usually 51-52 characters starting with 5, K, L, 9 or c (WIF), or 64 hex characters.",
        ),
        Chain::Other => return None,
    };
    (!ok).then_some(hint)
}

/// True for key text (already trimmed) that runs over several lines or starts like a PEM block
/// (`-----BEGIN`): key material written some other way than the chain's usual one-line key.
fn is_another_format(key: &str) -> bool {
    key.contains(['\n', '\r']) || key.starts_with("-----BEGIN")
}

/// A CLI keypair file: `[n, n, ...]` with exactly 64 numbers from 0 to 255.
///
/// Checked by hand, keeping nothing: a JSON parser's error message can quote the text it
/// stopped at, and that text is the key.
fn parse_solana_keypair(key: &str) -> bool {
    let Some(inner) = key.strip_prefix('[').and_then(|k| k.strip_suffix(']')) else {
        return false;
    };
    let is_byte = |item: &str| {
        let item = item.trim();
        let plain = !item.is_empty() && item.bytes().all(|b| b.is_ascii_digit());
        // JSON numbers have no leading zeros.
        plain && (item == "0" || !item.starts_with('0')) && item.parse::<u8>().is_ok()
    };
    let mut count = 0usize;
    for item in inner.split(',') {
        if !is_byte(item) {
            return false;
        }
        count += 1;
    }
    count == 64
}

fn is_hex_64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_base58(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() && !matches!(b, b'0' | b'O' | b'I' | b'l'))
}
