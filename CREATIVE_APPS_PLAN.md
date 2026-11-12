# Plan: Adobe and Figma plugins (stateful)

Status: plan, nothing built. Written 2026-11-08.

## Goal

Every competitor's "works with Photoshop / Figma" is the app's shortcuts sent to the focused
window. We talk to the app itself, so actions can **read state** (current tool, brush size,
selected layer, selection's opacity) and **set values** (a dial turns brush size and the tile
shows the number), and they keep working when the user remaps shortcuts.

Keybinds still ship, as **Keybind presets** per app (allowed by Rule 1: the user's own Keybind
action and its presets) for whatever the API can't reach. On macOS, mac-tools' **Menu Command**
already presses any menu item of these apps without a companion.

## Constraint that shapes the design

Neither host lets a plugin accept connections:

- **UXP** (Photoshop, InDesign, Premiere): WebSocket *client* only; the endpoint must be listed in
  `manifest.json` → `requiredPermissions.network.domains`. Listening needs a C++ "hybrid" plugin.
- **Figma**: plugin code runs sandboxed; its UI iframe can open a WebSocket to domains listed in
  `networkAccess`. No listening.

So the direction is the reverse of the VS Code plugin (there the extension is the server):

```
KnobDeck ──stdio── <app> plugin (Rust)  ── ws://localhost:<port> ──  companion inside the app
                    hosts the socket                                 (UXP panel / Figma plugin /
                                                                     Lightroom Lua) connects out
```

## Shared piece: a bridge server in the SDK

Add `knobdeck_sdk::bridge` (cargo feature `bridge`) so every companion-based plugin, ours or one an
AI agent writes with the skill, gets the same thing:

- WebSocket server on `127.0.0.1` (plus `::1`, since `localhost` may resolve to either), fixed
  default port per plugin, overridable in setup.
- Pairing: the plugin generates a token on first run and shows it in setup; the companion sends
  `hello {app, appVersion, companionVersion, token}`. Wrong token → closed. Only one client per
  app instance; a new one replaces the old.
- Messages (JSON): `call {id, method, params}` → `result {id, ok, value | error}`, and companion
  → plugin `event {name, data}` for state pushes. Timeouts per call (default 2 s).
- `bridge.connected()` drives visuals: actions show "not connected" instead of failing silently,
  and pressing one returns "Open the KnobDeck panel in Photoshop" (Rule 2: a clear error, not `Ok`).
- Tests with a fake client; `knobdeck-plugin drive` gets a `bridge` stub step.

Then add the pattern to the agent skill (`references/examples.md`): "app with a scripting host
that can't listen".

## Per app

| App | Route | State it can push | Notes |
|---|---|---|---|
| **Photoshop** | UXP panel plugin; `batchPlay` + DOM (`app.activeDocument`, layers) | tool changes, selection, active layer, document switch (`action.addNotificationListener`) | First target. UXP is mature here. |
| **Figma** (desktop) | Figma plugin; worker does `figma.*`, UI iframe holds the socket | `selectionchange`, `currentpagechange`, `documentchange` | Must be started by the user each session (Plugins menu, or mac-tools Menu Command "Plugins ▸ KnobDeck"). One plugin runs at a time. |
| **Lightroom Classic** | Lua SDK plugin; `LrSocket` + `LrDevelopController` (what MIDI2LR uses) | develop values, selected photo, module | Best dial fit (every develop slider). Lua can open the socket itself; sockets are send/receive pairs. |
| **Premiere** | UXP (GA since 25.6) | playhead, active sequence, selection | Older versions: skip, don't build CEP. |
| **InDesign** | UXP | selection, page | Low priority. |
| **After Effects, Illustrator** | Wait for UXP (Adobe: AE public beta targeted for Nov 2026; Illustrator later). Until then, menu commands and Keybind presets only. | | Don't start on CEP/ExtendScript: Adobe is removing CEP after the UXP transition. |

