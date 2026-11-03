#!/usr/bin/env bash
# Prepares this Mac for the Agent Companion (docs/agent-companion.md, "Before you start", step 3).
# Run it in your own account after the agent account exists. Safe to run again: each run resets
# the folder's permissions and writes the plan fresh from the app's example. macOS only.
#
#   scripts/companion-setup.sh [--dry-run] [--app PATH] AGENT_SHORT_NAME
#
#   --dry-run   Check, then print what it would do without changing anything.
#   --app PATH  The app with the helper inside (default /Applications/encrypted-note.app).
#
# It creates the private agent folder with one search entry for the agent account, adds a fake
# Agent Key file if ~/enote-agent-key.env doesn't exist (an existing one is never read or
# replaced), and fills in the plan. It never handles a real key and needs no administrator rights.
set -euo pipefail

[[ $(uname -s) == Darwin ]] || { echo "The Agent Companion is set up on macOS only." >&2; exit 1; }

DRY_RUN=0
APP=/Applications/encrypted-note.app
AGENT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --app) APP="$2"; shift 2 ;;
    -*) echo "Unknown option: $1" >&2; exit 2 ;;
    *) [[ -z "$AGENT" ]] || { echo "Give one agent account name." >&2; exit 2; }; AGENT="$1"; shift ;;
  esac
done
[[ -n "$AGENT" ]] || { echo "Usage: scripts/companion-setup.sh [--dry-run] [--app PATH] AGENT_SHORT_NAME" >&2; exit 2; }

need() { command -v "$1" >/dev/null 2>&1 || { echo "$1 not found: $2" >&2; exit 1; }; }
need node "install Node.js 22 or newer (docs/agent-companion.md)"

FOLDER=/Users/Shared/encrypted-note-companion
PLAN=/Users/Shared/encrypted-note-companion-plan.json
TOKEN="$FOLDER/agent-token"
KEY_FILE="$HOME/enote-agent-key.env"

# The same rule the helper applies to the plan's agentShortName.
[[ "$AGENT" =~ ^[a-z][a-z0-9_-]{0,30}$ ]] || { echo "\"$AGENT\" isn't a Mac account short name: use the Account name from System Settings → Users & Groups." >&2; exit 1; }
AGENT_UID=$(id -u "$AGENT" 2>/dev/null) || { echo "No Mac account named $AGENT. Check its Account name in System Settings → Users & Groups." >&2; exit 1; }
OWNER_UID=$(id -u)
[[ $AGENT_UID != "$OWNER_UID" ]] || { echo "$AGENT is your own account. Run this in your account and name the separate agent account." >&2; exit 1; }
(( AGENT_UID >= 501 && AGENT_UID < 2147483648 )) || { echo "$AGENT is not an ordinary user account." >&2; exit 1; }
if dseditgroup -o checkmember -m "$AGENT" admin >/dev/null 2>&1; then
  echo "$AGENT is an administrator. Make it a Standard account first, in System Settings → Users & Groups." >&2
  exit 1
fi

