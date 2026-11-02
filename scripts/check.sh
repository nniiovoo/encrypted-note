#!/usr/bin/env bash
# One command that must pass before any slice is done:
#   Source hygiene + helper typecheck and companion tests (Mac) + Rust formatting/tests/clippy +
#   cargo-deny + production WebView build.
set -euo pipefail
cd "$(dirname "$0")/.."

# Stop now, not after the tests, if something this script needs is missing.
need() { command -v "$1" >/dev/null 2>&1 || { echo "$1 not found: $2" >&2; exit 1; }; }
need node "install Node.js 24 or newer"
need npm "install Node.js 24 or newer"
need cargo "install rustup, which reads the version from rust-toolchain.toml"
need cargo-deny "run 'cargo install --locked cargo-deny --version 0.20.2'"
[[ -d node_modules ]] || { echo "node_modules not found: run 'npm ci --ignore-scripts'" >&2; exit 1; }
if [[ $(uname -s) == Darwin ]]; then
  xcrun --find swiftc >/dev/null 2>&1 || { echo "swiftc not found: run 'xcode-select --install'" >&2; exit 1; }
  [[ -d companion/node_modules ]] || { echo "companion/node_modules not found: run 'npm --prefix companion ci --ignore-scripts'" >&2; exit 1; }
fi

echo "==> networking source hygiene"
node scripts/check-network-source.mjs

# build.rs compiles only the client; service.swift is otherwise built by package-companion.sh alone.
if [[ $(uname -s) == Darwin ]]; then
  echo "==> companion helper typecheck"
  xcrun swiftc -parse-as-library -typecheck -target "$(uname -m)-apple-macos13.0" \
    src-tauri/native/companion/protocol.swift src-tauri/native/companion/service.swift

  # The helper exists only on Mac, and these tests rely on its accounts and file permissions. The
  # script runs one test file at a time: they share port 8789, so don't run them alongside
  # scripts/verify-companion.sh either.
  echo "==> companion tests"
  npm --prefix companion run --silent check
fi

echo "==> cargo fmt"
cargo fmt --all -- --check

echo "==> cargo test"
cargo test --locked --workspace --quiet

echo "==> cargo clippy"
cargo clippy --locked --workspace --all-targets --quiet -- -D warnings

echo "==> cargo deny"
cargo deny --locked --workspace check advisories bans licenses sources

echo "==> vendored serde_json advisories"
# cargo-deny skips the vendored copy, which has no registry source, so look in its database.
# After reviewing an advisory, narrow this rule; don't remove it.
for advisory in "${CARGO_HOME:-$HOME/.cargo}"/advisory-dbs/*/crates/serde_json; do
  if [[ -e $advisory ]]; then
    echo "A serde_json advisory exists ($advisory): check it against vendor/serde_json/VENDORED.md." >&2
    exit 1
  fi
done

echo "==> production WebView build"
npm run build

echo "==> dev fixture stays out of the build"
if [[ -e dist/fixture.html ]] || grep -rqF "Fake login" dist; then
  echo "The fictional dev fixture reached dist/: it must never ship." >&2
  exit 1
fi

echo "All checks passed."
