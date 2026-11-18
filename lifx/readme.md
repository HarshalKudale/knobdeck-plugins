# LIFX

Control LIFX bulbs, strips and panels from KnobDeck over your own network. Works on Windows, macOS
and Linux. No LIFX account or cloud token is needed.

## What it does

- **Power**, **Brightness** (dial or slider), **Set Brightness**: one light or all of them.
- **Color Temperature** (dial, 1500–9000 K; each light keeps to its own range) and **Set White**.
- **Hue** (dial, wraps around) and **Saturation** (dial).
- **Set Color** (`#RRGGBB`, with an optional fade).

Keys show each light's live state (on/off, brightness, temperature or "Color", hue or "White").

## Setup

Lights on the same network are found automatically (a broadcast on UDP port 56700; a firewall
must let it through). For a light on another subnet, type its address in setup.

## How it works

The LIFX LAN protocol: `GetService` discovery, `LightGet` for state and label, `LightSetColor`
and `LightSetPower`. Every request waits for the light's reply or acknowledgement and is sent
again if UDP drops it.
