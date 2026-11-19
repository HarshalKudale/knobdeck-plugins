//! Voicemeeter plugin for KnobDeck: strip and bus mute and gain, strip routing (A1–A5 / B1–B3),
//! Macro Buttons and configurations, through Voicemeeter's own Remote API
//! (`VoicemeeterRemote64.dll`). Standard, Banana and Potato. Windows only: Voicemeeter is.

#[cfg_attr(not(windows), allow(dead_code))]
mod layout;

#[cfg(not(windows))]
fn main() {
    eprintln!("voicemeeter only runs on Windows");
    std::process::exit(1);
}

#[cfg(windows)]
mod remote;

#[cfg(windows)]
fn main() {
    app::run()
}

#[cfg(windows)]
mod app {
    use crate::layout::{self, param, Edition, Target};
    use crate::remote;
    use knobdeck_sdk::input::{self, InputExt};
    use knobdeck_sdk::{json, Action, Ctx, Error, InitialCtx, Plugin, Result, Value, Visual};
    use std::time::Duration;

    const POLL: Duration = Duration::from_secs(1);

    fn index(raw: &str, what: &str, count: usize) -> Result<usize> {
        let i: usize = raw.trim().parse().map_err(|_| Error::msg(format!("Choose a {what}")))?;
        if i >= count {
            return Err(Error::msg(format!("This Voicemeeter edition has no {what} {}", i + 1)));
        }
        Ok(i)
    }

    fn strip(raw: &str) -> Result<Target> {
        Ok(Target::Strip(index(raw, "strip", remote::edition()?.strip_count())?))
    }

    fn bus(raw: &str) -> Result<Target> {
        Ok(Target::Bus(index(raw, "bus", remote::edition()?.bus_count())?))
    }

    fn target(kind: &str, raw: &str) -> Result<Target> {
        if kind == "bus" {
            bus(raw)
        } else {
            strip(raw)
        }
    }

    /// The label the user gave it in Voicemeeter, else the edition's name for it.
    fn name(ed: Edition, t: Target) -> String {
        let label = remote::get_str(&param(t, "Label")).unwrap_or_default();
        if !label.trim().is_empty() {
            return label;
        }
        match t {
            Target::Strip(i) => ed.strip_default(i),
            Target::Bus(i) => ed.bus_default(i),
        }
    }

    fn options(kind: &str) -> Result<Value> {
        let ed = remote::edition()?;
        let (count, make): (usize, fn(usize) -> Target) = if kind == "bus" { (ed.bus_count(), Target::Bus) } else { (ed.strip_count(), Target::Strip) };
        Ok(Value::Array((0..count).map(|i| json!({"value": i.to_string(), "label": format!("{} · {}", i + 1, name(ed, make(i)))})).collect()))
    }

    fn strip_input() -> Value {
        input::query_select("strip", "Input strip", "strips", &[])
    }

    fn bus_input() -> Value {
        input::query_select("bus", "Output bus", "buses", &[])
    }

