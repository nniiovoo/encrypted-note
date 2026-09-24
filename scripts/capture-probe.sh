#!/usr/bin/env bash
# Dev probe for ADR-0002: can another process capture the encrypted-note window?
# Run while the app is open (a normal build, NOT with ENOTE_DEV_ALLOW_CAPTURE=1).
# PASS = macOS refused to capture the window, or captured it blank.
set -euo pipefail

id=$(swift - <<'EOF'
import CoreGraphics
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
let ours = windows.first { ($0[kCGWindowOwnerName as String] as? String) == "encrypted-note" && ($0[kCGWindowLayer as String] as? Int) == 0 }
print((ours?[kCGWindowNumber as String] as? Int).map(String.init) ?? "")
EOF
)
# Must be on screen: screencapture also refuses windows on another Space, which would be a false pass.
[ -n "$id" ] || { echo "encrypted-note isn't open on this screen. Bring its window to this desktop and retry."; exit 2; }

out=$(mktemp -t enote-probe).png
if ! screencapture -x -o -l "$id" "$out" 2>/dev/null || [ ! -s "$out" ]; then
  echo "PASS: macOS refused to capture window $id."
  exit 0
fi
# A captured-but-blank window is also a pass: a blank PNG compresses to a few KB.
size=$(stat -f %z "$out")
/bin/rm -f "$out"
if [ "$size" -lt 20000 ]; then
  echo "PASS: window $id captured as a blank image ($size bytes)."
else
  echo "FAIL: window $id was captured with visible content ($size bytes). Check Capture Hiding (ADR-0002)."
  exit 1
fi
