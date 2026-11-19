//! GitHub plugin for KnobDeck — notifications, review requests, your pull requests, workflow
//! runs (status, re-run failed jobs, dispatch) and repository counters, through GitHub's REST and
//! GraphQL APIs with the user's sign-in or personal access token.

// Device sign-in helpers are used by the setup program only.
#[allow(dead_code)]
mod api;

use api::{encode, repo, Client};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, open_url, Action, Error, Plugin, Result, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Notifications: GitHub's X-Poll-Interval (60 s) is honoured on top of this.
const NOTIFY_POLL: Duration = Duration::from_secs(60);
/// Search allows 30 requests a minute; two counters every 2 min stay far below.
const SEARCH_POLL: Duration = Duration::from_secs(120);
const RUN_POLL: Duration = Duration::from_secs(30);
const REPO_POLL: Duration = Duration::from_secs(300);

/// A combobox value arrives as `{value, label}` (picked or typed), older hosts send a string.
fn pick(inputs: &Value, key: &str) -> String {
    match &inputs[key] {
        Value::String(s) => s.trim().to_string(),
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        _ => String::new(),
    }
}

fn web(path: &str) -> String {
    format!("{}{path}", api::web_base())
}

/// Only github pages from API replies are opened.
fn open_page(url: &str) -> Result<()> {
    if !url.starts_with("https://") {
        return Err(Error::msg("GitHub sent no page to open"));
    }
    open_url(url)
}

fn count_visual(n: u64, more: bool, unit: &str) -> Visual {
    let text = if more { format!("{n}+") } else { n.to_string() };
    Visual::new().state(if n > 0 { "some" } else { "none" }).label(if unit.is_empty() { text } else { format!("{text} {unit}") })
}

// ---- notifications and searches ---------------------------------------------------------------

fn notifications(participating: bool) -> Result<Visual> {
    let r = Client::get()?.get_reply(&format!("/notifications?per_page=50{}", if participating { "&participating=true" } else { "" }))?;
    let n = r.body.as_array().map_or(0, |a| a.len()) as u64;
    Ok(count_visual(n, r.next, "unread"))
}

/// Issue search count (`q` is GitHub search syntax).
fn search_count(q: &str) -> Result<u64> {
    let r = Client::get()?.get_json(&format!("/search/issues?q={}&per_page=1", encode(q)))?;
    r["total_count"].as_u64().ok_or_else(|| Error::msg("GitHub sent no count"))
}

fn search_action(id: &str, label: &str, icon: &str, q: &'static str, unit: &'static str, page: &'static str) -> Action {
    Action::toggle(id, label)
        .icon(icon)
        .state("none", "None", icon)
        .state("some", "Some", icon)
        .live(SEARCH_POLL, move |_| Ok(count_visual(search_count(q)?, false, unit)))
        .on_press(move |_| open_page(&web(page)))
}

// ---- workflow runs ---------------------------------------------------------------------------

fn run_inputs(workflow_query: &str, workflow_label: &str) -> Vec<Value> {
    vec![
        input::query_combobox("repo", "Repository", "repos").placeholder("owner/name"),
        input::query_select("workflow", workflow_label, workflow_query, &["repo"]),
    ]
}

fn runs_path(inputs: &Value) -> Result<String> {
    let r = repo(&pick(inputs, "repo"))?;
    let wf = pick(inputs, "workflow");
    let branch = pick(inputs, "branch");
    let mut p = if wf.is_empty() { format!("/repos/{r}/actions/runs?per_page=1") } else { format!("/repos/{r}/actions/workflows/{}/runs?per_page=1", encode(&wf)) };
    if !branch.is_empty() {
        p.push_str(&format!("&branch={}", encode(&branch)));
    }
    Ok(p)
}

fn latest_run(inputs: &Value) -> Result<Option<Value>> {
    let r = Client::get()?.get_json(&runs_path(inputs)?)?;
    Ok(r["workflow_runs"].get(0).cloned())
}

