# Vault file format (version 1)

This is the exact layout of an encrypted-note **Vault** file, written so that you (or anyone
you trust) can read your **Notes** without the app. A **Backup** and a **Safety Copy** are
complete Vault files in this same format.

If you just need your Notes back, use the **Emergency Reader** first (see the end of this
document). Read on if that doesn't work, or if you want to check the format yourself.

Sources of truth, and what this document was checked against:

* `crates/vault-format/src/lib.rs`, `header.rs`, `recovery_key.rs`: the file, the keys, the
  Recovery Key.
* `crates/notes/src/lib.rs`, `note.rs`: the decrypted body (the Vault document).
* The Python reference decryptor in [section 11](#11-reference-decryptor-python) was run
  against Vaults written by the Rust code (opened with both credentials), and it rebuilds the
  pinned known-answer Vault in [section 12](#12-test-vectors) byte for byte.

All integers are **little-endian**. Offsets are in bytes from the start of the file.

## 1. Overview

```text
header    308 bytes, plaintext but authenticated
  magic, format version, flags, vault id, Argon2id parameters
  slot 1  Master Password -> encrypted (Vault Key, Wallet Key)
  slot 2  Recovery Key    -> encrypted (Vault Key, Wallet Key)
  body nonce
body      4096*k + 16 bytes
  XChaCha20-Poly1305 under the Vault Key, with the whole header as associated data
  -> a padded JSON document holding every Note; the Hidden Fields of Seed Phrase and
     Private Key Notes are encrypted again, inside the JSON, under the Wallet Key
```

To read a Vault: derive a key from the Master Password (or the Recovery Key) with Argon2id,
use it to decrypt that credential's slot, which gives the **Vault Key** and the **Wallet
Key**; decrypt the body with the Vault Key; decrypt Seed Phrase and Private Key Hidden Fields
with the Wallet Key.

## 2. Algorithms

| Purpose | Algorithm | Parameters |
|---|---|---|
| Key derivation (KEK) | **Argon2id**, version 0x13 (decimal 19), RFC 9106 | memory `m` KiB, `t` passes, `p` lanes from the header; 16-byte salt from the slot; 32-byte output; no secret key, no associated data |
| Encryption | **XChaCha20-Poly1305** (draft-irtf-cfrg-xchacha; libsodium's `crypto_aead_xchacha20poly1305_ietf`) | 32-byte key, 24-byte nonce; output is ciphertext followed by the 16-byte Poly1305 tag |
| Recovery Key checksum | SHA-256 | first 10 bits of the digest |
| Master Password text | Unicode **NFC** normalisation, then UTF-8 | not NFKC; nothing is trimmed |

Every vault id, key, salt, nonce and Recovery Key comes from the operating system's random
generator. A nonce is never reused and never a counter: every encryption draws a fresh one.

Keys:

* **Vault Key**: 32 random bytes. Encrypts the body.
* **Wallet Key**: 32 random bytes. Encrypts the Hidden Fields of the **Wallet Kinds** (Seed
  Phrase and Private Key) a second time, inside the body (ADR-0004).
* **KEK** (key-encryption key): one per slot, derived with Argon2id from that slot's
  credential and salt. It encrypts the two keys above.

## 3. Byte layout

### Header (308 bytes)

| Offset | Size | Field | Contents |
|---:|---:|---|---|
| 0 | 8 | magic | `89 45 4E 4F 54 45 0D 0A` (`\x89ENOTE\r\n`) |
| 8 | 2 | format_version | u16 = `1` |
| 10 | 2 | flags | u16 = `0` (anything else is rejected) |
| 12 | 16 | vault_id | random; fixed for the life of the Vault, kept across Key Rotation |
| 28 | 1 | kdf_id | `1` = Argon2id, version 0x13 |
| 29 | 4 | kdf_m_kib | u32, Argon2id memory in KiB |
| 33 | 4 | kdf_t | u32, Argon2id passes (iterations) |
| 37 | 4 | kdf_p | u32, Argon2id lanes (parallelism) |
| 41 | 1 | slot_count | `2` |
| 42 | 121 | slot 1 | the Master Password slot (layout below) |
| 163 | 121 | slot 2 | the Recovery Key slot (layout below) |
| 284 | 24 | body_nonce | random, new on every save |

### Slot (121 bytes, starting at offset `s` = 42 or 163)

| Offset | Size | Field | Contents |
|---:|---:|---|---|
| s + 0 | 1 | slot_type | `1` = Master Password (must be slot 1), `2` = Recovery Key (must be slot 2) |
| s + 1 | 16 | salt | random, new whenever the slot is written |
| s + 17 | 24 | nonce | random, new whenever the slot is written |
| s + 41 | 80 | wrapped | XChaCha20-Poly1305 of the 64 bytes `Vault Key ‖ Wallet Key`, plus the 16-byte tag |

### Body (from offset 308 to the end of the file)

| Offset | Size | Contents |
|---:|---:|---|
| 308 | 4096·k + 16 (k ≥ 1) | XChaCha20-Poly1305 of `padded_plaintext`, plus the 16-byte tag |

So a Vault file is exactly `308 + 4096·k + 16` bytes long. The smallest possible file is 4420
bytes.

## 4. Limits and validation

The Argon2id parameters come from the file, so they are checked **before** any key derivation
runs. The floor stops a doctored file from lowering the protection; the ceiling stops a
doctored file from freezing the computer.

| Parameter | Floor | Ceiling |
|---|---:|---:|
| `kdf_m_kib` | 65,536 (64 MiB) | 2,097,152 (2 GiB) |
| `kdf_t` | 3 | 20 |
| `kdf_p` | 1 | 16 |

and also `kdf_m_kib ≥ 8 × kdf_p` (an Argon2 rule). The app and the Emergency Reader use exactly
these limits (`Limits::PRODUCTION`).

When the app creates a Vault it starts from m = 524,288 KiB (512 MiB), t = 3, p = 4 and
calibrates on that computer: while one derivation takes more than 1 s and memory is above the
floor, it halves the memory (never below the floor); then, while a derivation takes less than
0.5 s and t < 20, it adds one pass (and steps back one if that made it take more than 1 s).
The header holds one set of parameters, used by both slots. Changing the Master Password and
Key Rotation keep them.

A reader rejects the file, in this order, when:

1. the first 8 bytes aren't the magic: **not a Vault file** (a file too short to hold the
   magic and the version is **truncated**);
2. `format_version` isn't 1: **unsupported version** (checked before anything else, because
   another version may have another layout);
3. it is shorter than 4420 bytes: **truncated**;
4. `flags` isn't 0, `kdf_id` isn't 1, `slot_count` isn't 2, or the slot types aren't 1 then 2:
   **malformed**;
5. the body length (file length − 308) isn't 16 more than a non-zero multiple of 4096:
   **malformed**;
6. an Argon2id parameter is outside the limits above: **refused** (no derivation is run);
7. the chosen slot fails to decrypt: **wrong Master Password or Recovery Key** (editing the
   vault id, a KDF parameter or the slot itself also lands here);
8. the body fails to decrypt, or its padding is inconsistent (section 6): **damaged**.

Because every header byte is authenticated (the whole header is the body's associated data),
changing any byte of the file makes one of these steps fail.

## 5. Credential slots

Each slot holds the **same two keys**, `Vault Key (32 bytes) ‖ Wallet Key (32 bytes)`,
encrypted under that slot's KEK:

```text
KEK     = Argon2id(input, salt = slot.salt, m, t, p, version 0x13, output 32 bytes)
wrapped = XChaCha20-Poly1305-Encrypt(key = KEK, nonce = slot.nonce,
                                     plaintext = vault_key ‖ wallet_key,
                                     associated data = slot AAD)       -> 64 + 16 bytes
```

The Argon2id `input` is:

* **Master Password slot (type 1):** the Master Password normalised to Unicode NFC, as UTF-8
  bytes. (For example, `é` typed as `e` + U+0301 is the same as U+00E9; the `ﬁ` ligature
  U+FB01 stays a ligature, because this is NFC, not NFKC.)
* **Recovery Key slot (type 2):** the Recovery Key's **16 raw bytes** (decoded as in section
  9), not the printed text.

The slot AAD (associated data) is 40 bytes:

| Size | Contents |
|---:|---|
| 8 | magic |
| 2 | format_version (u16) |
| 16 | vault_id |
| 1 | kdf_id |
| 4 | kdf_m_kib (u32) |
| 4 | kdf_t (u32) |
| 4 | kdf_p (u32) |
| 1 | slot_type |

So editing a KDF parameter or the vault id, or moving one slot's contents into the other
slot, makes the slot fail to decrypt. (`flags` and `slot_count` are not in the slot AAD, but they are in the body's.)

When things change:

| Event | What is rewritten |
|---|---|
| Every save | the body, under a new `body_nonce`; the slots stay the same |
| Changing the Master Password | slot 1 only (new salt, new nonce, same two keys); the Recovery Key keeps working |
| Key Rotation | new Vault Key, new Wallet Key and new Recovery Key; both slots rewritten; every Wallet Kind field re-encrypted; vault id and KDF parameters kept |

Older copies (Backups, Safety Copies) keep the slots they had, so they still open with the
Master Password and Recovery Key that were current when they were saved.

## 6. Body

```text
padded_plaintext = len (u32) ‖ document (len bytes of UTF-8 JSON) ‖ zero bytes
body             = XChaCha20-Poly1305-Encrypt(key = Vault Key, nonce = body_nonce,
                                              plaintext = padded_plaintext,
                                              associated data = file bytes 0..308)
```

* The associated data is **every header byte**, `body_nonce` included (offsets 0 to 307).
* `padded_plaintext` is exactly the smallest multiple of 4096 bytes that holds `4 + len` bytes
  (so an empty document still takes one 4096-byte block). This hides the exact size of the
  Notes.
* On reading, the body is **damaged** if `padded_plaintext` has any other length than that, or
  if any padding byte isn't zero. Each document has exactly one valid encoding.
* `len` is a u32, so the document is at most 2³² − 1 bytes.

How the app checks a file it has just written (every save, every **Backup**): it reads the
file back and requires bytes 0..284 (everything before `body_nonce`) to be exactly the header
it holds in memory (same vault id, Argon2id parameters and both slots), then decrypts the body
with the Vault Key it already has and parses the document. No key derivation runs, so this
check takes milliseconds. A reader of the format doesn't need it.

## 7. Wallet Kind fields

The Hidden Fields of Seed Phrase and Private Key Notes (the words, the Passphrase, the key) are
encrypted a second time under the **Wallet Key**, so that a merely Unlocked app can't read them
(ADR-0004). Each value is stored inside the JSON document as a lowercase hex string of:

```text
sealed = nonce (24 random bytes) ‖ XChaCha20-Poly1305-Encrypt(key = Wallet Key, nonce,
                                                             plaintext = the value as UTF-8,
                                                             associated data = wallet AAD)
```

The wallet AAD binds the value to this Vault, this Note and this field:

| Size | Contents |
|---:|---|
| 15 | the ASCII bytes `enote-wallet-v1` |
| 16 | vault_id |
| 4 | u32 byte length of the Note id |
| n | the Note id (its 32-character lowercase hex string, as ASCII) |
| 4 | u32 byte length of the field name |
| f | the field name as ASCII: `words`, `passphrase` or `key` |

A value of `v` bytes becomes `24 + v + 16` bytes, which is `2 × (40 + v)` hex characters. A
sealed value copied to another field, Note or Vault fails to decrypt.

## 8. The Vault document (body JSON, schema version 1)

The decrypted document is one JSON object. The app writes it compactly (no whitespace) with
object keys in the order below; maps (`visible`, `hidden`, `sealed`) have their keys sorted.
The app's reader accepts any key order and whitespace.

### Top level

| Key | JSON type | Meaning |
|---|---|---|
| `schema_version` | integer | `1` (anything else is rejected) |
| `changed_at` | integer | Unix time in seconds of the last change to the document |
| `change_counter` | integer (u64) | goes up by one on every change; with `changed_at` it tells which of two copies is older |
| `notes` | array | every Note, Trash included |

### A Note

| Key | JSON type | Meaning |
|---|---|---|
| `id` | string | 16 random bytes as 32 lowercase hex characters; unique in the document |
| `kind` | string | `seed_phrase`, `private_key`, `login`, `api_key` or `text` |
| `title` | string | never a Hidden Field; not blank |
| `visible` | object: string → string | the Kind's visible fields (optional; defaults to `{}`) |
| `hidden` | object: string → string | Hidden Fields of **Login, API Key and Text** Notes, as plain text (optional; defaults to `{}`) |
| `sealed` | object: string → string | Hidden Fields of **Seed Phrase and Private Key** Notes, as lowercase hex of section 7 (optional; defaults to `{}`) |
| `favorite` | boolean | the Note is a Favorite (optional; defaults to `false`) |
| `created_at` | integer | Unix time in seconds |
| `updated_at` | integer | Unix time in seconds |
| `deleted_at` | integer or `null` | non-null means the Note is in **Trash** since that time; the app removes it for good 30 days (2,592,000 s) later (optional; defaults to `null`) |

### Fields by Kind

| Kind | `kind` | Visible fields | Hidden Fields | Hidden values are in |
|---|---|---|---|---|
| Seed Phrase | `seed_phrase` | `wallet_name`, `word_count` | `words`, `passphrase` | `sealed` |
| Private Key | `private_key` | `chain`, `address` | `key` | `sealed` |
| Login | `login` | `website`, `username` | `password` | `hidden` |
| API Key | `api_key` | `service` | `key` | `hidden` |
| Text | `text` | none | `body` | `hidden` |

* A field without a value is left out; empty strings are never stored.
* `word_count` is the string `"12"` or `"24"`, counted from the words. Every Seed Phrase has
  `words` in `sealed`.
* `words` is stored normalised: lowercase, separated by single spaces. The `passphrase` (the
  optional "25th word") is stored exactly as typed.
* `chain` is free text; the app's Private Key checks know `evm`, `solana`, `bitcoin` and
  `other`.

### What the app rejects

Unknown keys (at the top level or in a Note); a `schema_version` other than 1; an unknown
`kind`; a Note id that isn't 32 lowercase hex characters, or one used twice; a blank title;
a visible or Hidden Field name that doesn't belong to the Note's Kind; an empty value; a
`hidden` entry on a Seed Phrase or Private Key, or a `sealed` entry on another Kind; a `sealed`
value that isn't non-empty lowercase hex; a Seed Phrase without `words` or with a `word_count`
other than `"12"` or `"24"`.

### Example

A document with one Text Note and one Login (fake values), pretty-printed here; the file holds
it on one line:

```json
{
  "schema_version": 1,
  "changed_at": 1790000000,
  "change_counter": 2,
  "notes": [
    {
      "id": "51a997f97cae5f2aec4f7c3a77fa362b",
      "kind": "text",
      "title": "Test shopping list",
      "visible": {},
      "hidden": { "body": "fake-text-body-line-1\nfake-text-body-line-2" },
      "sealed": {},
      "favorite": false,
      "created_at": 1790000000,
      "updated_at": 1790000000,
      "deleted_at": null
    },
    {
      "id": "fe17e2f08b9821b758c386d78c9f3d25",
      "kind": "login",
      "title": "Test example login",
      "visible": { "username": "test-user@example.test", "website": "https://login.example.test" },
      "hidden": { "password": "fake-login-password-123" },
      "sealed": {},
      "favorite": false,
      "created_at": 1790000000,
      "updated_at": 1790000000,
      "deleted_at": null
    }
  ]
}
```

A Private Key Note looks the same, except that its key is under `sealed`, for example
`"sealed": {"key": "f22718a091b6f1d2…"}` (shortened here; a 64-character key gives 208 hex
characters).

## 9. Recovery Key encoding

The Recovery Key is **16 random bytes** (128 bits). The owner sees it once, at setup, on their
**Recovery Kit**, as 28 symbols of Crockford Base32 in 7 groups of 4:

```text
XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX
```

* Alphabet (a symbol's value is its position, 0 to 31): `0123456789ABCDEFGHJKMNPQRSTVWXYZ`
  (no I, L, O or U).
* **Symbols 1 to 26** carry the 16 bytes: write the bytes as 128 bits, most significant bit of
  the first byte first, append two `0` bits (130 bits), and cut into 26 groups of 5 bits, most
  significant first. The 26th symbol therefore carries the last 3 bits in its top bits, and its
  two low bits are always 0.
* **Symbols 27 and 28** are a checksum: take `d = SHA-256(the 16 bytes)` and its first 10 bits,
  `c = (d[0] << 2) | (d[1] >> 6)`. Symbol 27 is `c >> 5`, symbol 28 is `c & 31`.
* It is printed in uppercase, with `-` between the groups.

Reading what the owner typed:

1. Drop every `-` and every whitespace character.
2. Uppercase. Read `O` as `0`, and `I` or `L` as `1`. Any other character that isn't in the
   alphabet (including `U`) is an error.
3. There must be exactly 28 symbols.
4. The two low bits of symbol 26 must be 0.
5. Rebuild the 16 bytes from symbols 1 to 26, and check symbols 27 and 28 against the
   checksum. A mismatch almost always means a typo.

The Recovery Key is deliberately not made of BIP39 words, so it can't be mistaken for a Seed
Phrase. Test vectors are in section 12.

## 10. Decrypting by hand, step by step

Do this on a computer you trust, ideally offline. Don't save the decrypted document or any
key to a file.

1. **Read the whole file.** Check the magic (bytes 0 to 7), `format_version` = 1 (bytes 8 to
   9), `flags` = 0, `kdf_id` = 1 (byte 28), `slot_count` = 2 (byte 41), slot types 1 (byte 42)
   and 2 (byte 163), and the length rule from section 3.
2. **Read the Argon2id parameters**: `m` = u32 at 29, `t` = u32 at 33, `p` = u32 at 37. Refuse
   them if they are outside the limits of section 4.
3. **Choose the slot.** With the Master Password use slot 1 (offset 42); with the Recovery Key
   use slot 2 (offset 163). In that slot, `salt` is its bytes 1 to 16, `nonce` its bytes 17 to
   40, and `wrapped` its bytes 41 to 120.
4. **Make the Argon2id input.** Master Password: NFC-normalise it and encode as UTF-8.
   Recovery Key: decode it to 16 bytes as in section 9.
5. **Derive the KEK**: Argon2id version 0x13 with that input, the slot's salt, `m` KiB of
   memory, `t` passes, `p` lanes, and a 32-byte output. With the app's parameters this takes
   around a second and between 64 MiB and 512 MiB of memory (2 GiB at most).
6. **Open the slot**: build the 40-byte slot AAD (section 5) and XChaCha20-Poly1305-decrypt
   `wrapped` with the KEK and the slot's nonce. If authentication fails, the credential is
   wrong. Otherwise the 64 bytes are the **Vault Key** (first 32) and the **Wallet Key** (last
   32).
7. **Open the body**: XChaCha20-Poly1305-decrypt file bytes 308 to the end with the Vault Key,
   the nonce at bytes 284 to 307, and file bytes 0 to 307 as the associated data.
8. **Unpad**: `len` = u32 at the start of the result; the document is the next `len` bytes.
   Check the padding as in section 6.
9. **Read the JSON** (section 8). Titles, visible fields and the `hidden` values of Login, API
   Key and Text Notes are now in plain text.
10. **Open the Wallet Kind fields**: for each Note's `sealed` entry, hex-decode it; the first 24
    bytes are the nonce and the rest is the ciphertext and tag. Decrypt with the Wallet Key and
    the wallet AAD for that Note id and field name (section 7). The result is the value as
    UTF-8.

## 11. Reference decryptor (Python)

A complete, independent implementation of the steps above (about 130 lines). It needs Python 3
with `argon2-cffi` and `PyNaCl`. It asks for the credential without echo, prints the document
as JSON with the Wallet Kind fields opened (added under `hidden`), and writes nothing to disk.
Read it before running it, and don't redirect its output into a file.

```text
python3 enote_decrypt.py path/to/vault-or-backup [--recovery-key]
```

```python
#!/usr/bin/env python3
"""Independent decryptor for encrypted-note Vault files (format version 1), written from FORMAT.md.

Needs: argon2-cffi, PyNaCl.   Usage: enote_decrypt.py FILE [--recovery-key]
Prints the decrypted body JSON with Wallet Kind fields opened. Writes nothing to disk.
"""
import getpass
import hashlib
import json
import struct
import sys
import unicodedata

from argon2.low_level import Type, hash_secret_raw
from nacl.bindings import crypto_aead_xchacha20poly1305_ietf_decrypt
from nacl.exceptions import CryptoError

MAGIC = b"\x89ENOTE\r\n"
HEADER_LEN = 308
SLOT_LEN = 121
TAG_LEN = 16
BLOCK = 4096
CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def xchacha_open(ciphertext_and_tag, aad, nonce, key, failure):
    try:
        return crypto_aead_xchacha20poly1305_ietf_decrypt(ciphertext_and_tag, aad, nonce, key)
    except CryptoError:
        sys.exit(failure)


def parse_header(data):
    if data[0:8] != MAGIC:
        sys.exit("not a Vault file")
    version, flags = struct.unpack_from("<HH", data, 8)
    if version != 1:
        sys.exit(f"unsupported format version {version}")
    if len(data) < HEADER_LEN + BLOCK + TAG_LEN or flags != 0:
        sys.exit("malformed header")
    vault_id = data[12:28]
    kdf_id = data[28]
    m, t, p = struct.unpack_from("<III", data, 29)
    slot_count = data[41]
    if kdf_id != 1 or slot_count != 2:
        sys.exit("malformed header")
    slots = {}
    for i in range(2):
        at = 42 + i * SLOT_LEN
        slot_type = data[at]
        if slot_type != i + 1:
            sys.exit("malformed header")
        slots[slot_type] = (data[at + 1:at + 17], data[at + 17:at + 41], data[at + 41:at + 121])
    body_nonce = data[284:308]
    body = data[HEADER_LEN:]
    if (len(body) - TAG_LEN) % BLOCK != 0:
        sys.exit("malformed body length")
    if not (65536 <= m <= 2097152 and 3 <= t <= 20 and 1 <= p <= 16 and m >= 8 * p):
        sys.exit("KDF parameters outside the allowed range")
    return vault_id, (m, t, p), slots, body_nonce, body


def slot_aad(vault_id, kdf, slot_type):
    m, t, p = kdf
    return MAGIC + struct.pack("<H", 1) + vault_id + bytes([1]) + struct.pack("<III", m, t, p) + bytes([slot_type])


def kek(secret, salt, kdf):
    m, t, p = kdf
    return hash_secret_raw(secret, salt, time_cost=t, memory_cost=m, parallelism=p,
                           hash_len=32, type=Type.ID, version=19)


def recovery_key_bytes(typed):
    values = []
    for c in typed:
        if c == "-" or c.isspace():
            continue
        c = c.upper()
        c = {"O": "0", "I": "1", "L": "1"}.get(c, c)
        if c not in CROCKFORD:
            sys.exit("not a Recovery Key symbol: " + c)
        values.append(CROCKFORD.index(c))
    if len(values) != 28 or values[25] & 0b11:
        sys.exit("not a Recovery Key")
    n = 0
    for v in values[:26]:
        n = (n << 5) | v
    key = (n >> 2).to_bytes(16, "big")
    digest = hashlib.sha256(key).digest()
    top10 = (digest[0] << 2) | (digest[1] >> 6)
    if values[26:] != [top10 >> 5, top10 & 0x1F]:
        sys.exit("Recovery Key checksum mismatch (typo?)")
    return key


def wallet_aad(vault_id, note_id, field):
    n, f = note_id.encode(), field.encode()
    return b"enote-wallet-v1" + vault_id + struct.pack("<I", len(n)) + n + struct.pack("<I", len(f)) + f


def main():
    path, use_recovery = sys.argv[1], "--recovery-key" in sys.argv[2:]
    data = open(path, "rb").read()
    vault_id, kdf, slots, body_nonce, body = parse_header(data)

    if use_recovery:
        slot_type, secret = 2, recovery_key_bytes(getpass.getpass("Recovery Key: "))
    else:
        slot_type = 1
        secret = unicodedata.normalize("NFC", getpass.getpass("Master Password: ")).encode("utf-8")
    salt, nonce, wrapped = slots[slot_type]
    keys = xchacha_open(wrapped, slot_aad(vault_id, kdf, slot_type), nonce, kek(secret, salt, kdf),
                        "That credential didn't unlock this Vault.")
    vault_key, wallet_key = keys[:32], keys[32:]

    padded = xchacha_open(body, data[:HEADER_LEN], body_nonce, vault_key, "The body is damaged.")
    length = struct.unpack_from("<I", padded, 0)[0]
    if len(padded) != -(-(length + 4) // BLOCK) * BLOCK or any(padded[4 + length:]):
        sys.exit("The body padding is damaged.")
    document = json.loads(padded[4:4 + length].decode("utf-8"))

    for note in document["notes"]:
        for field, hex_value in note.get("sealed", {}).items():
            sealed = bytes.fromhex(hex_value)
            aad = wallet_aad(vault_id, note["id"], field)
            plain = xchacha_open(sealed[24:], aad, sealed[:24], wallet_key, "A wallet field is damaged.")
            note.setdefault("hidden", {})[field] = plain.decode("utf-8")
    print(json.dumps(document, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
```

## 12. Test vectors

**Primitives.** vault-format's tests check its Argon2id against RFC 9106 section 5.3 and its
XChaCha20-Poly1305 against draft-irtf-cfrg-xchacha-03 appendix A.3.1.

**Recovery Key encoding:**

| 16 bytes (hex) | Recovery Key |
|---|---|
| `00000000000000000000000000000000` | `0000-0000-0000-0000-0000-0000-006X` |
| `606162636465666768696a6b6c6d6e6f` | `C1GP-4RV4-CNK6-ET39-D9NP-RVBE-DW5V` |

**A known-answer Vault** (pinned by vault-format's conformance tests). It uses the tiny
test-only parameters m = 8 KiB, t = 1, p = 1, which are **below the floor**, so the app and the
Emergency Reader refuse it; it exists to check an implementation of the layout. Build it from
these inputs (`a..b` means the consecutive byte values from `a` to `b`):

| Input | Value |
|---|---|
| vault_id | `10..1f` |
| Vault Key | `20..3f` |
| Wallet Key | `40..5f` |
| Recovery Key bytes | `60..6f` (so the Recovery Key is `C1GP-4RV4-CNK6-ET39-D9NP-RVBE-DW5V`) |
| Master Password (NFC) | `correct-horse-t` U+00E9 `st-` U+FB01 `-password-1` |
| slot 1 salt, nonce | `80..8f`, `90..a7` |
| slot 2 salt, nonce | `a0..af`, `b0..c7` |
| body_nonce | `c0..d7` |
| document bytes | the ASCII text `fake known-answer vault body` (not a valid Vault document; it only exercises the layout) |

The result is a 4420-byte file whose SHA-256 is
`0fa453ecc8afbf46f0cf8a7ba140a2b09e10cf3c79324149edbc9f41c7ab756c`.

## 13. The Emergency Reader

`enote-reader` is a small terminal program, separate from the app, that does all of the above
for you:

```text
enote-reader <vault-or-backup-file> [--recovery-key] [--reveal] [--credential-stdin]
```

* It asks for the Master Password (or the Recovery Key with `--recovery-key`) without echo.
  `--credential-stdin` reads it from the first line of stdin instead, for scripts.
* It always uses the limits of section 4.
* It prints every Note, Trash included and marked: Kind, title, visible fields. Hidden Fields
  are shown as `••••••` unless you add `--reveal`; with `--reveal`, Wallet Kind fields are
  opened with the Wallet Key from the same credential.
* It writes nothing to disk, and warns on stderr not to redirect its output to a file.
* Exit codes: 0 ok, 1 wrong Master Password or Recovery Key, 2 damaged or unsupported file,
  3 usage or I/O error. If one Wallet Kind field fails to decrypt, it prints everything else,
  marks that field as damaged, and exits with 2.
