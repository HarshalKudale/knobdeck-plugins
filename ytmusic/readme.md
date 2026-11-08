# YouTube Music Desktop

Control the [YouTube Music Desktop App](https://ytmdesktop.app) (YTMDesktop 2.x) from KnobDeck.
Works on Windows, macOS and Linux.

## What it does

- **Play / Pause**, **Next**, **Previous**; **Now Playing** shows the song, artist and album art
  (press to play or pause).
- **Like** and **Dislike** the current song; the keys follow the song's rating.
- **Volume** on a dial or slider, and **Mute**.
- **Shuffle**, **Repeat** (off → all → one) and **Seek** forward or back by a number of seconds.
- **Play Playlist**, a **Playlists Wheel** of your playlists, and an **Up Next Wheel** of the queue.

## Setup

1. In YouTube Music Desktop, open **Settings ▸ Integrations** and turn on **Companion server** and
   **Enable companion authorization**.
2. Run this plugin's setup in KnobDeck. YouTube Music Desktop asks whether to allow KnobDeck:
   click **Allow** within 30 seconds.

Leave the address empty when the app runs on this computer. To control it on another computer,
type that computer's address, and make sure its Companion server accepts connections from the
network.

## How it works

The plugin talks to the app's Companion Server (`http://127.0.0.1:9863/api/v1`): commands are
sent to `/command`, and the app's realtime feed (Socket.IO at `/api/v1/realtime`) keeps the keys
up to date without polling. If the feed drops, the plugin reads `/state` instead. The token is
stored only in this plugin's data folder; if it is revoked in the app, the plugin asks for setup
again.

YouTube Music Desktop is a community app, not made by Google.
