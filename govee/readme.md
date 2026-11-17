# Govee

Control Govee lights and light strips from KnobDeck. Works on Windows, macOS and Linux.

## What it does

- **Power**, **Brightness** (dial or slider), **Color Temperature** (dial or slider), **Set Color**
  (`#RRGGBB`), **Set Brightness**: one device or all of them.
- **Scene**: one of the device's light scenes (needs your Govee API key).

## Setup

Most recent Govee lights can be controlled on your network: in the Govee Home app, open the
device's settings and turn on **LAN Control**. Those devices are found automatically and need no
account.

For models without LAN Control, and for scenes, add your Govee API key in setup (Govee Home app ▸
Profile ▸ Settings ▸ Apply for API Key; it arrives by email). The key is kept in this computer's
credential store. Govee allows 10,000 requests a day per key, so devices reached through the
cloud are read at most once a minute and changed only when you press a key.

## How it works

LAN devices get UDP messages from the Govee LAN API (scan on multicast port 4001, commands to the
device's port 4003, replies to port 4002 on this computer; a firewall must let these through).
Other devices go through Govee's developer API.
