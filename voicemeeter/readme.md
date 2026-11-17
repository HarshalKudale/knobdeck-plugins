# Voicemeeter

Control Voicemeeter, Voicemeeter Banana and Voicemeeter Potato from KnobDeck. Windows only, like
Voicemeeter.

## What it does

- **Strip Gain** and **Bus Gain**: a dial slider in dB (−60 to +12), with the step you choose.
- **Strip Mute** and **Bus Mute**: one key that shows the state and switches it.
- **Strip Routing**: turn an input strip's A1–A5 / B1–B3 button on or off (the outputs your
  edition has).
- **Macro Button**: press a push button in Voicemeeter's Macro Buttons, or switch an on/off button.
  Buttons are numbered from 0 as in the Macro Buttons app.
- **Load Configuration**: load a saved Voicemeeter `.xml` configuration.
- **Restart Audio Engine**.
- **Show Voicemeeter**: bring its window up, or start it when it isn't running.

Strips and buses are listed with the names you gave them in Voicemeeter.

## Setup

None. Install Voicemeeter from vb-audio.com and keep it running.

## How it works

It uses Voicemeeter's own Remote API (`VoicemeeterRemote64.dll`, installed with Voicemeeter),
loaded from the install folder when the first action runs. It never sends keyboard shortcuts.