**Without a companion** (one-shot, no events): Windows COM `Photoshop.Application.DoJavaScript`,
macOS AppleScript `do javascript`. Useful for a "companion not running" fallback on simple
buttons, not for dials (each call is slow). Decide in phase 0 whether it is worth it.

**Platforms**: Adobe apps are Windows and macOS only → `platforms: ["windows", "macos"]`. Figma
desktop is Windows and macOS; Figma in a browser on Linux could work if the browser lets
`figma.com`'s plugin iframe reach `localhost` (Chrome's local network access rules may block it).
Check in phase 0; if it fails, Linux is hidden, not broken.

## First actions

**Photoshop**
- Tool wheel: tools with their icons, the current one highlighted; buttons for single tools.
- Dials: brush size, hardness, opacity, flow; layer opacity and fill; zoom; history step
  (undo/redo by turning). Tile shows the live value.
- Layer: visibility, lock, new, duplicate, merge down, blend mode wheel.
- Run Action: set and action picked from a query of the Actions panel.
- Menu command by id (`batchPlay` `select` / `menuCommand`) on Windows too, with a query listing
  common ones.

**Figma**
- Dials on the selection: opacity, corner radius, stroke weight, rotation, auto-layout gap and
  padding, font size. Tile shows the value, or "mixed".
- Buttons: align / distribute, group / frame / auto-layout, flip, component create / detach.
- Wheels: swap instance among variants of its component, pages, run another plugin's command
  (if the API allows; otherwise via Menu Command on macOS).

**Lightroom Classic**: every develop slider as a dial (exposure, contrast, highlights, shadows,
whites, blacks, temp, tint, …), next / previous photo, rating, flag, reset slider, module switch.

**Premiere**: playhead jog dial (frames) and shuttle, add marker, razor at playhead, clip speed,
track mute / solo / lock toggles, sequence zoom.

## Packaging

- Each app is one directory in this repo: the Rust plugin plus `companion/` (UXP sources →
  `.ccx`; Figma `manifest.json` + `code.js` + `ui.html`; Lightroom `.lrplugin`). CI builds the
  companion with the plugin; the plugin's `setup.html` links the install.
- Distribution: Adobe Exchange / Creative Cloud for the `.ccx`; Figma Community for the Figma
  plugin (until published: "Import plugin from manifest" in the desktop app); Lightroom: copy the
  `.lrplugin` and add it in Plug-in Manager (setup can do the copy).

## Phases

0. **Spikes** (a day each, decide go/no-go):
   - Photoshop current version: UXP panel connects to `ws://localhost:<port>` (list both
     `localhost` and `ws://localhost:<port>`; reports conflict on whether localhost works).
   - Figma: a *published* or private-org plugin with `ws://localhost:<port>` in
     `networkAccess.allowedDomains` (with `reasoning`) passes review and connects. Dev imports
     only accept `localhost` host names, not `127.0.0.1`.
   - Figma in Chrome/Firefox on Linux reaching localhost.
   - Latency: a dial at 30 ticks/s round-trips under ~30 ms in Photoshop (`batchPlay`) and Figma.
1. `knobdeck_sdk::bridge` + skill update (SDK minor release).
2. Photoshop plugin + UXP companion.
3. Figma plugin + companion.
4. Lightroom Classic.
5. Premiere.
6. After Effects / Illustrator when their UXP public betas are usable.

## Open questions

- Port and token UX: one shared "KnobDeck bridge" port for all apps, or one per plugin? (Per
  plugin is simpler to reason about and matches plugin isolation; start there.)
- Dial rate: coalesce ticks in the Rust plugin (send the latest absolute value, drop stale ones)
  so slow hosts don't build a backlog.
- Whether Adobe Exchange review accepts a panel whose only job is a localhost bridge.

Sources checked 2026-11-08: Figma plugin manifest and network docs (developers.figma.com),
Adobe UXP network recipes (developer.adobe.com/photoshop/uxp, /premiere-pro/uxp), Adobe developer
blog on the CEP → UXP timeline (Sep 2026), the Lightroom Classic SDK "Externally Controlling
Lightroom Classic" addendum (13.3).
