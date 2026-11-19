//! Neovim plugin for KnobDeck: run Ex commands and Lua, save, switch buffers, toggle options and
//! show the mode and file, over Neovim's own msgpack-RPC API on its `--listen` address. Every
//! Neovim (0.9+) listens by default, so running ones are found without setup.

mod rpc;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use rpc::{Addr, Client};
use rmpv::Value as Mp;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(1500);
const AUTO: &str = "__auto__";
const ICON: &str = "terminal";

/// Settings in `.env`: an address to try before the discovered ones.
const ENV_ADDR: &str = "NVIM_ADDRESS";

/// Every reachable Neovim: the configured address, then discovered ones, newest first.
fn servers() -> Vec<Addr> {
    let mut list: Vec<Addr> = std::env::var(ENV_ADDR).ok().as_deref().and_then(rpc::parse_addr).into_iter().collect();
    for a in rpc::discover() {
        if !list.contains(&a) {
            list.push(a);
        }
    }
    list
}

/// The Neovim an action talks to: the one chosen (or typed) in the action, else the newest that
/// answers.
fn client(inputs: &Value) -> Result<Client> {
    let pick = inputs["server"].as_str().unwrap_or(AUTO).trim();
    if !pick.is_empty() && pick != AUTO {
        let addr = rpc::parse_addr(pick).ok_or_else(|| Error::msg("Choose a Neovim"))?;
        return Client::open(&addr);
    }
    for a in servers() {
        if let Ok(c) = Client::open(&a) {
            return Ok(c);
        }
    }
    Err(Error::msg("No Neovim is running (KnobDeck finds Neovim 0.9+ by its server socket)"))
}

fn server_input() -> Value {
    input::query_combobox("server", "Neovim", "servers").with("default", AUTO).placeholder("Newest Neovim, or an address (--listen)")
}

const DESCRIBE: &str = "return { vim.fn.fnamemodify(vim.fn.getcwd(), ':~'), vim.fn.expand('%:t') }";

fn describe(addr: &Addr) -> Option<String> {
    let mut c = Client::open(addr).ok()?;
    let (_, blocking) = c.mode().ok()?;
    let pid = rpc::pid_of(addr).map(|p| format!(" · pid {p}")).unwrap_or_default();
    if blocking {
        return Some(format!("Neovim{pid} (waiting for input)"));
    }
    let v = c.lua(DESCRIBE, vec![]).ok()?;
    let cwd = v[0].as_str().unwrap_or("").to_string();
    let file = v[1].as_str().unwrap_or("");
    Some(if file.is_empty() { format!("{cwd}{pid}") } else { format!("{cwd} — {file}{pid}") })
}

/// Each line is one Ex command; a leading ':' is dropped.
fn command_lines(text: &str) -> Vec<String> {
    text.lines().map(|l| l.trim().trim_start_matches(':').trim()).filter(|l| !l.is_empty() && !l.starts_with('"')).map(String::from).collect()
}

/// A Lua result as short text for a toast; nothing for nil.
fn show(v: &Mp) -> Option<String> {
    match v {
        Mp::Nil => None,
        Mp::String(s) => Some(s.as_str().unwrap_or("").to_string()),
        Mp::Boolean(b) => Some(b.to_string()),
        Mp::Integer(i) => Some(i.to_string()),
        Mp::F32(f) => Some(f.to_string()),
        Mp::F64(f) => Some(f.to_string()),
        other => Some(other.to_string()),
    }
}

const OPTIONS: &[(&str, &str)] = &[
    ("wrap", "Wrap lines (wrap)"),
    ("relativenumber", "Relative numbers (relativenumber)"),
    ("number", "Line numbers (number)"),
    ("spell", "Spell checking (spell)"),
    ("list", "Show whitespace (list)"),
    ("cursorline", "Cursor line (cursorline)"),
    ("hlsearch", "Highlight search (hlsearch)"),
    ("ignorecase", "Ignore case (ignorecase)"),
    ("expandtab", "Expand tabs (expandtab)"),
    ("custom", "Other option…"),
];

fn option_name(ctx_inputs: &Value) -> Result<String> {
    let pick = ctx_inputs["option"].as_str().unwrap_or("wrap");
    let name = if pick == "custom" { ctx_inputs["custom"].as_str().unwrap_or("").trim().to_string() } else { pick.to_string() };
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(Error::msg("Enter an option name, such as wrap"));
    }
    Ok(name)
}

/// `vim.o` sets an option the way `:set` does (window-local ones for the current window).
const OPTION_GET: &str = "local n = ...\nlocal v = vim.o[n]\nif type(v) ~= 'boolean' then error(n .. ' is not an on/off option', 0) end\nreturn v";
const OPTION_TOGGLE: &str = "local n = ...\nlocal v = vim.o[n]\nif type(v) ~= 'boolean' then error(n .. ' is not an on/off option', 0) end\nvim.o[n] = not v\nreturn vim.o[n]";

