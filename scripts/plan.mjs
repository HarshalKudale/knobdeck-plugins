// Decides what to build. A plugin's version is the one in its Cargo.toml; you bump it when you ship
// the plugin. A plugin is built (on every OS it ships on) when it has no release tagged
// <dir>-v<version> yet. Released versions are never rebuilt.
//   node scripts/plan.mjs <file listing existing tags, one per line | none>   -> GitHub Actions matrix JSON
import { existsSync, readFileSync } from "node:fs";
import { OS_PLUGINS, readIndex } from "./plugins.mjs";

const tagsPath = process.argv[2];
const tags = new Set(tagsPath && existsSync(tagsPath) ? readFileSync(tagsPath, "utf8").split("\n").map((t) => t.trim()).filter(Boolean) : []);
const index = readIndex();
// Linux builds on the oldest supported Ubuntu so the binaries' glibc (2.35) also runs on Debian 12.
const RUNNER = { windows: "windows-latest", macos: "macos-latest", linux: "ubuntu-22.04" };

const include = Object.entries(OS_PLUGINS).flatMap(([name, dirs]) => {
  const todo = dirs.filter((dir) => !tags.has(`${dir}-v${index[dir].version}`));
  return todo.length ? [{ os: RUNNER[name], name, plugins: todo.join(" ") }] : [];
});
console.log(JSON.stringify({ include }));
