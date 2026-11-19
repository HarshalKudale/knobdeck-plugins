//! Coding Agents plugin for KnobDeck: what Claude Code and Codex sessions are doing (working,
//! waiting for approval, your turn) on keys, and approve or deny Claude Code's permission
//! requests from the deck.
//!
//! The agents report through their own hook interfaces: Claude Code hooks and Codex's `notify`
//! program both run this binary in hook mode (`hook.rs`), which records each session in the data
//! folder (`store.rs`). Setup adds those entries to the agents' configs (`install.rs`).

mod codex;
mod hook;
#[allow(dead_code)] // the rest is used by setup
mod install;
mod store;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, Action, Ctx, Error, InitialCtx, Plugin, Result, Runtime, Value, Visual};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use store::{Session, Status};

const LIVE: Duration = Duration::from_millis(700);
/// "Your turn" stops counting as needing you after this long.
const DONE_FRESH_MS: u64 = 60 * 60 * 1000;
/// A turn not heard from in this long is shown as idle (the agent was closed mid-turn).
const STALE_MS: u64 = 30 * 60 * 1000;

fn data() -> PathBuf {
    knobdeck_sdk::data_dir()
}

// ── The current picture ───────────────────────────────────────────────────────

static SNAPSHOT: Mutex<Option<(Instant, Vec<Session>)>> = Mutex::new(None);

/// Every known session, what needs you first, newest first; read at most every 400 ms.
fn sessions() -> Vec<Session> {
    let mut guard = SNAPSHOT.lock().unwrap();
    if let Some((at, s)) = guard.as_ref() {
        if at.elapsed() < Duration::from_millis(400) {
            return s.clone();
        }
    }
    let now = store::now_ms();
    let mut all = store::load_sessions(&data());
    if knobdeck_sdk::env_or("WATCH_CODEX", "1") == "1" {
        codex::merge(&mut all, codex::active());
    }
    for s in &mut all {
        let age = now.saturating_sub(s.updated);
        if (s.status == Status::Working && age > STALE_MS) || (s.status == Status::Done && age > DONE_FRESH_MS) {
            s.status = Status::Idle;
        }
    }
    let filter = knobdeck_sdk::env_or("AGENTS", "claude,codex");
    all.retain(|s| filter.split(',').any(|a| a == s.agent));
    all.sort_by(|a, b| a.status.rank().cmp(&b.status.rank()).then(b.updated.cmp(&a.updated)));
    *guard = Some((Instant::now(), all.clone()));
    all
}

fn forget_snapshot() {
    *SNAPSHOT.lock().unwrap() = None;
}

fn of_agent(all: Vec<Session>, agent: &str) -> Vec<Session> {
    all.into_iter().filter(|s| agent.is_empty() || agent == "all" || s.agent == agent).collect()
}

/// " · 3m" once a status has lasted a minute (seconds would redraw the key every second).
fn ago(ms: u64) -> String {
    let secs = store::now_ms().saturating_sub(ms) / 1000;
    match secs {
        0..=59 => String::new(),
        60..=3599 => format!(" · {}m", secs / 60),
        _ => format!(" · {}h", secs / 3600),
    }
}

fn session_visual(s: &Session, show_agent: bool) -> Visual {
    let head = if show_agent { format!("{} · {}", store::agent_name(&s.agent), s.name()) } else { s.name() };
    let tail = match s.status {
        Status::Working | Status::Approval | Status::Input | Status::Done => format!("{}{}", s.status.label(), ago(s.since)),
        Status::Idle => s.status.label().to_string(),
    };
    Visual::new().label(format!("{head}\n{tail}")).icon(s.status.icon()).state(status_state(s.status))
}

fn status_state(s: Status) -> &'static str {
    match s {
        Status::Working => "working",
        Status::Approval | Status::Input | Status::Done => "attention",
        Status::Idle => "idle",
    }
}

// ── Agent Status: one key that steps through the sessions ─────────────────────

/// Which session (by agent and id) each status key shows after a press.
static PICKED: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None);

fn status_visual(ic: &InitialCtx) -> Result<Visual> {
    let list = of_agent(sessions(), &ic.input_str("agent"));
    let Some(first) = list.first() else {
        return Ok(Visual::new().label("No sessions").icon("bot").state("idle"));
    };
    let picked = PICKED.lock().unwrap().get_or_insert_with(HashMap::new).get(&ic.instance_id).cloned();
    // A session that needs you always wins; otherwise the one picked by pressing.
    let s = match picked {
        Some((agent, id)) if !first.status.needs_you() => list.iter().find(|s| s.agent == agent && s.id == id).unwrap_or(first),
        _ => first,
    };
    let mut v = session_visual(s, ic.input_bool("show_agent"));
    if list.len() > 1 {
        let pos = list.iter().position(|x| x.agent == s.agent && x.id == s.id).unwrap_or(0) + 1;
        let label = v.label.clone().unwrap_or_default();
        v = v.label(format!("{label} ({pos}/{})", list.len()));
    }
    Ok(v)
}

