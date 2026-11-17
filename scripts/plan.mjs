// Decides what to build. A plugin's version is the one in its Cargo.toml; you bump it when you ship
// the plugin. A plugin is built for an OS when that version differs from the one in the previous
// release's registry.json (or it is not there).
// Everything else is carried over from that release by registry.mjs.
//   node scripts/plan.mjs [previous registry.json]   -> GitHub Actions matrix JSON
import { existsSync, readFileSync } from "node:fs";
import { OS_PLUGINS, findEntry, readIndex } from "./plugins.mjs";

const prevPath = process.argv[2];
const prev = prevPath && existsSync(prevPath) ? JSON.parse(readFileSync(prevPath, "utf8")).plugins : [];
const index = readIndex();
// Linux builds on the oldest supported Ubuntu so the binaries' glibc (2.35) also runs on Debian 12.
const RUNNER = { windows: "windows-latest", macos: "macos-latest", linux: "ubuntu-22.04" };

const include = Object.entries(OS_PLUGINS).flatMap(([name, dirs]) => {
  const todo = dirs.filter((dir) => findEntry(prev, dir, name)?.version !== index[dir].version);
  return todo.length ? [{ os: RUNNER[name], name, plugins: todo.join(" ") }] : [];
});
console.log(JSON.stringify({ include }));
