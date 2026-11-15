//! Mac automation: run an Apple Shortcut, an AppleScript / JXA script, or a Keyboard Maestro
//! macro, handing it what the user is working on (the selected text or files, the clipboard, or
//! text with placeholders) and using what it returns.
//!
//! Scripts run through `osascript` and Shortcuts through the `shortcuts` command-line tool:
//! `NSAppleScript` is main-thread only and handlers run on worker threads. Each target app asks
//! once for the Automation permission (KnobDeck → Finder, Keyboard Maestro Engine…).

use crate::script_text::{clean_error, literal, parse_km_macros};
use knobdeck_sdk::os::placeholders::Escape;
use knobdeck_sdk::{json, Ctx, Error, Result, Value};
use std::path::PathBuf;
use std::process::Command;

pub const KM_ENGINE: &str = "com.stairways.keyboardmaestro.engine";

/// What the action passes in.
pub enum Input {
    None,
    Text(String),
    Files(Vec<PathBuf>),
}

impl Input {
    /// As text: files one path per line.
    pub fn text(&self) -> String {
        match self {
            Input::None => String::new(),
            Input::Text(t) => t.clone(),
            Input::Files(f) => f.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"),
        }
    }
    /// As command-line arguments: one per file, or the text.
    fn args(&self) -> Vec<String> {
        match self {
            Input::None => vec![],
            Input::Text(t) => vec![t.clone()],
            Input::Files(f) => f.iter().map(|p| p.display().to_string()).collect(),
        }
    }
}

/// The input chosen in the action's `input` setting.
pub fn read_input(ctx: &Ctx) -> Result<Input> {
    use knobdeck_sdk::Need;
    let why = |need: Need, what: &str| Error::msg(ctx.context_error(need).unwrap_or_else(|| format!("{what} first")));
    Ok(match ctx.input_str("input").as_str() {
        "selection" => Input::Text(ctx.selection().ok_or_else(|| why(Need::Selection, "Select some text"))?.to_string()),
        "files" => {
            let f = ctx.files();
            if f.is_empty() {
                return Err(why(Need::Files, "Select files in Finder"));
            }
            Input::Files(f.to_vec())
        }
        "clipboard" => match (ctx.clipboard_files(), ctx.clipboard_text()) {
            (f, _) if !f.is_empty() => Input::Files(f.to_vec()),
            (_, Some(t)) => Input::Text(t.to_string()),
            _ => return Err(Error::msg("The clipboard has no text or files")),
        },
        "text" => Input::Text(ctx.expand(&ctx.input_str("input_text"))?),
        _ => Input::None,
    })
}

/// The `input` / `input_text` settings shared by the three actions.
pub fn input_inputs(with_files: bool) -> Vec<Value> {
    use knobdeck_sdk::input::{self, InputExt};
    let mut opts = vec![("none", "Nothing"), ("selection", "Selected text")];
    if with_files {
        opts.push(("files", "Selected files (Finder)"));
    }
    opts.extend([("clipboard", "Clipboard"), ("text", "Text…")]);
    vec![
        input::select("input", "Input", &opts).default_value("none"),
        input::textarea("input_text", "Text").accepts_placeholders().with("showIf", json!({"key": "input", "equals": "text"})),
    ]
}

/// The `output` setting: what to do with the result.
pub fn output_input() -> Value {
    use knobdeck_sdk::input::{self, InputExt};
    input::select("output", "Result", &[("none", "Ignore"), ("paste", "Paste it"), ("copy", "Copy it"), ("notify", "Show it as a notification")]).default_value("none")
}

/// Use a result as the `output` setting says. An empty result does nothing.
pub fn use_output(ctx: &Ctx, out: &str) -> Result {
    let out = out.trim_end_matches(['\n', '\r']);
    if out.is_empty() {
        return Ok(());
    }
    match ctx.input_str("output").as_str() {
        "paste" => {
            crate::clipboard::set_text(out)?;
            crate::system::paste()
        }
        "copy" => crate::clipboard::set_text(out),
        "notify" => notify(out),
        _ => Ok(()),
    }
}

fn notify(text: &str) -> Result {
    let short: String = text.chars().take(240).collect();
    osascript("AppleScript", &format!("display notification {} with title \"KnobDeck\"", literal(&short)), &[]).map(|_| ())
}

