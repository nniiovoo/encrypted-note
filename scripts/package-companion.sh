#!/usr/bin/env bash
# Builds the optional Agent Companion helper (ADR-0009) as a signed XPC service bundle, and
# optionally an encrypted-note.app that contains it. macOS only.
#
#   scripts/package-companion.sh [--identifier ID] [--cache DIR] [--out DIR] [--app [--owner-build]]
#
# The companion comes from this commit's companion/ folder (ADR-0010). Its policy.json and the
# Node.js release archive are verified against src-tauri/companion/companion.lock.json. This
# script fetches build inputs (npm, nodejs.org); the app it produces still has no network code in
# its Vault core or WebView. Run it on a trusted machine; it never reads a Vault or a credential.
#
#   --identifier ID  App identifier (default: tauri.conf.json's). Use a test identifier
#                    (e.g. com.encryptednote.companiontest) so a test build gets its own
#                    sandbox containers and never touches the owner's real Vault.
#   --cache DIR      Where the verified Node.js archive is kept (default target/companion-cache).
#   --out DIR        Output folder (default target/companion).
#   --app            Also run `tauri build` with the helper bundled and verify the result. A release
#                    app with the real identifier opens the owner's real Vault folder, so --app
#                    refuses it (and any identifier under it) unless --owner-build is also given.
#   --owner-build    The owner only, building the app they install: allows --app with the real
#                    identifier, after typing that identifier in Terminal. Agents never pass it.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)

[[ $(uname -s) == Darwin ]] || { echo "The Agent Companion helper is built on macOS only." >&2; exit 1; }

IDENTIFIER=""
CACHE="$ROOT/target/companion-cache"
OUT="$ROOT/target/companion"
BUILD_APP=0
OWNER_BUILD=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --identifier) IDENTIFIER="$2"; shift 2 ;;
    --cache) CACHE="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --app) BUILD_APP=1; shift ;;
    --owner-build) OWNER_BUILD=1; shift ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "$1 not found: $2" >&2; exit 1; }; }
need node "install Node.js 24 or newer"
need npm "install Node.js 24 or newer"
need git "run 'xcode-select --install'"
need xcrun "run 'xcode-select --install'"
[[ $BUILD_APP == 0 || -d node_modules ]] || { echo "node_modules not found: run 'npm ci --ignore-scripts'" >&2; exit 1; }

LOCK="$ROOT/src-tauri/companion/companion.lock.json"
lock() { node -e 'const v = require(process.argv[1]); const r = process.argv[2].split(".").reduce((o, k) => o?.[k], v); if (typeof r !== "string") process.exit(1); process.stdout.write(r);' "$LOCK" "$1"; }
conf() { node -e 'const v = require(process.argv[1]); process.stdout.write(String(process.argv[2].split(".").reduce((o, k) => o?.[k], v)));' "$ROOT/src-tauri/tauri.conf.json" "$1"; }

REAL_IDENTIFIER=$(conf identifier)
[[ -n "$IDENTIFIER" ]] || IDENTIFIER=$REAL_IDENTIFIER
[[ "$IDENTIFIER" =~ ^[A-Za-z0-9.-]+$ ]] || { echo "Invalid identifier." >&2; exit 1; }
# Containers and app data folders are named by identifier, and a default Mac disk ignores case.
id_lc=$(tr '[:upper:]' '[:lower:]' <<<"$IDENTIFIER")
real_lc=$(tr '[:upper:]' '[:lower:]' <<<"$REAL_IDENTIFIER")
TEST_HINT="For a test build, pass --identifier com.encryptednote.companiontest."
[[ $OWNER_BUILD == 0 || $BUILD_APP == 1 ]] || { echo "--owner-build only applies with --app." >&2; exit 2; }
if [[ $BUILD_APP == 1 && $id_lc == "$real_lc".* ]]; then
  echo "$IDENTIFIER would share a folder with the real app or its helper. $TEST_HINT" >&2; exit 1
