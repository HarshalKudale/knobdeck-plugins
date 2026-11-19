// Generates every plugin's app icon, `<dir>/icons/app-icon.svg`: a 512 px squircle (the KnobDeck
// icon's superellipse) the app shows full-bleed in the Plugins store, sidebar and detail page.
//
//   brand plugins   the app's mark from Simple Icons (CC0 path data) in white, or ink on a light
//                   brand colour, on the brand's own colour
//   everything else a Lucide glyph in white on a gradient of its category's colour
//
// Plugins that ship their own logo (LOGO below) are left alone. Re-run after adding a plugin:
//
//   npm i --no-save --prefix /tmp/kd-icons simple-icons lucide-static
//   node scripts/plugin-icons.mjs /tmp/kd-icons/node_modules
//
// Then point the plugin's `Plugin::new(..).icon(..)` at "app-icon.svg" and bump its version
// (the icon is part of the manifest; CLAUDE.md rule 4).
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MODULES = process.argv[2];
if (!MODULES) { console.error("usage: node scripts/plugin-icons.mjs <node_modules with simple-icons and lucide-static>"); process.exit(1); }

/// Plugins with a logo file of their own (kept as they are).
const LOGO = new Set(["discord", "intellij", "obs", "vscode", "zoom"]);

/// Plugin dir → Simple Icons slug.
const BRAND = {
  spotify: "spotify", twitch: "twitch", youtube: "youtube", github: "github", docker: "docker",
  blender: "blender", "home-assistant": "homeassistant", hue: "philipshue", lifx: "lifx",
  "elgato-lights": "elgato", "apple-music": "applemusic", neovim: "neovim", vlc: "vlcmediaplayer",
  streamlabs: "streamlabs", teamspeak: "teamspeak", ytmusic: "youtubemusic", mqtt: "mqtt",
};

/// Plugin dir → Lucide glyph (one each, so no two plugins look alike).
const GLYPH = {
  "ai-actions": "sparkles", "ai-stats": "activity", "browser-plugin": "globe", clocks: "clock",
  "coding-agents": "code-xml", "daw-mackie": "sliders-vertical", files: "files", finance: "chart-candlestick",
  "game-launcher": "gamepad-2", govee: "lamp-ceiling", "hardware-monitor": "thermometer",
  "linux-tools": "square-terminal", "mac-tools": "command", midi: "piano", nanoleaf: "hexagon",
  network: "network", "now-playing": "disc-3", openrgb: "palette", reaper: "audio-waveform",
  slides: "presentation", soundboard: "speaker", speedtest: "gauge", streamerbot: "bot",
  "text-to-speech": "speech", "text-tools": "type", toolbox: "wrench", voicemeeter: "audio-lines",
  voicemod: "mic-vocal", "vtube-studio": "smile", weather: "cloud-sun", "web-requests": "webhook",
  "win-tools": "app-window", wled: "rainbow",
};

/// First matching tag → colour (the app's kind palette, so plugins sit with the rest of the UI).
const TAG_COLOR = [
  [["AI"], "#8152e8"],
  [["Developer tools", "Developer"], "#5b5fe6"],
  [["Lighting", "RGB"], "#e0911f"],
  [["Smart home"], "#24a865"],
  [["Streaming", "VTubing", "Voice"], "#e0457f"],
  [["Music", "Audio", "Media"], "#e8603f"],
  [["Gaming"], "#d9483b"],
  [["Network", "Monitoring", "System", "Information", "Weather", "Time", "Finance"], "#2f8fe8"],
  [["Linux", "macOS", "Windows", "Automation", "Browser"], "#1f9aa3"],
];
const DEFAULT_COLOR = "#1f9aa3";

const index = JSON.parse(readFileSync(join(ROOT, "index.json"), "utf8")).plugins;

