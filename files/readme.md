# Files

> **Linux: experimental.** On Linux this only works with Dolphin (KDE), which shares its
> selection through the accessibility interface (AT-SPI). Other file managers (GNOME Files, Nemo…)
> don't, and the plugin says so.

Act on the files you've selected in File Explorer (or on the desktop), Finder or Dolphin. Select
the files, then press a key or open the Files wheel.

## What it does

- **Convert Images**: to PNG, JPEG or WebP (lossless). Reads PNG, JPEG, WebP, GIF, BMP and TIFF,
  and HEIC on macOS.
- **Resize Images**: to a longest edge in pixels (never enlarges) or by a percentage.
- **Rotate Images**: 90° right, 180°, 90° left.
- **Remove Image Metadata**: a copy without EXIF (camera, location), XMP, IPTC and comments.
  JPEG and PNG keep their pixels exactly; other formats are saved again.
- **Images to PDF**: one page per image, in name order.
- **Merge PDFs** (in name order) and **Split PDF into Pages**.
- **Zip** and **Unzip**.
- **Copy Path**: full paths, names, or the folder's path, one per line.
- **Rename**: a pattern with `{name}` (the old name), `{ext}`, `{n}` (a counter; `{n:3}` gives
  `001`) and KnobDeck's placeholders (`{date}`, `{input:New name}`…). The extension is kept unless
  the pattern has `{ext}`.
- **Move or Copy to Folder**: a folder you choose (`~` is your home folder).
- **New File in Folder**: a file or folder in the folder you're looking at.
- **Terminal Here**: Windows Terminal (or Command Prompt), Terminal, Konsole or GNOME Console in
  that folder.
- **Files Wheel**: only the actions that fit the selection (image actions for images, Merge for
  two or more PDFs…).

New files go next to the originals with a new name (`photo (rotated).jpg`, `photo (2).png`):
nothing is ever overwritten. Everything runs on your computer; no file leaves it.

## Setup

None. macOS asks once whether KnobDeck may control Finder (that is how it reads the selection);
allow it, or later in System Settings ▸ Privacy & Security ▸ Automation.

## How it works

The selection comes from the KnobDeck SDK (`os-files`): the Shell's own COM objects for File
Explorer (the active tab) and the desktop, Finder's Apple Events, and on Linux Dolphin's
accessibility tree (AT-SPI: the front tab's folder and selected items; accessibility is switched
on for the session when it is off, as a screen reader does). Images, PDFs and zip files are processed in
Rust inside the plugin (`image`, `lopdf`, `zip`); HEIC is read with macOS's `sips`. No keyboard
shortcuts are used.

On Linux, only Dolphin shares its selection. Other file managers (GNOME Files, Nemo) don't; the plugin then says so.
