//! Docker plugin for KnobDeck: start, stop and restart containers and Compose projects, and show
//! their state, through the Docker Engine API (Docker Desktop, Docker Engine, rootless Docker,
//! Colima, OrbStack and Podman all serve it on a local socket).

mod engine;

use engine::{Container, Endpoint, Engine};
use knobdeck_sdk::input;
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::sync::Mutex;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(3);

/// The engine that answered last; searched again when it stops answering.
static ENDPOINT: Mutex<Option<Endpoint>> = Mutex::new(None);

fn engine() -> Result<Engine> {
    if let Some(ep) = ENDPOINT.lock().unwrap().clone() {
        return Ok(Engine { endpoint: ep });
    }
    let e = Engine::find()?;
    *ENDPOINT.lock().unwrap() = Some(e.endpoint.clone());
    Ok(e)
}

/// Run `f` against the engine; when the remembered socket fails, search once more.
fn with_engine<T>(f: impl Fn(&Engine) -> Result<T>) -> Result<T> {
    let e = engine()?;
    match f(&e) {
        Err(err) if err.message().starts_with("Could not reach") => {
            *ENDPOINT.lock().unwrap() = None;
            f(&engine()?)
        }
        r => r,
    }
}

fn containers() -> Result<Vec<Container>> {
    with_engine(|e| e.containers())
}

fn find(name: &str) -> Result<Container> {
    if name.is_empty() {
        return Err(Error::msg("Choose a container"));
    }
    containers()?.into_iter().find(|c| c.name == name || c.id.starts_with(name)).ok_or_else(|| Error::msg(format!("No container named {name}")))
}

fn project(name: &str) -> Result<Vec<Container>> {
    if name.is_empty() {
        return Err(Error::msg("Choose a Compose project"));
    }
    let list: Vec<Container> = containers()?.into_iter().filter(|c| c.project.as_deref() == Some(name)).collect();
    if list.is_empty() {
        return Err(Error::msg(format!("No containers in project {name}")));
    }
    Ok(list)
}

fn run_label(on: bool) -> &'static str {
    if on {
        "running"
    } else {
        "stopped"
    }
}

/// Short text for a key: "Up 3 hours", "healthy", "Exited (0)".
fn status_label(c: &Container) -> String {
    let s = c.status.split(" (").next().unwrap_or(&c.status).to_string();
    match (c.running(), c.health()) {
        (true, Some(h)) => format!("{}\n{h}", c.name),
        (true, None) => format!("{}\n{s}", c.name),
        (false, _) => format!("{}\n{}", c.name, if c.state == "exited" { "Stopped" } else { &c.state }),
    }
}

fn container_input() -> Value {
    input::query_combobox("container", "Container", "containers")
}

fn project_input() -> Value {
    input::query_select("project", "Compose project", "projects", &[])
}

