#!/usr/bin/env bash
# Prepares this Mac for agents storing keys (ADR-0012). Run it in your own account after the
# agent's Standard account exists. Safe to run again: each run resets the folder's permissions.
# macOS only.
#
#   scripts/agent-keys-setup.sh [--dry-run] [--binary PATH] AGENT_SHORT_NAME
#
#   --dry-run      Check, then print what it would do without changing anything.
#   --binary PATH  The agent's tool, enote-agent-keys (default: inside /Applications/encrypted-note.app).
#
# It creates /Users/Shared/encrypted-note-keys, owned by you, mode 700, with one ACL entry that
# lets the agent account reach the app's socket inside it, and prints the lines that add the tool
# to Claude Code, Codex or the Claude app in the agent account. It handles no key, copies nothing,
# writes nothing in the agent's account and needs no administrator rights.
set -euo pipefail

[[ $(uname -s) == Darwin ]] || { echo "Agents storing keys is set up on macOS only." >&2; exit 1; }

DRY_RUN=0
BINARY=/Applications/encrypted-note.app/Contents/MacOS/enote-agent-keys
AGENT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --binary) [[ $# -ge 2 ]] || { echo "--binary needs a path." >&2; exit 2; }; BINARY="$2"; shift 2 ;;
    -*) echo "Unknown option: $1" >&2; exit 2 ;;
    *) [[ -z "$AGENT" ]] || { echo "Give one agent account name." >&2; exit 2; }; AGENT="$1"; shift ;;
  esac
done
[[ -n "$AGENT" ]] || { echo "Usage: scripts/agent-keys-setup.sh [--dry-run] [--binary PATH] AGENT_SHORT_NAME" >&2; exit 2; }

FOLDER=/Users/Shared/encrypted-note-keys
SHARED=/Users/Shared

# The same rules the app applies when you choose Start (src-tauri/src/agent_route.rs).
[[ "$AGENT" =~ ^[a-z][a-z0-9_-]{0,30}$ ]] || { echo "\"$AGENT\" isn't a Mac account short name: use the Account name from System Settings → Users & Groups." >&2; exit 1; }
AGENT_UID=$(id -u "$AGENT" 2>/dev/null) || { echo "No Mac account named $AGENT. Check its Account name in System Settings → Users & Groups." >&2; exit 1; }
OWNER_UID=$(id -u)
[[ $AGENT_UID != "$OWNER_UID" ]] || { echo "$AGENT is your own account. Run this in your account and name the separate agent account." >&2; exit 1; }
(( AGENT_UID >= 501 && AGENT_UID < 2147483648 )) || { echo "$AGENT is not an ordinary user account." >&2; exit 1; }
for group in admin wheel operator; do
  if dseditgroup -o checkmember -m "$AGENT" "$group" >/dev/null 2>&1; then
    echo "$AGENT is in the $group group. Make it a Standard account first, in System Settings → Users & Groups." >&2
    exit 1
  fi
done

# /Users/Shared is writable by every account: it must be root's and sticky, so no other account
# can replace a folder you own there.
[[ -d "$SHARED" && ! -L "$SHARED" && $(stat -f %u "$SHARED") == 0 ]] || { echo "$SHARED isn't the system's shared folder. Nothing was changed." >&2; exit 1; }
(( 0$(stat -f %Mp%Lp "$SHARED") & 01000 )) || { echo "$SHARED isn't sticky, so another account could replace the folder. Nothing was changed." >&2; exit 1; }

# Anything already at the folder's place must be yours, and a folder.
mine() {
  [[ ! -L "$1" ]] || { echo "$1 is a symbolic link. Remove it yourself, then run this again." >&2; exit 1; }
  [[ ! -e "$1" || $(stat -f %u "$1") == "$OWNER_UID" ]] || { echo "$1 belongs to another account. Remove it yourself, then run this again." >&2; exit 1; }
}
mine "$FOLDER"
[[ ! -e "$FOLDER" || -d "$FOLDER" ]] || { echo "$FOLDER is not a folder." >&2; exit 1; }