fi
if [[ $BUILD_APP == 1 && $id_lc == "$real_lc" ]]; then
  [[ $OWNER_BUILD == 1 ]] || { echo "--app with the real identifier ($REAL_IDENTIFIER) builds an app that opens the owner's real Vault folder. $TEST_HINT The owner adds --owner-build to build the app they install." >&2; exit 1; }
  [[ -t 0 && -t 2 ]] || { echo "--owner-build needs the owner at a Terminal prompt. $TEST_HINT" >&2; exit 1; }
  read -r -p "This builds the app that opens your real Vault folder. Type $REAL_IDENTIFIER to continue: " confirm
  [[ $confirm == "$REAL_IDENTIFIER" ]] || { echo "Not confirmed. Nothing was built." >&2; exit 1; }
elif [[ $OWNER_BUILD == 1 ]]; then
  echo "--owner-build is only for the real identifier ($REAL_IDENTIFIER)." >&2; exit 2
fi

COMMIT=$(git rev-parse HEAD)
POLICY_SHA=$(lock companion.policySha256)
NODE_VERSION=$(lock node.version)
case "$(uname -m)" in
  arm64) ARCH=aarch64; SWIFT_ARCH=arm64 ;;
  x86_64) ARCH=x86_64; SWIFT_ARCH=x86_64 ;;
  *) echo "Unsupported architecture." >&2; exit 1 ;;
esac
NODE_FILE=$(lock "node.$ARCH.file")
NODE_SHA=$(lock "node.$ARCH.sha256")
SIGN_IDENTITY=$(conf bundle.macOS.signingIdentity)
VERSION=$(conf version)
SERVICE="$IDENTIFIER.companion"

WORK="$OUT/work"
/bin/rm -rf "$WORK" "$OUT/$SERVICE.xpc"
mkdir -p "$WORK" "$CACHE"

echo "==> companion (from $COMMIT)"
# Only what's committed reaches the helper: no uncommitted edits, node_modules or ._* files.
[[ -z $(git status --porcelain -- companion) ]] || { echo "companion/ has uncommitted changes. Commit them first: the helper ships only committed code." >&2; exit 1; }
git archive "$COMMIT" companion | tar -x -C "$WORK"
[[ $(shasum -a 256 "$WORK/companion/policy.json" | cut -d' ' -f1) == "$POLICY_SHA" ]] || { echo "The companion policy differs from the pinned hash." >&2; exit 1; }
(cd "$WORK/companion" && npm ci --ignore-scripts --omit=dev --no-audit --no-fund --loglevel=error)

echo "==> Node.js $NODE_VERSION ($ARCH)"
if [[ ! -f "$CACHE/$NODE_FILE" ]]; then
  curl -fsSL --proto '=https' --tlsv1.2 "https://nodejs.org/dist/$NODE_VERSION/$NODE_FILE" -o "$CACHE/$NODE_FILE.part"
  mv "$CACHE/$NODE_FILE.part" "$CACHE/$NODE_FILE"
fi
[[ $(shasum -a 256 "$CACHE/$NODE_FILE" | cut -d' ' -f1) == "$NODE_SHA" ]] || { echo "The Node.js archive does not match its pinned SHA-256." >&2; exit 1; }
mkdir -p "$WORK/node"
NODE_DIR=${NODE_FILE%.tar.gz}
tar -xzf "$CACHE/$NODE_FILE" -C "$WORK/node" "$NODE_DIR/bin/node" "$NODE_DIR/LICENSE"
[[ $("$WORK/node/$NODE_DIR/bin/node" --version) == "$NODE_VERSION" ]] || { echo "Unexpected Node.js version." >&2; exit 1; }

echo "==> helper"
xcrun swiftc -parse-as-library -O -target "$SWIFT_ARCH-apple-macos13.0" -o "$WORK/companion-helper" \
  src-tauri/native/companion/protocol.swift src-tauri/native/companion/service.swift

XPC="$OUT/$SERVICE.xpc"
CONTENTS="$XPC/Contents"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources/companion" "$CONTENTS/Resources/licenses"
cp "$WORK/companion-helper" "$CONTENTS/MacOS/companion-helper"
cp "$WORK/node/$NODE_DIR/bin/node" "$CONTENTS/MacOS/node"
cp "$WORK/node/$NODE_DIR/LICENSE" "$CONTENTS/Resources/licenses/node-LICENSE"
cp "$ROOT/LICENSE" "$CONTENTS/Resources/licenses/companion-LICENSE"
# Runtime files only: no tests, fixtures, docs, CI or npm command shims.
for item in src isolation node_modules policy.json package.json package-lock.json; do
  cp -R "$WORK/companion/$item" "$CONTENTS/Resources/companion/$item"
