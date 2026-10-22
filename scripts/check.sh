#!/usr/bin/env bash
# One command that must pass before any slice is done:
#   Source hygiene + Rust formatting/tests/clippy + cargo-deny + production WebView build.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> networking source hygiene"
node scripts/check-network-source.mjs

echo "==> cargo fmt"
cargo fmt --all -- --check

echo "==> cargo test"
cargo test --locked --workspace --quiet

echo "==> cargo clippy"
cargo clippy --locked --workspace --all-targets --quiet -- -D warnings

echo "==> cargo deny"
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny --locked --workspace check advisories bans licenses sources
else
  echo "cargo-deny not installed: run 'cargo install --locked cargo-deny'" >&2
  exit 1
fi

echo "==> production WebView build"
npm run build

echo "All checks passed."
