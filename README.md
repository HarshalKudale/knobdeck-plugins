# KnobDeck Plugins

The first-party [KnobDeck](https://knobdeck.harshalkudale.com) plugins, written in Rust on
[`knobdeck-sdk`](https://github.com/HarshalKudale/knobdeck-sdk). The repo is open so anyone can
read how they work and build their own; to start a new plugin, use `knobdeck-plugin new` from the
SDK (it includes an AI agent skill). Action ids are saved in users' layouts, so they never change.

Every cross-platform plugin ships as one package per OS (`<id>-windows`, `<id>-macos`, `<id>-linux`), each built on its own OS and declaring its `platforms`. `win-tools`, `mac-tools` and `linux-tools` are OS-specific. KnobDeck only offers and installs packages for its own OS.

**Linux** (Debian/Ubuntu and Arch based; KDE Plasma 6 and GNOME 45+, Wayland and X11): an app's
plugin ships for Linux only when the app has an official Linux build. Streamlabs Desktop and
Voicemod have none; Zoom's client does, but the local meeting API the plugin uses isn't documented
for it, so Zoom stays Windows/macOS for now. `linux-desktop` is a shared library crate (not a
plugin): the KWin / GNOME Shell window bridge, D-Bus helpers and desktop entries.

| Plugin | Description | OS |
|--------|-------------|----|
| [discord](./discord) | Discord voice controls over the desktop client's local RPC (named pipe on Windows, unix socket on macOS and Linux) | Windows, macOS, Linux |
| [browser-plugin](./browser-plugin) | Chrome / Edge / Brave / Firefox / Zen shortcuts, wheels, bookmarks, history (per-OS install paths, profiles and shortcuts) | Windows, macOS, Linux |
| [intellij](./intellij) | JetBrains IDE actions via the KnobDeck IntelliJ companion plugin | Windows, macOS, Linux |
| [obs](./obs) | OBS Studio control via obs-websocket v5 (own client, reconnecting) | Windows, macOS, Linux |
| [vscode](./vscode) | VS Code controls via the KnobDeck VS Code companion extension | Windows, macOS, Linux |
| [vlc](./vlc) | VLC media controls via its HTTP interface | Windows, macOS, Linux |
| [zoom](./zoom) | Zoom meeting controls via the local API | Windows, macOS |
| [win-tools](./win-tools) | Window management, per-app volume, clipboard history (`windows` crate) | Windows |
| [mac-tools](./mac-tools) | Window management (Accessibility API), window switcher, Spaces/system shortcuts, stats, audio devices + mic mute, clipboard history | macOS |
| [linux-tools](./linux-tools) | Window management (KWin scripting / KnobDeck GNOME extension), workspaces, desktop actions, stats, per-app volume, audio devices, power profiles, Wi-Fi/Bluetooth, clipboard history | Linux (KDE, GNOME) |
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
| [game-launcher](./game-launcher) | Launch installed Steam games (every OS) and Epic Games Store games (Windows, macOS) from a key or wheel; launcher pages | Windows, macOS, Linux |
| [ytmusic](./ytmusic) | YouTube Music Desktop App (YTMDesktop 2.x) Companion Server: playback, likes, volume, repeat, queue, playlists | Windows, macOS, Linux |
| [slides](./slides) | Slide shows: PowerPoint (COM on Windows, AppleScript on macOS), Keynote (macOS), LibreOffice Impress (Impress Remote protocol, every OS) | Windows, macOS, Linux |

The `vscode` and `intellij` plugins talk to companions that run *inside* VS Code / JetBrains IDEs
(an editor extension and an IDE plugin, not KnobDeck SDK plugins). They live in
[knobdeck-companion](https://github.com/HarshalKudale/knobdeck-companion), whose CI builds these two
plugins from this repo and drives them against the companions.

Spotify, Twitch, Voicemod and YouTube take app credentials at build time from repository variables
(`SPOTIFY_CLIENT_ID`, `TWITCH_CLIENT_ID`, `VOICEMOD_CLIENT_KEY`, `GOOGLE_CLIENT_ID`,
`KNOBDECK_GOOGLE_TOKEN_URL`); without them the plugin's setup asks for the user's own. No client
secret is built into any plugin.

## Build

```bash
cargo install --git https://github.com/HarshalKudale/knobdeck-sdk --bin knobdeck-plugin   # once

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
