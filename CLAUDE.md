# KnobDeck plugins (Rust) — working rules

Rust plugins for KnobDeck, built on `knobdeck-sdk` (`../knobdeck-sdk`). KnobDeck runs on **Windows,
macOS and Linux**; see README.md for the plugin list, build and packaging. SDK changes go to the
SDK repo through git (commit and push); plugins depend on its GitHub master, never a local path.

## Rule 1: no keybind actions; only stable routes ship

Applies to every plugin (`../Knobdeck/CLAUDE.md` has the host copy). A plugin action must go
through an OS API or the target app's own interface. **A plugin never ships an action that is a
keyboard shortcut** (`send_keys`, `ctx.send_keys`, `keys_button`, posting CGEvents/SendInput):
KnobDeck's built-in Keybind action already sends any shortcut, and app-specific shortcuts will ship
as predefined profiles. A smaller set of actions beats one that breaks. Shortcuts are fragile:

- the app in focus may bind the same chord and swallow it (e.g. an app using Ctrl+→);
- users remap or disable shortcuts (System Settings → Keyboard Shortcuts, PowerToys…);
- keyboard layouts move keys; injected keys may lack flags real keyboards send (macOS Fn/NumericPad);
- timing and focus changes (the dial, the deck) can send the chord to the wrong window.

Prefer, in this order:

1. **An OS API.** Windows: Win32 / COM / WinRT through the `windows` crate (window management,
   `LockWorkStation`, audio endpoints, clipboard). macOS: public frameworks (Accessibility
   `AXUIElement`, CoreAudio, CoreGraphics, AppKit, Text Input Sources, IOKit). Linux: the
   compositor (KWin scripting, the KnobDeck GNOME Shell extension, via `knobdeck_sdk::os::linux` and `linux-desktop`) and
   D-Bus services (KGlobalAccel, KRunner, Klipper, logind, NetworkManager, BlueZ,
   power-profiles-daemon, MPRIS, FileManager1).
2. **A long-stable private OS entry point**, resolved with `dlsym` / `GetProcAddress` at call time
   so its absence is a clear error and never a crash at launch (e.g. `CoreDockSendNotification`
   for Mission Control, `SACLockScreenImmediate` for Lock Screen).
