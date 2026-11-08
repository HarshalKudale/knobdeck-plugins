# Slides

Present from KnobDeck: move through a slide show, start and end it, jump to a slide, blank the
screen and see where you are.

| App | Windows | macOS | Linux |
|---|---|---|---|
| Microsoft PowerPoint | ✅ | ✅ | — |
| Apple Keynote | — | ✅ | — |
| LibreOffice Impress | ✅ | ✅ | ✅ |

## What it does

- **Next Slide** / **Previous Slide** (next animation step, like the presenter's clicker).
- **Start Slide Show** (from the first slide) and **End Slide Show**.
- **Go to Slide**: jump to a slide number.
- **Black Screen** / **White Screen**: blank the screen and press again to come back. Keynote has
  no command for this; LibreOffice blanks to black only.
- **Slide Counter**: shows "3 / 12" during a show; pressing it goes to the next slide.

Every key has an **App** option. "Whichever is presenting" picks the app running a slide show,
else the open one.

## Setup

- **PowerPoint** and **Keynote**: none. On macOS, the first time, allow KnobDeck to control the app
  (System Settings ▸ Privacy & Security ▸ Automation).
- **LibreOffice Impress**: turn on its remote control once:
  1. Tools ▸ Options ▸ LibreOffice Impress ▸ General: tick **Enable remote control**.
  2. Tools ▸ Options ▸ LibreOffice ▸ Advanced: tick **Enable experimental features**; then under
     LibreOffice Impress ▸ General, tick **Enable insecure WiFi connections** (KnobDeck connects
     over the network socket). Restart LibreOffice.
  3. Press a Slides key. It shows a 4-digit PIN. In LibreOffice, open **Slide Show ▸ Impress
     Remote**, choose **KnobDeck** and enter that PIN. LibreOffice remembers the pairing.

  The remote control listens on port 1599 on every network interface, so other devices on your
  network can also try to pair. Only paired devices are accepted.

## How it works

- Windows PowerPoint: COM automation of the running PowerPoint (`SlideShowWindows(1).View`:
  `Next`, `Previous`, `GotoSlide`, `Exit`, `State`), the same object model as its macros.
- macOS PowerPoint and Keynote: their AppleScript dictionaries (`go to next slide`, `show next`…).
- LibreOffice Impress: the Impress Remote protocol (the one LibreOffice's phone remote apps use)
  on `127.0.0.1:1599`, with one connection kept open.

No keyboard shortcuts are used, so the keys work even when the presentation window isn't in
front. The plugin never starts an app: open your presentation first.
