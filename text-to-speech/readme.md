# Text to Speech

Read text aloud with your computer's own voices: the selected text, the clipboard, or anything you
write on the key. Works on Windows, macOS and Linux.

## What it does

- **Speak Text**: reads the key's text aloud. The text takes KnobDeck placeholders: `{selection}`
  (the default) reads what is selected in the app in front, `{clipboard}` the clipboard, and you
  can mix in your own words (`It is {time}`) or ask each time with `{input:What should I say?}`.
  A key can pick its own voice. Pressed while speaking, it stops (press again to start over), or,
  if you choose, speaks its text instead.
- **Stop Speaking**: stops at once. The key lights up while something is being spoken.
- **Use Voice**: makes a voice the one every Speak Text key uses (unless a key picks its own). The
  key lights up while its voice is the one in use, so a row of them works as a voice picker.
- **Speech Rate** dial: 50 % to 300 % of the voice's normal speed.
- **Speech Volume** dial: 0 to 100 %.

The voice, rate and volume are kept for next time.

## Voices on each OS

- **Windows**: every voice in Settings ▸ Time & language ▸ Speech, the natural ones included
  (Windows' own speech synthesizer).
- **macOS**: every voice in System Settings ▸ Accessibility ▸ Spoken Content ▸ System Voice ▸
  Manage Voices, downloaded Enhanced and Premium voices included (through the system's `say`).
- **Linux**: Speech Dispatcher, the speech service GNOME, KDE and the Orca screen reader use.
  Install it with a voice if it isn't there: `sudo apt install speech-dispatcher espeak-ng`
  (Debian / Ubuntu) or `sudo pacman -S speech-dispatcher espeak-ng` (Arch). Every voice of every
  installed output module is listed (espeak-ng, piper, RHVoice, Festival…). The plugin starts
  Speech Dispatcher when it isn't running.
