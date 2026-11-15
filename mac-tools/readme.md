# Mac Tools

Window management and system tools for KnobDeck on **macOS** — the macOS counterpart of Win Tools.

## What it does

- **Current window:** minimize, fill the screen, restore, close, bring to front, hide the app,
  Fill / Restore and native Full Screen toggles (their state follows the focused window).
- **Position:** halves, quarters, thirds, center, fill a display, custom position/size, and move
  to the next display. Every action can target the current display or a chosen one.
- **Dynamic wheels:** pick any open window (with its app icon) and apply one of the actions above.
  **Switch to Window** brings the chosen window forward, including minimized ones.
- **Spaces and system:** previous/next Space, Mission Control, App Exposé, Show Desktop,
  Screenshot, Emoji & Symbols, Hide Others, Force Quit App (a wheel of running apps), Lock Screen,
  Sleep Display, and Finder / System Settings / Activity Monitor.
- **System Stats:** live CPU, GPU or memory use on the tile; pressing it opens Activity Monitor.
- **Audio:** Mute Microphone toggle and Microphone Level slider (for microphones that have those
  controls), and wheels to switch the output or input device.
- **Text Snippet:** types saved text into whatever has focus, or with Paste mode on, copies it to
  the clipboard and presses the app's Edit ▸ Paste.
- **Menu Command:** presses any item of an app's menu bar — pick the app and the command
  (File ▸ Export ▸ PNG…) from a list of its menus, or type the path. It runs the menu item itself,
  so it works when the item has no shortcut or you've changed it. Choose **App in front** to run
  the same command in whichever app is in front (type the path, since the list needs a chosen app).
- **Run Shortcut:** runs one of your Apple Shortcuts. Its input can be the selected text, the files
  selected in Finder, the clipboard, or text you write (with placeholders such as `{date}`), and
  its result can be pasted, copied or shown as a notification.
- **Run AppleScript:** a script written in the action (AppleScript or JavaScript) or a script file.
  Placeholders in the script become quoted strings (`{selection}` → `"the text"`, `{files}` → a
  list of paths), and the chosen input reaches the script's `on run argv`. The result can be
  pasted, copied or shown.
- **Run Keyboard Maestro Macro:** pick a macro from Keyboard Maestro's list; the chosen input is
  its parameter (`%TriggerValue%`). Needs Keyboard Maestro.

## Permissions

- **Accessibility** (already granted to KnobDeck) — moving, resizing and reading windows.
- Shortcut actions are sent by KnobDeck itself, like its built-in key actions.
- **Automation**, once per app the first time an action controls it: Finder (to read its
  selected files), Keyboard Maestro Engine, and any app your scripts talk to. Allow it when asked,
  or later in System Settings ▸ Privacy & Security ▸ Automation.

## Differences from Win Tools

macOS doesn't let one app change another app's window level, opacity, frame style or mouse
handling, or set per-app volume without installing an audio driver. So **Always on Top, Opacity,
Transparency, Click-Through, Borderless, Send to Back and per-app volume are not included**,
rather than shipped as actions that silently do nothing.

- "Maximize" fills the display's visible area (the menu bar and Dock stay visible). Native full
  screen (its own Space) is the separate **Full Screen** toggle. Windows that size in steps, such as
  Terminal, may stop a row short of the bottom.
- Fill / Restore and Restore go back to the size a window had before Mac Tools moved it; a window
  Mac Tools didn't move reports that instead of jumping to an invented size.
- System actions call macOS directly instead of pressing shortcuts, so the app in front can't
  intercept them and remapped shortcuts don't break them: Mission Control, App Exposé and Show
  Desktop ask the Dock, Lock Screen uses the system lock call, Emoji & Symbols opens the Character
  Viewer, Hide Others, Paste and Menu Command go through Accessibility (Paste presses the app's own Edit ▸ Paste),
  and Screenshot opens the Screenshot app.
- macOS has no API to switch Spaces, so previous/next Space post **your** configured system
  shortcut (read from Keyboard Shortcuts; macOS handles it before any app sees it) and report it if
  you turned it off. Nothing else presses a shortcut: Spotlight and the like are left to KnobDeck's
  own Keybind action.
- GPU use is read from the graphics driver's statistics. Machines without a GPU driver (VMs) show "n/a".
