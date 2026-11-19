//! DAW Control (Mackie) setup: the two MIDI ports to the DAW and back. Empty on macOS / Linux means
//! KnobDeck's own virtual "KnobDeck Mackie" ports; Windows has no virtual ports, so both are needed
//! there (a loopMIDI pair). Each name must match a port that is there now.

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let output = ctx.input_str("output").trim().to_string();
        let input = ctx.input_str("input").trim().to_string();
        if !midi_io::HAS_VIRTUAL && (output.is_empty() || input.is_empty()) {
            return ctx.fail(format!("{}: name the port to the DAW and the port from the DAW", midi_io::NO_VIRTUAL_ON_WINDOWS));
        }
        let check = |name: &str, list: midi_io::Result<Vec<String>>, what: &str| -> Result<String, String> {
            if name.is_empty() {
                return Ok(String::new());
            }
            let list = list?;
            midi_io::find(&list, name).ok_or_else(|| {
                if list.is_empty() {
                    format!("No MIDI {what} port is there now")
                } else {
                    format!("No MIDI {what} port matches \"{name}\" (now: {})", list.join(", "))
                }
            })
        };
        let output = match check(&output, midi_io::outputs(), "output") {
            Ok(o) => o,
            Err(e) => return ctx.fail(e),
        };
        let input = match check(&input, midi_io::inputs(), "input") {
            Ok(i) => i,
            Err(e) => return ctx.fail(e),
        };
        knobdeck_sdk::save_env(&[("DAW_MACKIE_OUTPUT", &output), ("DAW_MACKIE_INPUT", &input)])?;
        let shown = |p: &str| if p.is_empty() { "KnobDeck Mackie (virtual)".to_string() } else { p.to_string() };
        ctx.log(format!("[setup] Mackie Control: to the DAW {}, from the DAW {}", shown(&output), shown(&input)));
        Ok(())
    })
}
