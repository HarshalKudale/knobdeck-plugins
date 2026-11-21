//! Lightroom Classic setup: install the KnobDeck plug-in into Lightroom's Modules folder (unless
//! unticked), save the two ports, and on "Pair again" drop the pairing token so Lightroom asks to
//! allow KnobDeck once more.

#[path = "../install.rs"]
mod install;

fn port(ctx: &knobdeck_sdk::SetupCtx, key: &str, default: &str) -> Result<String, String> {
    let v = match ctx.inputs.get(key) {
        None | Some(knobdeck_sdk::Value::Null) => default.to_string(),
        Some(_) => ctx.input_str(key).trim().to_string(),
    };
    let v = if v.is_empty() { default.to_string() } else { v };
    match v.parse::<u32>() {
        Ok(1024..=65535) => Ok(v),
        _ => Err(format!("Enter a port from 1024 to 65535 (Lightroom shows its ports in File ▸ Plug-in Manager ▸ KnobDeck), not \"{v}\"")),
    }
}

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let command = port(ctx, "command_port", "58770").or_else(|e| ctx.fail(e))?;
        let event = port(ctx, "event_port", "58771").or_else(|e| ctx.fail(e))?;
        if command == event {
            return ctx.fail("The command and event ports must differ");
        }
        let wants_install = !matches!(ctx.inputs.get("install"), Some(knobdeck_sdk::Value::Bool(false)));
        if wants_install {
            let modules = install::modules_dir().or_else(|e| ctx.fail(e))?;
            let zip = install::download().or_else(|e| ctx.fail(format!("{e}. Install it yourself instead: download KnobDeck.lrplugin.zip from github.com/Knobdeck/knobdeck-lightroom-companion/releases, unzip it and add it in Lightroom (File ▸ Plug-in Manager ▸ Add), then run setup again with the install box unticked.")))?;
            let dest = install::install(&zip, &modules).or_else(|e| ctx.fail(e))?;
            ctx.log(format!("[setup] installed {} (restart Lightroom Classic to load it)", dest.display()));
        }
        knobdeck_sdk::save_env(&[("LIGHTROOM_COMMAND_PORT", &command), ("LIGHTROOM_EVENT_PORT", &event)])?;
        if ctx.input_bool("pair_again") {
            knobdeck_sdk::secrets::set("LIGHTROOM_TOKEN", "")?;
            ctx.log("[setup] pairing token dropped; Lightroom asks to allow KnobDeck again");
        }
        Ok(())
    })
}
