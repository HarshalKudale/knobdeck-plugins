# MQTT

Publish messages, toggle and dim devices, and show a topic's value on a key, through your own
MQTT broker (Mosquitto, the Home Assistant Mosquitto add-on, EMQX…). Works with Zigbee2MQTT,
Tasmota, Shelly and anything else on MQTT. Windows, macOS and Linux.

## What it does

- **Publish**: send a message to a topic (QoS 0/1/2, retain). The topic and message take
  placeholders (`{clipboard}`, `{date}`, `{input:Prompt}`…).
- **Toggle**: send an on or an off message. With a state topic (and a JSON path for JSON state
  messages), the key shows the device's real state and a press switches from it; without one it
  alternates.
- **Topic Value**: a key that shows the newest message on a topic, or a value inside a JSON
  message (`temperature`, `ENERGY.Power`, `list[0].name`), with a title and a text before / after.
  `+` and `#` wildcards work.
- **Value Dial**: turn to publish a number in your range (`From` / `To`, decimals); `{value}` in
  the message is the number. With a state topic, the dial starts from the device's value.

## Examples

| Device | Command topic | Message | State topic / path |
|---|---|---|---|
| Zigbee2MQTT light | `zigbee2mqtt/<name>/set` | `{"state":"ON"}` / `{"state":"OFF"}`; dial `{"brightness": {value}}`, 0–254 | `zigbee2mqtt/<name>`, `state` / `brightness` |
| Tasmota plug | `cmnd/<topic>/POWER` | `ON` / `OFF`; dimmer dial on `cmnd/<topic>/Dimmer`: `{value}` | `stat/<topic>/POWER` |
| Shelly (Gen 1) relay | `shellies/<id>/relay/0/command` | `on` / `off` | `shellies/<id>/relay/0` |

A message without placeholders is sent exactly as typed. With a placeholder in it, `{{` and `}}`
stand for literal braces.

## Setup

The broker's address and port (1883, or 8883 with TLS), and a username and password if it needs
them. Setup connects once before saving, so a wrong address or password shows there. The password
is kept in this computer's credential store. For a broker with a self-signed certificate, give
the CA's PEM file; otherwise the system's trusted certificates are used.

## How it works

One MQTT 3.1.1 session (rumqttc; TLS through rustls), opened at first use and reconnected on its
own. Keys that read state subscribe to their topics; retained messages give them the current
state at once.