    fn on_label(on: bool) -> &'static str {
        if on {
            "on"
        } else {
            "off"
        }
    }

    /// Mute toggle for a strip (`kind` "strip") or a bus.
    fn mute(kind: &'static str, label: &str) -> Action {
        Action::toggle(&format!("{kind}-mute"), label)
            .icon("volume-x")
            .inputs(vec![if kind == "bus" { bus_input() } else { strip_input() }])
            .state("off", "Unmuted", "volume-2")
            .state("on", "Muted", "volume-x")
            .live(POLL, move |ic: &InitialCtx| {
                let t = target(kind, &ic.input_str(kind))?;
                Ok(Visual::new().state(on_label(remote::get(&param(t, "Mute"))? != 0.0)))
            })
            .on_press(move |ctx| {
                let t = target(kind, &ctx.input_str(kind))?;
                let p = param(t, "Mute");
                let on = remote::get(&p)? == 0.0;
                remote::set(&p, if on { 1.0 } else { 0.0 })?;
                ctx.fan_out_by(&[kind], on_label(on));
                Ok(())
            })
    }

    /// Gain fader in dB for a strip or a bus.
    fn gain(kind: &'static str, label: &str) -> Action {
        Action::slider(&format!("{kind}-gain"), label, layout::GAIN_MIN, layout::GAIN_MAX)
            .icon("sliders-vertical")
            .unit("dB")
            .inputs(vec![
                if kind == "bus" { bus_input() } else { strip_input() },
                input::text("step", "Step (dB)").placeholder("1").default_value("1"),
            ])
            .live(POLL, move |ic: &InitialCtx| {
                let t = target(kind, &ic.input_str(kind))?;
                Ok(Visual::new().state((remote::get(&param(t, "Gain"))? * 10.0).round() / 10.0))
            })
            .on_adjust(move |ctx| {
                let t = target(kind, &ctx.input_str(kind))?;
                let p = param(t, "Gain");
                let step = ctx.input_str("step").trim().parse::<f64>().ok().filter(|s| *s > 0.0).unwrap_or(1.0);
                let next = layout::next_gain(remote::get(&p)?, ctx.value, ctx.delta.unwrap_or(0.0), step);
                remote::set(&p, next)?;
                ctx.fan_out_by(&[kind], next);
                Ok(())
            })
    }

    fn routing(ctx_strip: &str, output: &str) -> Result<String> {
        let ed = remote::edition()?;
        let t = strip(ctx_strip)?;
        let out = layout::routing(ed, output).ok_or_else(|| Error::msg(format!("{} has no output {output}", ed.name())))?;
        Ok(param(t, &out))
    }

    fn macro_button(ctx: &Ctx) -> Result<i32> {
        ctx.input_str("button").trim().parse::<i32>().ok().filter(|b| (0..80).contains(b)).ok_or_else(|| Error::msg("Enter a macro button number from 0 to 79"))
    }

    pub fn run() {
        knobdeck_sdk::load_env();
        Plugin::new("voicemeeter-windows", "Voicemeeter")
            .version(env!("CARGO_PKG_VERSION"))
            .platforms(&["windows"])
            .author("KnobDeck")
            .description("Voicemeeter, Banana and Potato: strip and bus mute and gain on a dial, A1–A5 / B1–B3 routing, Macro Buttons and configurations.")
            .icon("app-icon.svg")
            .query("strips", |q| options("strip").or_else(|e| q.fail(e.message().to_string())))
            .query("buses", |q| options("bus").or_else(|e| q.fail(e.message().to_string())))
            .query("outputs", |q| {
                let ed = remote::edition().or_else(|e| q.fail(e.message().to_string()))?;
                Ok(Value::Array(ed.outputs_names().into_iter().map(|o| json!({"value": o, "label": o})).collect()))
            })
            .action(mute("strip", "Strip Mute"))
            .action(gain("strip", "Strip Gain"))
            .action(mute("bus", "Bus Mute"))
            .action(gain("bus", "Bus Gain"))
            .action(
                Action::toggle("route", "Strip Routing")
                    .icon("route")
                    .inputs(vec![strip_input(), input::query_select("output", "Output (A1–A5, B1–B3)", "outputs", &[])])
                    .state("off", "Off", "circle")
                    .state("on", "On", "circle-dot")
                    .live(POLL, |ic| Ok(Visual::new().state(on_label(remote::get(&routing(&ic.input_str("strip"), &ic.input_str("output"))?)? != 0.0))))
                    .on_press(|ctx| {
                        let p = routing(&ctx.input_str("strip"), &ctx.input_str("output"))?;
                        let on = remote::get(&p)? == 0.0;
                        remote::set(&p, if on { 1.0 } else { 0.0 })?;
                        ctx.fan_out_by(&["strip", "output"], on_label(on));
                        Ok(())
                    }),
            )
            .action(
                Action::button("macro", "Macro Button")
                    .icon("square-mouse-pointer")
                    .inputs(vec![
                        input::text("button", "Button number").placeholder("0").with("help", "As numbered in Voicemeeter's Macro Buttons, counting from 0."),
                        input::select("mode", "Button type", &[("push", "Push button (press and release)"), ("toggle", "On/off button (switch it)")]).with("default", "push"),
                    ])
                    .on_press(|ctx| {
                        let b = macro_button(ctx)?;
                        if ctx.input_str("mode") == "toggle" {
                            remote::macro_set(b, !remote::macro_get(b)?)
                        } else {
                            remote::macro_set(b, true)?;
                            std::thread::sleep(Duration::from_millis(50));
                            remote::macro_set(b, false)
                        }
                    }),
            )
            .action(
                Action::button("load-config", "Load Configuration")
                    .icon("folder-open")
                    .inputs(vec![input::text("path", "Configuration file (.xml)").placeholder("C:\\Users\\me\\Documents\\Voicemeeter\\streaming.xml")])
                    .on_press(|ctx| {
                        let path = ctx.input_str("path");
                        let path = path.trim().trim_matches('"');
                        if path.is_empty() {
                            return ctx.fail("Enter the path of a Voicemeeter configuration (.xml)");
                        }
                        if !std::path::Path::new(path).is_file() {
                            return ctx.fail(format!("No file at {path}"));
                        }
                        remote::edition()?;
                        remote::set_str("Command.Load", path)
                    }),
            )
            .action(Action::button("restart-engine", "Restart Audio Engine").icon("rotate-cw").on_press(|_| {
                remote::edition()?;
                remote::set("Command.Restart", 1.0)
            }))
            .action(Action::button("show", "Show Voicemeeter").icon("app-window").on_press(|_| match remote::edition() {
                Ok(_) => remote::set("Command.Show", 1.0),
                Err(_) => remote::launch(None),
            }))
            .run()
    }
}
