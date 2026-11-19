# Win Tools

Advanced Windows window-management and system-volume tools for KnobDeck. Windows only; on macOS
use [Mac Tools](../mac-tools).

## What it does

- Snap, move, center, resize, maximize, restore, and close windows.
- Toggle always-on-top, send windows to the back, and move them between monitors.
- Control app-specific volume, mute, and monitor movement.
- Audio devices: mute the microphone (toggle or push-to-talk), switch the default playback or
  recording device (one key, an A/B toggle, or a wheel), and set any device's volume or mute.
  The switch can set the default device, the communications device, or both.
- Includes dynamic wheels for choosing target windows live.
- Move App Window: snap, center, maximize, minimize or place a chosen app's window
  (for example Discord to the right half of monitor 2) whether or not it has focus.
- Sleep Display: turns the monitors off (any mouse or key input wakes them).
- Task View, Show Desktop, Lock PC, Screen Snip, File Explorer, Settings, Run and Task Manager.
- Text Snippet: types saved text into the focused window.
- Wi-Fi / Bluetooth on-off, power plan keys and a power plan wheel, and a Ping key that shows
  the latency to a host, refreshed every 5 seconds.
- Display Brightness slider and Set Display Brightness key, per monitor (or all of them): the
  laptop screen through Windows' own brightness control, external monitors over DDC/CI (turn
  DDC/CI on in the monitor's menu if it's off).
- System Stats button: shows live CPU, GPU or memory use on the tile and opens Task Manager when pressed.
- Position and monitor moves now apply real window bounds, and moved windows are
  raised on the target monitor without taking focus away from the current app.

## Good for

- Stream decks and productivity wheels.
- Multi-monitor workflows.
- Fast window layout changes while gaming or working.

Task View, Show Desktop, Run, Screen Snip, Settings, File Explorer and Task Manager open through the
Windows shell (`ShellExecute`) rather than shortcuts, so the focused app can't intercept them.
Nothing here presses a shortcut; anything that only a shortcut can do (virtual desktops, the
Emoji Panel, clipboard history) is left to KnobDeck's own Keybind action. Restyling another app's
window (borderless, transparency, click-through) and "fullscreen" by resizing aren't offered:
they break many apps (Chromium, Store apps, games). Switching the default audio device uses `IPolicyConfig`, the undocumented
interface the Sound control panel itself uses (Windows has no public API for it); volume and mute
use the public `IAudioEndpointVolume`.