done
/bin/rm -rf "$CONTENTS/Resources/companion/node_modules/.bin" "$CONTENTS/Resources/companion/node_modules/.package-lock.json"
find "$CONTENTS/Resources/companion" -type l -print -quit | grep -q . && { echo "Unexpected symlink in the companion runtime." >&2; exit 1; }
chmod -R u=rwX,go=rX "$XPC"

PLIST="$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy \
  -c "Add :CFBundleIdentifier string $SERVICE" \
  -c "Add :CFBundleExecutable string companion-helper" \
  -c "Add :CFBundlePackageType string XPC!" \
  -c "Add :CFBundleName string Agent Companion" \
  -c "Add :CFBundleShortVersionString string $VERSION" \
  -c "Add :CFBundleVersion string $VERSION" \
  -c "Add :LSMinimumSystemVersion string 13.0" \
  -c "Add :XPCService dict" \
  -c "Add :XPCService:ServiceType string Application" \
  -c "Add :EnoteCompanionPolicySHA256 string $POLICY_SHA" \
  -c "Add :EnoteCompanionCommit string $COMMIT" \
  -c "Add :EnoteCompanionNodeVersion string $NODE_VERSION" \
  "$PLIST" >/dev/null

echo "==> sign ($SIGN_IDENTITY)"
codesign --force --sign "$SIGN_IDENTITY" --options runtime --entitlements src-tauri/companion/node.entitlements "$CONTENTS/MacOS/node"
codesign --force --sign "$SIGN_IDENTITY" --options runtime --entitlements src-tauri/companion/helper.entitlements "$XPC"
codesign --verify --strict --verbose=1 "$XPC"
node_ents=$(codesign -d --entitlements - --xml "$CONTENTS/MacOS/node" 2>/dev/null)
for banned in get-task-allow allow-dyld-environment-variables disable-library-validation disable-executable-page-protection allow-unsigned-executable-memory; do
  if grep -q "$banned" <<<"$node_ents"; then echo "Node kept the $banned entitlement." >&2; exit 1; fi
done
echo "Helper: $XPC"

if [[ $BUILD_APP == 1 ]]; then
  echo "==> app ($IDENTIFIER)"
  OVERLAY="$OUT/tauri.companion.conf.json"
  node -e '
    const [overlay, identifier, service, xpc] = process.argv.slice(1);
    const config = { identifier, bundle: { macOS: { files: { ["XPCServices/" + service + ".xpc"]: xpc } } } };
    require("node:fs").writeFileSync(overlay, JSON.stringify(config, null, 2));
  ' "$OVERLAY" "$IDENTIFIER" "$SERVICE" "$XPC"
  npm run --silent tauri -- build --bundles app --config "$OVERLAY" -- --locked
  APP=$(find "${CARGO_TARGET_DIR:-$ROOT/target}/release/bundle/macos" -maxdepth 1 -name '*.app' -print -quit)
  [[ -n "$APP" ]] || { echo "No app bundle was produced." >&2; exit 1; }
  BUNDLED="$APP/Contents/XPCServices/$SERVICE.xpc"
  [[ -d "$BUNDLED" ]] || { echo "The helper is missing from the app." >&2; exit 1; }
  codesign --verify --deep --strict "$APP"
  app_signature=$(codesign -dv "$APP" 2>&1)
  grep -q 'flags=.*runtime' <<<"$app_signature" || { echo "The app is not signed with the hardened runtime." >&2; exit 1; }
  helper_ents=$(codesign -d --entitlements - --xml "$BUNDLED" 2>/dev/null)
  grep -q network.server <<<"$helper_ents" || { echo "The bundled helper lost its own entitlements." >&2; exit 1; }
  app_ents=$(codesign -d --entitlements - --xml "$APP" 2>/dev/null)
  if grep -q network.server <<<"$app_ents"; then echo "The Vault app gained the helper's server entitlement." >&2; exit 1; fi
  if grep -q get-task-allow <<<"$app_ents"; then echo "The app kept the get-task-allow entitlement." >&2; exit 1; fi
  echo "App: $APP"
fi
