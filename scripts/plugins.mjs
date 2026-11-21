// Shared by plan.mjs and release-plugins.mjs.
import { readFileSync } from "node:fs";

/** Plugin folders that ship on each OS. */
// Disabled (kept in the repo, not shipped):
// - zoom: Zoom has no documented local API for its desktop client; the plugin's localhost:19421
//   endpoints were never a published interface.
// flight-sim ships on Windows only: Microsoft Flight Simulator and its SimConnect API are Windows-only.
export const OS_PLUGINS = {
  windows: ["ableton", "ai-actions", "ai-stats", "blender", "browser-plugin", "clocks", "coding-agents", "daw-mackie", "discord", "docker", "elgato-lights", "files", "finance", "flight-sim", "game-launcher", "github", "govee", "hardware-monitor", "home-assistant", "hue", "intellij", "lifx", "lightroom-classic", "midi", "mqtt", "nanoleaf", "neovim", "network", "now-playing", "obs", "openrgb", "reaper", "slides", "soundboard", "speedtest", "spotify", "streamerbot", "streamlabs", "teamspeak", "text-to-speech", "text-tools", "toolbox", "twitch", "vlc", "voicemeeter", "voicemod", "vscode", "vtube-studio", "weather", "web-requests", "win-tools", "wled", "youtube", "ytmusic"],
  macos: ["ableton", "ai-actions", "ai-stats", "apple-music", "blender", "browser-plugin", "clocks", "coding-agents", "daw-mackie", "discord", "docker", "elgato-lights", "files", "finance", "game-launcher", "github", "govee", "hardware-monitor", "home-assistant", "hue", "intellij", "lifx", "lightroom-classic", "mac-tools", "midi", "mqtt", "nanoleaf", "neovim", "network", "now-playing", "obs", "openrgb", "reaper", "slides", "soundboard", "speedtest", "spotify", "streamlabs", "teamspeak", "text-to-speech", "text-tools", "toolbox", "twitch", "vlc", "voicemod", "vscode", "vtube-studio", "weather", "web-requests", "wled", "youtube", "ytmusic"],
  // Only apps with an official Linux build (Streamlabs Desktop, Voicemod, Voicemeeter, Streamer.bot, VTube Studio, Ableton Live and Lightroom Classic have none).
  // Debian/Ubuntu and Arch; GNOME and KDE. Files works only in Dolphin (experimental; GNOME Files
  // doesn't share its selection).
  linux: ["ai-actions", "ai-stats", "blender", "browser-plugin", "clocks", "coding-agents", "daw-mackie", "discord", "docker", "elgato-lights", "files", "finance", "game-launcher", "github", "govee", "hardware-monitor", "home-assistant", "hue", "intellij", "lifx", "linux-tools", "midi", "mqtt", "nanoleaf", "neovim", "network", "now-playing", "obs", "openrgb", "reaper", "slides", "soundboard", "speedtest", "spotify", "teamspeak", "text-to-speech", "text-tools", "toolbox", "twitch", "vlc", "vscode", "weather", "web-requests", "wled", "youtube", "ytmusic"],
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

