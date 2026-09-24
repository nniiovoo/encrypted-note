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

# Control: without Screen Recording permission screencapture fails for EVERY window, which
# would look like a pass. Prove we can capture the display at all first.
control=$(mktemp -t enote-control).png
if ! screencapture -x -m "$control" 2>/dev/null || [ ! -s "$control" ]; then
  /bin/rm -f "$control"
  echo "CAN'T TEST: this terminal has no Screen Recording permission."
  echo "Allow it in System Settings > Privacy & Security > Screen & System Audio Recording, then retry."
  exit 2
fi
/bin/rm -f "$control"

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
