# Philips Hue

Control Philips Hue lights, rooms and zones from KnobDeck through your Hue Bridge. Works on
Windows, macOS and Linux. No Hue account is needed: everything stays on your network.

## What it does

- **Power**: turn a light, a room, a zone or all lights on and off. The key follows the lights,
  also when they are switched from the Hue app or a switch.
- **Brightness** and **Color Temperature** (2000–6500 K) on a dial or slider, plus a **Set
  Brightness** key for a fixed value. Turning brightness to 0 switches the lights off.
- **Set Color**: set a color (`#RRGGBB`) on color lights.
- **Scene**: recall a scene. **Scenes Wheel**: a wheel of up to 16 scenes of one room or zone.

## Setup

1. Press the round **link button** on top of the Hue Bridge.
2. Within 30 seconds, run this plugin's setup in KnobDeck. The bridge is found on your network
   automatically; type its address only if it is not (Hue app ▸ Settings ▸ My Hue system ▸ Bridge).

If the bridge gets a new address from your router, the plugin finds it again by its bridge id.
If KnobDeck is removed from the bridge's list of apps, the plugin asks you to run setup again.

## How it works

The plugin uses the bridge's local REST API (`http://<bridge>/api/<username>/…`). The bridge is
found by mDNS (`_hue._tcp`), or by Signify's discovery service when mDNS finds nothing. Pairing
creates a bridge username for KnobDeck, stored only in this plugin's data folder. Keys refresh
every 3 seconds from one shared reply, well within the bridge's request limits.
