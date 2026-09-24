#!/usr/bin/env bash
# One command that must pass before any slice is done:
#   Rust tests + clippy (deny warnings) + cargo-deny (advisories, network ban, licenses) + TypeScript type-check.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo test"
cargo test --workspace --quiet

echo "==> cargo clippy"
cargo clippy --workspace --all-targets --quiet -- -D warnings

echo "==> cargo deny"
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny --workspace check advisories bans licenses sources
else
  echo "cargo-deny not installed: run 'cargo install --locked cargo-deny'" >&2
  exit 1
fi

echo "==> TypeScript"
npx --no-install tsc --noEmit

echo "All checks passed."
