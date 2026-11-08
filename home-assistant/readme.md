# Home Assistant

Control and watch your Home Assistant smart home from KnobDeck. Works on Windows, macOS and Linux.

## What it does

- **Toggle Entity**: turn a light, switch, fan, input boolean, automation, siren or group on and
  off; open and close a cover or valve; lock and unlock a lock; play and pause a media player. The
  key follows the entity's state, also when it is changed elsewhere.
- **Run Scene / Script**: activate a scene, run a script, trigger an automation or press a button
  entity.
- **Show State**: show a sensor (or any entity) on a key, with its unit, or one of its attributes.
- **Level** (dial or slider, 0–100 %): light brightness, cover or valve position, fan speed, media
  player volume, or an `input_number` / `number` across its range. Turning a light or fan to 0 %
  switches it off.
- **Call Service**: call any service (`light.turn_on`, `notify.mobile_app_phone`…) with optional
  JSON service data. Services and entities are listed from your Home Assistant.
- **Entities Wheel**: a wheel of up to 16 lights, switches, scenes, scripts… (optionally filtered by
  name). Pressing a slice toggles or runs it.

## Setup

1. In Home Assistant, open your profile ▸ **Security** ▸ **Long-lived access tokens** ▸
   **Create token**, and copy it.
2. In KnobDeck's setup for this plugin, enter your Home Assistant URL (for example
   `http://homeassistant.local:8123`, or your Nabu Casa `https://…ui.nabu.casa` address) and paste
   the token.

If the token is revoked, the plugin asks you to run setup again.

## How it works

The plugin uses Home Assistant's REST API (`/api/states`, `/api/services/<domain>/<service>`) with
the token as a bearer token. Keys refresh every 2 seconds (sensor keys every 5) from one shared
`/api/states` reply. The token is stored only in this plugin's data folder on your computer.