fn press_status(ctx: &Ctx) -> Result {
    let list = of_agent(sessions(), &ctx.input_str("agent"));
    if list.is_empty() {
        return Ok(());
    }
    let mut guard = PICKED.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    let cur = map.get(&ctx.instance_id).and_then(|(a, i)| list.iter().position(|s| &s.agent == a && &s.id == i));
    let next = &list[cur.map_or(1, |c| c + 1) % list.len()];
    // Looking at a finished session is seeing it: it stops asking for attention.
    acknowledge(next);
    map.insert(ctx.instance_id.clone(), (next.agent.clone(), next.id.clone()));
    drop(guard);
    let ic = InitialCtx { instance_id: ctx.instance_id.clone(), action_id: ctx.action_id.clone(), inputs: ctx.inputs.clone() };
    ctx.set_visual(status_visual(&ic)?);
    Ok(())
}

/// "Your turn" → idle once the user has seen it.
fn acknowledge(s: &Session) {
    if s.status == Status::Done {
        let _ = store::update_session(&data(), &s.agent, &s.id, |x| x.set_status(Status::Idle));
        forget_snapshot();
    }
}

// ── Approvals ─────────────────────────────────────────────────────────────────

fn answer_visual(allow: bool) -> Result<Visual> {
    let pending = store::pending(&data());
    let (verb, icon) = if allow { ("Approve", "check") } else { ("Deny", "x") };
    Ok(match pending.first() {
        Some(p) => {
            let more = if pending.len() > 1 { format!(" (+{})", pending.len() - 1) } else { String::new() };
            let short: String = p.summary().chars().take(40).collect();
            Visual::new().label(format!("{verb}{more}\n{short}")).icon(icon).state("attention")
        }
        None => Visual::new().label(format!("{verb}\nNothing to answer")).icon(icon).state("idle"),
    })
}

fn press_answer(ctx: &Ctx, allow: bool) -> Result {
    let d = data();
    let Some(p) = store::pending(&d).into_iter().next() else {
        return Err(Error::msg(if setup_done(&d) { "No request is waiting for an answer" } else { "Run this plugin's setup to connect Claude Code" }));
    };
    store::answer(&d, &p.id, allow)?;
    ctx.log(format!("{} {}", if allow { "approved" } else { "denied" }, p.summary()));
    forget_snapshot();
    ctx.set_visual(answer_visual(allow)?);
    Ok(())
}

fn setup_done(d: &Path) -> bool {
    install::hook_path(d).exists()
}

// ── Background: heartbeat for the hooks ───────────────────────────────────────

fn touch(path: &Path) {
    let _ = std::fs::write(path, store::now_ms().to_string());
}

