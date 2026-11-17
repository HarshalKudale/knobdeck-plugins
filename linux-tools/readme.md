# Linux Tools

Window management, workspaces, audio and system tools for KnobDeck on **Linux**: the Linux
counterpart of Win Tools and Mac Tools.

Supported desktops: **KDE Plasma 6** and **GNOME 45 or newer**, on Wayland and X11, on Debian /
Ubuntu and Arch based distributions.

## What it does

- **Current window:** minimize, maximize, restore, close, send to back, Maximize / Restore, Full
  Screen, Always on Top, and on Plasma Borderless, Transparency and Set Opacity. Toggles show the
  focused window's state.
- **Position:** halves, quarters, thirds, center, fill a display, custom position/size, move to the
  next display, and **Move App Window** for a named app. Every action can target the current
  display or a chosen one.
- **Dynamic wheels:** pick any open window (with its app icon) and apply one of the actions above.
- **Workspaces and desktop:** previous / next / new / close desktop, Overview, Present Windows,
  Applications, Search (KRunner / GNOME search), Screenshot, Files, System Settings, System
  Monitor, Lock Screen, and on Plasma Show Desktop, Emoji Picker and Turn Off Display.
- **Display Brightness:** a slider and a Set Display Brightness key, per display (or all of
  them), for laptop screens and external monitors (DDC/CI).
- **System Stats:** live CPU, GPU or memory use on the tile; pressing it opens the system monitor.
- **Wi-Fi / Bluetooth** on/off, **Power Profile** (Power Saver / Balanced / Performance) and
  **Ping**.
- **Audio:** per-app volume and mute (the focused app or one you choose), mute microphone,
  push-to-talk, device volume and mute, and wheels / toggles to switch the output or input device.
- **Text Snippet**: pastes saved text into the focused app.

## How it works

Nothing here presses a shortcut, except Text Snippet's Ctrl+V (Linux has no app-independent paste
request; terminals paste with Ctrl+Shift+V, so pasting there won't work).

- **KDE Plasma:** windows through KWin scripting; Overview, Present Windows, Show Desktop and
  Turn Off Display call KWin's / PowerDevil's own actions by name (so they work whatever key you
  bound to them); KRunner for search; Klipper for the clipboard; Spectacle for screenshots.
- **GNOME:** GNOME doesn't let other programs see or move windows on Wayland, so Linux Tools
  installs a small **KnobDeck GNOME Shell extension** (`knobdeck@knobdeck.harshalkudale.com`)
  into `~/.local/share/gnome-shell/extensions` and enables it. GNOME loads new extensions at
  login: **log out and back in once** after installing the plugin.
- **Brightness:** on Plasma through PowerDevil's own brightness service (laptop screens and
  DDC/CI monitors, with Plasma's OSD). On GNOME the laptop screen through logind, and external
  monitors over DDC/CI on `/dev/i2c-*`, which needs the `i2c-dev` module and access to the bus:
  `sudo modprobe i2c-dev` (and `i2c-dev` in `/etc/modules-load.d/`), then
  `sudo usermod -aG i2c $USER` (or ddcutil's udev rule) and log in again.
- **Both:** lock through logind, Wi-Fi through NetworkManager, Bluetooth through BlueZ, power
  profiles through power-profiles-daemon, audio through `pactl` (PipeWire or PulseAudio).

## Requirements

- `pactl` for audio: preinstalled on most desktops; otherwise `sudo apt install pulseaudio-utils`
  (Debian/Ubuntu) or `sudo pacman -S libpulse` (Arch).
- Power profiles need `power-profiles-daemon` (or `tuned-ppd`).

## Differences from Win Tools

- GNOME can't remove window borders or set a window's opacity, and has no Show Desktop, emoji
  picker or display-only blanking, so those actions only work on Plasma (on GNOME they say so).
- GNOME's dynamic workspaces (its default) add and remove workspaces on their own, so New / Close
  Desktop need Dynamic Workspaces turned off in Settings ▸ Multitasking.
- Click-Through and Run dialog have no Linux equivalent and are left out; Power Plans are the
  desktop's power profiles.
- Text Snippet always pastes through the clipboard: typing through a virtual keyboard depends on
  the keyboard layout and can't enter most non-ASCII text.