/// The key state for a run: success, failure, running, cancelled (also skipped / neutral / stale).
fn run_state(run: &Value) -> &'static str {
    if run["status"].as_str() != Some("completed") {
        return "running";
    }
    match run["conclusion"].as_str().unwrap_or("") {
        "success" => "success",
        "failure" | "timed_out" | "startup_failure" | "action_required" => "failure",
        _ => "cancelled",
    }
}

fn run_label(run: &Value) -> String {
    let what = match run["status"].as_str().unwrap_or("") {
        "completed" => run["conclusion"].as_str().unwrap_or("done").replace('_', " "),
        s => s.replace('_', " "),
    };
    format!("{} · {what}", run["name"].as_str().unwrap_or("run"))
}

fn workflow_options(q: &knobdeck_sdk::QueryCtx, any: bool) -> Result<Value> {
    let r = repo(&pick(&q.inputs, "repo"))?;
    let list = Client::get()?.get_json(&format!("/repos/{r}/actions/workflows?per_page=100"))?;
    let mut opts = if any { vec![json!({"value": "", "label": "Any workflow"})] } else { vec![] };
    for w in list["workflows"].as_array().into_iter().flatten() {
        // The file name is a workflow id the API accepts, and survives re-creating the workflow.
        let file = w["path"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("");
        if !file.is_empty() && w["state"] != "disabled_manually" {
            opts.push(json!({"value": file, "label": w["name"].as_str().unwrap_or(file)}));
        }
    }
    Ok(Value::Array(opts))
}

/// `name=value` lines (placeholders already expanded) into dispatch inputs.
fn parse_inputs(text: &str) -> Result<Value> {
    let mut m = serde_json::Map::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((k, v)) = line.split_once('=') else {
            return Err(Error::msg(format!("\"{line}\" is not name=value")));
        };
        m.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
    }
    if m.len() > 25 {
        return Err(Error::msg("GitHub accepts at most 25 workflow inputs"));
    }
    Ok(Value::Object(m))
}

fn dispatch_request(repo: &str, workflow: &str, git_ref: &str, inputs: Value) -> (String, Value) {
    let mut body = json!({"ref": git_ref});
    if inputs.as_object().is_some_and(|o| !o.is_empty()) {
        body["inputs"] = inputs;
    }
    (format!("/repos/{repo}/actions/workflows/{}/dispatches", encode(workflow)), body)
}

fn rerun_path(repo: &str, run: &Value) -> Result<String> {
    if run_state(run) == "running" {
        return Err(Error::msg("The latest run is still going"));
    }
    if run_state(run) != "failure" {
        return Err(Error::msg(format!("The latest run didn't fail ({})", run["conclusion"].as_str().unwrap_or("?"))));
    }
    let id = run["id"].as_u64().ok_or_else(|| Error::msg("GitHub sent a run without an id"))?;
    Ok(format!("/repos/{repo}/actions/runs/{id}/rerun-failed-jobs"))
}

// ---- repository counters ----------------------------------------------------------------------

const REPO_QUERY: &str = "query($o:String!,$n:String!){repository(owner:$o,name:$n){stargazerCount issues(states:OPEN){totalCount} pullRequests(states:OPEN){totalCount}}}";

static REPO_CACHE: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

/// One GraphQL query per repository at most every 2 minutes, however many counters show it.
fn repo_counts(r: &str) -> Result<Value> {
    if let Some((at, v)) = REPO_CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(r) {
        if at.elapsed() < Duration::from_secs(120) {
            return Ok(v.clone());
        }
    }
    let (o, n) = r.split_once('/').unwrap_or((r, ""));
    let data = Client::get()?.graphql(REPO_QUERY, json!({"o": o, "n": n}))?;
    let repo = data["repository"].clone();
    if repo.is_null() {
        return Err(Error::msg(format!("No repository {r} (or the token can't see it)")));
    }
    REPO_CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(r.to_string(), (Instant::now(), repo.clone()));
    Ok(repo)
}

