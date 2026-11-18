# Apple Music

Control the Music app on your Mac from KnobDeck. macOS only.

## What it does

- **Now Playing**: title, artist and artwork on a key; pressing it plays or pauses.
- **Play / Pause**, **Next Track**, **Previous Track**.
- **Shuffle**: turns shuffle on or off; the key shows which.
- **Repeat**: goes through off → all → one, like Music's own repeat button.
- **Favorite Track**: favorites (loves) the current track or takes it back.
- **Music Volume**: Music's own volume (not the system's) on a dial or slider.
- **Play Playlist**: plays a playlist you pick from your library, optionally shuffled.

## Setup

None. The first time an action talks to Music, macOS asks whether KnobDeck may control Music;
allow it. If you said no, turn KnobDeck on under **System Settings ▸ Privacy & Security ▸
Automation ▸ Music**.

## How it works

Music's own AppleScript dictionary, through `osascript`. Only **Play / Pause** (and pressing
**Now Playing**) and **Play Playlist** open Music when it's closed; every other action and every
key's live state leave a closed Music alone.
