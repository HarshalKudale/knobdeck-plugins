# OBS

Control OBS Studio from KnobDeck through obs-websocket v5.

## What it does

- Start and stop recording, streaming, replay buffer, and virtual camera.
- Switch scenes, profiles, and scene collections.
- Toggle source visibility, mixer mute state, filters, and studio mode.
- Trigger transitions, chapter markers, screenshots, and media source controls.
- Go back to the previous scene (the key shows its name) and pick the Studio Mode preview scene.
- Source Volume on a dial or slider, kept in sync with OBS's mixer.
- OBS Stats on a key: CPU, FPS, dropped or skipped frames.
- Refresh a browser source, or point it at a new URL.
- Split Recording: start a new recording file without stopping (OBS 30.2 or newer, with
  Settings ▸ Output ▸ Advanced ▸ Recording ▸ Automatic File Splitting turned on; "Only split
  manually" is enough).
- Set Text: write a text source's text (GDI+ on Windows, FreeType 2 on macOS and Linux), with
  placeholders such as `{clipboard}` or `{date}`.
- Stream / Record Time on a key: "LIVE 12:34", "REC 01:02:03", "Paused …", lit while the output runs.

## Setup

1. Enable obs-websocket in OBS Studio.
2. Run the KnobDeck setup flow and enter your host, port, and password.
3. Keep OBS running while the plugin is active.