run() {
  if [[ $DRY_RUN == 1 ]]; then printf 'Would run:'; printf ' %q' "$@"; printf '\n'; else "$@"; fi
}

echo "==> private socket folder ($FOLDER)"
[[ -d "$FOLDER" ]] || run mkdir -m 700 "$FOLDER"
# Check again: another account could have put a link there since the first check. -h never
# follows a link.
mine "$FOLDER"
[[ $DRY_RUN == 1 || ( -d "$FOLDER" && ! -L "$FOLDER" ) ]] || { echo "$FOLDER is not a folder." >&2; exit 1; }
run chmod -h 700 "$FOLDER"
# Start from no ACL entries, so a run with a mistyped name leaves no second entry behind. The
# agent may only pass through the folder to the socket: it can't list, add or remove anything.
run chmod -h -N "$FOLDER"
run chmod -h +a "user:$AGENT allow search" "$FOLDER"
if [[ -e "$FOLDER/agent.sock" || -L "$FOLDER/agent.sock" ]]; then
  echo "Left $FOLDER/agent.sock alone: the app replaces it only if it is the app's own socket."
fi

echo "==> the agent's tool ($BINARY)"
# The agent account runs this file, so it must not be able to change it: the file and every
# folder above it must be root's or yours, and writable by nobody else.
binary_ok=1
if [[ ! -f "$BINARY" || -L "$BINARY" ]]; then
  echo "Not found (or a link): $BINARY. Build it with 'cargo build --release --locked -p agent-keys-mcp', put it where the agent account can run it but not change it, and pass --binary."
  binary_ok=0
else
  path="$BINARY"
  while :; do
    owner=$(stat -f %u "$path")
    mode=$(stat -f %Lp "$path")
    if [[ -L "$path" ]] || { [[ $owner != 0 && $owner != "$OWNER_UID" ]]; } || (( 0$mode & 022 )); then
      echo "$path can be changed by another account, so the agent could change the tool. Move it somewhere only you or root can change."
      binary_ok=0
      break
    fi
    # Other accounts must be able to pass through each folder, and to run the file.
    if (( ! (0$mode & 01) )); then
      echo "Other accounts can't reach or run $path, so the agent account can't run the tool from here."
      binary_ok=0
      break
    fi
    [[ $path == / ]] && break
    path=$(dirname "$path")
  done
fi

QUOTED_BINARY=$(printf '%q' "$BINARY")
cat <<EOF

==> add the tool in the agent account ($AGENT)
Sign in to $AGENT (or use 'su - $AGENT' in Terminal), then use one of these. Your uid, $OWNER_UID,
is how the tool knows it is talking to your app and no one else's.

Claude Code:
  claude mcp add --scope user enote-keys -- $QUOTED_BINARY --owner-uid $OWNER_UID

Codex, in ~/.codex/config.toml (a request waits up to 5 minutes for you, and Codex gives up on
a tool after 60 seconds unless told otherwise):
  [mcp_servers.enote-keys]
  command = "$BINARY"
  args = ["--owner-uid", "$OWNER_UID"]
  tool_timeout_sec = 330

The Claude app, in ~/Library/Application Support/Claude/claude_desktop_config.json:
  { "mcpServers": { "enote-keys": { "command": "$BINARY", "args": ["--owner-uid", "$OWNER_UID"] } } }
The Claude app may give up on a tool after about a minute, and that can't be changed here. If it
does, the request stays open in the app: answer it there, and the agent checks with list_keys.
EOF

if [[ $DRY_RUN == 1 ]]; then
  echo
  echo "Dry run: nothing was changed."
else
  echo
  echo "Done. Next, in the app: Settings → Agents storing keys, type $AGENT, then choose Start."
fi
if [[ $binary_ok == 0 ]]; then
  echo "Fix the tool's place before the agent uses it (see above)." >&2
fi
