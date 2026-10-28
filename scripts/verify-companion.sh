#!/usr/bin/env bash
# Fictional-data verification of a packaged Agent Companion helper (ADR-0009). macOS only.
#
#   scripts/verify-companion.sh HELPER.xpc [WORK_DIR]
#
# Builds a stand-in for the Vault app (the real src-tauri/Entitlements.plist, the same companion
# client code, the helper's test identifier) and drives the real signed helper through:
#   * the real pinned broker: refusal paths inside the actual sandbox;
#   * an isolation probe: the helper cannot read or list the stand-in's container or a FAKE
#     Vault canary in it, and can listen on loopback;
#   * fictional supervision payloads: stop, crash, expiry, ignored stop, protocol garbage with
#     fake key/token markers, oversized output, and both orphan directions.
# Use a test identifier (e.g. com.encryptednote.companiontest) so nothing touches a real Vault.
# Payload-swapped helpers are re-signed copies with the SAME helper binary and entitlements.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
HELPER=${1:?"usage: scripts/verify-companion.sh HELPER.xpc [WORK_DIR]"}
WORK=${2:-$ROOT/target/companion-verify}
BUILT=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$HELPER/Contents/Info.plist")
[[ "${BUILT%.companion}" != com.encryptednote.app ]] || { echo "Use a test identifier; never verify against the real app's container." >&2; exit 1; }
# A fresh identifier per run: new, empty, fictional containers every time. (Old test containers
# stay in ~/Library/Containers; remove them by hand if wanted.)
APP_ID="${BUILT%.companion}.v$(date +%s)"
SERVICE="$APP_ID.companion"
SIGN=$(node -e 'process.stdout.write(require(process.argv[1]).bundle.macOS.signingIdentity)' "$ROOT/src-tauri/tauri.conf.json")
NATIVE=src-tauri/native/companion
VERIFY=src-tauri/companion/verification
/bin/rm -rf "$WORK"
mkdir -p "$WORK/build"
FAILED=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1"; FAILED=1; }

# ---- the stand-in Vault app
cp "$VERIFY/Harness.swift" "$WORK/build/main.swift"
xcrun swiftc -O -target "$(uname -m | sed 's/x86_64/x86_64/;s/arm64/arm64/')-apple-macos13.0" -module-name Harness \
  -o "$WORK/build/Harness" "$WORK/build/main.swift" "$NATIVE/client.swift" "$NATIVE/protocol.swift"
APP="$WORK/Harness.app"
assemble() { # $1 = payload kind: real | probe | fake:MODE
  /bin/rm -rf "$APP"
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/XPCServices"
  cp "$WORK/build/Harness" "$APP/Contents/MacOS/Harness"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string $APP_ID" -c 'Add :CFBundleExecutable string Harness' \
    -c 'Add :CFBundlePackageType string APPL' -c 'Add :CFBundleName string Harness' \
    -c 'Add :LSMinimumSystemVersion string 13.0' -c 'Add :CFBundleVersion string 1' "$APP/Contents/Info.plist" >/dev/null
  local xpc="$APP/Contents/XPCServices/$SERVICE.xpc"
  cp -R "$HELPER" "$xpc"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $SERVICE" "$xpc/Contents/Info.plist"
  local entry="$xpc/Contents/Resources/companion/isolation/integrated-broker.mjs"
  case "$1" in
    real) ;;
    probe) sed "s/APP_ID/$APP_ID/g" "$VERIFY/probe-broker.mjs" > "$entry" ;;
    fake:*) sed "s/MODE_VALUE/${1#fake:}/" "$VERIFY/fake-broker.mjs" > "$entry" ;;
  esac
  codesign --force --sign "$SIGN" --options runtime --entitlements src-tauri/companion/helper.entitlements "$xpc" 2>/dev/null
  codesign --force --sign "$SIGN" --options runtime --entitlements src-tauri/Entitlements.plist "$APP" 2>/dev/null
}
harness() { "$APP/Contents/MacOS/Harness" "$@"; }
field() { sed -n "s/^$1 .*$2=\([^ ]*\).*/\1/p" <<<"$3" | head -1; }
helper_pids() { pgrep -f "$SERVICE.xpc/Contents/MacOS/companion-helper" || true; }
node_pids() { pgrep -f "$SERVICE.xpc/Contents/MacOS/node" || true; }
settle() { for _ in $(seq 1 20); do [[ -z "$(helper_pids)$(node_pids)" ]] && return 0; sleep 0.5; done; return 1; }
wait_for_broker() { for _ in $(seq 1 60); do [[ -n "$(node_pids)" ]] && return 0; sleep 0.5; done; return 1; }
expect() { # name, step, key, value, output
  local got; got=$(field "$2" "$3" "$5")
  [[ "$got" == "$4" ]] && pass "$1" || fail "$1 (expected $2 $3=$4, got ${got:-nothing})"
}

