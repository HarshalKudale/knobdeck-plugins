# Game Launcher

Launch your installed games from KnobDeck: Steam games on Windows, macOS and Linux, Epic Games
Store games on Windows and macOS, GOG Galaxy and Ubisoft Connect games on Windows, Heroic games on
every OS and Lutris games on Linux. And close a running game.

## What it does

- **Launch Game**: a key that starts one game, showing the game's icon (and name).
- **Games Wheel**: a wheel of up to 16 installed games, most recently played first (Steam and
  Lutris keep the play dates) or A to Z, from every store or one.
- **Open Launcher Page**: open Steam's Library, Store, Friends, Downloads, Settings or Big Picture,
  or the Epic Games Store.
- **Close Game**: asks a game (or whichever known game is running) to close, the way its window's
  close button does, and ends it after 5, 15 or 30 seconds if it's still running (or leaves it).
  Only processes started from a game's own install folder are touched.

## Setup

None. Games are found in the launchers' own files:

- **Steam**: every Steam library folder (`steamapps/libraryfolders.vdf` and the games'
  `appmanifest_*.acf`). Windows reads Steam's folder from the registry; Linux also checks the
  Flatpak and Snap installs. Icons come from Steam's library cache. Steam's runtimes, Proton and
  redistributables are left out.
- **Epic Games Launcher**: the install manifests in `Epic/EpicGamesLauncher/Data/Manifests`.
  Unreal Engine and other non-game items are left out.
- **GOG Galaxy** (Windows): the `GOG.com\Games` registry entries that Galaxy and GOG's offline
  installers write.
- **Ubisoft Connect** (Windows): its `Uplay Install <id>` uninstall entries.
- **Heroic** (every OS, native and Flatpak): its Epic (Legendary), GOG, Amazon (Nile) and
  sideloaded libraries in Heroic's config folder.
- **Lutris** (Linux, native and Flatpak): the installed games in its `pga.db` (Steam games Lutris
  lists are shown once, under Steam). Icons come from Lutris's icon folder.

## How it works

Games start through the launchers' URL schemes (`steam://rungameid/<id>`,
`com.epicgames.launcher://apps/<id>?action=launch`, `uplay://launch/<id>/0`,
`heroic://launch?appName=<app>&runner=<runner>`, `lutris:rungameid/<id>`), so the launcher handles
updates, cloud saves and sign-in exactly as its own Play button does. GOG games start through
GOG Galaxy (`GalaxyClient.exe /command=runGame`), or the game's own exe when Galaxy isn't
installed. No keyboard shortcuts are used.

Close Game posts `WM_CLOSE` to the game's windows on Windows and sends `SIGTERM` on macOS and
Linux, then terminates what's left. On Linux, Wine and Proton games are found by the Windows path
in their command line.

Not included yet: the Xbox app / Microsoft Store games, Battle.net, the EA app and itch.
