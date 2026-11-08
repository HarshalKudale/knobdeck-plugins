# Game Launcher

Launch your installed games from KnobDeck: Steam games on Windows, macOS and Linux, and Epic Games
Store games on Windows and macOS.

## What it does

- **Launch Game**: a key that starts one game, showing the game's icon (and name).
- **Games Wheel**: a wheel of up to 16 installed games, most recently played first (Steam keeps
  the play dates) or A to Z, from every store or one.
- **Open Launcher Page**: open Steam's Library, Store, Friends, Downloads, Settings or Big Picture,
  or the Epic Games Store.

## Setup

None. Games are found in the launchers' own files:

- **Steam**: every Steam library folder (`steamapps/libraryfolders.vdf` and the games'
  `appmanifest_*.acf`). Windows reads Steam's folder from the registry; Linux also checks the
  Flatpak and Snap installs. Icons come from Steam's library cache. Steam's runtimes, Proton and
  redistributables are left out.
- **Epic Games Launcher**: the install manifests in `Epic/EpicGamesLauncher/Data/Manifests`.
  Unreal Engine and other non-game items are left out.

## How it works

Games start through the launchers' URL schemes (`steam://rungameid/<id>`,
`com.epicgames.launcher://apps/<id>?action=launch`), so the launcher handles updates, cloud saves
and sign-in exactly as its own Play button does. No keyboard shortcuts are used.

Not included yet: GOG Galaxy, the Xbox app / Microsoft Store games, Battle.net, EA app, Ubisoft
Connect, and Heroic or Lutris on Linux.
