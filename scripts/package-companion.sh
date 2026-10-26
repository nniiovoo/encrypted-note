#!/usr/bin/env bash
# Builds the optional Agent Companion helper (ADR-0009) as a signed XPC service bundle, and
# optionally an encrypted-note.app that contains it. macOS only.
#
#   scripts/package-companion.sh [--identifier ID] [--companion-source DIR] [--cache DIR]
#                                [--out DIR] [--app]
#
# Everything comes from src-tauri/companion/companion.lock.json and is verified: the companion
# commit, its policy.json hash, and the Node.js release archive's SHA-256. This script fetches
# build inputs (git, npm, nodejs.org); the app it produces still has no network code in its
# Vault core or WebView. Run it on a trusted machine; it never reads a Vault or a credential.
#
#   --identifier ID       App identifier (default: tauri.conf.json's). Use a test identifier
#                         (e.g. com.encryptednote.companiontest) so a test build gets its own
#                         sandbox containers and never touches the owner's real Vault.
#   --companion-source D  A local clone of the companion repository at the pinned commit (clean),
#                         instead of fetching it.
#   --cache DIR           Where the verified Node.js archive is kept (default target/companion-cache).
#   --out DIR             Output folder (default target/companion).
#   --app                 Also run `tauri build` with the helper bundled and verify the result.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)

[[ $(uname -s) == Darwin ]] || { echo "The Agent Companion helper is built on macOS only." >&2; exit 1; }

IDENTIFIER=""
SOURCE=""
CACHE="$ROOT/target/companion-cache"
OUT="$ROOT/target/companion"
BUILD_APP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --identifier) IDENTIFIER="$2"; shift 2 ;;
    --companion-source) SOURCE="$2"; shift 2 ;;
    --cache) CACHE="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --app) BUILD_APP=1; shift ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

LOCK="$ROOT/src-tauri/companion/companion.lock.json"
lock() { node -e 'const v = require(process.argv[1]); const r = process.argv[2].split(".").reduce((o, k) => o?.[k], v); if (typeof r !== "string") process.exit(1); process.stdout.write(r);' "$LOCK" "$1"; }
conf() { node -e 'const v = require(process.argv[1]); process.stdout.write(String(process.argv[2].split(".").reduce((o, k) => o?.[k], v)));' "$ROOT/src-tauri/tauri.conf.json" "$1"; }

REPO=$(lock companion.repository)
COMMIT=$(lock companion.commit)
POLICY_SHA=$(lock companion.policySha256)
NODE_VERSION=$(lock node.version)
case "$(uname -m)" in
  arm64) ARCH=aarch64; SWIFT_ARCH=arm64 ;;
  x86_64) ARCH=x86_64; SWIFT_ARCH=x86_64 ;;
  *) echo "Unsupported architecture." >&2; exit 1 ;;
esac
NODE_FILE=$(lock "node.$ARCH.file")
NODE_SHA=$(lock "node.$ARCH.sha256")
[[ -n "$IDENTIFIER" ]] || IDENTIFIER=$(conf identifier)
[[ "$IDENTIFIER" =~ ^[A-Za-z0-9.-]+$ ]] || { echo "Invalid identifier." >&2; exit 1; }
SIGN_IDENTITY=$(conf bundle.macOS.signingIdentity)
VERSION=$(conf version)
SERVICE="$IDENTIFIER.companion"

WORK="$OUT/work"
/bin/rm -rf "$WORK" "$OUT/$SERVICE.xpc"
mkdir -p "$WORK" "$CACHE"

echo "==> companion $COMMIT"
if [[ -n "$SOURCE" ]]; then
  [[ $(git -C "$SOURCE" rev-parse HEAD) == "$COMMIT" ]] || { echo "The companion source is not at the pinned commit." >&2; exit 1; }
  [[ -z $(git -C "$SOURCE" status --porcelain) ]] || { echo "The companion source has uncommitted changes." >&2; exit 1; }
  git clone --quiet --no-local "$SOURCE" "$WORK/companion"
else
  git clone --quiet "$REPO" "$WORK/companion"
fi
git -C "$WORK/companion" -c advice.detachedHead=false checkout --quiet "$COMMIT"
[[ $(git -C "$WORK/companion" rev-parse HEAD) == "$COMMIT" ]] || { echo "Pinned commit not found." >&2; exit 1; }
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
  npm run --silent tauri -- build --bundles app --config "$OVERLAY"
  APP=$(find "${CARGO_TARGET_DIR:-$ROOT/target}/release/bundle/macos" -maxdepth 1 -name '*.app' -print -quit)
  [[ -n "$APP" ]] || { echo "No app bundle was produced." >&2; exit 1; }
  BUNDLED="$APP/Contents/XPCServices/$SERVICE.xpc"
  [[ -d "$BUNDLED" ]] || { echo "The helper is missing from the app." >&2; exit 1; }
  codesign --verify --deep --strict "$APP"
  helper_ents=$(codesign -d --entitlements - --xml "$BUNDLED" 2>/dev/null)
  grep -q network.server <<<"$helper_ents" || { echo "The bundled helper lost its own entitlements." >&2; exit 1; }
  app_ents=$(codesign -d --entitlements - --xml "$APP" 2>/dev/null)
  if grep -q network.server <<<"$app_ents"; then echo "The Vault app gained the helper's server entitlement." >&2; exit 1; fi
  echo "App: $APP"
fi