fn metric(repo: &Value, which: &str) -> (u64, &'static str) {
    match which {
        "issues" => (repo["issues"]["totalCount"].as_u64().unwrap_or(0), "issues"),
        "prs" => (repo["pullRequests"]["totalCount"].as_u64().unwrap_or(0), "PRs"),
        _ => (repo["stargazerCount"].as_u64().unwrap_or(0), "★"),
    }
}

fn compact(n: u64) -> String {
    match n {
        0..=9_999 => n.to_string(),
        10_000..=999_999 => format!("{:.1}k", n as f64 / 1000.0).replace(".0k", "k"),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0).replace(".0M", "M"),
    }
}

fn repo_label(inputs: &Value) -> Result<String> {
    let r = repo(&pick(inputs, "repo"))?;
    let (n, unit) = metric(&repo_counts(&r)?, &pick(inputs, "metric"));
    Ok(if unit == "★" { format!("★ {}", compact(n)) } else { format!("{} {unit}", compact(n)) })
}

fn main() {
    load_env();
    Plugin::new(&format!("github-{}", std::env::consts::OS), "GitHub")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Notifications, review requests, your pull requests, workflow runs and repository counters on GitHub.")
        .icon("app-icon.svg")
        .ack_timeout_ms(5000)
        .setup("github-setup", None)
        .setup_inputs(vec![input::text("token", "Personal access token (optional)")
            .with("type", "password")
            .placeholder("leave empty to sign in with GitHub")
            .with("help", "Instead of signing in: a fine-grained token (Actions: read and write, Pull requests, Issues and Metadata: read). Notifications need a classic token with the notifications and repo scopes. Kept in this computer's credential store.")])
        .query("repos", |_| {
            let c = Client::get()?;
            let mut opts = vec![];
            for page in 1..=3 {
                let r = c.get_reply(&format!("/user/repos?per_page=100&sort=pushed&page={page}"))?;
                opts.extend(r.body.as_array().into_iter().flatten().filter_map(|x| x["full_name"].as_str()).map(|n| json!({"value": n, "label": n})));
                if !r.next {
                    break;
                }
            }
            Ok(Value::Array(opts))
        })
        .query("workflows", |q| workflow_options(q, true))
        .query("dispatch-workflows", |q| workflow_options(q, false))
        .action(
            Action::toggle("notifications", "Notifications")
                .icon("bell")
                .state("none", "None", "bell")
                .state("some", "Unread", "bell-dot")
                .inputs(vec![input::checkbox("participating", "Only threads I take part in")])
                .live(NOTIFY_POLL, |ic| notifications(ic.input_bool("participating")))
                .on_press(|_| open_page(&web("/notifications"))),
        )
        .action(search_action("review-requests", "Review Requests", "git-pull-request-arrow", "is:open is:pr review-requested:@me archived:false", "to review", "/pulls/review-requested"))
        .action(search_action("my-prs", "My Open PRs", "git-pull-request", "is:open is:pr author:@me archived:false", "open", "/pulls"))
        .action(
            Action::toggle("workflow-status", "Workflow Status")
                .icon("workflow")
                .inputs([run_inputs("workflows", "Workflow"), vec![input::text("branch", "Branch (optional)").placeholder("main")]].concat())
                .state("none", "No runs", "circle-dashed")
                .state("running", "Running", "loader")
                .state("success", "Success", "circle-check")
                .state("failure", "Failed", "circle-x")
                .state("cancelled", "Cancelled", "circle-slash")
                .live(RUN_POLL, |ic| {
                    if pick(&ic.inputs, "repo").is_empty() {
                        return Ok(Visual::new());
                    }
                    Ok(match latest_run(&ic.inputs)? {
                        Some(run) => Visual::new().state(run_state(&run)).label(run_label(&run)),
                        None => Visual::new().state("none").label("No runs"),
                    })
                })
                // Press opens the latest run (or the Actions page when there is none).
                .on_press(|ctx| match latest_run(&ctx.inputs)? {
                    Some(run) => open_page(run["html_url"].as_str().unwrap_or("")),
                    None => open_page(&web(&format!("/{}/actions", repo(&pick(&ctx.inputs, "repo"))?))),
                }),
        )
        .action(
            Action::button("rerun-failed", "Re-run Failed Jobs")
                .icon("rotate-ccw")
                .inputs([run_inputs("workflows", "Workflow"), vec![input::text("branch", "Branch (optional)").placeholder("main")]].concat())
                .on_press(|ctx| {
                    let r = repo(&pick(&ctx.inputs, "repo"))?;
                    let Some(run) = latest_run(&ctx.inputs)? else {
                        return ctx.fail("No runs to re-run");
                    };
                    let path = match rerun_path(&r, &run) {
                        Ok(p) => p,
                        Err(e) => return ctx.fail(e.message()),
                    };
                    Client::get()?.post(&path, &json!({}))?;
                    ctx.set_label("Re-running");
                    Ok(())
                }),
        )
        .action(
            Action::button("run-workflow", "Run Workflow")
                .icon("play")
                .inputs(
                    [
                        run_inputs("dispatch-workflows", "Workflow (needs a workflow_dispatch trigger)"),
                        vec![
                            input::text("ref", "Branch or tag (optional)").placeholder("the default branch"),
                            input::textarea("inputs", "Inputs, one name=value per line (optional)").placeholder("environment=staging").accepts_placeholders(),
                        ],
                    ]
                    .concat(),
                )
                .on_press(|ctx| {
                    let r = repo(&pick(&ctx.inputs, "repo"))?;
                    let wf = pick(&ctx.inputs, "workflow");
                    if wf.is_empty() {
                        return ctx.fail("Choose the workflow to run");
                    }
                    let inputs = match parse_inputs(&ctx.expand(&ctx.input_str("inputs"))?) {
                        Ok(v) => v,
                        Err(e) => return ctx.fail(e.message()),
                    };
                    let c = Client::get()?;
                    let mut git_ref = ctx.input_str("ref").trim().to_string();
                    if git_ref.is_empty() {
                        git_ref = c.get_json(&format!("/repos/{r}"))?["default_branch"].as_str().unwrap_or("main").to_string();
                    }
                    let (path, body) = dispatch_request(&r, &wf, &git_ref, inputs);
                    c.post(&path, &body)?;
                    ctx.set_label("Started");
                    Ok(())
                }),
        )
        .action(
            Action::button("repo-counter", "Repository Counter")
                .icon("star")
                .inputs(vec![
                    input::query_combobox("repo", "Repository", "repos").placeholder("owner/name"),
                    input::select("metric", "Show", &[("stars", "Stars"), ("issues", "Open issues"), ("prs", "Open pull requests")]).with("default", "stars"),
                ])
                .live(REPO_POLL, |ic| {
                    if pick(&ic.inputs, "repo").is_empty() {
                        return Ok(Visual::new());
                    }
                    Ok(Visual::new().label(repo_label(&ic.inputs)?))
                })
                .on_press(|ctx| {
                    let r = repo(&pick(&ctx.inputs, "repo"))?;
                    let page = match pick(&ctx.inputs, "metric").as_str() {
                        "issues" => "/issues",
                        "prs" => "/pulls",
                        _ => "/stargazers",
                    };
                    open_page(&web(&format!("/{r}{page}")))
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combobox_values() {
        assert_eq!(pick(&json!({"repo": {"value": "a/b", "label": "a/b"}}), "repo"), "a/b");
        assert_eq!(pick(&json!({"repo": " a/b "}), "repo"), "a/b");
        assert_eq!(pick(&json!({}), "repo"), "");
    }

    #[test]
    fn run_states() {
        assert_eq!(run_state(&json!({"status": "in_progress"})), "running");
        assert_eq!(run_state(&json!({"status": "queued"})), "running");
        assert_eq!(run_state(&json!({"status": "completed", "conclusion": "success"})), "success");
        assert_eq!(run_state(&json!({"status": "completed", "conclusion": "timed_out"})), "failure");
        assert_eq!(run_state(&json!({"status": "completed", "conclusion": "skipped"})), "cancelled");
        assert_eq!(run_label(&json!({"name": "CI", "status": "completed", "conclusion": "startup_failure"})), "CI · startup failure");
    }

    #[test]
    fn run_paths() {
        assert_eq!(runs_path(&json!({"repo": "a/b"})).unwrap(), "/repos/a/b/actions/runs?per_page=1");
        assert_eq!(runs_path(&json!({"repo": "a/b", "workflow": "ci.yml", "branch": "feat/x"})).unwrap(), "/repos/a/b/actions/workflows/ci.yml/runs?per_page=1&branch=feat%2Fx");
        assert!(rerun_path("a/b", &json!({"status": "completed", "conclusion": "success"})).unwrap_err().message().contains("didn't fail"));
        assert!(rerun_path("a/b", &json!({"status": "in_progress"})).is_err());
        assert_eq!(rerun_path("a/b", &json!({"id": 42, "status": "completed", "conclusion": "failure"})).unwrap(), "/repos/a/b/actions/runs/42/rerun-failed-jobs");
    }

    #[test]
    fn dispatch_inputs() {
        assert_eq!(parse_inputs("env = staging\n\n# note\ndry=true").unwrap(), json!({"env": "staging", "dry": "true"}));
        assert!(parse_inputs("oops").is_err());
        let (p, b) = dispatch_request("a/b", "deploy.yml", "main", json!({}));
        assert_eq!(p, "/repos/a/b/actions/workflows/deploy.yml/dispatches");
        assert_eq!(b, json!({"ref": "main"}));
        let (_, b) = dispatch_request("a/b", "deploy.yml", "v1", json!({"env": "x"}));
        assert_eq!(b, json!({"ref": "v1", "inputs": {"env": "x"}}));
    }

    #[test]
    fn counts_read_well() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(12_345), "12.3k");
        assert_eq!(compact(20_000), "20k");
        assert_eq!(compact(1_500_000), "1.5M");
        let v = count_visual(50, true, "unread");
        assert_eq!((v.state.as_deref(), v.label.as_deref()), (Some("some"), Some("50+ unread")));
        assert_eq!(count_visual(0, false, "").state.as_deref(), Some("none"));
    }

    /// The re-run and dispatch requests as GitHub receives them (mock server; nothing real is run).
    #[test]
    fn rerun_and_dispatch_requests() {
        let m = api::mock::serve(vec![(201, "", "{}"), (204, "", "")]);
        let c = Client { api: m.base.clone(), token: "t".into() };
        let path = rerun_path("o/r", &json!({"id": 7, "status": "completed", "conclusion": "failure"})).unwrap();
        assert_eq!(c.post(&path, &json!({})).unwrap().0, 201);
        let req = m.requests.recv().unwrap();
        assert!(req.starts_with("POST /repos/o/r/actions/runs/7/rerun-failed-jobs "), "{req}");
        let (path, body) = dispatch_request("o/r", "deploy.yml", "main", parse_inputs("env=prod").unwrap());
        assert_eq!(c.post(&path, &body).unwrap().0, 204);
        let req = m.requests.recv().unwrap();
        assert!(req.starts_with("POST /repos/o/r/actions/workflows/deploy.yml/dispatches "), "{req}");
        assert!(req.ends_with(r#"{"inputs":{"env":"prod"},"ref":"main"}"#) || req.ends_with(r#"{"ref":"main","inputs":{"env":"prod"}}"#), "{req}");
    }
}
