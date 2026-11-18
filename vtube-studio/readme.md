# VTube Studio

Control VTube Studio from KnobDeck through its public API. Windows and macOS (VTube Studio has no
Linux build).

## What it does

- **Trigger Hotkey**: any hotkey of the model loaded now, picked from a list (triggered through
  the API, not as a key press).
- **Load Model**: switch to a model; the key shows when it's loaded.
- **Toggle Expression**: turn an expression on and off, with a fade time; the key follows it.
- **Move / Resize Model** on a dial: size, left / right, down / up or rotation.
- **Parameter Dial**: set a tracking parameter across its range from a dial, in place of face
  tracking. Use one of KnobDeck's own parameters (`KnobDeckDial1`–`4`, created the first time) and
  map it to any model parameter in VTube Studio, or pick an existing one. The value is held (sent
  again twice a second, as VTube Studio requires) unless you turn that off.
- **Current Model** on a key.

## Setup

In VTube Studio, open **Settings** and turn on **Start API** (port 8001). The first time KnobDeck
connects, VTube Studio asks whether to allow the plugin "KnobDeck"; allow it, then press the key
again. The token is kept in this computer's credential store. Change the host or port in the
plugin's setup if needed.

## How it works

VTube Studio's WebSocket API (`ws://127.0.0.1:8001`): `AuthenticationTokenRequest` once, then
`AuthenticationRequest` per connection; `HotkeysInCurrentModelRequest` / `HotkeyTriggerRequest`,
`AvailableModelsRequest` / `ModelLoadRequest`, `ExpressionStateRequest` /
`ExpressionActivationRequest`, `CurrentModelRequest` / `MoveModelRequest`,
`InputParameterListRequest` / `ParameterValueRequest` / `ParameterCreationRequest` /
`InjectParameterDataRequest`.