/// Tell the hooks the plugin runs and whether a key can answer requests; keep the hook binary in
/// step with this version of the plugin; prune old files.
fn heartbeat(rt: Runtime) {
    let d = rt.data_dir().to_path_buf();
    if setup_done(&d) {
        if let Ok(exe) = std::env::current_exe() {
            match install::refresh_hook_binary(&exe, &d) {
                Ok(true) => rt.log("updated the hook program"),
                Ok(false) => {}
                Err(e) => rt.log(format!("couldn't update the hook program: {e}")),
            }
        }
    }
    let mut last_prune = Instant::now() - Duration::from_secs(3600);
    loop {
        touch(&store::alive_path(&d));
        let answering = ["approve", "deny", "approvals-wheel"].iter().any(|a| !rt.instances_of(a).is_empty());
        if answering {
            touch(&store::approvals_flag(&d));
        } else {
            let _ = std::fs::remove_file(store::approvals_flag(&d));
        }
        if last_prune.elapsed() > Duration::from_secs(600) {
            store::prune(&d);
            last_prune = Instant::now();
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn agent_input() -> Value {
    input::select("agent", "Sessions of", &[("all", "Claude Code and Codex"), ("claude", "Claude Code"), ("codex", "Codex")]).default_value("all")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("hook") {
        hook::run(&args[1..]);
    }
    load_env();
    Plugin::new(&format!("coding-agents-{}", std::env::consts::OS), "Coding Agents")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("See what your Claude Code and Codex sessions are doing (working, waiting for approval, your turn) and approve or deny Claude Code's requests from a key.")
        .icon("app-icon.svg")
        .setup("coding-agents-setup", None)
        .setup_inputs(vec![
            input::checkbox("claude", "Connect Claude Code (adds KnobDeck's hooks to ~/.claude/settings.json)").default_value(true),
            input::checkbox("codex", "Connect Codex (sets notify in ~/.codex/config.toml)").default_value(true),
            input::select("approval_wait", "Approve or Deny keys: wait for a press for up to", &[("15", "15 seconds"), ("30", "30 seconds"), ("60", "1 minute"), ("120", "2 minutes"), ("0", "Don't wait (answer in the terminal)")])
                .default_value("30")
                .with("help", "Only while an Approve or Deny key is placed. Claude Code shows its own prompt when the time is up."),
        ])
        .on_ready(|rt| {
            let rt = rt.clone();
            std::thread::spawn(move || heartbeat(rt));
            Ok(())
        })
        .on_shutdown(|rt| {
            // A hook waiting for a key hands the request back to the terminal at once.
            let _ = std::fs::remove_file(store::approvals_flag(rt.data_dir()));
            let _ = std::fs::remove_file(store::alive_path(rt.data_dir()));
        })
        .action(
            Action::button("status", "Agent Status")
                .icon("bot")
                .inputs(vec![agent_input(), input::checkbox("show_agent", "Show the agent's name").default_value(false)])
                .live(LIVE, status_visual)
                .on_press(press_status),
        )
        .action(
            Action::button("attention", "Needs Me")
                .icon("bell")
                .inputs(vec![agent_input()])
                .live(LIVE, |ic| {
                    let list = of_agent(sessions(), &ic.input_str("agent"));
                    let need: Vec<&Session> = list.iter().filter(|s| s.status.needs_you()).collect();
                    let working = list.iter().filter(|s| s.status == Status::Working).count();
                    Ok(match need.first() {
                        Some(s) => Visual::new().label(format!("{} need{} you\n{}", need.len(), if need.len() == 1 { "s" } else { "" }, s.name())).icon("bell-ring").state("attention"),
                        None if working > 0 => Visual::new().label(format!("{working} working")).icon("loader-circle").state("working"),
                        None => Visual::new().label("All quiet").icon("bell").state("idle"),
                    })
                })
                .on_press(|ctx| {
                    // Mark every finished session as seen.
                    for s in of_agent(sessions(), &ctx.input_str("agent")) {
                        acknowledge(&s);
                    }
                    Ok(())
                }),
        )
        .action(Action::button("approve", "Approve").icon("check").live(LIVE, |_| answer_visual(true)).on_press(|ctx| press_answer(ctx, true)))
        .action(Action::button("deny", "Deny").icon("x").live(LIVE, |_| answer_visual(false)).on_press(|ctx| press_answer(ctx, false)))
        .action(
            Action::wheel("approvals-wheel", "Approvals Wheel")
                .icon("shield-question")
                .on_open(|o| {
                    let pending = store::pending(&data());
                    if pending.is_empty() {
                        return o.fail("No request is waiting for an answer");
                    }
                    Ok(pending
                        .iter()
                        .take(8)
                        .flat_map(|p| {
                            let short: String = p.summary().chars().take(28).collect();
                            [json!({"id": format!("allow:{}", p.id), "label": format!("Allow {short}"), "icon": "check"}), json!({"id": format!("deny:{}", p.id), "label": format!("Deny {short}"), "icon": "x"})]
                        })
                        .collect())
                })
                .on_child_press(|ctx| {
                    let Some((verb, id)) = ctx.child_id.as_deref().and_then(|c| c.split_once(':')) else { return Ok(()) };
                    if !store::pending(&data()).iter().any(|p| p.id == id) {
                        return Err(Error::msg("That request was already answered"));
                    }
                    store::answer(&data(), id, verb == "allow")?;
                    Ok(())
                }),
        )
        .action(
            Action::wheel("sessions-wheel", "Sessions Wheel")
                .icon("layout-grid")
                .inputs(vec![agent_input()])
                .on_open(|o| {
                    let list = of_agent(sessions(), &o.input_str("agent"));
                    if list.is_empty() {
                        return o.fail("No Claude Code or Codex sessions yet");
                    }
                    Ok(list.iter().take(16).map(|s| json!({"id": format!("{}:{}", s.agent, s.id), "label": format!("{} · {}", s.name(), s.status.label()), "icon": s.status.icon()})).collect())
                })
                .on_child_press(|ctx| {
                    // Pressing a finished session marks it seen.
                    let Some((agent, id)) = ctx.child_id.as_deref().and_then(|c| c.split_once(':')) else { return Ok(()) };
                    if let Some(s) = sessions().into_iter().find(|s| s.agent == agent && s.id == id) {
                        acknowledge(&s);
                    }
                    Ok(())
                }),
        )
        .run()
}
