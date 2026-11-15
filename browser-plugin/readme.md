# Browser Plugin

Shared browser controls for KnobDeck across Chrome, Edge, Brave, Firefox and Zen, on Windows, macOS
and Linux. The actions run through the **KnobDeck Browser Companion**, a small extension you install
in each browser ([knobdeck-companion/browser](https://github.com/Knobdeck/knobdeck-companion/tree/master/browser)).
The extension connects to this plugin on `127.0.0.1:32147` (`BROWSER_PORT` in the plugin's `.env`
changes it), so the **Browser** choice lists the browsers that are connected right now, and an
action reaches the browser's own tabs, windows and zoom without focusing it or sending shortcuts.

## What it does

- Opens URLs, bookmarks, and history items in the browser you pick.
- New / close / reopen / next / previous tab, new window, private window, reload, hard reload, back,
  forward, stop, print, bookmark, fullscreen, page zoom (slider and hold), tab cycling (hold).
- Custom Keybind: key events sent to the page (a page can ignore them).
- Focus Address Bar, Find in Page and Save Page (all browsers) and Downloads / History / Bookmarks
  Manager (Firefox, Zen) have no extension API, so they still bring the browser forward and send the
  shortcut.

## Good for

- Launching browser workflows from a wheel.
- Quick navigation commands during streaming or demos.
- Using one plugin across several browsers, and across profiles (each profile with the extension is
  listed separately).

Open KnobDeck settings after updating: saved actions picked an installed browser before; pick the
connected one again.
