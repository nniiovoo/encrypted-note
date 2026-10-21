# Screen-capture test checklist (ADR-0002)

Re-run after every macOS or meeting-app update. Use a **release build** (`npx tauri build`; release builds have no screenshot override), open it on a **normal desktop, not a full-screen Space**, and keep it on the welcome or lock screen. A test **passes** when the capture shows whatever is *behind* the encrypted-note window, or a blank or black box, and never the app's contents.

Quick check, run from a terminal that has Screen Recording permission. It passes when macOS refuses the window capture or the captured pixels are uniform. A nonuniform capture requires manual inspection and exits with status 2; file size alone can't tell whether pixels are the app or the desktop behind it. In that case the script keeps the captured image and prints its path, so you can look at it; delete it afterwards (the command is printed too).

```bash
./scripts/capture-probe.sh
```

## Recording a macOS run

Run this checklist only in a prepared test session without real Notes or private windows. Use the welcome or lock screen for Capture Hiding; use a separate fake-data Vault for the Sharing Guard check. Do not collect screenshots or recordings of real data.

For each capture path, record the date, release-build commit, exact macOS version, capture-app version and mode, Capture Hiding result, Sharing Guard state, and a sanitized observation or issue link. A shared version heading applies only to rows actually run in that session. Leave unrun paths pending; an unavailable app or device needs a reason and remains untested.

Check what the receiver or saved recording sees, not just the presenter preview. Record Capture Hiding and Sharing Guard separately: a captured shield is a Capture Hiding failure even when the Sharing Guard correctly hides Notes. A reported window-server flag is supporting evidence, not a substitute for captured pixels. For a failure, record the app and mode, open a follow-up issue, and add a missing supported app to `crates/guard` with its regression check. Browser meetings are not detected by the Sharing Guard (ADR-0002); record that limit rather than claiming coverage. Keep raw captures private and delete them after inspection.

Use this record for each route below:

```text
Capture path and mode:
Date / release commit / macOS version:
Capture app and version (or built-in OS tool):
Capture Hiding: pending | pass | fail
Sharing Guard: pending | covered | not covered | not applicable
Observation (no private paths, titles, or pixels):
Follow-up issue / guard change, if needed:
```

## Results

The three dated results below are historical. No pending path was run when this recording template was added.

macOS 26.4.1 · Zoom 7.0.6 · Discord 0.0.413 · OBS 32.1.2 · Chrome 153 · QuickTime 10.5

