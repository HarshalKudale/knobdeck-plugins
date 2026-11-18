# KnobDeck Plugins

The first-party [KnobDeck](https://knobdeck.harshalkudale.com) plugins, written in Rust on
[`knobdeck-sdk`](https://github.com/Knobdeck/knobdeck-sdk). The repo is open so anyone can
read how they work and build their own; to start a new plugin, use `knobdeck-plugin new` from the
SDK (it includes an AI agent skill). Action ids are saved in users' layouts, so they never change.

Every cross-platform plugin ships as one package per OS (`<id>-windows`, `<id>-macos`, `<id>-linux`), each built on its own OS and declaring its `platforms`. `win-tools`, `mac-tools` and `linux-tools` are OS-specific. KnobDeck only offers and installs packages for its own OS.

**Linux** (Debian/Ubuntu and Arch based; KDE Plasma 6 and GNOME 45+, Wayland and X11): an app's
plugin ships for Linux only when the app has an official Linux build. Streamlabs Desktop and
Voicemod have none. Files on Linux works only with Dolphin (experimental; GNOME Files doesn't
share its selection).
`linux-desktop` is a shared library crate (not a
plugin): the KWin / GNOME Shell window bridge, D-Bus helpers and desktop entries.

| Plugin | Description | OS |
|--------|-------------|----|
| [discord](./discord) | Discord voice controls over the desktop client's local RPC (named pipe on Windows, unix socket on macOS and Linux) | Windows, macOS, Linux |
| [browser-plugin](./browser-plugin) | Chrome / Edge / Brave / Firefox / Zen tabs, windows, zoom, wheels, bookmarks, history through the KnobDeck browser extension | Windows, macOS, Linux |
| [intellij](./intellij) | JetBrains IDE actions via the KnobDeck IntelliJ companion plugin | Windows, macOS, Linux |
| [obs](./obs) | OBS Studio control via obs-websocket v5 (own client, reconnecting) | Windows, macOS, Linux |
| [vscode](./vscode) | VS Code controls via the KnobDeck VS Code companion extension | Windows, macOS, Linux |
| [vlc](./vlc) | VLC media controls via its HTTP interface | Windows, macOS, Linux |
| [zoom](./zoom) | Zoom meeting controls. **Not shipped:** Zoom has no documented local API for its desktop client | — |
| [win-tools](./win-tools) | Window management, per-app volume, audio devices + mic, system actions (`windows` crate) | Windows |
| [mac-tools](./mac-tools) | Window management (Accessibility API), window switcher, Spaces/system actions, stats, audio devices + mic mute and level, Menu Command, automation | macOS |
| [linux-tools](./linux-tools) | Window management (KWin scripting / KnobDeck GNOME extension), workspaces, desktop actions, stats, per-app volume, audio devices, power profiles, Wi-Fi/Bluetooth | Linux (KDE, GNOME) |
| [ai-stats](./ai-stats) | Claude and Codex plan limits (5 h / weekly gauges) and 30-day token usage, from the CLIs' local logins and transcripts | Windows, macOS, Linux |
| [elgato-lights](./elgato-lights) | Key Light, Ring Light and Light Strip over their local HTTP API, found by mDNS | Windows, macOS, Linux |
| [weather](./weather) | Current weather, 7-day and hourly forecast, air quality (Open-Meteo, no key) | Windows, macOS, Linux |
| [clocks](./clocks) | Clock for any time zone, world clock wheel, countdown timer, stopwatch | Windows, macOS, Linux |
| [streamlabs](./streamlabs) | Streamlabs Desktop over its local API (go live, record, replay, scenes, audio) | Windows, macOS |
| [voicemod](./voicemod) | Voicemod Control API: voices, voice changer, hear myself, soundboard | Windows, macOS |
| [spotify](./spotify) | Playback through the OS (SMTC / AppleScript / MPRIS); likes, shuffle, repeat, volume, playlists through the Web API | Windows, macOS, Linux |
| [twitch](./twitch) | Helix API: chat, chat modes, ads, stream info, markers, clips, viewers, channel monitor | Windows, macOS, Linux |
| [youtube](./youtube) | YouTube Data API: viewers, chat, ads, dashboard, go live / end stream | Windows, macOS, Linux |
| [home-assistant](./home-assistant) | Home Assistant REST API: toggle entities, scenes and scripts, sensor states, level dial, any service, entities wheel | Windows, macOS, Linux |
| [hue](./hue) | Philips Hue Bridge local API: power, brightness, color temperature, color, scenes (mDNS discovery, link-button pairing) | Windows, macOS, Linux |
| [files](./files) | Act on the files selected in File Explorer, Finder or Dolphin: convert, resize, rotate and clean images; merge and split PDFs; zip, unzip, rename, move, copy paths, new file; a wheel that fits the selection | Windows, macOS, Linux (Dolphin only, experimental) |
| [game-launcher](./game-launcher) | Launch installed Steam games (every OS) and Epic Games Store games (Windows, macOS) from a key or wheel; launcher pages | Windows, macOS, Linux |
| [ytmusic](./ytmusic) | YouTube Music Desktop App (YTMDesktop 2.x) Companion Server: playback, likes, volume, repeat, queue, playlists | Windows, macOS, Linux |
| [slides](./slides) | Slide shows: PowerPoint (COM on Windows, AppleScript on macOS), Keynote (macOS), LibreOffice Impress (Impress Remote protocol, every OS) | Windows, macOS, Linux |
| [coding-agents](./coding-agents) | Claude Code and Codex session status (working, needs approval, your turn) through their hooks / `notify`; approve or deny Claude Code permission requests from a key | Windows, macOS, Linux |
| [ai-actions](./ai-actions) | Fix, rewrite, translate, summarize or explain the selected text with the user's own model (Anthropic, OpenAI-compatible, Ollama); the answer replaces the text, goes on the clipboard or in a notification | Windows, macOS, Linux |
| [web-requests](./web-requests) | HTTP requests and webhooks with placeholders, a live JSON value on a key, an on/off toggle; tokens in the credential store | Windows, macOS, Linux |
| [now-playing](./now-playing) | Any media player's title, artist and cover, play/pause, next, previous, position: Windows SMTC, Linux MPRIS, macOS Music and Spotify (AppleScript) | Windows, macOS, Linux |
| [docker](./docker) | Docker Engine API on the local socket / named pipe (Docker Desktop, Engine, rootless, Colima, OrbStack, Podman): containers and Compose projects | Windows, macOS, Linux |
| [speedtest](./speedtest) | Download / upload / latency against Cloudflare's speed test servers; live latency to any host | Windows, macOS, Linux |
| [nanoleaf](./nanoleaf) | Nanoleaf OpenAPI: power, brightness, color temperature, color, effects (mDNS discovery, power-button pairing) | Windows, macOS, Linux |
| [github](./github) | GitHub REST and GraphQL APIs (device sign-in or a personal access token): notifications, review requests, your open PRs, workflow status, re-run failed jobs, run workflow, repo counters | Windows, macOS, Linux |
| [govee](./govee) | Govee LAN API (UDP), or Govee's cloud API with the user's key for other models and scenes | Windows, macOS, Linux |
| [wled](./wled) | WLED JSON API: power, brightness, presets, effects (picker, dial, wheel) with speed and intensity, palettes, color, segment selection (mDNS discovery) | Windows, macOS, Linux |
| [lifx](./lifx) | LIFX LAN protocol (UDP 56700): power, brightness, white temperature, hue, saturation, color (broadcast discovery, no account) | Windows, macOS, Linux |
| [voicemeeter](./voicemeeter) | Voicemeeter Remote API (`VoicemeeterRemote64.dll`): strip / bus mute and gain, A1–A5 / B1–B3 routing, Macro Buttons, load configuration; Standard, Banana, Potato | Windows |
| [hardware-monitor](./hardware-monitor) | CPU / GPU temperatures, fans, power, GPU load and VRAM: Linux hwmon + amdgpu, LibreHardwareMonitor's WMI on Windows, Apple silicon HID sensors and the SMC on macOS, NVIDIA through NVML | Windows, macOS, Linux |
| [streamerbot](./streamerbot) | Streamer.bot WebSocket server: run actions (with arguments) and code triggers, an action-group wheel, global variables on a key | Windows |
| [apple-music](./apple-music) | Music app (AppleScript): play/pause, next, previous, shuffle, repeat, favorite, volume dial, playlists, now playing with artwork | macOS |
| [neovim](./neovim) | Neovim over its msgpack-RPC server socket / pipe: Ex commands, Lua, save all, buffer dial, option toggles, mode and file | Windows, macOS, Linux |
| [soundboard](./soundboard) | Play mp3 / wav / ogg / flac files from a key to any output, or two at once (a virtual cable into your mic plus your headphones); master volume dial; Soundpad on Windows | Windows, macOS, Linux |
| [blender](./blender) | Blender through the KnobDeck add-on (companion hub, `bpy`): play / pause, frame dial, jumps, workspaces, shading, overlays, X-ray, render, save, undo, mode, brush size / strength and object transform dials, live mode / object key | Windows, macOS, Linux |

The `vscode`, `intellij`, `browser-plugin` and `blender` plugins talk to companions that run
*inside* VS Code, JetBrains IDEs, browsers and Blender (an editor extension, an IDE plugin, a
browser extension and a Blender add-on, not KnobDeck SDK plugins). Each lives in its own repo, listed in
[knobdeck-companion](https://github.com/Knobdeck/knobdeck-companion):
[vscode](https://github.com/Knobdeck/knobdeck-vscode-companion),
[intellij](https://github.com/Knobdeck/knobdeck-intellij-companion),
[browser](https://github.com/Knobdeck/knobdeck-browser-companion),
[blender](https://github.com/Knobdeck/knobdeck-blender-companion). Each one's CI builds its plugin
from this repo and drives it against the companion.

Spotify, Twitch, Voicemod and YouTube take app credentials at build time from repository variables
(`SPOTIFY_CLIENT_ID`, `TWITCH_CLIENT_ID`, `VOICEMOD_CLIENT_KEY`, `GOOGLE_CLIENT_ID`,
`KNOBDECK_GOOGLE_TOKEN_URL`); without them the plugin's setup asks for the user's own. No client
secret is built into any plugin.

## Build

```bash
cargo install --git https://github.com/Knobdeck/knobdeck-sdk --bin knobdeck-plugin   # once

cd vlc
knobdeck-plugin dev        # debug build -> dist/vlc/ ("Load Unpacked" in KnobDeck)
knobdeck-plugin build      # release build -> self-contained dist/vlc/
knobdeck-plugin package    # dist/vlc.knobdeck
```

All crates share one Cargo workspace (`cargo build --workspace`, one `target/`).

The workspace depends on the SDK's git master (`[workspace.dependencies]` in `Cargo.toml`).
`knobdeck-plugin drive [steps.json]` (in a plugin folder, after `dev`) runs it over the real
protocol without KnobDeck; `scripts/drive.mjs` does the same from Node.

### Checking Windows from a Mac (or Linux)

```bash
rustup target add x86_64-pc-windows-msvc   # once
./scripts/check-windows-from-mac.sh        # type-checks every plugin for Windows (no linking)
TARGET=aarch64-apple-darwin ./scripts/check-windows-from-mac.sh   # macOS, from Linux
```

### Building on Linux

`knobdeck-plugin package` writes the zip with `bsdtar` (Debian/Ubuntu: `libarchive-tools`, Arch:
`libarchive`). CI builds the Linux packages on Ubuntu 22.04, so they run on Debian 12+, Ubuntu
22.04+ and Arch.

## Notes

- **Native executables.** Each plugin ships as a native executable (plus a `<id>-setup` binary for plugins with setup).
- **Config and credentials.** Passwords, tokens and API keys never go in files: setup hands them over with
  `knobdeck_sdk::secrets::set` and the plugin binary keeps them in the OS credential store (macOS Keychain,
  Windows Credential Manager, Linux Secret Service; Discord does the same with its own code). A plugin whose
  older setup wrote a secret to `.env` calls `secrets::migrate_env(&[..])` at start-up to move it. Other
  settings: setup writes `.env` into the plugin data folder (`%APPDATA%/KnobDeck/plugin-data/<id>/` on Windows, `~/Library/Application Support/KnobDeck/plugin-data/<id>/` on macOS, the host's config folder on Linux, via `ctx.data_dir()`), which survives updates and reinstalls; the plugin reads it at start-up (`load_env()`, which also moves a `.env` left in the plugin folder by an older version). On an auth failure a plugin calls `mark_setup_required(reason)` and KnobDeck asks the user to run setup again. Discord ships no client secret: it signs in with PKCE as a public client and renews tokens with Discord directly.
- **OBS** subscribes to the non-high-volume event set (no input-volume-meter spam, which the plugin never used).
- **Browser plugin** reads bookmarks/history from a *copy* of the browser's SQLite file (bundled SQLite via `rusqlite`), because running browsers lock the original.

## Verification

- Automated: SDK protocol test against a mock host, OBS client against a mock obs-websocket server (auth hash cross-checked with `obs-websocket-js`), Discord IPC handshake against a live client (`cargo test -p discord -- --ignored`), Win32/WASAPI/icon smoke tests.
- **Not exercised end-to-end** against the real services: VLC, Zoom, VS Code/IntelliJ companions, a real OBS instance, an authenticated Discord session, browser bookmarks/history reads, clipboard history, and the host app loading the generated `dist/`.
