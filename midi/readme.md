# MIDI

Send and receive MIDI from KnobDeck: play notes, send control changes and program changes, turn a
CC on a dial, and see incoming values on a key. Works with any MIDI port: synths, interfaces, DAWs,
other apps. Windows, macOS and Linux.

## What it does

- **Note**: hold the key to play a note (note on while held, note off on release). Number or name
  (`C4` is 60, middle C), channel and velocity.
- **Control Change**: send a CC with a fixed value.
- **CC Toggle**: alternate a CC between an on and an off value (sustain pedal, an effect on / off).
- **CC Dial**: a CC (0–127) on a dial.
- **Program Change**: program 1–128, optionally after a Bank Select (CC 0 / CC 32).
- **Send MIDI Message**: any bytes in hex, several messages or a SysEx (`F0 … F7`) at once.
- **Received Value**: a key that shows the last value of a CC, a note, program change or pitch
  bend received on a port (or the last message of any kind).

CC Toggle and CC Dial show the last value sent; pick a port under **Follow values from** and they
also follow that controller when it comes in (a DAW or synth sending it back).

## Ports

- **macOS and Linux**: the plugin creates its own virtual port, **KnobDeck**, in both directions.
  Pick "KnobDeck" as a MIDI input (or output) in your DAW or synth. It's the default port of every
  action; any other port can be picked instead.
- **Windows** has no virtual ports for apps. Pick an existing port: a hardware interface, or a
  virtual cable made with [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html) (or
  Windows MIDI Services' loopback). Some Windows drivers let only one app open a port at a time.

## How it works

OS MIDI through the shared `midi-io` crate: WinMM on Windows and CoreMIDI on macOS (through
[midir](https://crates.io/crates/midir)), the ALSA sequencer on Linux (`libasound.so.2`, loaded at
run time; PipeWire bridges ALSA MIDI, so its MIDI ports show here too).
