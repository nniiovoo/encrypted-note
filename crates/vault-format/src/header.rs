//! Parsing and encoding of the plaintext (but authenticated) header. Layout in the crate docs.

use crate::{Error, FORMAT_VERSION, KdfParams, MAGIC, PADDING_BLOCK};

const KDF_ARGON2ID: u8 = 1;
const SLOT_COUNT: u8 = 2;
const WRAPPED_LEN: usize = 64 + TAG_LEN;
const SLOT_LEN: usize = 1 + 16 + 24 + WRAPPED_LEN;
const TAG_LEN: usize = 16;
/// magic, version, flags, vault id, kdf id, m, t, p, slot count, two slots, body nonce.
const HEADER_LEN: usize = 8 + 2 + 2 + 16 + 1 + 4 + 4 + 4 + 1 + 2 * SLOT_LEN + 24;
/// The smallest body: one padding block plus the tag.
const MIN_BODY_LEN: usize = PADDING_BLOCK + TAG_LEN;

/// Which credential a slot belongs to; the value is its `slot_type` byte. Slots are stored in
/// this order.
#[derive(Clone, Copy)]
pub(crate) enum SlotType {
    MasterPassword = 1,
    RecoveryKey = 2,
}

impl SlotType {
    /// Position in [`Header::slots`].
    pub(crate) fn index(self) -> usize {
        self as usize - 1
    }
}

/// One credential slot: `vault_key || wallet_key` encrypted under the credential's KEK.
pub(crate) struct Slot {
    pub salt: [u8; 16],
    pub nonce: [u8; 24],
    pub wrapped: [u8; WRAPPED_LEN],
}

/// Every header field except the body nonce, which [`crate::seal`] draws fresh each time.
pub(crate) struct Header {
    pub vault_id: [u8; 16],
    pub kdf: KdfParams,
    /// Indexed by [`SlotType::index`].
    pub slots: [Slot; 2],
}

/// A structurally valid file, split into its parts. KDF limits are not checked yet.
pub(crate) struct ParsedFile<'a> {
    pub header: Header,
    pub body_nonce: [u8; 24],
    /// All header bytes, body nonce included: the body's associated data.
    pub header_bytes: &'a [u8],
    /// Body ciphertext including its tag.
    pub body: &'a [u8],
}

impl Header {
    /// The full header bytes, ending with `body_nonce`.
    pub(crate) fn encode(&self, body_nonce: &[u8; 24]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + MIN_BODY_LEN);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.vault_id);
        out.push(KDF_ARGON2ID);
        out.extend_from_slice(&self.kdf.m_kib.to_le_bytes());
        out.extend_from_slice(&self.kdf.t.to_le_bytes());
        out.extend_from_slice(&self.kdf.p.to_le_bytes());
        out.push(SLOT_COUNT);
        for (slot_type, slot) in [SlotType::MasterPassword, SlotType::RecoveryKey]
            .into_iter()
            .zip(&self.slots)
        {
            out.push(slot_type as u8);
            out.extend_from_slice(&slot.salt);
            out.extend_from_slice(&slot.nonce);
            out.extend_from_slice(&slot.wrapped);
        }
        out.extend_from_slice(body_nonce);
        debug_assert_eq!(out.len(), HEADER_LEN);
        out
    }
}

/// magic || format_version || vault_id || kdf_id || m || t || p || slot_type.
pub(crate) fn slot_aad(vault_id: &[u8; 16], kdf: KdfParams, slot_type: SlotType) -> Vec<u8> {
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    aad.extend_from_slice(vault_id);
    aad.push(KDF_ARGON2ID);
    aad.extend_from_slice(&kdf.m_kib.to_le_bytes());
    aad.extend_from_slice(&kdf.t.to_le_bytes());
    aad.extend_from_slice(&kdf.p.to_le_bytes());
    aad.push(slot_type as u8);
    aad
}

/// Check structure and version (in that order: a newer version may have a different layout).
pub(crate) fn parse(bytes: &[u8]) -> Result<ParsedFile<'_>, Error> {
    let mut reader = Reader { rest: bytes };
    if reader.array::<8>()? != MAGIC {
        return Err(Error::Malformed("not a Vault file"));
    }
    let version = u16::from_le_bytes(reader.array()?);
    if version != FORMAT_VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    if bytes.len() < HEADER_LEN + MIN_BODY_LEN {
        return Err(Error::Malformed("truncated"));
    }
    if u16::from_le_bytes(reader.array()?) != 0 {
        return Err(Error::Malformed("unknown flags"));
    }
    let vault_id = reader.array()?;
    if reader.byte()? != KDF_ARGON2ID {
        return Err(Error::Malformed("unknown KDF"));
    }
    let kdf = KdfParams {
        m_kib: u32::from_le_bytes(reader.array()?),
        t: u32::from_le_bytes(reader.array()?),
        p: u32::from_le_bytes(reader.array()?),
    };
    if reader.byte()? != SLOT_COUNT {
        return Err(Error::Malformed("wrong slot count"));
    }
    let mut read_slot = |slot_type: SlotType| -> Result<Slot, Error> {
        if reader.byte()? != slot_type as u8 {
            return Err(Error::Malformed("wrong slot type"));
        }
        Ok(Slot {
            salt: reader.array()?,
            nonce: reader.array()?,
            wrapped: reader.array()?,
        })
    };
    let slots = [
        read_slot(SlotType::MasterPassword)?,
        read_slot(SlotType::RecoveryKey)?,
    ];
    let body_nonce = reader.array()?;

    let (header_bytes, body) = bytes
        .split_at_checked(HEADER_LEN)
        .ok_or(Error::Malformed("truncated"))?;
    if body.len() < MIN_BODY_LEN || !(body.len() - TAG_LEN).is_multiple_of(PADDING_BLOCK) {
        return Err(Error::Malformed("body length is not whole padding blocks"));
    }
    Ok(ParsedFile {
        header: Header {
            vault_id,
            kdf,
            slots,
        },
        body_nonce,
        header_bytes,
        body,
    })
}

/// Reads fixed-size fields from the front of a byte slice; running out is `Malformed`.
struct Reader<'a> {
    rest: &'a [u8],
}

impl Reader<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let (field, rest) = self
            .rest
            .split_first_chunk::<N>()
            .ok_or(Error::Malformed("truncated"))?;
        self.rest = rest;
        Ok(*field)
    }

    fn byte(&mut self) -> Result<u8, Error> {
        self.array().map(|[byte]| byte)
    }
}