fn run(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| Error::msg(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(Error::msg(clean_error(&String::from_utf8_lossy(&out.stderr))))
    }
}

/// Run script text (AppleScript or JavaScript) or a script file; `args` reach its `run` handler.
pub fn osascript(language: &str, script: &str, args: &[String]) -> Result<String> {
    run(Command::new("/usr/bin/osascript").args(["-l", language, "-e", script]).args(args))
}

pub fn osascript_file(path: &str, args: &[String]) -> Result<String> {
    run(Command::new("/usr/bin/osascript").arg(path).args(args))
}

// ── Actions ───────────────────────────────────────────────────────────────────

pub fn run_applescript(ctx: &Ctx) -> Result {
    let input = read_input(ctx)?;
    let js = ctx.input_str("language") == "javascript";
    let out = if ctx.input_str("source") == "file" {
        let path = ctx.input_str("file");
        if path.trim().is_empty() {
            return Err(Error::msg("Choose a script file"));
        }
        let path = match path.trim().strip_prefix("~/") {
            Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
            None => path.trim().to_string(),
        };
        osascript_file(&path, &input.args())?
    } else {
        // Placeholders become string literals, so a value can't break out of its quotes.
        let script = ctx.expand_as(&ctx.input_str("script"), Escape::AppleScript)?;
        if script.trim().is_empty() {
            return Err(Error::msg("Write the script first"));
        }
        osascript(if js { "JavaScript" } else { "AppleScript" }, &script, &input.args())?
    };
    use_output(ctx, &out)
}

/// The user's shortcuts, by name.
pub fn shortcuts() -> Result<Vec<String>> {
    let out = run(Command::new("/usr/bin/shortcuts").arg("list"))?;
    let mut v: Vec<String> = out.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    v.sort_by_key(|s| s.to_lowercase());
    Ok(v)
}

pub fn run_shortcut(ctx: &Ctx) -> Result {
    let name = ctx.input_str("shortcut");
    if name.trim().is_empty() {
        return Err(Error::msg("Choose a shortcut"));
    }
    let input = read_input(ctx)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = std::env::temp_dir().join(format!("knobdeck-shortcut-{}-{nanos}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let mut cmd = Command::new("/usr/bin/shortcuts");
    cmd.arg("run").arg(name.trim());
    match &input {
        Input::None => {}
        Input::Files(f) => {
            for p in f {
                cmd.arg("--input-path").arg(p);
            }
        }
        Input::Text(t) => {
            let p = tmp.join("input.txt");
            std::fs::write(&p, t).map_err(|e| Error::msg(e.to_string()))?;
            cmd.arg("--input-path").arg(p);
        }
    }
    let want_output = ctx.input_str("output") != "none" && !ctx.input_str("output").is_empty();
    let out_path = tmp.join("output.txt");
    if want_output {
        cmd.arg("--output-path").arg(&out_path).args(["--output-type", "public.plain-text"]);
    }
    let r = run(&mut cmd);
    let out = std::fs::read_to_string(&out_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&tmp);
    r?;
    use_output(ctx, &out)
}

/// Keyboard Maestro's macros as `(uid, "Group ▸ Name")`, from the engine's `getmacros`.
pub fn km_macros() -> Result<Vec<(String, String)>> {
    if !crate::system::app_installed(KM_ENGINE) {
        return Err(Error::msg("Keyboard Maestro isn't installed"));
    }
    let xml = osascript("AppleScript", "tell application id \"com.stairways.keyboardmaestro.engine\" to getmacros with asstring", &[])?;
    parse_km_macros(xml.as_bytes())
}

pub fn run_km_macro(ctx: &Ctx) -> Result {
    let uid = ctx.input_str("macro");
    if uid.trim().is_empty() {
        return Err(Error::msg("Choose a macro"));
    }
    if !crate::system::app_installed(KM_ENGINE) {
        return Err(Error::msg("Keyboard Maestro isn't installed"));
    }
    let param = read_input(ctx)?.text();
    let mut script = format!("tell application id \"{KM_ENGINE}\" to do script {}", literal(uid.trim()));
    if !param.is_empty() {
        script.push_str(&format!(" with parameter {}", literal(&param)));
    }
    osascript("AppleScript", &script, &[]).map(|_| ())
}
