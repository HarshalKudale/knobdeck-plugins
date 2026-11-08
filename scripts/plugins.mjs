// Shared by plan.mjs and registry.mjs.
import { readFileSync } from "node:fs";

/** Plugin folders that ship on each OS. */
export const OS_PLUGINS = {
  windows: ["ai-stats", "browser-plugin", "clocks", "discord", "elgato-lights", "game-launcher", "home-assistant", "hue", "intellij", "obs", "slides", "spotify", "streamlabs", "twitch", "vlc", "voicemod", "vscode", "weather", "win-tools", "youtube", "ytmusic", "zoom"],
  macos: ["ai-stats", "browser-plugin", "clocks", "discord", "elgato-lights", "game-launcher", "home-assistant", "hue", "intellij", "mac-tools", "obs", "slides", "spotify", "streamlabs", "twitch", "vlc", "voicemod", "vscode", "weather", "youtube", "ytmusic", "zoom"],
  // Only apps with an official Linux build (Streamlabs Desktop and Voicemod have none; Zoom's
  // local meeting API isn't documented for its Linux client). Debian/Ubuntu and Arch; GNOME and KDE.
  linux: ["ai-stats", "browser-plugin", "clocks", "discord", "elgato-lights", "game-launcher", "home-assistant", "hue", "intellij", "linux-tools", "obs", "slides", "spotify", "twitch", "vlc", "vscode", "weather", "youtube", "ytmusic"],
};

/** index.json: { plugins: { "<dir>": { version, name, summary, tags, featured, sort } } } */
export const readIndex = () => JSON.parse(readFileSync("index.json", "utf8")).plugins;

/** The registry entry of `dir` on `os` in a registry.json plugin list (the id is the dir, or dir-os for cross-platform plugins). */
export const findEntry = (entries, dir, os) => entries.find((e) => e.id === dir || e.id === `${dir}-${os}`);
