// Publishes one GitHub Release per plugin built in this run, tagged <dir>-v<version>
// (e.g. obs-v1.3.0), holding that plugin's per-OS bundles and readmes plus <dir>.json (its
// index.json display text and version). The catalog in knobdeck-registry reads these releases.
// A tag that already exists is left alone: a released version is never rebuilt in place.
// Release notes are the top section of <dir>/CHANGELOG.md when there is one.
// Plugin releases are never marked "Latest": the catalog's index is the only "latest".
// Writes out/released.json: [{ id, version }] for the dispatch to the catalog.
//   node scripts/release-plugins.mjs <commit sha>
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readIndex } from "./plugins.mjs";

const sha = process.argv[2];
if (!sha) throw new Error("usage: release-plugins.mjs <commit sha>");
const index = readIndex();

const built = readdirSync("out")
  .filter((f) => /^part-.*\.json$/.test(f))
  .flatMap((f) => JSON.parse(readFileSync(join("out", f), "utf8")));
const byDir = new Map();
for (const e of built) byDir.set(e.dir, [...(byDir.get(e.dir) ?? []), e]);

const tagExists = (tag) => {
  try {
    execFileSync("gh", ["release", "view", tag, "--json", "tagName"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
};

/** The first `## ` section of CHANGELOG.md, without its heading; null when there is none. */
const changelogTop = (dir) => {
  const file = join(dir, "CHANGELOG.md");
  if (!existsSync(file)) return null;
  const sections = readFileSync(file, "utf8").split(/^## /m).slice(1);
  return sections.length ? sections[0].split("\n").slice(1).join("\n").trim() || null : null;
};

const work = mkdtempSync(join(tmpdir(), "plugin-release-"));
const released = [];
for (const [dir, entries] of [...byDir].sort(([a], [b]) => a.localeCompare(b))) {
  const versions = new Set(entries.map((e) => e.version));
  if (versions.size !== 1) throw new Error(`${dir}: built with different versions ${[...versions]}`);
  const [version] = versions;
  const tag = `${dir}-v${version}`;
  if (tagExists(tag)) {
    console.log(`${tag} exists, skipped`);
    continue;
  }
  const meta = index[dir] ?? {};
  const metaFile = join(work, `${dir}.json`);
  writeFileSync(metaFile, JSON.stringify({ dir, version, ...meta }, null, 2) + "\n");
  const files = entries.flatMap((e) => [e.file, e.readmeFile]).filter(Boolean).map((f) => join("out", f));
  const name = meta.name ?? dir;
  const notes = changelogTop(dir) ?? `${name} ${version} for ${entries.map((e) => e.platforms).flat().sort().join(", ")}.`;
  const notesFile = join(work, `${dir}.notes.md`);
  writeFileSync(notesFile, notes + "\n");
  execFileSync(
    "gh",
    ["release", "create", tag, ...files, metaFile, "--target", sha, "--title", `${name} ${version}`, "--notes-file", notesFile, "--latest=false"],
    { stdio: "inherit" },
  );
  released.push({ id: dir, version });
}
writeFileSync("out/released.json", JSON.stringify(released) + "\n");
console.log(`${released.length} plugin release(s)`);
