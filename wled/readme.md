# WLED

Control LED strips and matrices running [WLED](https://kno.wled.ge) from KnobDeck. Works on
Windows, macOS and Linux.

## What it does

- **Power**, **Brightness** (dial or slider), **Set Brightness**: one device or all of them.
- **Preset** (shows when it is the active one) and a **Presets Wheel**.
- **Effect** (picker), **Effect (dial)** (turn through the effects, the key shows the name), an
  **Effects Wheel**, **Effect Speed** and **Effect Intensity** dials.
- **Palette**, **Set Color** (primary, secondary or tertiary color slot).
- **Select Segment**: makes one segment the selected one. Effect, speed, intensity, palette and
  color act on the segments selected in WLED, on all segments, or on one segment you pick.

Keys show the device's live state (on/off, brightness, the active preset, effect and palette).

## Setup

Devices on the same network are found automatically (mDNS, `_wled._tcp`). For a device that is
not found (another subnet, mDNS blocked), type its address in setup; `host:port` works too.
No account or key is needed.

## How it works

WLED's JSON API over HTTP: `/json/state` (read and change), `/json/info`, `/json/effects`,
`/json/palettes` and `/presets.json`.
