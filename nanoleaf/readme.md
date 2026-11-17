# Nanoleaf

Control Nanoleaf Shapes, Elements, Canvas, Lines, Light Panels and lightstrips from KnobDeck.
Works on Windows, macOS and Linux.

## What it does

- **Power**: one device or all of them; the key follows the device.
- **Brightness** and **Color Temperature** on a dial or slider.
- **Set Color** (`#RRGGBB`).
- **Play Effect**: a key for one of the device's effects (scenes); it lights up while it plays.
- **Effects Wheel**: the device's effects on a wheel.

## Setup

Devices on the same network are found automatically. Start setup, then hold each device's power
button for 5–7 seconds until its lights flash: that lets KnobDeck pair with it (setup waits up to
a minute). Pair every device in the same setup run. If a device isn't found (a network that blocks
multicast), type its address. A device that gets a new address from your router needs pairing
again, so a fixed address (DHCP reservation) helps.

## How it works

Each device has a local API on port 16021 (the Nanoleaf OpenAPI). The pairing token is kept in this
computer's credential store, never in a file.
