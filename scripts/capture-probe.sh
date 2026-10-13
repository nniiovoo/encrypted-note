#!/usr/bin/env bash
# Dev probe for ADR-0002: can another process capture the encrypted-note window?
# Run while the app is open (a normal build, NOT with ENOTE_DEV_ALLOW_CAPTURE=1).
# PASS = macOS refused to capture the window, or captured it blank.
# REVIEW = it captured something that isn't blank: the image is kept and its path printed.
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

# Both captures go in one folder that only you can read (mktemp -d). It is removed however the
# script ends, Ctrl-C included, unless the window capture has to be looked at (REVIEW below).
work=$(mktemp -d "${TMPDIR:-/tmp}/enote-probe.XXXXXX")
control="$work/display.png"
out="$work/window.png"
keep_capture=false
cleanup() {
  /bin/rm -f "$control"
  if [ "$keep_capture" = false ]; then
    /bin/rm -f "$out"
    /bin/rmdir "$work" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Control: without Screen Recording permission screencapture fails for EVERY window, which
# would look like a pass. Prove we can capture the display at all first.
if ! screencapture -x -m "$control" 2>/dev/null || [ ! -s "$control" ]; then
  echo "CAN'T TEST: this terminal has no Screen Recording permission."
  echo "Allow it in System Settings > Privacy & Security > Screen & System Audio Recording, then retry."
  exit 2
fi
# The control is a picture of the whole display: remove it now rather than at the end.
/bin/rm -f "$control"

if ! screencapture -x -o -l "$id" "$out" 2>/dev/null; then
  echo "PASS: macOS refused to capture window $id."
  exit 0
fi
if [ ! -s "$out" ]; then
  echo "CAN'T TEST: window capture produced no image."
  exit 2
fi
# File size says nothing reliable about whether an image contains visible content.
classification=$(swift "$(dirname "$0")/capture-image-classifier.swift" "$out") || {
  echo "CAN'T TEST: the captured image couldn't be inspected."
  exit 2
}
if [ "$classification" = "blank" ]; then
  echo "PASS: window $id captured as a uniform blank image."
else
  keep_capture=true
  echo "REVIEW: window $id produced a nonblank image. Inspect it for app content; file size cannot determine whether it shows the app or what is behind it."
  echo "The image is kept at: $out"
  printf 'It may show your Notes. Delete it when you have looked: /bin/rm -r %q\n' "$work"
  exit 2
fi
