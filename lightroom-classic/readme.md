# Lightroom Classic

Edit in Adobe Lightroom Classic from KnobDeck: every Develop slider on a dial, with its live value
on the tile, and keys for modules, photos, flags, ratings, labels and undo. Windows and macOS.

It works through the **KnobDeck plug-in for Lightroom Classic** (`KnobDeck.lrplugin`, from
[knobdeck-lightroom-companion](https://github.com/Knobdeck/knobdeck-lightroom-companion)), which
runs each action with Lightroom's own SDK. No keyboard shortcuts are sent.

## Setup

1. Install this plugin and run its setup. With **Install the KnobDeck plug-in** ticked (the
   default), setup downloads the latest `KnobDeck.lrplugin` into Lightroom's Modules folder,
   where Lightroom loads it by itself:
   - Windows: `%APPDATA%\Adobe\Lightroom\Modules`
   - macOS: `~/Library/Application Support/Adobe/Lightroom/Modules`

   Or install it yourself: download `KnobDeck.lrplugin.zip` from the
   [releases](https://github.com/Knobdeck/knobdeck-lightroom-companion/releases), unzip it
   somewhere it can stay, and add the folder in Lightroom with **File ▸ Plug-in Manager ▸ Add**.
   Then run setup with the box unticked.
2. Restart Lightroom Classic (10 or newer).
3. The first time KnobDeck links, Lightroom asks **Allow KnobDeck to control Lightroom Classic?**
   Click **Allow**. Lightroom remembers it; **File ▸ Plug-in Manager ▸ KnobDeck ▸ Forget KnobDeck**
   undoes it. Setup's **Pair again** makes KnobDeck ask once more (with a new pairing key).

The link uses ports **58770** (commands) and **58771** (events) on this computer only. If another
app uses them, change both in Lightroom (Plug-in Manager ▸ KnobDeck, then Reload Plug-in) and in
this plugin's setup.

## What it does

- **Dials for the Basic panel**: Temp, Tint, Exposure, Contrast, Highlights, Shadows, Whites,
  Blacks, Texture, Clarity, Dehaze, Vibrance, Saturation. **Develop Slider**: one dial for any
  other setting: Tone Curve, HSL (hue / saturation / luminance per colour), Color Grading,
  Detail, Lens Corrections, Transform, Effects, Calibration.
  - Turning moves the setting by a fine step (Exposure 0.05, Temp 50 K on raw photos, most others
    1) or by the step you type; or pick **Lightroom's own steps** (as its + / - keys).
  - Pressing the dial resets the setting. The tile shows the photo's current value, also when you
    change it in Lightroom.
  - The dial opens Develop if another module is showing and, unless turned off, shows the
    setting's panel.
- **Module** (one of Library, Develop, Map, Book, Slideshow, Print, Web; lit while it is current)
  and a **Modules** wheel. **Lightroom Status** shows the module and the photo.
- **Next Photo**, **Previous Photo**, **Flag** (pick / reject; pressing again unflags; shows the
  photo's flag), **Rating** (0 to 5 stars, or one more / less; shows the stars) and a **Rating
  (dial)**, **Color Label** (red, yellow, green, blue, purple toggle, or none; lit when the photo
  has it), **Rotate Left / Right**.
- **Copy Settings** / **Paste Settings**: the global adjustments of the photo, pasted on every
  selected photo (not the crop, orientation, healing, red eye or masks, which belong to one photo).
- **Reset All Settings**, **Auto Tone**, **Auto White Balance**, **Before / After** (toggle, or
  a chosen view), **Develop Tool** (Crop, Healing, Red Eye, Masking, none), **Show Develop
  Panel**, **Undo**, **Redo**.

Not offered: anything Lightroom has only as a keyboard shortcut and not in its SDK.

## How it works

Lightroom's plug-in sockets (`LrSocket`) listen on localhost and each one only sends or only
receives, so the plug-in listens on two ports and this plugin connects to both; messages are
tab-separated lines. Before anything else the plugin sends its pairing key; Lightroom accepts only
the key you allowed, so other programs on the computer can't drive it. The plug-in reports the
module, the photo's flag / rating / label, and the Develop values (Lightroom's adjustment
observer) as they change. Fast dial turns are combined, so Lightroom never falls behind.

When Lightroom isn't running, or the plug-in isn't installed, keys say so ("Lightroom Classic
isn't linked") and the status key shows "Lightroom offline".
