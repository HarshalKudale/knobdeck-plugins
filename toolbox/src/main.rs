//! Toolbox plugin for KnobDeck: counters (on a key or a dial, kept across restarts), dice, a coin
//! flip, a random pick from your own list and a tally that writes each press to a CSV file.
//! Everything runs locally; nothing leaves the computer.

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::collections::hash_map::RandomState;
use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Counters are clamped to ± this (and the dial's range is the same).
const LIMIT: i64 = 999_999;

// ── Random numbers ───────────────────────────────────────────────────────────

/// A random u64 from std's randomly seeded hasher (no extra crate): each call hashes a fresh
/// `RandomState` key with a counter and the time.
fn random_u64() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish()
}

/// Uniform in `0..n` (rejection sampling, so no modulo bias). `n` must be > 0.
fn below_with(n: u64, mut next: impl FnMut() -> u64) -> u64 {
    let zone = u64::MAX - (u64::MAX % n + 1) % n;
    loop {
        let r = next();
        if r <= zone {
            return r % n;
        }
    }
}

fn below(n: u64) -> u64 {
    below_with(n, random_u64)
}

// ── Counters ─────────────────────────────────────────────────────────────────

static COUNTERS: Mutex<()> = Mutex::new(());

fn counters_file() -> PathBuf {
    knobdeck_sdk::data_file("counters.json")
}

fn load_counters(path: &Path) -> BTreeMap<String, i64> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Write via a temp file and rename, so a crash never leaves half a file.
fn save_counters(path: &Path, map: &BTreeMap<String, i64>) -> Result {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(map).unwrap_or_default())?;
    std::fs::rename(&tmp, path).map_err(|e| Error::msg(format!("Couldn't save the counter: {e}")))
}

fn counter_name(inputs: &Value) -> String {
    let n = inputs["name"].as_str().unwrap_or("").trim().to_string();
    if n.is_empty() {
        "Counter".into()
    } else {
        n
    }
}

fn counter_value(name: &str) -> i64 {
    let _g = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
    load_counters(&counters_file()).get(name).copied().unwrap_or(0)
}

/// Change counter `name` with `f` (old → new), save it, return the new value.
fn update_counter(name: &str, f: impl FnOnce(i64) -> i64) -> Result<i64> {
    let _g = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
    let path = counters_file();
    let mut map = load_counters(&path);
    let v = f(map.get(name).copied().unwrap_or(0)).clamp(-LIMIT, LIMIT);
    map.insert(name.to_string(), v);
    save_counters(&path, &map)?;
    Ok(v)
}

fn step_of(inputs: &Value) -> i64 {
    let v = &inputs["step"];
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).map_or(1, |f: f64| f.round().abs() as i64).clamp(1, LIMIT)
}

fn counter_label(name: &str, v: i64) -> String {
    format!("{name}\n{v}")
}

/// Show counter `name` on every Counter key and Counter Dial with that name.
fn show_counter(ctx: &Ctx, name: &str, v: i64) {
    let filter = json!({ "name": ctx.inputs["name"].clone() });
    let rt = ctx.runtime();
    rt.notify_visual_matching("counter", &filter, Visual::new().state(v).label(counter_label(name, v)));
    rt.notify_visual_matching("counter-dial", &filter, Visual::new().state(v));
    // This key too, in case its name input is empty and the filter didn't match it.
    if ctx.action_id == "counter" {
        ctx.set_visual(Visual::new().state(v).label(counter_label(name, v)));
    } else {
        ctx.set_state(v);
    }
}

fn name_input() -> Value {
    input::text("name", "Counter name").placeholder("Counter").default_value("Counter").with("help", "Keys and dials with the same name share one count.")
}

// ── Dice, coin, pick ─────────────────────────────────────────────────────────

fn roll(sides: u64, count: u64) -> Vec<u64> {
    (0..count).map(|_| below(sides) + 1).collect()
}

/// "4" for one die, "3 + 5 = 8" for several.
fn roll_text(rolls: &[u64]) -> String {
    let total: u64 = rolls.iter().sum();
    if rolls.len() == 1 {
        total.to_string()
    } else {
        format!("{} = {total}", rolls.iter().map(u64::to_string).collect::<Vec<_>>().join(" + "))
    }
}