/// Listed buffers: current index, count, current name.
const BUFFERS: &str = "local cur = vim.api.nvim_get_current_buf()\nlocal ids, at = {}, 0\nfor _, b in ipairs(vim.api.nvim_list_bufs()) do\n  if vim.bo[b].buflisted then\n    ids[#ids + 1] = b\n    if b == cur then at = #ids end\n  end\nend\nlocal name = vim.fn.fnamemodify(vim.api.nvim_buf_get_name(cur), ':t')\nif name == '' then name = '[No Name]' end\nreturn { at, ids, name }";

struct Buffers {
    /// 1-based; 0 when the current buffer isn't listed.
    at: usize,
    ids: Vec<i64>,
    name: String,
}

fn buffers(c: &mut Client) -> Result<Buffers> {
    let v = c.lua(BUFFERS, vec![])?;
    Ok(Buffers {
        at: v[0].as_u64().unwrap_or(0) as usize,
        ids: v[1].as_array().map(|a| a.iter().filter_map(|i| i.as_i64()).collect()).unwrap_or_default(),
        name: v[2].as_str().unwrap_or("").to_string(),
    })
}

fn share(b: &Buffers) -> f64 {
    match b.ids.len() {
        0 | 1 => 0.0,
        n => ((b.at.max(1) - 1) as f64 / (n - 1) as f64 * 100.0).round(),
    }
}

/// The buffer at a slider position (0–100).
fn buffer_at(ids: &[i64], value: f64) -> Option<i64> {
    let n = ids.len();
    if n == 0 {
        return None;
    }
    let i = ((value.clamp(0.0, 100.0) / 100.0) * (n - 1) as f64).round() as usize;
    ids.get(i.min(n - 1)).copied()
}

const STATUS: &str = "local name = vim.fn.expand('%:t')\nif name == '' then name = '[No Name]' end\nreturn { name, vim.bo.modified }";

