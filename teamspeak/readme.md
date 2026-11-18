# TeamSpeak

Mute your microphone and speakers, go away, push to talk, and see your channel and who is talking,
from KnobDeck. Works with TeamSpeak 5 and 6 (Remote Apps) and TeamSpeak 3 (ClientQuery), on
Windows, macOS and Linux.

## What it does

- **Mute Microphone**, **Mute Speakers**, **Away**: toggles whose key follows TeamSpeak, also when
  you change it in TeamSpeak itself.
- **Push to Talk**: hold the key to talk.
- **Current Channel** and **Who's Talking** on a key (the server tab you're connected to first).
- **TeamSpeak Button (5/6)**: a named button you bind to any TeamSpeak action (whisper lists,
  soundboard, switching channel). Held while you hold the key.

## Setup

**TeamSpeak 5 / 6** (the default): turn on **Settings ▸ Remote Apps** (port 5899). When you first
use a TeamSpeak key, TeamSpeak asks whether to allow KnobDeck; allow it. The key TeamSpeak gives
KnobDeck is kept in this computer's credential store.

TeamSpeak 5/6 let apps press *buttons*, not mute directly, so bind each KnobDeck key once: in
**Settings ▸ Key Bindings**, add the action (Toggle Microphone Mute, Toggle Sound Mute, Toggle Away,
Push-to-Talk), and when TeamSpeak waits for a key, press the KnobDeck key. Until then the key says
which binding is missing.

**TeamSpeak 3**: enable the **ClientQuery** plugin (Settings ▸ Plugins), copy its API key from
**Settings ▸ Addons ▸ ClientQuery ▸ Settings**, and in the plugin's setup choose TeamSpeak 3 and
paste the key (kept in the credential store). No bindings needed: mute, sound and away are set
directly. Push to Talk deactivates the microphone while the key is up (`client_input_deactivated`),
so leave TeamSpeak 3's own capture mode on continuous transmission or voice activation.

## How it works

- TeamSpeak 5/6: the Remote Apps WebSocket (`ws://127.0.0.1:5899`): `auth` with KnobDeck's
  identifier and stored key, `buttonPress` for the buttons (`mute-microphone`, `mute-speakers`,
  `away`, `push-to-talk`, `custom.<name>`), and the state from TeamSpeak's events
  (`clientSelfPropertyUpdated`, `clientMoved`, `talkStatusChanged`, `channels`, …).
- TeamSpeak 3: ClientQuery on `127.0.0.1:25639` (`auth apikey=`, `whoami`, `clientvariable`,
  `clientupdate`, `channelvariable`, `clientlist -voice`), on the server tab in front.