fn dice_icon(sides: u64, rolls: &[u64]) -> &'static str {
    if sides != 6 || rolls.len() != 1 {
        return "dices";
    }
    ["dice-1", "dice-2", "dice-3", "dice-4", "dice-5", "dice-6"][(rolls[0] as usize - 1).min(5)]
}

fn items(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

/// Pick an index in `0..n`, avoiding `last` when asked and there is another choice.
fn pick_index(n: usize, last: Option<usize>, no_repeat: bool) -> usize {
    match last {
        Some(l) if no_repeat && n > 1 && l < n => {
            let i = below(n as u64 - 1) as usize;
            if i >= l {
                i + 1
            } else {
                i
            }
        }
        _ => below(n as u64) as usize,
    }
}

static LAST_PICK: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

// ── Tally ────────────────────────────────────────────────────────────────────

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// The tally file: the input ("~" for the home folder), else `~/KnobDeck Tally.csv`.
fn tally_path(raw: &str) -> Result<PathBuf> {
    let raw = raw.trim().trim_matches('"');
    if raw.is_empty() {
        return home().map(|h| h.join("KnobDeck Tally.csv")).ok_or_else(|| Error::msg("Choose a file for the tally"));
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return home().map(|h| h.join(rest)).ok_or_else(|| Error::msg("Couldn't find your home folder; give the full path"));
    }
    let p = PathBuf::from(raw);
    if p.is_relative() {
        return Err(Error::msg("Give the tally file's full path (or start it with ~/)"));
    }
    Ok(p)
}

/// One CSV field, quoted when it needs to be.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

const TALLY_HEADER: &str = "time,entry";

/// How many rows of `entry` the tally file holds (rows are `time,entry`).
fn count_entries(csv: &str, entry: &str) -> usize {
    let field = csv_field(entry);
    csv.lines().skip_while(|l| *l == TALLY_HEADER).filter(|l| l.split_once(',').is_some_and(|(_, e)| e == field)).count()
}

fn tally_count(path: &Path, entry: &str) -> usize {
    std::fs::read_to_string(path).map(|s| count_entries(&s, entry)).unwrap_or(0)
}

fn append_tally(path: &Path, entry: &str, time: &str) -> Result {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| Error::msg(format!("Couldn't create {}: {e}", dir.display())))?;
    }
    let new = !path.exists() || std::fs::metadata(path).map_or(true, |m| m.len() == 0);
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| Error::msg(format!("Couldn't open {}: {e}", path.display())))?;
    let mut line = String::new();
    if new {
        line.push_str(TALLY_HEADER);
        line.push('\n');
    }
    line.push_str(&format!("{time},{}\n", csv_field(entry)));
    f.write_all(line.as_bytes()).map_err(|e| Error::msg(format!("Couldn't write {}: {e}", path.display())))
}

fn tally_entry(inputs: &Value) -> String {
    let e = inputs["entry"].as_str().unwrap_or("").trim().to_string();
    if e.is_empty() {
        "Tally".into()
    } else {
        e
    }
}

fn tally_visual(path: &Path, entry: &str) -> Visual {
    Visual::new().label(format!("{entry}\n{}", tally_count(path, entry)))
}

// ── Plugin ───────────────────────────────────────────────────────────────────

