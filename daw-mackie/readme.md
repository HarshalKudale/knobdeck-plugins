# DAW Control (Mackie)

Control your DAW from KnobDeck as a **Mackie Control** surface: any DAW that supports one (Logic
Pro, Cubase, Nuendo, Studio One, REAPER, Ableton Live, Bitwig Studio, Ardour, FL Studio…). The keys
follow the DAW: LEDs, fader positions, track names and the time
display come back from it. Windows, macOS and Linux.

## What it does

- **Play**, **Stop**, **Record**, **Rewind**, **Fast Forward**, **Loop**: the key lights when the
  DAW lights the button.
- **Mute Track**, **Solo Track**, **Record Arm Track**, **Select Track** for strip 1–8, with the
  track's name from the DAW's display.
- **Fader** on a dial for strip 1–8 or the master; it moves when the fader moves in the DAW.
- **V-Pot** on a dial (pan, sends, plug-in parameters: whatever the DAW assigns), showing its ring.
- **Jog Wheel** on a dial, showing the time display.
- **Bank / Track** key and **Bank / Track (dial)**: move the 8 strips by a bank or one track.
- **Mackie Button**: any other Mackie Control button, held while you hold the key (F1–F8,
  markers, nudge, save, undo, automation modes, assign modes, flip, Shift / Option / Control /
  Alt, cursor, zoom, scrub, click, …); it lights with the DAW's LED.
- **Time Display**: the timecode or bars / beats; press to switch between them.

Strips are the 8 tracks of the DAW's current bank, as on a hardware Mackie Control.

## Setup

**macOS and Linux**: the plugin creates a virtual port, **KnobDeck Mackie**. In the DAW, add a
**Mackie Control** control surface and pick "KnobDeck Mackie" as both its MIDI input and output.

**Windows** has no virtual ports: create two ports in
[loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html), e.g. "KnobDeck to DAW" and "DAW
to KnobDeck". Run the plugin's setup and enter them (port to the DAW, port from the DAW), and in the
DAW pick "KnobDeck to DAW" as the Mackie Control's input and "DAW to KnobDeck" as its output. Setup
can name existing ports on macOS and Linux too.

## How it works

The Mackie Control Universal protocol over MIDI (through the shared `midi-io` crate: WinMM, CoreMIDI,
the ALSA sequencer): buttons are notes on channel 1, faders are pitch bend on channels 1–9 with a
touch note around each move, the jog wheel and V-Pots are relative CCs; the DAW lights LEDs with
the same notes, moves faders with pitch bend, writes the LCD over SysEx and the time display with
CCs 0x40–0x49. The plugin answers the device query handshake as a Mackie Control (main unit).