| Capture path | How to test | Result |
|---|---|---|
| Window server flag | automatic (window server reports `kCGWindowSharingState`) | ✅ 0 = excluded (2026-09-24) |
| ScreenCaptureKit window capture | an automation tool captured the release window | ✅ blank (2026-09-24) |
| Sharing Guard | open Zoom / Discord / QuickTime while unlocked | ✅ shield named Discord + QuickTime Player (2026-09-23) |
| `scripts/capture-probe.sh` | terminal with Screen Recording permission | ⬜ |
| Cmd+Shift+3 | capture the whole screen, inspect it | ⬜ |
| Cmd+Shift+4 | capture an area containing the window, inspect it | ⬜ |
| Screenshot app | test its screen, area and window modes separately | ⬜ |
| QuickTime screen recording | record 5 s, play it back | ⬜ |
| Cmd+Shift+5 screen recording | test whole-screen and selected-area recording separately | ⬜ |
| OBS | add a "macOS Screen Capture" source (display), look at the preview | ⬜ |
| Zoom, normal share | Share Screen → whole screen; check on another device or the recording | ⬜ |
| Zoom, advanced / GPU share mode | Settings → Share Screen → Advanced → capture mode | ⬜ (known leak elsewhere) |
| Google Meet in Chrome | present "Entire screen" in a test meeting | ⬜ |
| Discord | Go Live → Screen | ⬜ |
| Microsoft Teams | share screen (not installed here) | ⬜ |
| iPhone Mirroring | record the exact direction and device being mirrored; inspect the receiver | ⬜ |
| AirPlay | mirror the Mac display; inspect the receiver | ⬜ |
| Sidecar | test the Mac window on the connected iPad display | ⬜ |
| Windows: Snipping Tool, Win+Shift+S, Recall, Teams/OBS | see [#11](https://github.com/nniiovoo/encrypted-note/issues/11); also minimise then restore the window and re-check (Tauri #14189) | ⬜ |

A ❌ result means that tool can see the app. Add it to the Sharing Guard list if it's missing (`crates/guard`), and note it here. Nothing here stops a phone camera.

## Physical Windows validation ([#11](https://github.com/nniiovoo/encrypted-note/issues/11))

Status: pending. Cross-compilation and unit tests do not establish these results. Run this on a physical Windows PC in an owner-prepared test account with only fake Notes. The Mac side of the Backup test also needs a separate fake-data session. Release builds ignore the development data-directory override; do not open, replace, or capture a real Vault for this checklist.

Record the date, release commit and app version, exact Windows version/build, CPU architecture, WebView2 version, and each capture app's version and capture mode. Record the Mac OS/app version and USB filesystem for the Backup rows. Follow the [build instructions](../README.md#build) on the Windows PC and record both the release-build result and whether the built app launches. Keep evidence to sanitized observations and issue links; exclude real values, account names, private paths, Note titles and raw screen recordings. Every row remains pending until actually run; an unavailable feature needs a reason and remains untested.

### Capture Hiding

Use the welcome or lock screen. Inspect the saved capture or remote receiver. Test each path once normally and again after minimizing and restoring the app (Tauri [#14189](https://github.com/tauri-apps/tauri/issues/14189)). Record whether the window is absent, a blank/black box, or visible. A blank/black box hides content but does not establish full window exclusion; record this separately from a privacy failure that shows app contents. Record the app's Capture Hiding status as supporting evidence, since the status alone cannot prove the captured result.

| Capture path | Before minimize | After restore | App version / mode / observation / issue |
|---|---|---|---|
| Snipping Tool | pending | pending | |
| Win+Shift+S | pending | pending | |
| Recall, if present | pending | pending | |
| Microsoft Teams | pending | pending | |
| Zoom | pending | pending | |
| OBS display capture | pending | pending | |
| Discord Go Live | pending | pending | |
| Google Meet in Chrome, entire-screen share | pending | pending | |

### Clipboard, locking and Sharing Guard

Use a fake-data Vault for these checks. Copy a fake Hidden Field through the app; never type a real value into a clipboard viewer or record it in evidence. For cloud clipboard, the owner must already have arranged an approved test-device pair; unavailable devices leave that row pending. Do not enable cloud clipboard on a real-data session for this test.

| Check | Expected observation | Result / sanitized evidence / issue |
|---|---|---|
| Clipboard history (Win+V) | The copied fake Hidden Field never appears in history, immediately or after its clear timer | pending |
| Cloud clipboard | The copied fake Hidden Field never reaches the other test device | pending |
| Clipboard clear | The fake Hidden Field pastes during the chosen 30/60-second period, then stops pasting; Wallet Kinds always clear after 30 seconds | pending |
| Sleep and resume | An Unlocked fake Vault is Locked on return, with no shown Hidden Field or unsaved edit | pending |
| Screen lock and unlock | Same Locked state on return | pending |
| User switch and return | Same Locked state on return | pending |
| Sharing Guard | Start each installed known sharing app while Unlocked: whole window is covered, titles absent, Show blocked; record each app separately | pending |

Capture Hiding and Sharing Guard are separate observations. A captured shield is not proof of Capture Hiding. Browser meetings are not detected by the Sharing Guard (ADR-0002). File each failure as a separate issue and link it in its row.

### Mac ↔ Windows Backup round trip

Use current builds on both computers, one exFAT USB stick prepared by the owner, and only fake Vaults. No formatting, deletion, or replacement of real data is part of this test. Include at least one fake Note with an optional schema-2 field (for example the purpose of a Login), so portability covers the current file format. Opening a Backup replaces this computer's Vault; confirm the destination is the disposable fake Vault before accepting.

| Direction | Steps and expected observation | Result / sanitized evidence / issue |
|---|---|---|
| Mac → Windows | Back up now on the Mac to USB; open that Backup on Windows with its fake Master Password; compare the fake Notes and optional fields | pending |
| Windows → Mac | Change a fake Note on Windows, make a new Backup on USB, open it on the Mac; verify the change and remaining fake Notes | pending |
| Replaced fake Vault | Verify the destination's previous fake Vault is available as a Safety Copy, without changing the source Backup | pending |

Completing this document requires actual Windows results and separate issues for observed bugs. Adding the checklist does not complete #11.
