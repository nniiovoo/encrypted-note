//! Recovery Key: 128 random bits shown to the owner once, for their Recovery Kit.
//!
//! Display form: 28 Crockford Base32 symbols in 7 groups of 4, e.g. `7K3M-Q9TD-...`.
//! * Symbols 1-26 encode the 16 random bytes (big-endian bit order; the last symbol carries the
//!   final 3 bits, shifted into its high bits, low bits zero).
//! * Symbols 27-28 encode the top 10 bits of SHA-256(the 16 bytes): a checksum that catches
//!   typos.
//!
//! Parsing is forgiving about what humans do: case-insensitive, ignores spaces and dashes, and
//! maps Crockford's look-alikes (`O`/`o` -> `0`, `I`/`i`/`L`/`l` -> `1`). It rejects `U` and any
//! other symbol, the wrong length, non-zero padding bits in symbol 26, and a checksum mismatch.
//!
//! The Recovery Key is deliberately NOT made of BIP39 words, so it can't be confused with a
//! Seed Phrase.

use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Crockford Base32 alphabet (no I, L, O, U).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Symbols carrying the 16 random bytes.
const DATA_SYMBOLS: usize = 26;
/// Data symbols plus the two checksum symbols.
const SYMBOLS: usize = DATA_SYMBOLS + 2;
/// Symbols per display group.
const GROUP: usize = 4;
/// `XXXX-` x 6 + `XXXX`.
const DISPLAY_LEN: usize = SYMBOLS + SYMBOLS / GROUP - 1;

/// A parsed, checksum-valid Recovery Key. Wiped on drop; never printed by `Debug`.
pub struct RecoveryKey {
    bytes: Zeroizing<[u8; 16]>,
    display: SecretString,
}

redacted_debug!(RecoveryKey);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryKeyError {
    /// Not 28 symbols after removing spaces and dashes.
    Length,
    /// A character that isn't Crockford Base32 (0-based position within the cleaned input).
    InvalidSymbol(usize),
    /// Symbols are valid but the checksum doesn't match: almost certainly a typo.
    Checksum,
}

impl RecoveryKey {
    /// 16 bytes from the OS random generator.
    pub fn generate() -> Result<RecoveryKey, crate::Error> {
        Ok(Self::from_bytes(crate::random(Zeroizing::new([0; 16]))?))
    }

    /// Parse what the owner typed (see module docs for the forgiving rules).
    pub fn parse(input: &str) -> Result<RecoveryKey, RecoveryKeyError> {
        let mut values = Zeroizing::new([0u8; SYMBOLS]);
        let mut count = 0usize;
        for c in input.chars().filter(|c| *c != '-' && !c.is_whitespace()) {
            let value = symbol_value(c).ok_or(RecoveryKeyError::InvalidSymbol(count))?;
            if let Some(slot) = values.get_mut(count) {
                *slot = value;
            }
            count += 1;
        }
        if count != SYMBOLS {
            return Err(RecoveryKeyError::Length);
        }

        // The last data symbol carries 3 data bits; its 2 low (padding) bits must be zero.
        if values[DATA_SYMBOLS - 1] & 0b11 != 0 {
            return Err(RecoveryKeyError::Checksum);
        }
        let mut bytes = Zeroizing::new([0u8; 16]);
        for bit in 0..128 {
            let value = values[bit / 5];
            if (value >> (4 - bit % 5)) & 1 == 1 {
                bytes[bit / 8] |= 0x80 >> (bit % 8);
            }
        }
        if [values[DATA_SYMBOLS], values[DATA_SYMBOLS + 1]] != checksum_symbols(&bytes) {
            return Err(RecoveryKeyError::Checksum);
        }
        Ok(Self::from_bytes(bytes))
    }

    /// `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`, uppercase.
    pub fn expose_display(&self) -> &str {
        self.display.expose_secret()
    }

    /// The 16 random bytes (the KDF input for the recovery slot).
    pub fn expose_bytes(&self) -> &[u8; 16] {
        &self.bytes
    }

    fn from_bytes(bytes: Zeroizing<[u8; 16]>) -> RecoveryKey {
        let mut values = Zeroizing::new([0u8; SYMBOLS]);
        for bit in 0..DATA_SYMBOLS * 5 {
            // Bits past the 128th are the zero padding of the last data symbol.
            let set = bit < 128 && (bytes[bit / 8] >> (7 - bit % 8)) & 1 == 1;
            values[bit / 5] = (values[bit / 5] << 1) | u8::from(set);
        }
        values[DATA_SYMBOLS..].copy_from_slice(&checksum_symbols(&bytes));

        // Exact capacity, so converting into the secret box never reallocates (and leaves no copy).
        let mut display = String::with_capacity(DISPLAY_LEN);
        for (i, value) in values.iter().enumerate() {
            if i > 0 && i % GROUP == 0 {
                display.push('-');
            }
            display.push(char::from(ALPHABET[usize::from(*value)]));
        }
        RecoveryKey {
            bytes,
            display: SecretString::from(display),
        }
    }
}

/// Top 10 bits of SHA-256(bytes), as two 5-bit symbol values.
fn checksum_symbols(bytes: &[u8; 16]) -> [u8; 2] {
    let digest = Sha256::digest(bytes);
    let top = (u16::from(digest[0]) << 2) | u16::from(digest[1] >> 6);
    // Both values are below 32.
    [(top >> 5) as u8, (top & 0x1f) as u8]
}

/// Value of one typed symbol, with Crockford's look-alike mapping.
fn symbol_value(c: char) -> Option<u8> {
    let c = c.to_ascii_uppercase();
    match c {
        'O' => Some(0),
        'I' | 'L' => Some(1),
        _ => ALPHABET
            .iter()
            .position(|symbol| char::from(*symbol) == c)
            .and_then(|index| u8::try_from(index).ok()),
    }
}
