# Soundboard

Play your own sound files from KnobDeck keys: mp3, wav, ogg (Vorbis) and flac. Windows, macOS
and Linux.

## What it does

- **Play Sound**: a sound file on a key, with its own volume and output device. Choose what a
  press does while the sound is still playing: restart it, stop it, play another copy over it,
  or loop it until you press again.
- **Hold to Play**: plays while the key is held (looping, or once), stops when you let go.
- **Also play on**: a second output device per sound. Send the sound into a virtual cable that
  your microphone app listens to (VB-Cable on Windows, BlackHole on macOS, a PipeWire / PulseAudio
  null sink on Linux) and hear it in your headphones at the same time.
- **Stop All Sounds**.
- **Soundboard Volume**: a master volume dial for every soundboard sound (it keeps its level).
- **Soundpad** (Windows): Play Sound (on your speakers, your microphone or both), Stop and
  Pause / Resume, through Soundpad's own remote control. Sounds are listed from Soundpad.

## Setup

None. For a virtual microphone, install a virtual audio cable and choose it in **Also play on**;
in your voice app, pick the cable's input as the microphone. For Soundpad, keep Soundpad running.

## How it works

Files are decoded in the plugin (symphonia) and mixed per output device. On Windows it plays
through WASAPI and on macOS through CoreAudio; on Linux it talks to the PulseAudio sound server
(PipeWire provides it on GNOME and KDE), so every output, null sinks included, is listed by name.
An output with nothing playing is closed after 30 seconds. Soundpad is driven through its
documented remote control pipe (`\\.\pipe\sp_remote_control`); the Soundpad actions report
"Soundpad isn't running" when it isn't. It never sends keyboard shortcuts.
