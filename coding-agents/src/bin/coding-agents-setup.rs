//! Coding Agents setup: put the hook program in the data folder and connect the chosen agents
//! (or disconnect the ones unticked).

#[allow(dead_code)]
#[path = "../install.rs"]
mod install;
#[allow(dead_code)]
#[path = "../store.rs"]
mod store;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let data = ctx.data_dir();
        let claude = ctx.input_bool("claude");
        let codex = ctx.input_bool("codex");
        if !claude && !codex {
            return ctx.fail("Tick Claude Code, Codex or both");
        }
        let wait: u64 = ctx.input_str("approval_wait").trim().parse().unwrap_or(30);
        let exe = install::plugin_binary().or_else(|e| ctx.fail(e))?;
        install::refresh_hook_binary(&exe, &data).or_else(|e| ctx.fail(format!("Couldn't copy the hook program into {}: {e}", data.display())))?;
        let mut done = vec![];
        match install::setup_claude(&data, wait, claude) {
            Ok(changed) => done.push(format!("Claude Code {}", if !claude { "disconnected" } else if changed { "connected" } else { "already connected" })),
            Err(e) => return ctx.fail(e),
        }
        match install::setup_codex(&data, codex) {
            Ok(changed) => done.push(format!("Codex {}", if !codex { "disconnected" } else if changed { "connected" } else { "already connected" })),
            Err(e) => return ctx.fail(e),
        }
        ctx.log(format!("[setup] {}", done.join(", ")));
        let agents: Vec<&str> = [("claude", claude), ("codex", codex)].iter().filter(|(_, on)| *on).map(|(a, _)| *a).collect();
        knobdeck_sdk::save_env(&[("AGENTS", &agents.join(",")), ("WATCH_CODEX", if codex { "1" } else { "0" }), ("APPROVAL_WAIT", &wait.to_string())])?;
        Ok(())
    })
}
