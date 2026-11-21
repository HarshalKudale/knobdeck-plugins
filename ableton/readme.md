# Ableton Live

Control Ableton Live from KnobDeck: transport and recording, tempo, the mixer, scenes, track and
scene selection, locators and the selected device's parameters, on keys and dials that show
Live's state as it changes.

Windows and macOS, Live 11 or newer (Live has no Linux build).

## Setup

1. Install this plugin and run its setup. With **Install the KnobDeck Remote Script** ticked
   (the default) it downloads the newest
   [KnobDeck Remote Script](https://github.com/Knobdeck/knobdeck-ableton-companion/releases) and
   copies its `KnobDeck` folder into Live's User Library, `Remote Scripts/KnobDeck`:
   - Windows: `%USERPROFILE%\Documents\Ableton\User Library\Remote Scripts`
   - macOS: `~/Music/Ableton/User Library/Remote Scripts`

   A User Library you moved is found from Live's preferences, or enter its folder (Live ▸
   Settings ▸ Library ▸ Location of User Library).
2. By hand instead: download `knobdeck-ableton-<version>.zip` from the releases, copy the
   `KnobDeck` folder in it into the `Remote Scripts` folder above (create it if it's missing), and
   run setup with the box unticked.
3. Restart Live if it was open. In **Live ▸ Settings ▸ Link, Tempo & MIDI**, pick **KnobDeck** as
   a Control Surface; leave its Input and Output at None.

There is nothing else to configure: the script links to KnobDeck on this computer by itself and
reconnects when either restarts. Live's status bar says "KnobDeck linked".

## What it does

- **Transport:** Play / Stop (shows playing), Stop, Continue Playing, Arrangement Record,
  Session Record, MIDI Arrangement Overdub, Automation Arm, Metronome, Loop, Punch In, Punch Out
  (each shows on / off), Capture MIDI, Undo, Redo.
- **Tempo:** Tempo dial (BPM per tick; drag to set), Tap Tempo (shows the tempo).
- **Locators:** Previous / Next Locator, Add / Delete Locator.
- **Mixer:** Master Volume and Track Volume dials in dB (Live's own fader values, -inf at the
  bottom), Track Pan dial (50L … 50R), Mute, Solo and Record Arm for the selected track, the
  Master or a picked track (return tracks too, except arm), Selected Track (name and level).
- **Session:** Launch Scene (the selected one, which moves on when Live's "Select Next Scene on
  Launch" is on, or a picked one), Stop All Clips, Select Track and Select Scene dials, Tracks and
  Scenes wheels (16 around the selection; a scene slice selects or launches).
- **Device:** the device with Live's blue hand (the selected device): Device Parameter dials, one
  per parameter 1–8 of a bank, following Previous / Next Parameter Bank keys ("Bank 2/4") or a
  fixed bank; Selected Parameter dial (the control last clicked in Live); Device On / Off;
  Selected Device (track and device name).

Tracks are picked by name, so a renamed track needs picking again.

## How it works

The Remote Script runs inside Live as a control surface, links to KnobDeck's companion hub on
`127.0.0.1` and runs each action with Live's Python API on Live's main thread; no keyboard
shortcuts are sent. This plugin opens no port. When Live isn't linked, actions say so and the
Selected Track key shows "Live offline".
