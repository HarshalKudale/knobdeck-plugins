# Twitch

Run your Twitch channel from KnobDeck, on Windows and macOS.

## What it does

- Chat Message (posts as you), Clear Chat.
- Emote-Only, Follower-Only, Subscriber-Only and Slow Mode toggles that follow the chat's settings.
- Run Ad (30 s – 3 min), Stream Info (title and/or category), Stream Marker.
- Create Clip (optionally opens the clip editor), Viewer Count on a key.
- Channel Monitor: shows whether a streamer is live, with their viewer count; press to open the channel.
- Alert Pager: the latest follow, sub (also resubs and gifted subs), raid or cheer on your channel.
  A new alert flashes, then stays lit until you press the key; further presses page back through
  the last 20. Tick which kinds each key shows.

## Setup

Setup opens twitch.tv/activate with a code already filled in; approve KnobDeck there. If you
registered your own app at dev.twitch.tv (client type **Public**), paste its client ID in setup.

If you signed in before version 1.3.0, run setup again for the Alert Pager to show follows, subs
and cheers (it needs three more read permissions: followers, subscriptions, Bits); raids work
without it. The key says which kinds are missing.

## How it works

The Twitch Helix API with your sign-in (device-code flow; no password or secret is stored, only a
token, kept in this computer's credential store: macOS Keychain, Windows Credential Manager, Linux
Secret Service). Alerts come from Twitch EventSub over a WebSocket
(`wss://eventsub.wss.twitch.tv/ws`), opened only while an Alert Pager key is placed.
