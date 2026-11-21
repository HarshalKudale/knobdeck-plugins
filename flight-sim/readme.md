# Flight Simulator

Fly Microsoft Flight Simulator 2020 and 2024 from KnobDeck: autopilot, radios, gear, flaps and
lights on keys and dials that show what the aircraft is doing. Windows only, like the sim.

## What it does

**Autopilot**
- **Autopilot Master** and **Autopilot Mode** (HDG, ALT, NAV, APR, VS, FLC, IAS, BC, yaw damper,
  flight director): keys lit while the mode is on.
- **Heading Bug**, **Autopilot Altitude**, **Vertical Speed**, **Speed Bug** and **Course (OBS)**
  (NAV1, NAV2 or GPS): dials that show the value the autopilot holds.

**Radios**
- **COM Standby MHz / kHz** and **NAV Standby MHz / kHz**: the outer and inner knob of COM1 / COM2
  and NAV1 / NAV2, on 25 kHz or 8.33 kHz COM spacing. The kHz knob doesn't carry into the MHz,
  like a real radio.
- **Swap Radio**: active ⇄ standby; the key shows the active frequency.
- **Set Transponder** (a code such as 7000 on a key) and **Transponder Digit** (one digit on a dial).

**Aircraft**
- **Landing Gear** (shows up, down and moving), **Flaps** (a dial, notch by notch) and **Flaps Up /
  Down**, **Spoilers** (a dial) and **Spoilers Arm**, **Parking Brake**.
- **Lights**: landing, taxi, nav, beacon, strobe, panel, logo, wing, recognition, cabin.
- **Electrical Master**: battery, alternator, avionics.
- **Trim**: elevator, aileron or rudder, in percent.
- **Altimeter (inHg)**, **Altimeter (hPa)** and **Altimeter STD / Local**.

**Sim**
- **Pause**, **Sim Rate** (faster / slower) and **Flight Readout**: airspeed (indicated, true,
  ground), altitude, height above ground, vertical speed, heading, fuel %, sim rate, outside air
  temperature, wind.

**Power users**
- **Custom Event**: any K-event from the SDK's Event IDs list (like `TOGGLE_FUEL_VALVE_ALL`), with
  up to five values (`0x…` for BCD codes).
- **SimVar Readout**: any SimVar in any unit (like `PLANE ALTITUDE` in `feet`), with decimals and
  a label.

Add-on aircraft with their own systems (some airliners) may ignore the standard events for parts
of their cockpit. Their own events work through Custom Event; their L:vars need a WASM module and
are not supported yet.

## Setup

KnobDeck talks to the sim through SimConnect, which needs `SimConnect.dll`. The plugin finds it in:

1. the MSFS SDK. In the sim, turn on Developer Mode (Options ▸ General Options ▸ Developers), then
   use Help ▸ SDK Installer in the developer menu. The SDK for your sim (2020 or 2024) is
   preferred while that sim runs;
2. the sim's own folder (the Steam edition of MSFS 2020 has one next to `FlightSimulator.exe`);
3. the plugin's data folder, `%APPDATA%\KnobDeck\plugin-data\flight-sim-windows`: copy a
   `SimConnect.dll` there yourself (for example from another SimConnect app's folder);
4. `SIMCONNECT_DLL=<path>` in the `.env` file of that same folder.

Then start the sim. KnobDeck connects within a few seconds, and reconnects when you restart the
sim. Until then every key says "Flight Simulator isn't running".

## How it works

It is a SimConnect client: it maps K-events by name and sends them to your aircraft, and reads
SimVars through data requests the sim answers every frame they change. It never sends
keystrokes, so it works whatever window has the focus and whatever your key bindings are.
`SimConnect.dll` isn't bundled because the MSFS SDK licence doesn't clearly allow passing it on.
