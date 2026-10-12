# Product demo

This is a real recording of encrypted-note on macOS, edited for pacing with English captions and short opening and closing cards. The app fills the frame for the whole walkthrough. The app shown is v0.1.0 from commit `947b6f5`.

## Files

- [Full video](product-demo.mp4): approximately 36 seconds, 1920 × 1080, 30 fps, H.264, no audio.
- [README preview](product-demo.gif): a looping excerpt at 960 × 540 (find a Note, reveal, hide).
- [English caption track](product-demo.vtt): the same lines appear in the video.
- [Poster](product-demo-poster.png): a frame from the exported video.

## Written walkthrough

1. Unlock the demo Vault with its fictional Master Password.
2. See the Sharing Guard cover the app when a known recording app is detected, then dismiss it with "I'm not sharing right now".
3. Search for a Login and open its Note.
4. Reveal an obviously fake password, then hide it again.
5. Edit the visible Username while leaving the Password replacement field blank.
6. Save, then reveal the same fake password to demonstrate that it was retained.
7. Hide the password and lock the Vault.

## Recording conditions

The development build used a separate Vault containing only fictional sample data. `ENOTE_DEV_DATA_DIR` isolated that data, and `ENOTE_DEV_ALLOW_CAPTURE=1` disabled Capture Hiding so the app window could be recorded. The Sharing Guard was left on: it covered the window until it was dismissed on screen.

The capture included only the test app's window. No microphone, system audio, personal Vault, desktop, real password, key, or Seed Phrase was recorded. The footage demonstrates the workflow; it does not test production screen-capture exclusion.

## What the edit changed

Every app frame comes from that one recording. No app screens were generated or reconstructed.

- **Cuts and zooms.** Waiting and typing were trimmed with hard cuts, and the picture zooms in on the part of the window where each action happens.
- **Time-lapse.** The 30-second reveal countdown plays at ten times normal speed, marked "10x speed" on screen.
- **Lock screen holds.** The lock screen at the start and end is the last half second of the recording, slowed down.
- **Annotations.** The caption line, the rings on each click and the outlines around a few fields were added in the edit. They are not part of the app.
- **Cards.** The opening and closing cards use the app's own icon. The closing card repeats the recording conditions.
- **Dates.** The sample Note was seeded with an "Updated" time in the future, so the time shown moves back to the real recording time after Save.

The video was composed with HyperFrames and encoded with FFmpeg.