fn act(ctx: &Ctx, verb: &str) -> Result<()> {
    let c = find(&ctx.input_str("container"))?;
    with_engine(|e| e.act(verb, &c.name))?;
    ctx.fan_out_by(&["container"], run_label(verb != "stop"));
    Ok(())
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new(&format!("docker-{}", std::env::consts::OS), "Docker")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Start, stop and restart containers and Compose projects, and see what is running. Works with Docker Desktop, Docker Engine, Colima, OrbStack and Podman.")
        .icon("container")
        .query("containers", |q| {
            let list = containers().or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(list.into_iter().map(|c| json!({"value": c.name, "label": format!("{} ({})", c.name, c.image)})).collect()))
        })
        .query("projects", |q| {
            let mut names: Vec<String> = containers().or_else(|e| q.fail(e.message().to_string()))?.into_iter().filter_map(|c| c.project).collect();
            names.sort();
            names.dedup();
            Ok(Value::Array(names.into_iter().map(|n| json!({"value": n, "label": n})).collect()))
        })
        .action(
            Action::toggle("container", "Start / Stop Container")
                .icon("container")
                .inputs(vec![container_input()])
                .state("stopped", "Stopped", "square")
                .state("running", "Running", "play")
                .live(POLL, |ic| Ok(Visual::new().state(run_label(find(&ic.input_str("container"))?.running()))))
                .on_press(|ctx| {
                    let c = find(&ctx.input_str("container"))?;
                    let verb = if c.running() { "stop" } else { "start" };
                    with_engine(|e| e.act(verb, &c.name))?;
                    ctx.fan_out_by(&["container"], run_label(verb == "start"));
                    Ok(())
                }),
        )
        .action(Action::button("start", "Start Container").icon("play").inputs(vec![container_input()]).on_press(|ctx| act(ctx, "start")))
        .action(Action::button("stop", "Stop Container").icon("square").inputs(vec![container_input()]).on_press(|ctx| act(ctx, "stop")))
        .action(Action::button("restart", "Restart Container").icon("rotate-cw").inputs(vec![container_input()]).on_press(|ctx| act(ctx, "restart")))
        .action(
            Action::button("status", "Container Status")
                .icon("activity")
                .inputs(vec![container_input()])
                .live(POLL, |ic| {
                    let c = find(&ic.input_str("container"))?;
                    let icon = match (c.running(), c.health()) {
                        (true, Some("unhealthy")) => "heart-crack",
                        (true, _) => "circle-check",
                        (false, _) => "circle-stop",
                    };
                    Ok(Visual::new().label(status_label(&c)).icon(icon))
                }),
        )
        .action(
            Action::toggle("project", "Start / Stop Compose Project")
                .icon("layers")
                .inputs(vec![project_input()])
                .state("stopped", "Stopped", "square")
                .state("running", "Running", "play")
                .live(POLL, |ic| {
                    let list = project(&ic.input_str("project"))?;
                    let up = list.iter().filter(|c| c.running()).count();
                    Ok(Visual::new().state(run_label(up > 0)).label(format!("{}\n{up}/{} up", ic.input_str("project"), list.len())))
                })
                .on_press(|ctx| {
                    let list = project(&ctx.input_str("project"))?;
                    // Any container up: stop them all; none up: start them all.
                    let start = !list.iter().any(Container::running);
                    let verb = if start { "start" } else { "stop" };
                    let mut failed = vec![];
                    for c in &list {
                        if let Err(e) = with_engine(|en| en.act(verb, &c.name)) {
                            failed.push(format!("{}: {}", c.name, e.message()));
                        }
                    }
                    ctx.fan_out_by(&["project"], run_label(start));
                    if failed.is_empty() {
                        Ok(())
                    } else {
                        ctx.fail(failed.join("; "))
                    }
                }),
        )
        .action(
            Action::button("running-count", "Running Containers")
                .icon("boxes")
                .live(POLL, |_| {
                    let list = containers()?;
                    Ok(Visual::new().label(format!("{} / {}\nrunning", list.iter().filter(|c| c.running()).count(), list.len())))
                }),
        )
        .action(
            Action::wheel("containers-wheel", "Containers Wheel")
                .icon("container")
                .inputs(vec![input::checkbox("running_only", "Only running containers")])
                .on_open(|o| {
                    let only = o.input_bool("running_only");
                    let list = containers().or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list.into_iter().filter(|c| !only || c.running()).take(16).map(|c| json!({"id": c.name, "label": c.name, "icon": if c.running() { "square" } else { "play" }})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(name) = ctx.child_id.clone() else { return Ok(()) };
                    let c = find(&name)?;
                    with_engine(|e| e.act(if c.running() { "stop" } else { "start" }, &c.name))
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let c = Container { id: "a".into(), name: "db".into(), state: "running".into(), status: "Up 3 hours".into(), image: "pg".into(), project: None };
        assert_eq!(status_label(&c), "db\nUp 3 hours");
        let c = Container { status: "Up 3 hours (unhealthy)".into(), ..c };
        assert_eq!(status_label(&c), "db\nunhealthy");
        let c = Container { state: "exited".into(), status: "Exited (0) 2 days ago".into(), ..c };
        assert_eq!(status_label(&c), "db\nStopped");
    }
}
