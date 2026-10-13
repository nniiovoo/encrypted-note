# Vendored serde_json

This folder is serde_json **1.0.151** from crates.io with a small local patch. The root
`Cargo.toml` swaps it in for the whole workspace with `[patch.crates-io]`, so every crate that
uses serde_json (Tauri included) runs this copy, and `Cargo.lock` has no registry checksum for it.

Upstream package checksum (sha256 of `serde_json-1.0.151.crate`, the value `Cargo.lock` carried
before vendoring): `c841b55ecdae098c80dcae9cf767f6f8a0c2cdb3416bbef72181df4d0fe73f14`

## Why

The decrypted Vault document is JSON. serde_json's parser copies every string that contains an
escape sequence (`\n`, `\"`, `\u…`) into a temporary "scratch" buffer and frees that buffer
without wiping it, so Hidden Field text would stay in freed memory. Upstream has no option for
this.

## What is changed

Everything else is byte-identical to upstream. All changes are added lines; nothing upstream is
removed or rewritten.

1. `Cargo.toml`: one new dependency block, `zeroize` pinned to `=1.9.0` (the workspace's pin),
   `default-features = false`, `features = ["alloc"]`. `Cargo.toml.orig` is upstream's own file,
   is not read by cargo, and is left alone.
2. `src/de.rs`:
   - `use zeroize::Zeroize;`
   - before each of the nine `scratch.clear()` calls, one line that wipes the bytes in use:
     `self.scratch.as_mut_slice().zeroize();` (twice as `self.de.scratch…`).
   - a new function, `from_slice_zeroizing`, after `from_slice`. It reserves the scratch buffer
     once at the input's length, parses, then wipes the buffer's whole capacity on success and on
     error.
3. `src/lib.rs`: `from_slice_zeroizing` added to the `pub use crate::de::{…}` line.

`notes::Document::from_json` calls `from_slice_zeroizing`. The Solana keypair check in
`crates/notes/src/keys.rs` validates the number list directly, without making parser copies
or errors that might quote key text.

## What it does and does not promise

- After `from_slice_zeroizing` returns, the scratch buffer holds no decoded text. The buffer never
  moves while parsing: decoded text is never longer than its JSON source, so the one reservation
  is always enough and no unwiped copy is freed.
- The per-string wipe covers only the bytes in use, not the whole capacity. An earlier version of
  this patch wiped the whole capacity (the whole document's length) before every string, which
  made parsing quadratic: 3,000 fake Notes (1.1 MB) took 662 s in a debug build, against 0.08 s
  now. `crates/notes/tests/parse_speed.rs` fails if that comes back.
- Wiping only the bytes in use is enough because bytes past the length are either already wiped
  or never written. The one exception: `ignore_value` (skipping an unknown field) uses the buffer
  as a stack of `[` and `{` and pops them without wiping. Those are JSON structure, not Note
  text, and the final whole-capacity wipe clears them too.
- The other entry points (`from_slice`, `from_str`, `from_reader`) get the per-string wipe but no
  reservation and no final wipe, so they make no promise.
- Not covered: the values handed to the caller's own types, serde_json's error value (which can
  quote a string), and a panic inside a `Deserialize` impl (it unwinds past the final wipe).
- The `float_roundtrip` feature is off in this workspace. Its two `clear()` calls are patched the
  same way; that buffer use holds only the digits of a number.

## Moving to a new upstream version

1. Replace this folder with the new release's source, keeping this file.
2. Re-apply the three changes above. To see them exactly, compare with the registry's copy of the
   same version: `diff -ru ~/.cargo/registry/src/*/serde_json-1.0.151 vendor/serde_json`. Check
   that every `scratch.clear()` in `src/de.rs` still has its wipe line, and that nothing new
   shortens the buffer without wiping.
3. Update the version and checksum here and the `serde_json = "=…"` pin in the root `Cargo.toml`.
4. Run `cargo test -p notes --offline`.