fn status_label(mode: &str, file: &str, modified: bool) -> String {
    format!("{}\n{}{}", rpc::mode_name(mode), file, if modified { " ●" } else { "" })
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn run_commands(ctx: &Ctx) -> Result<()> {
    let text = ctx.expand(&ctx.input_str("command"))?;
    let lines = command_lines(&text);
    if lines.is_empty() {
        return ctx.fail("Enter a command, such as :write");
    }
    let mut c = client(&ctx.inputs)?;
    c.ready()?;
    for l in lines {
        c.command(&l)?;
    }
    Ok(())
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new(&format!("neovim-{}", std::env::consts::OS), "Neovim")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Run Ex commands and Lua in Neovim, save all, switch buffers on a dial, toggle options and see the mode and file, over Neovim's own RPC API.")
        .icon("app-icon.svg")
        .query("servers", |_| {
            let mut opts = vec![json!({"value": AUTO, "label": "Newest Neovim"})];
            opts.extend(servers().into_iter().filter_map(|a| describe(&a).map(|label| json!({"value": a.to_string(), "label": label}))));
            Ok(Value::Array(opts))
        })
        .action(
            Action::button("command", "Run Command")
                .icon(ICON)
                .inputs(vec![server_input(), input::textarea("command", "Ex command (one per line)").placeholder(":write").accepts_placeholders()])
                .on_press(run_commands),
        )
        .action(
            Action::button("lua", "Run Lua")
                .icon("code")
                .inputs(vec![
                    server_input(),
                    input::textarea("code", "Lua").placeholder("vim.cmd.nohlsearch()").accepts_placeholders(),
                    input::checkbox("show", "Show what it returns").default_value(false),
                ])
                .on_press(|ctx| {
                    let code = ctx.expand(&ctx.input_str("code"))?;
                    if code.trim().is_empty() {
                        return ctx.fail("Enter some Lua");
                    }
                    let mut c = client(&ctx.inputs)?;
                    c.ready()?;
                    let r = c.lua(&code, vec![])?;
                    if ctx.input_bool("show") {
                        if let Some(t) = show(&r) {
                            ctx.toast(t)?;
                        }
                    }
                    Ok(())
                }),
        )
        .action(
            Action::button("save-all", "Save All")
                .icon("save")
                .inputs(vec![server_input()])
                .on_press(|ctx| {
                    let mut c = client(&ctx.inputs)?;
                    c.ready()?;
                    c.command("wall")
                }),
        )
        .action(
            Action::slider("buffers", "Switch Buffer", 0.0, 100.0)
                .icon("files")
                .inputs(vec![server_input()])
                .live(POLL, |ic| {
                    let Ok(mut c) = client(&ic.inputs) else { return Ok(Visual::new().state(0).label("No Neovim")) };
                    if c.mode().is_ok_and(|(_, blocking)| blocking) {
                        return Ok(Visual::new());
                    }
                    let b = buffers(&mut c)?;
                    Ok(Visual::new().state(share(&b)).label(format!("{} ({}/{})", b.name, b.at, b.ids.len())))
                })
                .on_adjust(|ctx| {
                    let mut c = client(&ctx.inputs)?;
                    c.ready()?;
                    match ctx.value {
                        Some(v) => {
                            let b = buffers(&mut c)?;
                            let id = buffer_at(&b.ids, v).ok_or_else(|| Error::msg("No listed buffers"))?;
                            c.command(&format!("buffer {id}"))?;
                        }
                        None => {
                            let d = ctx.delta.unwrap_or(0.0).round() as i64;
                            if d == 0 {
                                return Ok(());
                            }
                            c.command(&format!("{}{}", d.abs(), if d > 0 { "bnext" } else { "bprevious" }))?;
                        }
                    }
                    let b = buffers(&mut c)?;
                    ctx.set_visual(Visual::new().state(share(&b)).label(format!("{} ({}/{})", b.name, b.at, b.ids.len())));
                    Ok(())
                }),
        )
        .action(
            Action::toggle("toggle-option", "Toggle Option")
                .icon("toggle-left")
                .inputs(vec![
                    server_input(),
                    input::select("option", "Option", OPTIONS).default_value("wrap"),
                    input::text("custom", "Option name").placeholder("cursorcolumn").with("showIf", json!({"key": "option", "equals": "custom"})),
                ])
                .state("off", "Off", "toggle-left")
                .state("on", "On", "toggle-right")
                .live(POLL, |ic| {
                    let Ok(name) = option_name(&ic.inputs) else { return Ok(Visual::new()) };
                    let Ok(mut c) = client(&ic.inputs) else { return Ok(Visual::new()) };
                    if c.mode().is_ok_and(|(_, blocking)| blocking) {
                        return Ok(Visual::new());
                    }
                    let v = c.lua(OPTION_GET, vec![Mp::from(name.as_str())])?;
                    Ok(Visual::new().state(on_off(v.as_bool().unwrap_or(false))))
                })
                .on_press(|ctx| {
                    let name = option_name(&ctx.inputs)?;
                    let mut c = client(&ctx.inputs)?;
                    c.ready()?;
                    let v = c.lua(OPTION_TOGGLE, vec![Mp::from(name.as_str())])?;
                    ctx.set_state(on_off(v.as_bool().unwrap_or(false)));
                    Ok(())
                }),
        )
        .action(
            Action::button("status", "Mode and File")
                .icon("file-code")
                .inputs(vec![server_input(), input::checkbox("save", "Press to save the file").default_value(false)])
                .live(POLL, |ic| {
                    let Ok(mut c) = client(&ic.inputs) else { return Ok(Visual::new().label("No Neovim")) };
                    let (mode, blocking) = c.mode()?;
                    if blocking {
                        return Ok(Visual::new().label(format!("{}\nwaiting", rpc::mode_name(&mode))));
                    }
                    let v = c.lua(STATUS, vec![])?;
                    Ok(Visual::new().label(status_label(&mode, v[0].as_str().unwrap_or(""), v[1].as_bool().unwrap_or(false))))
                })
                .on_press(|ctx| {
                    if !ctx.input_bool("save") {
                        return Ok(());
                    }
                    let mut c = client(&ctx.inputs)?;
                    c.ready()?;
                    c.command("write")
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines() {
        assert_eq!(command_lines(":w\n\n  :set wrap  \n\" comment\nbnext"), vec!["w", "set wrap", "bnext"]);
    }

    #[test]
    fn options() {
        assert_eq!(option_name(&json!({"option": "spell"})).unwrap(), "spell");
        assert_eq!(option_name(&json!({"option": "custom", "custom": " cursorcolumn "})).unwrap(), "cursorcolumn");
        assert!(option_name(&json!({"option": "custom", "custom": "wrap | !rm"})).is_err());
    }

    #[test]
    fn buffer_positions() {
        let b = Buffers { at: 3, ids: vec![1, 4, 7], name: "a".into() };
        assert_eq!(share(&b), 100.0);
        assert_eq!(buffer_at(&b.ids, 0.0), Some(1));
        assert_eq!(buffer_at(&b.ids, 50.0), Some(4));
        assert_eq!(buffer_at(&b.ids, 100.0), Some(7));
        assert_eq!(buffer_at(&[], 50.0), None);
        assert_eq!(status_label("i", "main.rs", true), "INSERT\nmain.rs ●");
    }
}