# ---- 1. real broker, nothing installed: runtime ready, plan missing
assemble real
out=$(harness status start)
expect "real: runtime verified, nothing else ready" status flags 1 "$out"
expect "real: start without a plan is refused (plan_missing)" start error 1 "$out"
settle && pass "real: no broker left running" || fail "real: processes left running"

# ---- 2. real broker with a fictional plan and FAKE key, no agent account on this Mac
assemble real
out=$(harness install-fixtures status start)
expect "real: owner files installed into the helper's own container" install rc 0 "$out"
expect "real: plan and credential ready; handoff/agent/code not" status flags 7 "$out"
expect "real: start refused until the agent account is verified (agent_unverified)" start error 5 "$out"
grep -q FAKE_OPENAI_AGENT_KEY_FOR_VERIFY_ONLY <<<"$out" && fail "real: key marker in output" || pass "real: no key marker in output"
settle && pass "real: no broker left running" || fail "real: processes left running"

# ---- 3. isolation probe against a FAKE Vault canary in the stand-in's container
assemble probe
out=$(harness canary start sleep:1 stop)
expect "isolation: helper cannot read/list the Vault container; loopback allowed" start state 3 "$out"
expect "isolation: stopped" stop state 1 "$out"
settle && pass "isolation: no process left" || fail "isolation: processes left running"

# ---- 4-9. supervision with fictional payloads
assemble fake:normal
out=$(harness start status stop status)
expect "supervision: running with a grant expiry" start state 3 "$out"
expect "supervision: grant expiry reported" start expires set "$out"
expect "supervision: owner stop" stop state 1 "$out"
settle && pass "supervision: nothing left after stop" || fail "supervision: processes left after stop"

assemble fake:crash
out=$(harness start sleep:3 status sleep:2)
expect "crash: reported as unexpected_exit" status error 24 "$out"
[[ -z "$(node_pids)" ]] && pass "crash: never restarted automatically" || fail "crash: a broker was restarted"
settle || true

assemble fake:expire
out=$(harness start sleep:4 status)
expect "expiry: reported as expired" status state 5 "$out"
settle && pass "expiry: nothing left" || fail "expiry: processes left"

assemble fake:ignore-stop
out=$(harness start stop)
expect "unresponsive broker: stop escalates to a forced stop" stop state 1 "$out"
# Stop request, SIGTERM after 5 s, SIGKILL after 2 more: about 7 s, never more than 12.
stop_ms=$(field stop ms "$out")
(( ${stop_ms:-99999} <= 12000 )) && pass "unresponsive broker: stopped within 12 s (${stop_ms} ms)" || fail "unresponsive broker: stop took too long (${stop_ms:-?} ms)"
settle && pass "unresponsive broker: nothing left" || fail "unresponsive broker: processes left"

since=$(date '+%Y-%m-%d %H:%M:%S')
assemble fake:markers
out=$(harness start)
expect "garbage output: protocol_invalid" start error 25 "$out"
grep -Eq 'FAKE_|sk-FAKE|Bearer' <<<"$out" && fail "garbage output: marker reached the app" || pass "garbage output: no marker reached the app"
settle || true
logs=$(log show --style compact --start "$since" --predicate "process == \"companion-helper\" OR process == \"Harness\"" 2>/dev/null || true)
grep -Eq 'FAKE_OPENAI_AGENT_KEY_MARKER|FAKE_TOKEN_MARKER|FAKE_PROVIDER_ERROR_MARKER|sk-FAKE' <<<"$logs" && fail "garbage output: marker in the system log" || pass "garbage output: no marker in the system log"

assemble fake:oversized
out=$(harness start)
expect "oversized output: protocol_invalid" start error 25 "$out"
settle || true

# ---- 10-11. orphans
assemble fake:normal
"$APP/Contents/MacOS/Harness" start pid sleep:60 > "$WORK/hold.log" 2>&1 &
holder=$!
wait_for_broker && pass "orphans: broker running before the app is killed" || fail "orphans: broker did not start"
kill -9 "$holder"; wait "$holder" 2>/dev/null || true
settle && pass "orphans: app killed -> helper and broker gone" || fail "orphans: left running after the app was killed"

"$APP/Contents/MacOS/Harness" start sleep:60 > "$WORK/hold2.log" 2>&1 &
holder=$!
wait_for_broker && pass "orphans: broker running before the helper is killed" || fail "orphans: broker did not start before the helper was killed"
helper=$(helper_pids)
[[ -n "$helper" ]] && kill -9 $helper
for _ in $(seq 1 20); do [[ -z "$(node_pids)" ]] && break; sleep 0.5; done
[[ -z "$(node_pids)" ]] && pass "orphans: helper killed -> broker gone" || fail "orphans: broker outlived its helper"
kill "$holder" 2>/dev/null; wait "$holder" 2>/dev/null || true
settle || true

[[ $FAILED == 0 ]] && echo "All companion verification checks passed." || { echo "Companion verification FAILED." >&2; exit 1; }
