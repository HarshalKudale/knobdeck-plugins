//! Text Tools plugin for KnobDeck: change the case of the selected text, trim it, slugify it,
//! URL / Base64 encode or decode it, format JSON, count its words, and keep text in numbered
//! slots. The result goes on the clipboard, or in place of the selection.

mod output;
mod transform;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Need, Plugin, Result, Value, Visual};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SLOTS: usize = 9;

/// Where the text comes from: the selection, else (unless turned off) the clipboard.
fn source_text(ctx: &Ctx) -> Result<String> {
    if let Some(s) = ctx.selection().filter(|s| !s.is_empty()) {
        return Ok(s.to_string());
    }
    if !ctx.input_bool("no_clipboard") {
        if let Some(c) = ctx.clipboard_text().filter(|s| !s.is_empty()) {
            return Ok(c.to_string());
        }
    }
    let why = ctx.context_error(Need::Selection).map(|e| format!(" ({e})")).unwrap_or_default();
    Err(Error::msg(format!("Select some text first{why}")))
}

fn output_input() -> Value {
    input::select("output", "Put the result", &[("copy", "On the clipboard"), ("replace", "In place of the selected text")]).default_value("copy")
}

fn no_clipboard_input() -> Value {
    input::checkbox("no_clipboard", "Don't fall back to the clipboard when nothing is selected")
}

/// Deliver `text`: on the clipboard (with a short notice) or entered in place of the selection.
fn deliver(ctx: &Ctx, text: &str, what: &str) -> Result {
    if ctx.input_str("output") == "replace" {
        if let Err(e) = output::enter_text(text) {
            let _ = output::set_clipboard(text);
            return Err(e);
        }
        return Ok(());
    }
    output::set_clipboard(text)?;
    ctx.toast(format!("{what}: {}", transform::preview(text, 60)))
}

fn run(ctx: &Ctx, id: &str) -> Result {
    let op = transform::op(id).ok_or_else(|| Error::msg("Choose a transform"))?;
    let text = source_text(ctx)?;
    let out = transform::apply(id, &text).map_err(Error::msg)?;
    ctx.log(format!("{id}: {} chars in, {} chars out", text.len(), out.len()));
    deliver(ctx, &out, op.label)
}

// ── Slots ────────────────────────────────────────────────────────────────────

static SLOT_LOCK: Mutex<()> = Mutex::new(());

fn slots_file() -> PathBuf {
    knobdeck_sdk::data_file("slots.json")
}

fn load_slots(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_slots(path: &Path, slots: &BTreeMap<String, String>) -> Result {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(slots).unwrap_or_default())?;
    std::fs::rename(&tmp, path).map_err(|e| Error::msg(format!("Couldn't save the slot: {e}")))
}

fn slot_of(inputs: &Value) -> String {
    let n = inputs["slot"].as_str().map(str::to_string).or_else(|| inputs["slot"].as_u64().map(|n| n.to_string())).unwrap_or_default();
    match n.trim().parse::<usize>() {
        Ok(n) if (1..=SLOTS).contains(&n) => n.to_string(),
        _ => "1".into(),
    }
}

fn slot_text(slot: &str) -> Option<String> {
    let _g = SLOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_slots(&slots_file()).remove(slot)
}

fn slot_label(slot: &str, text: Option<&str>) -> String {
    match text {
        Some(t) => format!("Slot {slot}\n{}", transform::preview(t, 24)),
        None => format!("Slot {slot}\nempty"),
    }
}

fn slot_input() -> Value {
    let opts: Vec<(String, String)> = (1..=SLOTS).map(|n| (n.to_string(), format!("Slot {n}"))).collect();
    let opts: Vec<(&str, &str)> = opts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    input::select("slot", "Slot", &opts).default_value("1")
}

/// Show slot `slot` on every Save / Paste key for it.
fn show_slot(ctx: &Ctx, slot: &str, text: Option<&str>) {
    let filter = json!({ "slot": ctx.inputs["slot"].clone() });
    let v = Visual::new().label(slot_label(slot, text));
    for a in ["save-slot", "paste-slot"] {
        ctx.runtime().notify_visual_matching(a, &filter, v.clone());
    }
    ctx.set_visual(v);
}

fn ops_options() -> Vec<(&'static str, &'static str)> {
    transform::OPS.iter().map(|o| (o.id, o.label)).collect()
}

