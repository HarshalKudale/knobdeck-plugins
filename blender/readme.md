# Blender

Control Blender from KnobDeck: scrub the timeline on a dial, switch workspaces and viewport
shading, render, save and undo from keys, and turn the brush size and strength or the selected
objects' location, rotation and scale. Keys show live values from Blender.

## Setup

1. Install this plugin.
2. In Blender (4.2 or newer): **Edit ▸ Preferences ▸ Get Extensions**, search **KnobDeck** and
   install the KnobDeck add-on. Or install `knobdeck-<version>.zip` from
   [its releases](https://github.com/Knobdeck/knobdeck-blender-companion/releases) with
   **Install from Disk…**.

There is nothing to configure: the add-on links to KnobDeck on this computer by itself and
reconnects when either restarts. With more than one Blender open, actions go to the one you used
last.

## What it does

- **Play / Pause** (shows the playback state), **Frame** dial (frames per tick), **Jump to Start /
  End**, **Previous / Next Keyframe**.
- **Workspace** (a button for one) and **Workspaces** (a wheel of them), **Object Interaction
  Mode**, **Mode and Object** (shows e.g. "Sculpt · Cube").
- **Viewport Shading** (one mode, or next / previous), **Overlays**, **X-Ray**.
- **Render Image**, **Render Animation** (Blender's render window), **Save**, **Undo**, **Redo**.
- Dials: **Brush Size** and **Brush Strength** in sculpt and paint modes; **Object Transform**:
  the selected objects' location, rotation (degrees) or scale on one axis, in Object Mode (one
  undo step per turn).

## How it works

The add-on links Blender to KnobDeck's companion hub on `127.0.0.1` and runs each action with
Blender's Python API on Blender's main thread; no keyboard shortcuts are sent. This plugin opens
no port. When Blender isn't linked, actions say so and the Mode and Object key shows "Blender
offline".
