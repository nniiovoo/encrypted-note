//! Test helpers: a fake WalletCipher and Note builders. No real secrets anywhere.

use std::collections::BTreeMap;

use notes::{Kind, NewNote, NotesError, WalletCipher, field};
use zeroize::Zeroizing;

/// Reversible XOR transform plus a tag naming the key, note id and field, so opening with the
/// wrong key or under the wrong note/field is detected.
pub struct FakeCipher {
    pub key: u8,
}

impl FakeCipher {
    fn tag(&self, note_id: &str, field: &str) -> Vec<u8> {
        format!("FAKE:{}:{}:{}|", self.key, note_id, field).into_bytes()
    }
}

impl WalletCipher for FakeCipher {
    fn seal(&self, note_id: &str, field: &str, plaintext: &[u8]) -> Result<Vec<u8>, NotesError> {
        let mut out = self.tag(note_id, field);
        out.extend(plaintext.iter().map(|b| b ^ self.key));
        Ok(out)
    }

    fn open(
        &self,
        note_id: &str,
        field: &str,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, NotesError> {
        let tag = self.tag(note_id, field);
        let rest = sealed
            .strip_prefix(tag.as_slice())
            .ok_or(NotesError::WalletCipher)?;
        Ok(Zeroizing::new(rest.iter().map(|b| b ^ self.key).collect()))
    }
}

pub const CIPHER: FakeCipher = FakeCipher { key: 0x5a };

pub const SEED_12: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
pub const SEED_24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

pub fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

pub fn hidden(pairs: &[(&str, &str)]) -> BTreeMap<String, Zeroizing<String>> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), Zeroizing::new(v.to_string())))
        .collect()
}

pub fn login(title: &str, website: &str, username: &str, password: &str) -> NewNote {
    NewNote {
        kind: Kind::Login,
        title: title.into(),
        visible: map(&[(field::WEBSITE, website), (field::USERNAME, username)]),
        hidden: hidden(&[(field::PASSWORD, password)]),
    }
}

pub fn text(title: &str, body: &str) -> NewNote {
    NewNote {
        kind: Kind::Text,
        title: title.into(),
        visible: BTreeMap::new(),
        hidden: hidden(&[(field::BODY, body)]),
    }
}

pub fn api_key(title: &str, service: &str, key: &str) -> NewNote {
    NewNote {
        kind: Kind::ApiKey,
        title: title.into(),
        visible: map(&[(field::SERVICE, service)]),
        hidden: hidden(&[(field::KEY, key)]),
    }
}

pub fn seed(title: &str, wallet: &str, words: &str) -> NewNote {
    NewNote {
        kind: Kind::SeedPhrase,
        title: title.into(),
        visible: map(&[(field::WALLET_NAME, wallet)]),
        hidden: hidden(&[(field::WORDS, words)]),
    }
}

pub fn private_key(title: &str, chain: &str, address: &str, key: &str) -> NewNote {
    NewNote {
        kind: Kind::PrivateKey,
        title: title.into(),
        visible: map(&[(field::CHAIN, chain), (field::ADDRESS, address)]),
        hidden: hidden(&[(field::KEY, key)]),
    }
}