3. **The target app's own interface**: its local API / socket (OBS websocket, Discord RPC, VLC HTTP,
   the VS Code / IntelliJ companions), its scripting dictionary (AppleScript `do script`,
   iTerm2 `write text`), or pressing its menu item through Accessibility (mac-tools pastes by
   pressing the front app's Edit ▸ Paste item, found by its ⌘V binding, not by sending ⌘V).
4. **Launching the system app / URL** (`open -b <bundle id>`, `ms-settings:` URIs).
When none of these exists, or the route only imitates the feature (resizing a window to "go
fullscreen", restyling another app's window, a history the plugin keeps itself), the action is
**disabled, never deleted**: its code stays and it is offered on no OS with `.platforms(&["none"])`
(the `off()` helper in win-tools / mac-tools / linux-tools / browser-plugin). An empty `platforms`
means "every OS", so it can't disable anything. For a plugin that can't ship at all on an OS, drop
it from that OS in `scripts/plugins.mjs` (Zoom everywhere). Desktop-specific gaps on
Linux (GNOME has no X) can't be expressed in `platforms`: the action fails there with a clear
error and says "(Plasma)" in its label.

Exceptions, deliberately kept:
- Text Snippet, whose job *is* entering text (macOS/Windows type it with Unicode key events; Linux
  pastes it with Ctrl+V because Wayland offers no other way).
- AI Actions' "in place of the selected text", which likewise enters text: Windows and Linux paste
  the answer (clipboard + Ctrl+V: long answers arrive at once, line breaks don't press Enter in
  chat apps), macOS types it like Text Snippet (line breaks as ⇧Return).
- macOS previous/next Space: macOS has no API, so the user's own configured symbolic hotkey is read
  from `com.apple.symbolichotkeys` and posted; WindowServer handles it before any app sees it.

When reviewing or adding an action, check it against this rule.

Current decisions (keep this list up to date):

| Plugin | Route | Keystrokes left, and why |
|---|---|---|
| discord, obs, vlc, vscode, intellij, streamlabs, voicemod, elgato-lights | the app's / device's own API / pipe / websocket / HTTP | none |
| spotify | Windows SMTC, macOS AppleScript and Linux MPRIS for playback; Spotify Web API for the rest | none |
| twitch, youtube, weather, github | web APIs (Helix, YouTube Data, Open-Meteo, GitHub REST / GraphQL); keys open pages in the default browser | none |
| clocks | none needed (time and timers only) | none |
| mac-tools | Dock (`CoreDockSendNotification`), `SACLockScreenImmediate`, Text Input Sources, Accessibility (windows, hide, Edit ▸ Paste, Menu Command presses any menu item), CoreAudio (mic mute and level: the device's own controls only), `NSRunningApplication.forceTerminate` (Force Quit App), Screenshot app, the `shortcuts` CLI (Run Shortcut), `osascript` (Run AppleScript / JXA), Keyboard Maestro Engine's AppleScript `do script`, display brightness through DisplayServices (`DisplayServicesSetBrightness`, built-in and Apple displays) and DDC/CI over `IOAVServiceWriteI2C` (external monitors, Apple silicon) | Spaces ←/→ (no API; the user's configured system hotkey is posted); Text Snippet types its text. Disabled: Spotlight, Force Quit… (⌥⌘Esc), Clipboard History, Switch to Window, Focus App |
| win-tools | Win32 (windows, audio sessions and endpoints, `LockWorkStation`; per-monitor DPI aware), undocumented `IPolicyConfig` for the default audio device (no public API; unchanged since Windows 7), `ShellExecuteW` for Task View / Show Desktop / Run (`shell:::{CLSID}`), `ms-screenclip:`, `ms-settings:`, Explorer, Task Manager, display brightness through WMI `WmiMonitorBrightnessMethods` (laptop panel) and DDC/CI through `dxva2` (external monitors) | Text Snippet types its text (no paste mode). Disabled: virtual desktops (no public API), Emoji Panel, Clipboard History, Borderless / Transparency / Set Opacity / Click-Through (restyle other apps' windows), Fullscreen (only resized the window), Switch to Window, Focus App and Move App Window's focus option (Windows only lets a background process take the foreground by a workaround) |
| linux-tools | KDE: KWin scripts over `org.kde.kwin.Scripting` (windows, desktops), KGlobalAccel `invokeShortcut` (KWin's / PowerDevil's own actions by name: Overview, Expose, Show Desktop, Turn Off Screen), KRunner, Klipper, Spectacle. GNOME: the KnobDeck Shell extension (`knobdeck-os/src/linux/gnome-extension` in the SDK repo: windows, workspaces, overview, search, screenshot UI). Both: logind, NetworkManager, BlueZ, power-profiles-daemon, FileManager1, `pactl`. Brightness: KDE's `org.kde.ScreenBrightness`; elsewhere logind `SetBrightness` (backlight) and DDC/CI on `/dev/i2c-*` | Ctrl+V for Text Snippet (no app-independent paste request on Linux; Wayland lets no client act on another's text field). Disabled: Clipboard History, Switch to Window, Focus App. Plasma only (clear error on GNOME): Show Desktop, Emoji Picker, Turn Off Display, Transparency / Set Opacity; GNOME New / Close Desktop need dynamic workspaces off |
| browser-plugin | the KnobDeck Browser Companion extension (`Knobdeck/knobdeck-browser-companion`, Chrome MV3 + Firefox MV2): the browser connects to the plugin's loopback WebSocket and runs tabs, windows, zoom, bookmarks, history, URLs, print/stop through its own extension APIs, on every OS | none. Disabled: Focus Address Bar, Find in Page, Save Page (no extension API in any browser), Custom Keybind. Open Downloads / History / Bookmarks Manager report "not possible" in Firefox/Zen (extensions can't open `about:` pages) |
| ai-stats | providers' usage endpoints, local transcripts | none |
| home-assistant, hue, ytmusic | the service's / device's / app's own API (HA REST, Hue Bridge local API, YTMDesktop Companion Server + its Socket.IO feed) | none |
| files | the selection from `knobdeck_sdk::os::files` (Explorer Shell COM, Finder Apple Events, Dolphin's AT-SPI tree); images, PDFs and zips in Rust (`image`, `lopdf`, `zip`; HEIC through macOS `sips`); clipboard through Win32 / NSPasteboard / Klipper / the GNOME extension. No terminal or "open with" launching (which apps exist differs per machine) | none |
| game-launcher | the launchers' own files (Steam `libraryfolders.vdf` / `appmanifest_*.acf`, Epic manifests) and URL schemes (`steam://rungameid/…`, `com.epicgames.launcher://apps/…`) | none |
| slides | Windows PowerPoint COM (`SlideShowWindows(1).View`), macOS PowerPoint and Keynote AppleScript, LibreOffice Impress Remote protocol (TCP 1599) | none; Keynote has no blank-screen command, so Black/White Screen report that instead of sending B/W |
| coding-agents | Claude Code hooks and Codex `notify` (setup adds them to the agents' own configs); the hook is this plugin's binary in hook mode, writing small files in the data folder; Codex activity from its session logs | none. No focusing a session's terminal and no starting sessions (KnobDeck doesn't focus other apps; terminals differ per machine) |
| ai-actions | the user's model over its HTTP API (Anthropic Messages, Chat Completions, Ollama); the selection from `knobdeck_sdk::os` | Entering the answer in place of the selection, like Text Snippet (its job is entering text): Windows and Linux paste it (clipboard + Ctrl+V), macOS types it as Unicode key events |
| web-requests | HTTP(S) to the user's own URLs | none |
| now-playing | Windows SMTC (every session), Linux MPRIS (every player), macOS Music and Spotify AppleScript (macOS 15.4 closed MediaRemote to other apps) | none |
| docker | Docker Engine API over the local Unix socket / `docker_engine` named pipe / `DOCKER_HOST` (Podman serves the same API) | none |
| speedtest | Cloudflare's speed test endpoints; latency is a TCP connect time | none |
| nanoleaf, govee | the devices' own local APIs (Nanoleaf OpenAPI; Govee LAN UDP API), Govee's cloud API with the user's key | none |
| wled, lifx | the devices' own local APIs (WLED JSON API over HTTP, found by mDNS `_wled._tcp`; LIFX LAN protocol over UDP 56700, found by broadcast `GetService`) | none |
| voicemeeter | Voicemeeter's Remote API (`VoicemeeterRemote64.dll`, loaded at run time from the install folder in its uninstall registry entry) | none |
| streamerbot | Streamer.bot's WebSocket server (`GetActions`, `DoAction`, `ExecuteCodeTrigger`, `GetGlobal`; SHA-256 challenge auth) | none. No setting globals: the server has no request for it (an action can) |
| apple-music | Music.app's AppleScript dictionary through `osascript` (only Play / Pause and Play Playlist open Music) | none |
| neovim | Neovim's msgpack-RPC API (`nvim_command`, `nvim_exec_lua`, `nvim_get_mode`) on its `--listen` socket / named pipe | none: no `nvim_input` / `nvim_feedkeys` |
| hardware-monitor | read-only sensors: Linux hwmon sysfs, amdgpu device files, RAPL; Windows LibreHardwareMonitor / OpenHardwareMonitor WMI (the user runs it; Windows has no sensor API for apps), `MSAcpi_ThermalZoneTemperature`; macOS `IOHIDEventSystemClient` (private, `dlsym`) and the AppleSMC user client (IOKit); NVIDIA NVML (`nvml.dll` / `libnvidia-ml.so.1`, loaded at run time) | none |
| soundboard | its own playback: WASAPI (Windows) and CoreAudio (macOS) through cpal, the PulseAudio protocol on Linux (pure Rust; PipeWire serves it); Soundpad through its documented remote control pipe `\\.\pipe\sp_remote_control` (Windows) | none |
| teamspeak | TeamSpeak 5/6 Remote Apps WebSocket (`auth` with the key TeamSpeak issues once the user allows KnobDeck; `buttonPress` for named buttons the user binds to TeamSpeak actions under Key Bindings, the only request the API has; state from its events) and TeamSpeak 3 ClientQuery (`clientupdate client_input_muted/output_muted/away/input_deactivated`) | none. TeamSpeak 5/6 can't be muted directly: the user binds each KnobDeck button once in TeamSpeak (a TeamSpeak-side binding, not a keystroke) |
| vtube-studio | VTube Studio's public WebSocket API (`HotkeyTriggerRequest`, `ModelLoadRequest`, `ExpressionActivationRequest`, `MoveModelRequest`, `InjectParameterDataRequest`; token from its Allow popup) | none. Hotkeys are triggered through the API by id, never as key presses |
| blender | the KnobDeck add-on for Blender (`Knobdeck/knobdeck-blender-companion`) over KnobDeck's companion hub; the add-on runs every action with Blender's Python API (`bpy`: `frame_set`, `screen.animation_play` / `keyframe_jump`, `window.workspace`, `SpaceView3D.shading` / `overlay`, `render.render`, `wm.save_mainfile`, `ed.undo` / `redo`, the brush's / unified paint settings, object location / rotation / scale) on Blender's main thread | none |

## Rule 2: every OS stays working

- Cross-platform plugins ship one package per OS: ids `<name>-windows` / `<name>-macos` / `<name>-linux`
  (`format!("<name>-{}", std::env::consts::OS)`), `.platforms(&[std::env::consts::OS])` (actions
  inherit it). CI builds each on its own OS; `scripts/plugins.mjs` lists which plugins ship where.
- **Linux scope:** Debian/Ubuntu and Arch based distributions (not Fedora yet); KDE Plasma 6 and
  GNOME 45+, Wayland and X11. Window and desktop work goes through `linux-desktop`, a thin layer over
  `knobdeck_sdk::os::linux` (the `knobdeck-os` crate in the SDK repo owns `kwin.js` and the GNOME
  extension; keep them in step there and bump the extension's `version` in `metadata.json` when it
  changes). Other desktops get a clear "needs GNOME or KDE
  Plasma" error, never a silent no-op.
- **An app's plugin ships for Linux only when the app itself has an official Linux build** (from
  its vendor, or a distro package of that build), not a third-party client or workaround. Then
  check that the interface the plugin uses exists on Linux too. Not shipped: Streamlabs Desktop,
  Voicemod, Voicemeeter, Streamer.bot and VTube Studio (no Linux build). Zoom isn't shipped anywhere (no documented local API for the
  desktop client). Files ships for Linux as experimental: only Dolphin shares its selection
  (through AT-SPI); GNOME Files doesn't.
- To ship a plugin, bump `version` in its `Cargo.toml` (the only place it is written; `index.json` holds display text only and CI fails if it has a `version`) and merge to master. CI builds only plugins that have no `<dir>-v<version>` release yet and publishes one release per plugin under that tag (notes: the top section of `<dir>/CHANGELOG.md`, if any); the catalog in `knobdeck-registry` lists it (see `.github/workflows/release.yml`). A **new** plugin also needs `catalog/plugins/<dir>.toml` in `knobdeck-registry` (tags, sort, featured as in `index.json`); without it the release exists but Get Plugins never offers it.

- A plugin that runs on both OSes keeps one action set; OS differences live behind one `cfg` seam
  per concern (e.g. `win-tools`-style `win.rs` / `mac.rs` behind `mod os`).
- An OS-only plugin or action declares `.platforms(&["windows"])` / `.platforms(&["macos"])` / `.platforms(&["linux"])`.
  `win-tools` is Windows-only; `mac-tools` and `linux-tools` are its macOS and Linux counterparts.
- Don't regress one OS while working on another. Before finishing, run
  `./scripts/check-windows-from-mac.sh` (Windows type-check, no warnings; from Linux also
  `TARGET=aarch64-apple-darwin` for macOS) and `cargo test --workspace`.
- OS context (the target window, window list, focus, installed apps) comes from `knobdeck_sdk::os` (features
  `os-window`, `os-apps`), read in the plugin's own process; the host sends none. `os::window::target()` is
  the frontmost window that isn't KnobDeck's, so window actions reach the app under the dial even while a
  slider keeps the dial focused. A window's `app_id` is the exe name on Windows, the bundle id on macOS and
  the window class / app id on Linux.
- User-written text (snippets, commands, URLs) goes through `ctx.expand` (SDK feature `placeholders`) and
  its input is marked `.accepts_placeholders()`: one `{selection}` / `{clipboard}` / `{date}` syntax across
  KnobDeck and every plugin. Don't invent another.

## Rule 3: verify, don't assume

- Drive a built plugin over the real stdio protocol (`hello` → `sync` → `request`), not just unit
  tests. Owning FFI wrappers must never be built around NULL (use `.then(|| ..)`, not
  `.then_some(..)`, for anything with a `Drop` that releases).
- Live tests that touch the user's screen (moving windows, opening Mission Control) are
  `#[ignore]` and act only on windows they create themselves, matched exactly (a terminal's title
  can contain the test command). Ask before running anything that takes over the user's display.
  On Linux, resize/maximize results need an unlocked screen: hidden clients don't acknowledge
  configures.

## Rule 4: a change to a plugin bumps its version

- Any code change to a plugin bumps its `version` in `Cargo.toml`.
- CI builds only plugins whose `Cargo.toml` version has no `<dir>-v<version>` release yet, so without the bump your change is not published. A released version is never rebuilt: its `<dir>-v<version>` release stays as it is.
- Optionally add a `## <version>` section at the top of `<dir>/CHANGELOG.md`; it becomes the release notes.
- Icons are not release assets: the registry links `<dir>/icons/<icon>` by raw GitHub URL, so changing an icon file needs no version bump.

## Rule 5: secrets live in the OS credential store

Passwords, tokens, API keys, pairing keys and a user's own client secret never go in `.env` or any
file. Enable the SDK's `secrets` feature (`oauth` includes it): setup calls
`knobdeck_sdk::secrets::set("KEY", &value)` (an empty value removes it), the plugin reads
`knobdeck_sdk::secrets::env_or_stored("KEY")`. Only the plugin binary touches the store (setup's value
reaches it through a user-only hand-off file), so on macOS the Keychain item trusts only the plugin.
When a released setup wrote a secret to `.env`, the plugin calls
`knobdeck_sdk::secrets::migrate_env(&["KEY"])` right after `load_env()`. Host, port and other
settings stay in `.env` (`save_env`). Discord has its own equivalent code in `discord/src/common.rs`.