shopt -s nullglob
examples=("$APP"/Contents/XPCServices/*.companion.xpc/Contents/Resources/companion/isolation/plan.example.json)
shopt -u nullglob
[[ ${#examples[@]} == 1 ]] || { echo "No Agent Companion helper found in $APP. Build the app with scripts/package-companion.sh --app --owner-build (docs/agent-companion.md)." >&2; exit 1; }
EXAMPLE=${examples[0]}

# /Users/Shared is writable by every account, so anything already there must be yours.
mine() {
  [[ ! -L "$1" ]] || { echo "$1 is a symbolic link. Remove it yourself, then run this again." >&2; exit 1; }
  [[ ! -e "$1" || $(stat -f %u "$1") == "$OWNER_UID" ]] || { echo "$1 belongs to another account. Remove it yourself, then run this again." >&2; exit 1; }
}
mine "$FOLDER"
mine "$PLAN"
[[ ! -e "$FOLDER" || -d "$FOLDER" ]] || { echo "$FOLDER is not a folder." >&2; exit 1; }
[[ ! -e "$PLAN" || -f "$PLAN" ]] || { echo "$PLAN is not a file." >&2; exit 1; }

run() {
  if [[ $DRY_RUN == 1 ]]; then printf 'Would run:'; printf ' %q' "$@"; printf '\n'; else "$@"; fi
}

echo "==> private agent folder ($FOLDER)"
[[ -d "$FOLDER" ]] || run mkdir -m 700 "$FOLDER"
# Check again: another account could have put a link there since the first check. Once your own
# folder is there, the sticky /Users/Shared lets no other account replace it. -h never follows a link.
mine "$FOLDER"
[[ $DRY_RUN == 1 || ( -d "$FOLDER" && ! -L "$FOLDER" ) ]] || { echo "$FOLDER is not a folder." >&2; exit 1; }
run chmod -h 700 "$FOLDER"
# Start from no ACL entries, so a run with a mistyped name leaves no second entry behind.
run chmod -h -N "$FOLDER"
run chmod -h +a "user:$AGENT allow search" "$FOLDER"

echo "==> fake Agent Key file ($KEY_FILE)"
FAKE_KEY=0
if [[ -e "$KEY_FILE" || -L "$KEY_FILE" ]]; then
  echo "Kept the existing file: this script never reads or replaces it."
elif [[ $DRY_RUN == 1 ]]; then
  echo "Would write a fake key line to it, readable only by you."
else
  ( umask 077; set -o noclobber; printf 'OPENAI_AGENT_API_KEY=FAKE_KEY_FOR_SETUP_ONLY_0123456789\n' > "$KEY_FILE" )
  FAKE_KEY=1
fi

echo "==> plan ($PLAN)"
fill='
  const fs = require("node:fs");
  const [example, out, ownerUid, agentUid, agentShortName, bearerTokenFile] = process.argv.slice(1);
  const plan = JSON.parse(fs.readFileSync(example, "utf8"));
  if (plan.ownerUid !== null || plan.agentUid !== null || plan.bearerTokenFile !== null || plan.grantExpiresAt !== null) {
    throw new Error("The example plan in the app is not the unfilled one this script expects.");
  }
  Object.assign(plan, { ownerUid: Number(ownerUid), agentUid: Number(agentUid), agentShortName, bearerTokenFile });
  fs.writeFileSync(out, JSON.stringify(plan, null, 2) + "\n");
  const check = JSON.parse(fs.readFileSync(out, "utf8"));
  if (check.ownerUid !== plan.ownerUid || check.agentUid !== plan.agentUid ||
      check.agentShortName !== agentShortName || check.bearerTokenFile !== bearerTokenFile) {
    throw new Error("The written plan does not read back as expected.");
  }
'
if [[ $DRY_RUN == 1 ]]; then
  echo "Would copy $EXAMPLE"
  echo "and fill in ownerUid $OWNER_UID, agentUid $AGENT_UID, agentShortName $AGENT, bearerTokenFile $TOKEN."
  run chmod 644 "$PLAN"
else
  tmp=$(mktemp "$PLAN.XXXXXX")
  trap '/bin/rm -f "$tmp"' EXIT
  node -e "$fill" "$EXAMPLE" "$tmp" "$OWNER_UID" "$AGENT_UID" "$AGENT" "$TOKEN"
  chmod 644 "$tmp"
  mv -f "$tmp" "$PLAN"
fi

if [[ $DRY_RUN == 1 ]]; then
  echo "Dry run: nothing was changed."
else
  echo "Done. Next, in the app: Settings → Agent Companion → Set up…, then choose $PLAN, then $KEY_FILE."
fi
if [[ $FAKE_KEY == 1 ]]; then
  echo "Set up… also replaces the helper's Agent Key with $KEY_FILE, which now holds a fake key."
  echo "If the helper already has your real key, first make a new key file with write-agent-key.mjs (docs/agent-companion.md, \"The real Agent Key\"), or the fake key replaces the real one."
fi
