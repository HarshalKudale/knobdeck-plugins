# OpenRGB

Control the RGB lighting [OpenRGB](https://openrgb.org) runs (motherboards, RAM, GPUs, keyboards,
mice, fans, LED strips) from KnobDeck. Windows, macOS and Linux.

## What it does

- **Load Profile** and a **Profiles Wheel**: the profiles saved in OpenRGB.
- **Set Color**: one colour on every device or on one device (`#000000` turns them off). A device
  running an effect switches to its Direct / Static mode first.
- **Brightness** (dial or slider), on every device or one: the mode's own brightness when it has
  one; otherwise the plugin scales the device's colours (and turns them back up from the colour
  it scaled, not the dimmed one).

## Setup

In OpenRGB, open the **SDK Server** tab and press **Start Server** (OpenRGB's settings can
start the server whenever OpenRGB starts, or run it with `--server`). The plugin connects to `127.0.0.1:6742`; setup changes the
host and port, for OpenRGB on another computer.

## How it works

OpenRGB's SDK server protocol over TCP (protocol versions 1–4; newer servers answer in version
4): `REQUEST_CONTROLLER_DATA`, `REQUEST_PROFILE_LIST` / `REQUEST_LOAD_PROFILE`,
`RGBCONTROLLER_SETCUSTOMMODE`, `RGBCONTROLLER_UPDATELEDS` and `RGBCONTROLLER_UPDATEMODE`.
