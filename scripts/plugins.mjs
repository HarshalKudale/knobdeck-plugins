// Shared by plan.mjs and release-plugins.mjs.
import { readFileSync } from "node:fs";

/** Plugin folders that ship on each OS. */
// Disabled (kept in the repo, not shipped):
// - zoom: Zoom has no documented local API for its desktop client; the plugin's localhost:19421
//   endpoints were never a published interface.
export const OS_PLUGINS = {
  windows: ["ai-actions", "ai-stats", "browser-plugin", "clocks", "coding-agents", "discord", "docker", "elgato-lights", "files", "game-launcher", "govee", "home-assistant", "hue", "intellij", "nanoleaf", "now-playing", "obs", "slides", "speedtest", "spotify", "streamlabs", "twitch", "vlc", "voicemod", "vscode", "weather", "web-requests", "win-tools", "youtube", "ytmusic"],
  macos: ["ai-actions", "ai-stats", "browser-plugin", "clocks", "coding-agents", "discord", "docker", "elgato-lights", "files", "game-launcher", "govee", "home-assistant", "hue", "intellij", "mac-tools", "nanoleaf", "now-playing", "obs", "slides", "speedtest", "spotify", "streamlabs", "twitch", "vlc", "voicemod", "vscode", "weather", "web-requests", "youtube", "ytmusic"],
  // Only apps with an official Linux build (Streamlabs Desktop and Voicemod have none).
  // Debian/Ubuntu and Arch; GNOME and KDE. Files works only in Dolphin (experimental; GNOME Files
  // doesn't share its selection).
  linux: ["ai-actions", "ai-stats", "browser-plugin", "clocks", "coding-agents", "discord", "docker", "elgato-lights", "files", "game-launcher", "govee", "home-assistant", "hue", "intellij", "linux-tools", "nanoleaf", "now-playing", "obs", "slides", "speedtest", "spotify", "twitch", "vlc", "vscode", "weather", "web-requests", "youtube", "ytmusic"],
};

/** A plugin's version: its Cargo.toml is the only place it is written (the binary reports it). */
export const cargoVersion = (dir) => {
  const v = readFileSync(`${dir}/Cargo.toml`, "utf8").match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!v) throw new Error(`${dir}/Cargo.toml has no version`);
  return v;
};

/**
 * index.json holds display text only: { plugins: { "<dir>": { name, summary, tags, featured, sort } } }.
 * Returned with each plugin's `version` read from its Cargo.toml. A `version` left in index.json is
 * an error, so the two can't drift apart again.
 */
export const readIndex = () => {
  const plugins = JSON.parse(readFileSync("index.json", "utf8")).plugins;
  for (const [dir, p] of Object.entries(plugins)) {
    if ("version" in p) throw new Error(`index.json: ${dir} has a version; it lives in ${dir}/Cargo.toml only`);
    p.version = cargoVersion(dir);
  }
  return plugins;
};