fn main() {
    Plugin::new(&format!("toolbox-{}", std::env::consts::OS), "Toolbox")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Counters on a key or a dial (kept across restarts), dice, a coin flip, a random pick from your own list and a tally that writes each press to a file.")
        .icon("wrench")
        .action(
            Action::button("counter", "Counter")
                .icon("hash")
                .inputs(vec![
                    name_input(),
                    input::select("do", "Press", &[("add", "Adds"), ("subtract", "Subtracts"), ("reset", "Resets")]).default_value("add"),
                    input::text("step", "By").placeholder("1").default_value("1").with("showIf", json!({"key": "do", "in": ["add", "subtract"]})),
                    input::text("reset_to", "Reset to").placeholder("0").default_value("0").with("showIf", json!({"key": "do", "equals": "reset"})),
                ])
                .initial_with(|ic| Some(counter_value(&counter_name(&ic.inputs)).to_string()))
                .live(std::time::Duration::from_secs(60), |ic| {
                    let name = counter_name(&ic.inputs);
                    Ok(Visual::new().label(counter_label(&name, counter_value(&name))))
                })
                .on_press(|ctx| {
                    let name = counter_name(&ctx.inputs);
                    let step = step_of(&ctx.inputs);
                    let v = match ctx.input_str("do").as_str() {
                        "subtract" => update_counter(&name, |v| v.saturating_sub(step))?,
                        "reset" => {
                            let to = ctx.input_f64("reset_to").unwrap_or(0.0).round() as i64;
                            update_counter(&name, |_| to)?
                        }
                        _ => update_counter(&name, |v| v.saturating_add(step))?,
                    };
                    show_counter(ctx, &name, v);
                    Ok(())
                }),
        )
        .action(
            Action::slider("counter-dial", "Counter Dial", -LIMIT as f64, LIMIT as f64)
                .icon("hash")
                .step(1.0)
                .inputs(vec![name_input(), input::text("step", "Per tick").placeholder("1").default_value("1")])
                .initial_with(|ic| Some(counter_value(&counter_name(&ic.inputs)).to_string()))
                .on_adjust(|ctx| {
                    let name = counter_name(&ctx.inputs);
                    let step = step_of(&ctx.inputs);
                    let v = match (ctx.delta, ctx.value) {
                        (Some(d), _) => update_counter(&name, |v| v.saturating_add((d.round() as i64).saturating_mul(step)))?,
                        (None, Some(abs)) => update_counter(&name, |_| abs.round() as i64)?,
                        _ => return Ok(()),
                    };
                    show_counter(ctx, &name, v);
                    Ok(())
                }),
        )
        .action(
            Action::button("dice", "Roll Dice")
                .icon("dices")
                .inputs(vec![
                    input::select("sides", "Die", &[("4", "d4"), ("6", "d6"), ("8", "d8"), ("10", "d10"), ("12", "d12"), ("20", "d20"), ("100", "d100")]).default_value("6"),
                    input::select("count", "How many", &[("1", "1"), ("2", "2"), ("3", "3"), ("4", "4"), ("5", "5"), ("6", "6")]).default_value("1"),
                ])
                .on_press(|ctx| {
                    let sides = ctx.input_f64("sides").map_or(6, |f| f as u64).clamp(2, 1000);
                    let count = ctx.input_f64("count").map_or(1, |f| f as u64).clamp(1, 10);
                    let rolls = roll(sides, count);
                    ctx.set_visual(Visual::new().label(roll_text(&rolls)).icon(dice_icon(sides, &rolls)));
                    Ok(())
                }),
        )
        .action(Action::button("coin", "Flip a Coin").icon("coins").on_press(|ctx| {
            let heads = below(2) == 0;
            ctx.set_visual(Visual::new().label(if heads { "Heads" } else { "Tails" }).icon(if heads { "circle-user-round" } else { "circle-dot" }));
            Ok(())
        }))
        .action(
            Action::button("pick", "Random Pick")
                .icon("shuffle")
                .inputs(vec![
                    input::textarea("items", "Pick from, one per line").placeholder("Pizza\nSushi\nTacos"),
                    input::checkbox("no_repeat", "Never the same one twice in a row").default_value(true),
                ])
                .on_press(|ctx| {
                    let text = ctx.input_str("items");
                    let list = items(&text);
                    if list.is_empty() {
                        return ctx.fail("Add some things to pick from, one per line");
                    }
                    let mut last = LAST_PICK.lock().unwrap_or_else(|e| e.into_inner());
                    let i = pick_index(list.len(), last.get(&ctx.instance_id).copied(), ctx.input_bool("no_repeat"));
                    last.insert(ctx.instance_id.clone(), i);
                    ctx.set_label(list[i]);
                    Ok(())
                }),
        )
        .action(
            Action::button("tally", "Tally to File")
                .icon("notebook-pen")
                .inputs(vec![
                    input::text("entry", "Entry").placeholder("Coffee").default_value("Tally").with("help", "Written with the date and time on each press; the key shows how many the file holds."),
                    input::text("file", "File (optional)").placeholder("~/KnobDeck Tally.csv").with("help", "A CSV file (time,entry). Empty: KnobDeck Tally.csv in your home folder."),
                ])
                .initial_with(|ic| tally_path(&ic.input_str("file")).ok().map(|p| tally_count(&p, &tally_entry(&ic.inputs)).to_string()))
                .live(std::time::Duration::from_secs(30), |ic| {
                    let entry = tally_entry(&ic.inputs);
                    Ok(match tally_path(&ic.input_str("file")) {
                        Ok(p) => tally_visual(&p, &entry),
                        Err(e) => Visual::new().label(e.message().to_string()),
                    })
                })
                .on_press(|ctx| {
                    let path = tally_path(&ctx.input_str("file"))?;
                    let entry = tally_entry(&ctx.inputs);
                    let time = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S").to_string();
                    append_tally(&path, &entry, &time)?;
                    let v = tally_visual(&path, &entry);
                    ctx.fan_out_visual_by(&["entry", "file"], v);
                    Ok(())
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_is_in_range_and_unbiased_enough() {
        let mut seen = [0u32; 6];
        for _ in 0..6000 {
            seen[below(6) as usize] += 1;
        }
        assert!(seen.iter().all(|&c| (800..1200).contains(&c)), "{seen:?}");
        assert_eq!(below(1), 0);
        // Rejection: values above the last full block are redrawn.
        let mut vals = [u64::MAX, 7].into_iter();
        assert_eq!(below_with(10, || vals.next().unwrap()), 7);
    }

    #[test]
    fn dice() {
        for _ in 0..500 {
            let r = roll(20, 3);
            assert_eq!(r.len(), 3);
            assert!(r.iter().all(|v| (1..=20).contains(v)));
        }
        assert_eq!(roll_text(&[4]), "4");
        assert_eq!(roll_text(&[3, 5]), "3 + 5 = 8");
        assert_eq!(dice_icon(6, &[6]), "dice-6");
        assert_eq!(dice_icon(20, &[6]), "dices");
    }

    #[test]
    fn pick_avoids_the_last_one() {
        assert_eq!(items(" a \n\n b\n"), vec!["a", "b"]);
        for _ in 0..200 {
            assert_ne!(pick_index(3, Some(1), true), 1);
            assert!(pick_index(3, Some(1), true) < 3);
        }
        assert_eq!(pick_index(1, Some(0), true), 0, "one item: it is picked anyway");
        let mut hit_last = false;
        for _ in 0..200 {
            hit_last |= pick_index(2, Some(0), false) == 0;
        }
        assert!(hit_last, "repeats are allowed when not asked to avoid them");
    }

    #[test]
    fn counters_persist() {
        let dir = std::env::temp_dir().join(format!("toolbox-test-{}", random_u64()));
        let path = dir.join("counters.json");
        let mut m = load_counters(&path);
        assert!(m.is_empty());
        m.insert("Push-ups".into(), 12);
        save_counters(&path, &m).unwrap();
        assert_eq!(load_counters(&path).get("Push-ups"), Some(&12));
        std::fs::write(&path, "not json").unwrap();
        assert!(load_counters(&path).is_empty(), "a broken file starts over instead of failing");
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(step_of(&json!({"step": "5"})), 5);
        assert_eq!(step_of(&json!({"step": "-3"})), 3);
        assert_eq!(step_of(&json!({})), 1);
        assert_eq!(step_of(&json!({"step": "0"})), 1);
        assert_eq!(counter_name(&json!({"name": "  "})), "Counter");
    }

    #[test]
    fn tally_file() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        let dir = std::env::temp_dir().join(format!("toolbox-tally-{}", random_u64()));
        let path = dir.join("sub").join("t.csv");
        append_tally(&path, "Coffee", "2026-11-19 09:00:00").unwrap();
        append_tally(&path, "Tea, green", "2026-11-19 10:00:00").unwrap();
        append_tally(&path, "Coffee", "2026-11-19 11:00:00").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "time,entry\n2026-11-19 09:00:00,Coffee\n2026-11-19 10:00:00,\"Tea, green\"\n2026-11-19 11:00:00,Coffee\n");
        assert_eq!(tally_count(&path, "Coffee"), 2);
        assert_eq!(tally_count(&path, "Tea, green"), 1);
        assert_eq!(tally_count(&path, "entry"), 0, "the header isn't an entry");
        std::fs::remove_dir_all(dir).unwrap();
        assert!(tally_path("relative.csv").is_err());
        assert!(tally_path("").unwrap().ends_with("KnobDeck Tally.csv"));
        assert!(tally_path("~/x.csv").unwrap().is_absolute());
    }
}
