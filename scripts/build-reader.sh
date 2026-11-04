#!/usr/bin/env bash
# Builds the Emergency Reader for real use from reviewed code: a fresh clone of a commit that is on
# origin/main, built in a new temporary folder with its own target folder. It never reads a Vault,
# a Backup or a credential, and never runs the Reader.
#
#   scripts/build-reader.sh --out DIR [--commit COMMIT] [--offline]
#
#   --out DIR        Folder for the enote-reader binary. Keep it outside this project folder, for
#                    example next to your Backups. An existing binary there is never overwritten.
#   --commit COMMIT  A commit on origin/main (default: origin/main's tip, as of your last
#                    `git fetch origin`).
#   --offline        Build only from crates already in Cargo's cache.
#
# Build in a session with no AI agent running, then write the commit and SHA-256 it prints on
# paper. The clone comes from this checkout's git objects, so only committed code is built.
set -euo pipefail
CALLER=$(pwd)
cd "$(dirname "$0")/.."
ROOT=$(pwd -P)

OUT=""
COMMIT=""
OFFLINE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --commit) COMMIT="$2"; shift 2 ;;
    --offline) OFFLINE=1; shift ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$OUT" ]] || { echo "Choose where the Reader goes: --out DIR" >&2; exit 2; }

need() { command -v "$1" >/dev/null 2>&1 || { echo "$1 not found: $2" >&2; exit 1; }; }
need git "install Git (on Mac, run 'xcode-select --install')"
need cargo "install rustup, which reads the version from rust-toolchain.toml"
need node "install Node.js 24 or newer, as for the app build"
sha256() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1; else sha256sum "$1" | cut -d' ' -f1; fi
}

git rev-parse --verify --quiet refs/remotes/origin/main >/dev/null || { echo "origin/main not found: run 'git fetch origin' first." >&2; exit 1; }
[[ -n "$COMMIT" ]] || COMMIT=origin/main
COMMIT=$(git rev-parse --verify --quiet "$COMMIT^{commit}") || { echo "Not a commit in this checkout: run 'git fetch origin', then check the name." >&2; exit 1; }
git merge-base --is-ancestor "$COMMIT" refs/remotes/origin/main || { echo "$COMMIT is not on origin/main. Build only merged, reviewed code." >&2; exit 1; }

[[ $OUT == /* ]] || OUT="$CALLER/$OUT"
# Refuses a folder that is this project or inside it. -ef compares the actual folders, so case,
# links and .. in the path make no difference.
outside() {
  local dir
  dir=$(cd "$1" && pwd -P)
  while :; do
    [[ ! $dir -ef $ROOT ]] || { echo "Choose an --out folder outside this project, where agents don't work." >&2; exit 1; }
    [[ $dir != / ]] || break
    dir=$(dirname "$dir")
  done
}
# Check the nearest folder that already exists before making any, so a refused path leaves none.
EXISTING=$OUT
while [[ ! -d $EXISTING ]]; do EXISTING=$(dirname "$EXISTING"); done
case "/${OUT#"$EXISTING"}/" in */../*) echo "Choose an --out path without .. after a folder that doesn't exist yet." >&2; exit 1 ;; esac
outside "$EXISTING"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd -P)
outside "$OUT"
[[ ! -e "$OUT/enote-reader" && ! -L "$OUT/enote-reader" && ! -e "$OUT/enote-reader.exe" && ! -L "$OUT/enote-reader.exe" ]] || { echo "$OUT already has an enote-reader. Choose an empty folder, or move the old one yourself." >&2; exit 1; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/enote-reader-build.XXXXXX")
trap '/bin/rm -rf "$WORK"' EXIT

echo "==> source ($COMMIT)"
# A new repository holding only origin/main's history: no working-tree edits, hooks or config.
clone() { git -C "$WORK/src" -c core.hooksPath=/dev/null -c advice.detachedHead=false "$@"; }
git init --quiet "$WORK/src"
clone fetch --quiet --no-tags "$ROOT" refs/remotes/origin/main:refs/remotes/origin/main
clone checkout --quiet --detach "$COMMIT"
[[ $(clone rev-parse HEAD) == "$COMMIT" ]] || { echo "The clone is not at $COMMIT." >&2; exit 1; }
clone merge-base --is-ancestor HEAD refs/remotes/origin/main || { echo "The clone's commit is not on origin/main." >&2; exit 1; }

echo "==> enote-reader (release)"
# Flags, wrappers, compilers and targets from the calling shell must not change what gets compiled.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER \
  RUSTC CARGO_BUILD_RUSTC RUSTUP_TOOLCHAIN CARGO_BUILD_TARGET
for var in $(compgen -e); do
  [[ $var == CARGO_TARGET_DIR || ! $var =~ ^CARGO_(TARGET|PROFILE)_ ]] || unset "$var"
done
(cd "$WORK/src" && CARGO_TARGET_DIR="$WORK/target" cargo build --release --locked ${OFFLINE:+--offline} -p enote-reader)

BIN="$WORK/target/release/enote-reader"
[[ -f "$BIN" ]] || BIN="$BIN.exe"
[[ -f "$BIN" ]] || { echo "No enote-reader binary was produced." >&2; exit 1; }
SAVED="$OUT/$(basename "$BIN")"
# Exclusive creation also refuses special files another build put here while this compiled.
( umask 077; node -e '
  const fs = require("node:fs");
  try { fs.copyFileSync(process.argv[1], process.argv[2], fs.constants.COPYFILE_EXCL); }
  catch { process.exit(1); }
' "$BIN" "$SAVED" ) || { echo "Could not save $SAVED without replacing an existing file. Choose an empty folder." >&2; exit 1; }
chmod 755 "$SAVED"

echo "Reader:  $SAVED"
echo "Commit:  $COMMIT"
echo "SHA-256: $(sha256 "$SAVED")"
echo "Write the commit and SHA-256 on paper. Before using the Reader, check its SHA-256 again."
echo "The SHA-256 is this copy's: another build of the same commit may differ."
