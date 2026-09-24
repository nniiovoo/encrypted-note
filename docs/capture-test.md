# Screen-capture test checklist (ADR-0002)

Re-run after every macOS or meeting-app update. Use a **release build** (`npx tauri build`; release builds have no screenshot override), open it on a **normal desktop, not a full-screen Space**, and keep it on the welcome or lock screen. A test **passes** when the capture shows whatever is *behind* the encrypted-note window, or a blank or black box, and never the app's contents.

Quick automated check, run from a terminal that has Screen Recording permission:

```bash
./scripts/capture-probe.sh
```

## Results

macOS 26.4.1 · Zoom 7.0.6 · Discord 0.0.413 · OBS 32.1.2 · Chrome 153 · QuickTime 10.5

| Capture path | How to test | Result |
|---|---|---|
| Window server flag | automatic (window server reports `kCGWindowSharingState`) | ✅ 0 = excluded (2026-09-24) |
| ScreenCaptureKit window capture | an automation tool captured the release window | ✅ blank (2026-09-24) |
| Sharing Guard | open Zoom / Discord / QuickTime while unlocked | ✅ shield named Discord + QuickTime Player (2026-09-23) |
| `scripts/capture-probe.sh` | terminal with Screen Recording permission | ⬜ |
| Cmd+Shift+3 / Cmd+Shift+4 / Screenshot app | take a screenshot, open it | ⬜ |
| QuickTime / Cmd+Shift+5 screen recording | record 5 s, play it back | ⬜ |
| OBS | add a "macOS Screen Capture" source (display), look at the preview | ⬜ |
| Zoom, normal share | Share Screen → whole screen; check on another device or the recording | ⬜ |
| Zoom, advanced / GPU share mode | Settings → Share Screen → Advanced → capture mode | ⬜ (known leak elsewhere) |
| Google Meet in Chrome | present "Entire screen" in a test meeting | ⬜ |
| Discord | Go Live → Screen | ⬜ |
| Microsoft Teams | share screen (not installed here) | ⬜ |
| iPhone Mirroring / AirPlay / Sidecar | mirror, look at the other device | ⬜ |
| Windows: Snipping Tool, Win+Shift+S, Recall, Teams/OBS | see issue 23; also minimise then restore the window and re-check (Tauri #14189) | ⬜ |

A ❌ result means that tool can see the app. Add it to the Sharing Guard list if it's missing (`crates/guard`), and note it here. Nothing here stops a phone camera.
