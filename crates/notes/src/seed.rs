//! Seed Phrase checks (BIP39 English). These produce **warnings only**: the owner may save a
//! phrase that fails them (PRD user story 35). Word count is the one hard rule (12 or 24).

use zeroize::Zeroizing;

/// Allowed Seed Phrase lengths in v1.
pub const ALLOWED_WORD_COUNTS: [usize; 2] = [12, 24];

/// Result of checking a phrase.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SeedCheck {
    /// 0-based positions of words not in the BIP39 English list.
    pub unknown_words: Vec<usize>,
    /// True if every word is known and the BIP39 checksum matches. False otherwise, including
    /// when the count isn't 12 or 24.
    pub checksum_ok: bool,
}

/// The 2048 BIP39 English words, in order (for autocomplete in the editor).
pub fn wordlist() -> &'static [&'static str] {
    bip39::Language::English.word_list()
}

/// Split on whitespace and lowercase before checking.
pub fn check(phrase: &str) -> SeedCheck {
    let words = Zeroizing::new(normalize(phrase));
    let list = wordlist();
    let unknown_words = words
        .split_whitespace()
        .enumerate()
        .filter(|(_, w)| list.binary_search(w).is_err())
        .map(|(i, _)| i)
        .collect();
    SeedCheck {
        unknown_words,
        checksum_ok: checksum_matches(&words),
    }
}

/// True if all words are known, there are 12 or 24 of them, and their BIP39 checksum matches.
///
/// Done here rather than with `bip39::Mnemonic`, which keeps the whole phrase as word indices
/// and never wipes them (nor its parsing buffers). Every buffer here that holds seed material
/// is zeroized.
fn checksum_matches(words: &str) -> bool {
    let list = wordlist();
    // 24 words * 11 bits = 264 bits = 33 bytes: the entropy followed by its checksum bits.
    let mut bits = Zeroizing::new([0u8; 33]);
    let mut n_bits = 0usize;
    for word in words.split_whitespace() {
        let Ok(index) = list.binary_search(&word) else {
            return false;
        };
        for bit in (0..11).rev() {
            if n_bits >= bits.len() * 8 {
                return false;
            }
            if (index >> bit) & 1 == 1 {
                bits[n_bits / 8] |= 0x80 >> (n_bits % 8);
            }
            n_bits += 1;
        }
    }
    if n_bits != 132 && n_bits != 264 {
        return false;
    }
    let checksum_bits = n_bits / 33;
    let entropy_len = (n_bits - checksum_bits) / 8;
    let first = sha256_first_byte(&bits[..entropy_len]);
    let mask = 0xffu8 << (8 - checksum_bits);
    (first & mask) == (bits[entropy_len] & mask)
}

/// First byte of SHA-256 of a message of at most 55 bytes (one block), wiping its state.
fn sha256_first_byte(message: &[u8]) -> u8 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    debug_assert!(message.len() <= 55);
    let len = message.len().min(55);
    let mut block = Zeroizing::new([0u8; 64]);
    block[..len].copy_from_slice(&message[..len]);
    block[len] = 0x80;
    block[56..].copy_from_slice(&((len as u64) * 8).to_be_bytes());

    let mut w = Zeroizing::new([0u32; 64]);
    for (slot, b) in w.iter_mut().zip(block.chunks_exact(4)) {
        *slot = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    // a..h
    let mut v = Zeroizing::new(H0);
    for i in 0..64 {
        let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
        let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
        let t1 = v[7]
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
        let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let t2 = s0.wrapping_add(maj);
        v.copy_within(0..7, 1);
        v[4] = v[4].wrapping_add(t1);
        v[0] = t1.wrapping_add(t2);
    }
    H0[0].wrapping_add(v[0]).to_be_bytes()[0]
}

/// Words (lowercased, trimmed) from a phrase, for storage as a single space-separated string.
pub fn normalize(phrase: &str) -> String {
    // Built in one pre-sized buffer so no stray partial copies of the words are left behind.
    let mut out = String::with_capacity(phrase.len() * 2);
    for word in phrase.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(word.chars().flat_map(char::to_lowercase));
    }
    out
}
