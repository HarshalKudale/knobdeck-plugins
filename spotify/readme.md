# Spotify

Control Spotify from KnobDeck, on Windows, macOS and Linux. Playback needs no sign-in, account or
Premium; the other keys use the Spotify Web API through your own (free) Spotify app.

## What it does

No setup needed (the Spotify desktop app must be open):

- Play / Pause (follows the player), Next Track, Previous Track.
- Now Playing: title and artist on a key; press to play or pause.

With your Spotify app (see Setup):

- **Like Song**: add the playing song to Liked Songs or remove it; the key follows it.
- **Shuffle** and **Repeat** (off, all, one), following the player.
- **Spotify Volume** on a dial or slider (the volume of the device playing).
- **Play On Device** and a **Devices** wheel: move playback to another Spotify Connect device
  (this computer, your phone, a speaker).
- **Play Playlist** and a **Playlists** wheel (Liked Songs and your first 15 playlists).
- **Add Song to Playlist**: add the playing song to one of your playlists.
- **Add to Queue**: queue a song or episode by its link (`https://open.spotify.com/track/…` or
  `spotify:track:…`); `{clipboard}` queues the link you copied.

Spotify only accepts player commands (shuffle, repeat, volume, devices, playing a playlist, the
queue) from Premium accounts; likes and playlists work on free accounts too.

## Setup

Spotify no longer lets a new app serve more than 25 users, so the plugin uses an app you create in
your own Spotify developer account. It takes about two minutes:

1. Open [developer.spotify.com/dashboard](https://developer.spotify.com/dashboard), sign in with
   your Spotify account and accept the developer terms.
2. **Create app**. Name and description can be anything (for example "KnobDeck").
3. Under **Redirect URIs** add exactly `http://127.0.0.1:47811/callback` and press **Add**.
4. Under **Which API/SDKs are you planning to use?** tick **Web API**, agree to the terms and save.
5. Open the app's **Settings** and copy its **Client ID** (32 letters and digits). There is no
   client secret to copy: the plugin signs in with PKCE.
6. In KnobDeck, run this plugin's setup, paste the client ID and sign in to Spotify in the browser
   window that opens.

An app in development mode only lets in the account that created it and up to 25 accounts you
add under the app's **User Management**. To use another Spotify account, add it there first.

Leave the client ID empty in setup to use only the playback keys. Running setup again with an empty
client ID signs out.

## How it works

Playback goes through the OS, not keyboard shortcuts: Windows' system media controls (the session
Spotify shows in the volume flyout), Spotify's AppleScript dictionary on macOS and MPRIS on Linux.

The other keys call the Spotify Web API (`/me/player`, `/me/library`, `/me/playlists`,
`/playlists/{id}/items`, `/me/player/devices`, `/me/player/queue`) with your sign-in. The client
ID is kept with the plugin's settings; the sign-in token is stored only in this computer's
credential store (macOS Keychain, Windows Credential Manager, Linux Secret Service) and refreshed
when it expires. Web API keys refresh every 5 seconds from one shared player reply.
