# REAPER

Control [REAPER](https://www.reaper.fm) from KnobDeck through REAPER's built-in web browser
interface. Works on Windows, macOS and Linux.

## Turn on REAPER's web interface

1. In REAPER, open **Preferences ▸ Control/OSC/web** (Options ▸ Preferences, or ⌘, / Ctrl+P).
2. Click **Add**, choose **Web browser interface** as the control surface mode.
3. Leave the port at **8080** (or pick another free one and type the same port in KnobDeck setup).
   Any of the offered web pages works: KnobDeck only uses the `/_/` command endpoint behind them.
4. Optionally set **Username:password** to protect it; then type the same pair in KnobDeck setup
   (the password is kept in this computer's credential store).
5. Click **OK**. REAPER answers on `http://127.0.0.1:<port>/` while it runs. To control REAPER on
   another computer, type that computer's address as the host and allow the port in its firewall.

## What it does

- **Play / Pause** (shows stopped, playing, paused or recording), **Stop**, **Record** (shows when
  recording; pressing it while recording stops), **Repeat** and **Metronome** toggles with live
  state, **Go to Start**, **Go to End**, and a **Position** key (time or measures.beats).
- **Add Marker**, **Previous Marker**, **Next Marker**, and **Go to Marker** (pick one of the
  project's markers).
- **Run Action**: any action by its command id, numeric (`40044`) or named (`_SWS_SAVESEL`,
  scripts, custom actions). Copy the id from Actions ▸ Show action list ▸ right-click ▸ Copy
  selected action command ID. Actions with an on/off state show it on the key.
- **Track Volume** (dB, −60 dB is silent, up to +12 dB) and **Track Pan** dials; **Mute**,
  **Solo** and **Record Arm** toggles. Each acts on the tracks selected in REAPER (the default) or
  on one track you pick (the master included), and shows the track's live state and name.
- **Selected Track** shows the selected track and its volume; **Select Track (dial)** moves the
  selection up and down the track list, so "Selected tracks" keys follow it.

Picked tracks are remembered by number: moving tracks around in REAPER changes which one a key
controls. Use "Selected tracks" to follow whatever you select.

## How it works

REAPER's web interface over HTTP: a request to `/_/CMD;CMD;…` runs the commands and answers one
tab-separated line each (`TRANSPORT`, `TRACK`, `MARKER`, `GET/REPEAT`, `SET/TRACK/n/VOL` …,
`SET/POS_STR/m<id>`, `GET/<command id>`, and command ids that run actions). No keystrokes are
sent: transport and marker keys run REAPER's own actions by id (Transport: Play 1007, Stop 1016,
Record 1013, Play/pause 40073; Markers: Insert marker 40157, Go to previous / next 40172 / 40173;
Metronome 40364; Go to start / end 40042 / 40043).