fn main() {
    Plugin::new(&format!("text-tools-{}", std::env::consts::OS), "Text Tools")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Change case, trim, slugify, URL / Base64 encode and decode, or format JSON in the selected text; count words; keep text in numbered slots. Results go on the clipboard or in place of the selection.")
        .icon("type")
        .action(
            Action::button("transform", "Transform Text")
                .icon("type")
                .needs(&[Need::Selection, Need::Clipboard])
                .inputs(vec![input::select("op", "Do", &ops_options()).default_value("upper"), output_input(), no_clipboard_input()])
                .on_press(|ctx| {
                    let id = ctx.input_str("op");
                    run(ctx, if id.is_empty() { "upper" } else { &id })
                }),
        )
        .action(
            Action::wheel("wheel", "Text Tools Wheel")
                .icon("whole-word")
                .needs(&[Need::Selection, Need::Clipboard])
                .inputs(vec![output_input(), no_clipboard_input()])
                .on_open(|_| Ok(transform::OPS.iter().map(|o| json!({"id": o.id, "label": o.label, "icon": o.icon})).collect()))
                .on_child_press(|ctx| {
                    let Some(id) = ctx.child_id.clone() else { return Ok(()) };
                    run(ctx, &id)
                }),
        )
        .action(
            Action::button("word-count", "Word Count")
                .icon("text")
                .needs(&[Need::Selection, Need::Clipboard])
                .inputs(vec![
                    input::select("show", "Show", &[("words", "Words and characters"), ("lines", "Words and lines"), ("chars", "Characters only")]).default_value("words"),
                    no_clipboard_input(),
                ])
                .on_press(|ctx| {
                    let c = transform::counts(&source_text(ctx)?);
                    use transform::plural;
                    let label = match ctx.input_str("show").as_str() {
                        "lines" => format!("{}\n{}", plural(c.words, "word", "words"), plural(c.lines, "line", "lines")),
                        "chars" => plural(c.chars, "char", "chars"),
                        _ => format!("{}\n{}", plural(c.words, "word", "words"), plural(c.chars, "char", "chars")),
                    };
                    ctx.set_label(label);
                    Ok(())
                }),
        )
        .action(
            Action::button("save-slot", "Save to Slot")
                .icon("bookmark-plus")
                .needs(&[Need::Selection, Need::Clipboard])
                .inputs(vec![slot_input(), no_clipboard_input()])
                // The label is pushed on sync and refreshed each minute (another key may save to it).
                .live(std::time::Duration::from_secs(60), |ic| {
                    let slot = slot_of(&ic.inputs);
                    Ok(Visual::new().label(slot_label(&slot, slot_text(&slot).as_deref())))
                })
                .on_press(|ctx| {
                    let slot = slot_of(&ctx.inputs);
                    let text = source_text(ctx)?;
                    {
                        let _g = SLOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                        let path = slots_file();
                        let mut slots = load_slots(&path);
                        slots.insert(slot.clone(), text.clone());
                        save_slots(&path, &slots)?;
                    }
                    show_slot(ctx, &slot, Some(&text));
                    Ok(())
                }),
        )
        .action(
            Action::button("paste-slot", "Paste Slot")
                .icon("clipboard-paste")
                .inputs(vec![
                    slot_input(),
                    input::select("output", "Put the text", &[("replace", "Where the cursor is (in place of the selection)"), ("copy", "On the clipboard")]).default_value("replace"),
                ])
                .live(std::time::Duration::from_secs(60), |ic| {
                    let slot = slot_of(&ic.inputs);
                    Ok(Visual::new().label(slot_label(&slot, slot_text(&slot).as_deref())))
                })
                .on_press(|ctx| {
                    let slot = slot_of(&ctx.inputs);
                    let Some(text) = slot_text(&slot) else {
                        return ctx.fail(format!("Slot {slot} is empty: save some text to it first"));
                    };
                    show_slot(ctx, &slot, Some(&text));
                    deliver(ctx, &text, &format!("Slot {slot}"))
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::transform::*;

    fn t(id: &str, s: &str) -> String {
        apply(id, s).unwrap()
    }

    #[test]
    fn cases() {
        assert_eq!(t("upper", "Hello wörld ß"), "HELLO WÖRLD SS");
        assert_eq!(t("lower", "HeLLo ÄÖÜ"), "hello äöü");
        assert_eq!(t("title", "the QUICK brown fox"), "The Quick Brown Fox");
        assert_eq!(t("title", "don't stop well-known o’clock"), "Don't Stop Well-Known O’clock");
        assert_eq!(t("title", "  two\n\nlines  "), "  Two\n\nLines  ");
        assert_eq!(t("title", "élan vital 3rd"), "Élan Vital 3rd");
        assert_eq!(t("upper", ""), "");
    }

    #[test]
    fn trimming() {
        assert_eq!(t("trim", "  \n  hello   \n    indented  \t\n\n"), "hello\n    indented");
        assert_eq!(t("trim", "a  \r\nb  \r\n"), "a\r\nb");
        assert_eq!(t("trim", "   "), "");
        assert_eq!(t("trim", "no change"), "no change");
    }

    #[test]
    fn slugs() {
        assert_eq!(t("slug", "Hello, World!"), "hello-world");
        assert_eq!(t("slug", "  Crème brûlée -- Recipe #2  "), "creme-brulee-recipe-2");
        assert_eq!(t("slug", "Don't Panic"), "dont-panic");
        assert_eq!(t("slug", "Straße & Œuvre"), "strasse-oeuvre");
        assert_eq!(t("slug", "already-a-slug"), "already-a-slug");
        assert_eq!(t("slug", "日本語 テキスト"), "日本語-テキスト");
        assert_eq!(t("slug", "!!!"), "");
        assert_eq!(t("slug", "snake_case_name"), "snake-case-name");
    }

    #[test]
    fn url() {
        assert_eq!(t("url-encode", "a b&c=d/é?~._-"), "a%20b%26c%3Dd%2F%C3%A9%3F~._-");
        assert_eq!(t("url-decode", "a%20b%26c%3Dd%2F%C3%A9%3F~._-"), "a b&c=d/é?~._-");
        assert_eq!(t("url-decode", "q=hello+world&x=%2B"), "q=hello world&x=+");
        assert_eq!(t("url-decode", "100% sure %zz %4"), "100% sure %zz %4", "bad escapes stay");
        assert_eq!(t("url-decode", "%e2%9c%93"), "✓", "lower-case hex");
        assert!(apply("url-decode", "%FF%FE").is_err(), "not UTF-8");
        for s in ["", "plain", "ünïcödé 🎉 & = + % /", "line\nbreak"] {
            assert_eq!(t("url-decode", &t("url-encode", s)), s, "round trip {s:?}");
        }
    }

    #[test]
    fn base64() {
        assert_eq!(t("base64-encode", "hello"), "aGVsbG8=");
        assert_eq!(t("base64-encode", "é🎉"), "w6nwn46J");
        assert_eq!(t("base64-decode", "aGVsbG8="), "hello");
        assert_eq!(t("base64-decode", "aGVsbG8"), "hello", "no padding");
        assert_eq!(t("base64-decode", " aGVs\nbG8= \n"), "hello", "whitespace and line breaks");
        // "??>" → standard "Pz8+", URL-safe "Pz8-".
        assert_eq!(t("base64-decode", "Pz8+"), "??>");
        assert_eq!(t("base64-decode", "Pz8-"), "??>");
        assert!(apply("base64-decode", "not base64!").is_err());
        assert!(apply("base64-decode", "").is_err());
        assert_eq!(apply("base64-decode", "/w==").unwrap_err(), "That Base64 holds binary data, not text");
        for s in ["", "a", "ab", "abc", "ünïcödé 🎉", "multi\nline"] {
            if !s.is_empty() {
                assert_eq!(t("base64-decode", &t("base64-encode", s)), s);
            }
        }
    }

    #[test]
    fn json_format() {
        assert_eq!(t("json-format", r#"{"b":1,"a":[1,2,{"c":null}]}"#), "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2,\n    {\n      \"c\": null\n    }\n  ]\n}");
        assert_eq!(t("json-minify", "{\n  \"z\": \"é\",\n  \"a\": true\n}\n"), r#"{"z":"é","a":true}"#, "key order kept");
        assert_eq!(t("json-format", "  [ ]  "), "[]");
        let e = apply("json-format", "{ nope }").unwrap_err();
        assert!(e.starts_with("Not valid JSON: "), "{e}");
    }

    #[test]
    fn counting() {
        assert_eq!(counts("Hello  world\nsecond line here\n"), Counts { words: 5, chars: 30, lines: 2 });
        assert_eq!(counts(""), Counts { words: 0, chars: 0, lines: 0 });
        assert_eq!(counts("a\r\nb"), Counts { words: 2, chars: 3, lines: 2 });
        assert_eq!(counts("héllo"), Counts { words: 1, chars: 5, lines: 1 });
        assert_eq!(plural(1, "word", "words"), "1 word");
        assert_eq!(plural(0, "word", "words"), "0 words");
        assert_eq!(plural(1234567, "word", "words"), "1,234,567 words");
        assert_eq!(plural(123, "char", "chars"), "123 chars");
    }

    #[test]
    fn previews_and_ops() {
        assert_eq!(preview("short", 10), "short");
        assert_eq!(preview("a much longer line", 6), "a much…");
        assert_eq!(preview("one\ntwo", 10), "one…");
        assert!(apply("nope", "x").is_err());
        for (i, o) in OPS.iter().enumerate() {
            assert!(OPS[i + 1..].iter().all(|p| p.id != o.id), "duplicate {}", o.id);
            assert!(o.label.chars().count() <= 18);
            assert!(apply(o.id, "{}").is_ok() || o.id.ends_with("decode"), "{}", o.id);
        }
        assert!(OPS.len() <= 16, "fits a wheel");
    }

    #[test]
    fn slots_persist() {
        let dir = std::env::temp_dir().join(format!("text-tools-test-{}", std::process::id()));
        let path = dir.join("slots.json");
        let mut s = super::load_slots(&path);
        assert!(s.is_empty());
        s.insert("3".into(), "kept\ntext".into());
        super::save_slots(&path, &s).unwrap();
        assert_eq!(super::load_slots(&path).get("3").map(String::as_str), Some("kept\ntext"));
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(super::slot_of(&serde_json::json!({"slot": "4"})), "4");
        assert_eq!(super::slot_of(&serde_json::json!({"slot": 9})), "9");
        assert_eq!(super::slot_of(&serde_json::json!({"slot": "12"})), "1");
        assert_eq!(super::slot_label("2", None), "Slot 2\nempty");
    }
}
