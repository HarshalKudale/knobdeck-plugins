# Changelog

## 1.3.1

- A proper app icon in the Plugins store and sidebar (`icons/app-icon.svg`).

## 1.3.0

- Web API keys, through your own Spotify app (setup asks for its client ID; see the readme): Like
  Song, Shuffle, Repeat, Spotify Volume (dial), Play On Device and a Devices wheel (Spotify Connect
  transfer), Play Playlist, a Playlists wheel, Add Song to Playlist and Add to Queue (a song or
  episode link; `{clipboard}` works). Player commands need Spotify Premium.
- Play / Pause, Next, Previous and Now Playing are unchanged and need no setup: leave the client ID
  empty. Web API keys say "Set up your Spotify app first" until it is set.
- Uses Spotify's current library and playlist endpoints (`/me/library`, `/playlists/{id}/items`).
