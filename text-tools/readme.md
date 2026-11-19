# Text Tools

Work on the selected text from a key. Windows, macOS and Linux.

## What it does

- **Transform Text** (or the **Text Tools Wheel** with all of them): UPPERCASE, lowercase,
  Title Case, Trim Spaces (both ends, and trailing spaces on every line), Slugify
  ("Crème brûlée!" → `creme-brulee`), URL Encode / Decode, Base64 Encode / Decode (standard or
  URL-safe, padded or not), Format JSON and Minify JSON (key order kept).
  The result goes on the clipboard, or in place of the selected text.
- **Word Count**: words and characters (or lines) of the selection, on the key.
- **Save to Slot** / **Paste Slot**: nine numbered slots. Save puts the selected text in slot N;
  Paste enters slot N where the cursor is (or puts it on the clipboard). Slots are kept across
  restarts and each key shows the start of its slot. It is not a clipboard history: a slot holds
  only what you saved to it.

When nothing is selected, every action works on the clipboard's text instead (turn that off per
key).

## Setup

None. On macOS, entering text in place of the selection needs KnobDeck allowed under System
Settings ▸ Privacy & Security ▸ Accessibility.

## How it works

The selection is read in the plugin through the OS (UI Automation on Windows, Accessibility on
macOS, AT-SPI on Linux). The clipboard is written through Win32, `NSPasteboard`, Klipper (KDE
Plasma) or the KnobDeck GNOME Shell extension. "In place of the selected text" enters the result
the way AI Actions does: Windows and Linux paste it (clipboard + Ctrl+V; Wayland lets no app type
into another), macOS types it as Unicode key events. That is the only input this plugin produces.
Slots are saved in the plugin's data folder (`slots.json`), as plain text: don't keep passwords
in them.
