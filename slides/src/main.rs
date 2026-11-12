//! Slides plugin for KnobDeck — run a slide show from KnobDeck: next / previous slide, start, end,
//! go to a slide, black or white screen and a slide counter. PowerPoint (Windows, macOS), Keynote
//! (macOS) and LibreOffice Impress (every OS), each through its own automation interface.

mod impress;
mod office;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{Action, Ctx, Error, Plugin, Result, Value, Visual};
use office::Op;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
enum App {
    PowerPoint,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Keynote,
    Impress,
}

/// Apps on this OS, in the order Auto looks for them.
fn apps() -> Vec<App> {
    if cfg!(target_os = "macos") {
        vec![App::PowerPoint, App::Keynote, App::Impress]
    } else if cfg!(windows) {
        vec![App::PowerPoint, App::Impress]
    } else {
        vec![App::Impress]
    }
}

fn app_options() -> Vec<(&'static str, &'static str)> {
    let mut v = vec![("auto", "Whichever is presenting")];
    for a in apps() {
        v.push(match a {
            App::PowerPoint => ("powerpoint", "PowerPoint"),
            App::Keynote => ("keynote", "Keynote"),
            App::Impress => ("impress", "LibreOffice Impress"),
        });
    }
    v
}

#[allow(unused_variables)]
fn show_running(app: App) -> bool {
    match app {
        #[cfg(any(windows, target_os = "macos"))]
        App::PowerPoint => office::powerpoint::show_running(),
        #[cfg(target_os = "macos")]
        App::Keynote => office::keynote::show_running(),
        App::Impress => impress::running(),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

#[allow(unused_variables)]
fn app_running(app: App) -> bool {
    match app {
        #[cfg(any(windows, target_os = "macos"))]
        App::PowerPoint => office::powerpoint::running(),
        #[cfg(target_os = "macos")]
        App::Keynote => office::keynote::running(),
        App::Impress => impress::show().is_ok(),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

/// The app a key drives: the one chosen, or (Auto) the one presenting, else the one open.
fn pick(inputs: &Value) -> Result<App> {
    let chosen = match inputs["app"].as_str().unwrap_or("auto") {
        "powerpoint" => Some(App::PowerPoint),
        "keynote" => Some(App::Keynote),
        "impress" => Some(App::Impress),
        _ => None,
    };
    if let Some(a) = chosen.filter(|a| apps().contains(a)) {
        return Ok(a);
    }
    let all = apps();
    if let Some(a) = all.iter().copied().find(|a| show_running(*a)) {
        return Ok(a);
    }
    // Nothing presenting: an open app (so Start works). LibreOffice is last: it needs pairing.
    if let Some(a) = all.iter().copied().find(|a| *a != App::Impress && app_running(*a)) {
        return Ok(a);
    }
    match impress::show() {
        Ok(_) => Ok(App::Impress),
        // A pairing prompt is worth showing; "not reachable" is not, when other apps exist.
        Err(e) if e.message() != impress::NOT_LISTENING || all.len() == 1 => Err(e),
        Err(_) => Err(Error::msg(if cfg!(target_os = "macos") { "Open a presentation in PowerPoint, Keynote or LibreOffice Impress" } else { "Open a presentation in PowerPoint or LibreOffice Impress" })),
    }
}

fn run(app: App, op: Op) -> Result<()> {
    match app {
        #[cfg(any(windows, target_os = "macos"))]
        App::PowerPoint => office::powerpoint::run(op),
        #[cfg(target_os = "macos")]
        App::Keynote => office::keynote::run(op),
        App::Impress => {
            let n = match op {
                Op::Goto(n) => (n - 1).max(0).to_string(),
                _ => String::new(),
            };
            let cmd: &[&str] = match op {
                Op::Next => &["transition_next"],
                Op::Previous => &["transition_previous"],
                Op::Start => &["presentation_start"],
                Op::Stop => &["presentation_stop"],
                Op::Goto(_) => &["goto_slide", &n],
                // LibreOffice blanks to black only.
                Op::Black | Op::White => &["presentation_blank_screen"],
                Op::Resume => &["presentation_resume"],
            };
            if !matches!(op, Op::Start) && !impress::show()?.running {
                return Err(Error::msg(office::NO_SHOW));
            }
            impress::send(cmd)
        }
        #[allow(unreachable_patterns)]
        _ => Err(Error::msg("This app isn't available on this computer")),
    }
}

/// (current slide, slide count), 1-based, while a show runs.
fn position(app: App) -> Result<Option<(i64, i64)>> {
    match app {
        #[cfg(any(windows, target_os = "macos"))]
        App::PowerPoint => office::powerpoint::position(),
        #[cfg(target_os = "macos")]
        App::Keynote => office::keynote::position(),
        App::Impress => {
            let s = impress::show()?;
            Ok(s.running.then_some((s.slide + 1, s.count)))
        }
        #[allow(unreachable_patterns)]
        _ => Ok(None),
    }
}

/// PowerPoint reports whether the screen is blanked; the others don't, so the key remembers.
#[allow(unused_variables)]
fn blanked(app: App, ctx: &Ctx, black: bool) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    if app == App::PowerPoint {
        if let Ok(b) = office::powerpoint::blank() {
            return b == Some(black);
        }
    }
    ctx.state.as_deref() == Some("blank")
}

fn app_input() -> Value {
    input::select("app", "App", &app_options()).with("default", "auto")
}

fn step_button(id: &str, label: &str, icon: &str, op: Op) -> Action {
    Action::button(id, label).icon(icon).inputs(vec![app_input()]).on_press(move |ctx| {
        let app = pick(&ctx.inputs)?;
        run(app, op)
    })
}

fn blank_toggle(id: &str, label: &str, black: bool) -> Action {
    Action::toggle(id, label)
        .icon(if black { "square" } else { "square-dashed" })
        .inputs(vec![app_input()])
        .state("showing", "Showing", if black { "square" } else { "square-dashed" })
        .state("blank", if black { "Black" } else { "White" }, "eye-off")
        .on_press(move |ctx| {
            let app = pick(&ctx.inputs)?;
            let is_blank = blanked(app, ctx, black);
            run(app, if is_blank { Op::Resume } else if black { Op::Black } else { Op::White })?;
            ctx.fan_out_by(&["app"], if is_blank { "showing" } else { "blank" });
            Ok(())
        })
}

fn counter_text(p: Option<(i64, i64)>) -> String {
    match p {
        Some((cur, total)) if total > 0 => format!("{cur} / {total}"),
        Some((cur, _)) => cur.to_string(),
        None => "No show".into(),
    }
}

fn main() {
    Plugin::new(&format!("slides-{}", std::env::consts::OS), "Slides")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description(if cfg!(target_os = "macos") {
            "Present from KnobDeck: next and previous slide, start, end, go to slide, black screen and a slide counter in PowerPoint, Keynote and LibreOffice Impress."
        } else if cfg!(windows) {
            "Present from KnobDeck: next and previous slide, start, end, go to slide, black screen and a slide counter in PowerPoint and LibreOffice Impress."
        } else {
            "Present from KnobDeck: next and previous slide, start, end, go to slide, black screen and a slide counter in LibreOffice Impress."
        })
        .icon("presentation")
        .ack_timeout_ms(6000)
        .action(step_button("next", "Next Slide", "chevron-right", Op::Next))
        .action(step_button("previous", "Previous Slide", "chevron-left", Op::Previous))
        .action(step_button("start", "Start Slide Show", "play", Op::Start))
        .action(step_button("end", "End Slide Show", "square", Op::Stop))
        .action(Action::button("goto", "Go to Slide").icon("hash").inputs(vec![app_input(), input::text("slide", "Slide number").placeholder("1").default_value("1")]).on_press(|ctx| {
            let Some(n) = ctx.input_f64("slide").filter(|n| *n >= 1.0) else {
                return ctx.fail("Slide number must be 1 or more");
            };
            run(pick(&ctx.inputs)?, Op::Goto(n as i64))
        }))
        .action(blank_toggle("black", "Black Screen", true))
        .action(blank_toggle("white", "White Screen", false))
        .action(
            Action::button("counter", "Slide Counter")
                .icon("presentation")
                .inputs(vec![app_input()])
                .live(Duration::from_secs(1), |ic| {
                    let app = match pick(&ic.inputs) {
                        Ok(a) => a,
                        // While LibreOffice waits for pairing, show the PIN to type there.
                        Err(e) if e.message().starts_with("Pair KnobDeck") => return Ok(Visual::new().label(format!("PIN {}", impress::pin()))),
                        Err(_) => return Ok(Visual::new().label("No show")),
                    };
                    Ok(Visual::new().label(counter_text(position(app).unwrap_or(None))))
                })
                .on_press(|ctx| {
                    let app = pick(&ctx.inputs)?;
                    run(app, Op::Next)?;
                    if app == App::Impress {
                        impress::settle(300);
                    }
                    ctx.set_label(counter_text(position(app).unwrap_or(None)));
                    Ok(())
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter() {
        assert_eq!(counter_text(Some((3, 12))), "3 / 12");
        assert_eq!(counter_text(None), "No show");
        assert_eq!(app_options()[0].0, "auto");
        assert_eq!(apps().contains(&App::Impress), true);
    }
}
