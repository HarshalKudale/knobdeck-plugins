//! Ableton Live setup: copy the KnobDeck Remote Script (the newest release of
//! Knobdeck/knobdeck-ableton-companion) into `<User Library>/Remote Scripts/KnobDeck`. Unticked,
//! it only finishes setup (the user copied the folder themselves).

#[path = "../install.rs"]
mod install;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let install = match ctx.inputs.get("install") {
            None | Some(knobdeck_sdk::Value::Null) => true,
            Some(_) => ctx.input_bool("install"),
        };
        if !install {
            ctx.log("[setup] Remote Script install skipped (installed by hand)");
            return Ok(());
        }
        let library = install::find_user_library(&ctx.input_str("user_library")).or_else(|e| ctx.fail(e.message()))?;
        let zip = install::fetch_zip().or_else(|e| ctx.fail(e.message()))?;
        let scripts = library.join("Remote Scripts");
        let version = install::install_zip(&zip, &scripts).or_else(|e| ctx.fail(e.message()))?;
        ctx.log(format!(
            "[setup] KnobDeck Remote Script {version} installed in {}. In Live: Settings ▸ Link, Tempo & MIDI ▸ Control Surface ▸ KnobDeck (restart Live if it was open).",
            scripts.join(install::FOLDER).display()
        ));
        Ok(())
    })
}