// The KnobDeck icon's tile: a superellipse (n = 5) filling the 512 box.
const squircle = (() => {
  const C = 256, a = 256, n = 5;
  let d = "";
  for (let i = 0; i < 360; i++) {
    const t = (i / 360) * 2 * Math.PI, ct = Math.cos(t), st = Math.sin(t);
    const x = C + a * Math.sign(ct) * Math.abs(ct) ** (2 / n), y = C + a * Math.sign(st) * Math.abs(st) ** (2 / n);
    d += `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`;
  }
  return `${d}Z`;
})();

const hex = (h) => h.replace("#", "").padStart(6, "0");
const rgb = (h) => [0, 2, 4].map((i) => parseInt(hex(h).slice(i, i + 2), 16));
const toHex = (c) => `#${c.map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0")).join("")}`;
const mix = (a, b, t) => toHex(rgb(a).map((v, i) => v + (rgb(b)[i] - v) * t));
const luminance = (h) => {
  const [r, g, b] = rgb(h).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

function tile(color, inner, { flat = false } = {}) {
  const top = flat ? color : mix(color, "#ffffff", 0.22);
  const bottom = flat ? color : mix(color, "#000000", 0.18);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${top}"/><stop offset="1" stop-color="${bottom}"/></linearGradient>
    <linearGradient id="shine" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.22"/><stop offset="0.5" stop-color="#fff" stop-opacity="0"/></linearGradient>
  </defs>
  <path d="${squircle}" fill="url(#bg)"/>
  <path d="${squircle}" fill="url(#shine)"/>
${inner}
</svg>
`;
}

function brandIcon(slug) {
  const file = join(MODULES, "simple-icons/icons", `${slug}.svg`);
  const svg = readFileSync(file, "utf8");
  const path = svg.match(/<path d="([^"]+)"/)?.[1];
  const data = JSON.parse(readFileSync(join(MODULES, "simple-icons/data/simple-icons.json"), "utf8"));
  const list = Array.isArray(data) ? data : data.icons;
  const entry = list.find((i) => (i.slug ?? i.title.toLowerCase().replace(/[^a-z0-9]/g, "")) === slug);
  const color = `#${entry?.hex ?? "1f2328"}`;
  // A mark on a light brand colour reads in ink, on everything else in white.
  const ink = luminance(color) > 0.5 ? "#161b20" : "#ffffff";
  // Simple Icons draw in a 24 box; the mark takes 60% of the tile.
  const s = (512 * 0.6) / 24, off = (512 - 24 * s) / 2;
  return tile(color, `  <path transform="translate(${off.toFixed(2)} ${off.toFixed(2)}) scale(${s.toFixed(4)})" d="${path}" fill="${ink}"/>`, { flat: true });
}

function glyphIcon(name, color) {
  const svg = readFileSync(join(MODULES, "lucide-static/icons", `${name}.svg`), "utf8");
  const body = svg.slice(svg.indexOf(">", svg.indexOf("<svg")) + 1, svg.lastIndexOf("</svg>")).trim();
  // Lucide draws in a 24 box with 2 px strokes; the glyph takes 56% of the tile.
  const s = (512 * 0.56) / 24, off = (512 - 24 * s) / 2;
  return tile(color, `  <g transform="translate(${off.toFixed(2)} ${off.toFixed(2)}) scale(${s.toFixed(4)})" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    ${body.replace(/\n\s*/g, "\n    ")}
  </g>`);
}

const colorFor = (tags = []) => TAG_COLOR.find(([names]) => tags.some((t) => names.includes(t)))?.[1] ?? DEFAULT_COLOR;

let made = 0;
for (const [dir, meta] of Object.entries(index)) {
  if (LOGO.has(dir)) continue;
  let svg;
  if (BRAND[dir]) svg = brandIcon(BRAND[dir]);
  else if (GLYPH[dir]) svg = glyphIcon(GLYPH[dir], colorFor(meta.tags));
  else { console.warn(`no icon rule for ${dir}`); continue; }
  const out = join(ROOT, dir, "icons", "app-icon.svg");
  if (!existsSync(dirname(out))) mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, svg);
  made++;
}
console.log(`${made} app icons written`);
