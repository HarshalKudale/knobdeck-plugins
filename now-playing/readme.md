# Now Playing

See what is playing and control it, whatever the player. Works on Windows, macOS and Linux.

## What it does

- **Now Playing**: title, artist and cover on a key; pressing it plays or pauses.
- **Play / Pause**, **Next Track**, **Previous Track**.
- **Position**: where the track is, as a slider; drag to jump, or turn the dial to skip a few
  seconds back or forward.
- **Players Wheel**: every player with something loaded; a slice plays or pauses it.

Each action works with **Whatever is playing** (the player that is playing, else the one the
system shows) or with one player you pick.

## Which players

- **Windows**: every app in the volume flyout's media controls (browsers, Spotify, the Media
  Player app, VLC 4…).
- **Linux**: every MPRIS player (browsers, Spotify, VLC, mpv with mpv-mpris, Rhythmbox, Elisa…),
  the same ones GNOME's and Plasma's media controls show.
- **macOS**: Music and Spotify. Since macOS 15.4 other apps can't read the system's now-playing
  information, so only players with their own AppleScript support are covered.

## How it works

Windows' system media transport controls, MPRIS over D-Bus on Linux, and AppleScript on macOS
(only for players that already run: it never launches one). Covers are shrunk to key size first.
