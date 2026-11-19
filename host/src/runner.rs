//! The script runner: Spumcake-Skills' scripts, and any other command the
//! assistant asks for, run in the project after the user approves each
//! one, as the Studio-as-a-setup contract says.
//!
//! A command that spends, `request.py run` or `pipeline.py rerun` without
//! `--stand-in`, never runs as a plain command: it is asked for with its
//! plan, which `request.py plan` or `pipeline.py rerun --plan` works out
//! offline, and runs only with `--expect` and the hash of the plan the
//! person approved. Every other command is shown in full and runs on their
//! yes. A command runs off the window's thread; its output streams into its
//! tool row, and its answer reaches the assistant when it ends. Keys reach
//! the scripts as environment variables only, never written anywhere.
//!
//! Research runs here too: a page's text is read at once, since it costs
//! nothing; searches are asked for once per research request, with their
//! cost; and each download is asked for, naming its page and licence, and
//! staged in Studio's own folder until the assistant files it into
//! `sources/` with `request.py`'s research kind.

use crate::workspace::{EndpointKind, Settings};
use protocol::{Tool, ToolResult};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub(crate) const RUN_COMMAND: &str = "run_command";
pub(crate) const SEND_REQUEST: &str = "send_request";
pub(crate) const RERUN_STEP: &str = "rerun_step";
/// The assistant's fix of the dependency chain, as the data model guide
/// has it: every out-of-date step made again on one plan card.
pub(crate) const FIX_CHAIN: &str = "fix_dependency_chain";
pub(crate) const WEB_SEARCH: &str = "web_search";
pub(crate) const FETCH_PAGE: &str = "fetch_page";
pub(crate) const DOWNLOAD_FILE: &str = "download_file";

/// What one search costs on Brave Search: $5 for 1,000 requests.
const SEARCH_PRICE: f64 = 0.005;
/// The most searches one research request asks for at once.
const MOST_SEARCHES: usize = 20;
/// The most of a page's text an answer carries.
const MOST_PAGE: usize = 60_000;
/// The largest file a download keeps.
const MOST_DOWNLOAD: u64 = 500 * 1024 * 1024;
/// Where downloads wait, inside Studio's own folder, until they are filed.
const STAGED: &str = "cache/research";

/// The most of a command's output its answer carries, from its end.
const MOST_OUTPUT: usize = 20_000;
/// The lines of a running command's output its tool row shows.
const LIVE_LINES: usize = 8;
/// What `request.py` prints once a fal request is submitted and paid for.
const SUBMITTED: &str = "fal request ";

/// The runner's tools, in the host's Project group.
pub(crate) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: RUN_COMMAND.into(),
            description: "Run a command in the project, such as a Spumcake-Skills script (request.py and pipeline.py may be named by their file name alone), a builder, or a script of the agent's own, after the user approves it. Its output is answered when it ends. A command that spends is refused: send it with send_request or rerun_step.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "array", "items": { "type": "string" }, "description": "The program and its arguments, one per item, such as [\"request.py\", \"show\", \"<file>\"]." },
                    "why": { "type": "string", "description": "One line for the user: what it does and why." }
                },
                "required": ["command", "why"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: SEND_REQUEST.into(),
            description: "Send one request to a model with request.py, from its spec: its plan, worked out offline, is shown to the user with its cost, its prompt as sent, and every reference in order, and it runs only on their yes, exactly as approved. Answers with what request.py tell says.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "spec": { "type": "string", "description": "The spec's file, relative to the project, as the skill built it." },
                    "stand_in": { "type": "boolean", "description": "Run it on request.py's stand-in, which spends nothing, for a dry run." }
                },
                "required": ["spec"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: RERUN_STEP.into(),
            description: "Make a take of a pipeline's step with pipeline.py rerun: the first that is out of date, the first a change reaches, or the one named. Its plan is shown to the user and runs only on their yes. It stops after that one step.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "pipeline": { "type": "string", "description": "The pipeline's number, such as 002." },
                    "chain": { "type": "string", "description": "With step, the chain of the one step to make a take of." },
                    "step": { "type": "string", "description": "With chain, the one step to make a take of, out of date or not." },
                    "fact": { "type": "string", "description": "A changed fact, as \"<element>: <fact>\", to rerun what it reaches." },
                    "artifact": { "type": "string", "description": "A changed artifact, as \"<element>: <role>\", to rerun what it reaches." },
                    "stand_in": { "type": "boolean", "description": "Run it on the stand-in, which spends nothing." }
                },
                "required": ["pipeline"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: FIX_CHAIN.into(),
            description: "Fix the dependency chain: make again every step whose artifact is out of date and not ignored, in chain order, all on one plan card the user approves before anything is spent. A reference an earlier fix will replace is marked on the card. Each result is published at once as its artifact's next version, a fix, so it is a new iteration of the same asset. A later step runs only when its plan, made again after the fixes before it, differs from the approved one in nothing but those files; anything else stops the fix there and says why, and a new fix plans the rest.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "pipeline": { "type": "string", "description": "The pipeline's number, such as 002." },
                    "stand_in": { "type": "boolean", "description": "Run it on the stand-in, which spends nothing." }
                },
                "required": ["pipeline"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: WEB_SEARCH.into(),
            description: "Search the web, or its images, with Brave Search, for research such as a skill's references. All the searches one research request needs are asked for together: the user approves them with their cost first. Answers each search's results, with each one's title, address, and the page it is on.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "searches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "query": { "type": "string" },
                                "images": { "type": "boolean", "description": "Search images, not pages." }
                            },
                            "required": ["query"],
                            "additionalProperties": false
                        },
                        "description": "Every search this research needs, at most 20."
                    },
                    "why": { "type": "string", "description": "One line for the user: what is being researched." }
                },
                "required": ["searches", "why"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: FETCH_PAGE.into(),
            description: "Read a web page's text, to learn what an image found is, who made it, and the use its licence allows. Costs nothing and needs no approval.".into(),
            input: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "The page's address." } },
                "required": ["url"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: DOWNLOAD_FILE.into(),
            description: "Download one file found in research, after the user approves it: the file, the page it is on, and its licence are shown. It waits in Studio's own folder until you file it into sources/ with send_request and a spec of request.py's research kind, whose outputs are the files and whose sources give each file's address, page, and the use allowed.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The file's own address." },
                    "page": { "type": "string", "description": "The page the file is on." },
                    "licence": { "type": "string", "description": "Its licence and the use it allows, as the page says, or \"unknown\"." }
                },
                "required": ["url", "page", "licence"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
    ]
}

/// Whether `name` is one of the runner's tools.
pub(crate) fn is_runner_tool(name: &str) -> bool {
    matches!(name, RUN_COMMAND | SEND_REQUEST | RERUN_STEP | WEB_SEARCH | FETCH_PAGE | DOWNLOAD_FILE)
}

/// What a script is run with: its folder, and its environment.
#[derive(Clone, Default)]
pub(crate) struct ScriptEnv {
    /// The folder holding Spumcake-Skills' `request.py` and `pipeline.py`.
    pub scripts: Option<PathBuf>,
    /// The keys and `SKILLS_SETUP`, as NAME and value.
    pub vars: Vec<(String, String)>,
    /// Brave Search's key, for the assistant's own searches. It is never
    /// given to a script.
    pub search_key: Option<String>,
}

/// Names only: a key's value is never printed.
impl std::fmt::Debug for ScriptEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEnv")
            .field("scripts", &self.scripts)
            .field("vars", &self.vars.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>())
            .field("search_key", &self.search_key.as_ref().map(|_| "set"))
            .finish()
    }
}

/// Where Skills' scripts are: the `scripts/` of the first listed skill's
/// repository that holds `request.py`.
pub(crate) fn scripts_folder(skills: &[crate::skills_setup::Skill]) -> Option<PathBuf> {
    skills.iter().map(|skill| skill.root.join("scripts")).find(|folder| folder.join("request.py").is_file())
}

/// The environment Skills' scripts run with: each endpoint's key, by its
/// kind, and the setup file, written into the data folder with no key in
/// it.
pub(crate) fn script_env(settings: &Settings, key: &mut dyn FnMut(&str) -> Option<String>, data: Option<&Path>, scripts: Option<PathBuf>) -> ScriptEnv {
    let mut vars = Vec::new();
    let mut give = |name: &str, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.trim().is_empty())
            && !vars.iter().any(|(known, _): &(String, String)| known == name)
        {
            vars.push((name.to_string(), value));
        }
    };
    let mut search_key = None;
    for endpoint in &settings.endpoints {
        if matches!(endpoint.kind, EndpointKind::Brave) {
            search_key = search_key.or_else(|| key(&endpoint.id).filter(|value| !value.trim().is_empty()));
            continue;
        }
        let name = match endpoint.kind {
            EndpointKind::Openrouter => "OPENROUTER_API_KEY",
            EndpointKind::Fal => "FAL_KEY",
            EndpointKind::Openai if endpoint.address.contains("api.openai.com") => "OPENAI_API_KEY",
            _ => continue,
        };
        give(name, key(&endpoint.id));
    }
    if let Some(data) = data
        && let Some(setup) = write_setup(data)
    {
        vars.push((String::from("SKILLS_SETUP"), setup.display().to_string()));
    }
    ScriptEnv { scripts, vars, search_key }
}

/// The setup file, `setup-v1`, with the keys `request.py` reads: its
/// prices, each saying where it came from, and Studio's `.env` when one is
/// kept. It never holds a key.
pub(crate) fn write_setup(data: &Path) -> Option<PathBuf> {
    let path = data.join("skills-setup.json");
    let mut setup = json!({ "format": "setup-v1", "prices": crate::prices::known(data) });
    let env = data.join(".env");
    if env.is_file() {
        setup["env_file"] = json!(env.display().to_string());
    }
    let text = serde_json::to_string_pretty(&setup).ok()?;
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&path, &text).ok()?;
    }
    Some(path)
}

/// The command as it runs: `request.py` and `pipeline.py` named alone are
/// Skills' own, run with Python 3.
fn resolve(command: &[String], env: &ScriptEnv) -> Result<Vec<String>, String> {
    let first = command.first().ok_or("No command was given.")?;
    let script = |name: &str| -> Result<String, String> {
        let folder = env.scripts.as_ref().ok_or(crate::skills_checkout::MISSING)?;
        Ok(folder.join(name).display().to_string())
    };
    Ok(match first.as_str() {
        "request.py" | "pipeline.py" => [String::from("python3"), script(first)?].into_iter().chain(command[1..].iter().cloned()).collect(),
        _ => command.to_vec(),
    })
}

/// Which of Skills' scripts a command runs, and its subcommand. Only the
/// configured checkout's own script, found by its real path and run
/// directly by Python, as `resolve` makes it, counts: a shell string that
/// ends in a script's name, or another file named like one, such as one
/// the agent wrote, never does.
fn script_of(argv: &[String], scripts: Option<&Path>, cwd: &Path) -> Option<(&'static str, String)> {
    // Python itself, by its own bare names, such as python3 or python3.12,
    // found on the PATH: never a file in the project named like it.
    let program = argv.first()?;
    if program.contains(['/', '\\']) {
        return None;
    }
    let version = program.strip_prefix("python").unwrap_or("x");
    if !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let script = Path::new(argv.get(1)?);
    let real = if script.is_absolute() { script.to_path_buf() } else { cwd.join(script) }.canonicalize().ok()?;
    let scripts = scripts?;
    let name = ["request.py", "pipeline.py"].into_iter().find(|name| scripts.join(name).canonicalize().is_ok_and(|own| own == real))?;
    Some((name, argv.get(2).cloned().unwrap_or_default()))
}

/// Whether a command spends money: the contract's list.
/// A command given to a shell, such as `sh -c "…request.py run …"`, is
/// read as the words it runs, so it cannot pass as a plain command.
pub(crate) fn spends(argv: &[String]) -> bool {
    let words: Vec<String> = argv.iter().flat_map(|arg| arg.split_whitespace().map(|word| word.trim_matches(['"', '\'', ';']).to_string())).collect();
    let stand_in = words.iter().any(|word| word == "--stand-in");
    let plan_only = words.iter().any(|word| word == "--plan");
    words.windows(2).any(|pair| {
        (pair[0].ends_with("request.py") && pair[1] == "run" && !stand_in)
            || (pair[0].ends_with("pipeline.py") && pair[1] == "rerun" && !stand_in && !plan_only)
    })
}

/// Whether a command publishes into canon, which only the cells do. A
/// command given to a shell is read as the words it runs.
pub(crate) fn publishes(argv: &[String]) -> bool {
    let words: Vec<String> = argv.iter().flat_map(|arg| arg.split_whitespace().map(|word| word.trim_matches(['"', '\'', ';']).to_string())).collect();
    words.windows(2).any(|pair| pair[0].ends_with("pipeline.py") && pair[1] == "publish")
}

/// Whether a command only reads, and so runs without asking, as the owner
/// approved on 2026-11-16: the contract's exact list of Skills' commands
/// that render, download, spend, and change nothing. `pipeline.py sql`
/// counts only with `--read-only`. Commands run with no shell, so a command
/// given to one names no script of its own and never counts.
pub(crate) fn only_reads(argv: &[String], scripts: Option<&Path>, cwd: &Path) -> bool {
    match script_of(argv, scripts, cwd) {
        Some(("request.py", sub)) => matches!(sub.as_str(), "show" | "tell" | "list" | "check" | "plan" | "version" | "record"),
        Some(("pipeline.py", sub)) => match sub.as_str() {
            "touches" | "history" => true,
            "sql" => argv.iter().any(|arg| arg == "--read-only"),
            _ => false,
        },
        _ => false,
    }
}

/// Whether a command runs Skills' own scripts, the only commands the keys
/// are given to.
fn runs_skills(argv: &[String], scripts: Option<&Path>, cwd: &Path) -> bool {
    script_of(argv, scripts, cwd).is_some()
}

/// The plan a script printed, with the hash its approval binds to. A plan
/// with no hash is refused, so an approval never runs unbound. A list of
/// plans, one per step, is read as its first, the step that runs.
fn bound(plan: Value) -> Result<(Value, String), String> {
    let plan = match plan {
        Value::Array(mut plans) if !plans.is_empty() => plans.remove(0),
        plan => plan,
    };
    match plan.get("hash").and_then(Value::as_str).filter(|hash| !hash.trim().is_empty()) {
        Some(hash) => {
            let hash = hash.to_string();
            Ok((plan, hash))
        }
        None => Err(String::from("The plan has no hash, so an approval could not be bound to what runs. Nothing ran.")),
    }
}

/// Whether a Skills script can plan offline and run only what was
/// approved: an older checkout's would spend while being asked for a plan.
fn plans_offline(script: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script).map_err(|error| format!("Could not read {}: {error}", script.display()))?;
    if text.contains("--expect") && text.contains("--plan") || text.contains("--expect") && text.contains("def plan") {
        Ok(())
    } else {
        Err(format!("{} cannot plan offline and run only an approved plan, so nothing was asked of it. Update the Spumcake-Skills checkout.", script.display()))
    }
}

/// What the user is asked to approve.
#[derive(Debug, Clone)]
pub(crate) enum Asking {
    /// A command shown in full.
    Command { call: String, tool: String, argv: Vec<String>, why: String },
    /// One or more requests, each with its plan, each run with its own
    /// `--expect`.
    Plans { call: String, tool: String, plans: Vec<Value>, runs: Vec<Vec<String>>, stand_in: bool, fix: Option<Fix> },
    /// A research request's searches, each a query and whether it searches
    /// images, asked for together with their cost.
    Searches { call: String, tool: String, searches: Vec<(String, bool)>, why: String },
    /// One file to download, with its page and licence, and where it waits.
    Download { call: String, tool: String, url: String, page: String, licence: String, to: String },
}

impl Asking {
    fn call(&self) -> (&str, &str) {
        match self {
            Asking::Command { call, tool, .. } | Asking::Plans { call, tool, .. } | Asking::Searches { call, tool, .. } | Asking::Download { call, tool, .. } => (call, tool),
        }
    }
}

/// A fix of the dependency chain the user is asked to approve: each step
/// to make again, in chain order, with the hash of its plan as shown.
#[derive(Clone, Debug)]
pub(crate) struct Fix {
    scripts: PathBuf,
    pipeline: String,
    steps: Vec<FixStep>,
}

#[derive(Clone, Debug)]
struct FixStep {
    chain: String,
    step: String,
    artifact: String,
    hash: String,
}

/// The mark on a reference an earlier fix in the same plan replaces.
pub(crate) const FIXED_BY: &str = "fixed_by";

/// A Skills script's command, as `resolve` makes it.
fn skills_argv(scripts: &Path, script: &str, args: &[&str]) -> Vec<String> {
    [String::from("python3"), scripts.join(script).display().to_string()].into_iter().chain(args.iter().map(|arg| arg.to_string())).collect()
}

/// What a Skills script printed, run in the project with its setup; an
/// error with what it said when it failed.
fn skills_run(argv: &[String], cwd: &Path, vars: &[(String, String)]) -> Result<String, String> {
    let out = Command::new(&argv[0]).args(&argv[1..]).current_dir(cwd).envs(vars.iter().map(|(name, value)| (name.as_str(), value.as_str()))).output().map_err(|error| format!("Could not run {}: {error}", argv[1]))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The steps to fix, in chain order: each step whose artifact Skills finds
/// out of date and was not ignored, as `touches --json` lists them.
fn steps_to_fix(scripts: &Path, cwd: &Path, vars: &[(String, String)], pipeline: &str) -> Result<Vec<(String, String, String)>, String> {
    let touches: Value = serde_json::from_str(&skills_run(&skills_argv(scripts, "pipeline.py", &["touches", pipeline, "--json"]), cwd, vars)?).map_err(|error| format!("What is out of date could not be read: {error}"))?;
    let stale: Vec<&str> = touches.get("out_of_date").and_then(Value::as_array).into_iter().flatten().filter_map(|entry| entry.get("artifact").and_then(Value::as_str)).collect();
    let query = "SELECT json_array(c.ord, s.ord, s.chain, s.name, s.artifact) FROM step s JOIN chain c ON c.name = s.chain WHERE s.artifact IS NOT NULL AND s.artifact != ''";
    let mut steps: Vec<(i64, i64, String, String, String)> = skills_run(&skills_argv(scripts, "pipeline.py", &["sql", pipeline, query, "--read-only"]), cwd, vars)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|row| {
            let text = |index: usize| row.get(index).and_then(Value::as_str).map(String::from);
            Some((row.get(0)?.as_i64().unwrap_or(99), row.get(1)?.as_i64().unwrap_or(99), text(2)?, text(3)?, text(4)?))
        })
        .filter(|(_, _, _, _, artifact)| stale.contains(&artifact.as_str()))
        .collect();
    steps.sort();
    Ok(steps.into_iter().map(|(_, _, chain, step, artifact)| (chain, step, artifact)).collect())
}

/// Marks each reference of a plan that an earlier fix in the same plan
/// replaces, by the artifact it names, with `FIXED_BY`.
fn mark_fixed(plan: &mut Value, fixed: &[String]) {
    for list in ["references", "inputs"] {
        for reference in plan.get_mut(list).and_then(Value::as_array_mut).into_iter().flatten() {
            if let Some(artifact) = reference.get("canon").and_then(Value::as_str).filter(|artifact| fixed.iter().any(|known| known == artifact)).map(String::from) {
                reference[FIXED_BY] = json!(artifact);
            }
        }
    }
}

/// Whether a later step's plan, made again after the fixes before it, is
/// the approved one in all but the references marked `FIXED_BY`, each of
/// which now names the file its fix published. Otherwise, what differs.
fn same_but_fixed(approved: &Value, now: &Value, published: &HashMap<String, String>) -> Result<(), String> {
    let left_out = ["hash", "estimate", "price_note", "price_hint", "references", "inputs"];
    let rest = |plan: &Value| -> Value { plan.as_object().map(|fields| Value::Object(fields.iter().filter(|(key, _)| !left_out.contains(&key.as_str())).map(|(key, value)| (key.clone(), value.clone())).collect())).unwrap_or(Value::Null) };
    if rest(approved) != rest(now) {
        let differs = approved.as_object().into_iter().flatten().filter(|(key, value)| !left_out.contains(&key.as_str()) && now.get(key.as_str()) != Some(*value)).map(|(key, _)| key.clone()).collect::<Vec<_>>();
        return Err(format!("its {} changed", if differs.is_empty() { String::from("plan") } else { differs.join(", ") }));
    }
    for list in ["references", "inputs"] {
        let (was, is) = (approved.get(list).and_then(Value::as_array).cloned().unwrap_or_default(), now.get(list).and_then(Value::as_array).cloned().unwrap_or_default());
        if was.len() != is.len() {
            return Err(format!("its {list} changed"));
        }
        for (was, is) in was.iter().zip(&is) {
            let mut kept = was.clone();
            if let Some(object) = kept.as_object_mut() {
                object.remove(FIXED_BY);
            }
            match was.get(FIXED_BY).and_then(Value::as_str) {
                Some(artifact) => {
                    let wanted = published.get(artifact).ok_or_else(|| format!("{artifact} was not fixed first"))?;
                    if is.get("file").and_then(Value::as_str) != Some(wanted.as_str()) {
                        return Err(format!("its reference to {artifact} is not the fix just published"));
                    }
                    let strip = |reference: &Value| -> Value { reference.as_object().map(|fields| Value::Object(fields.iter().filter(|(key, _)| !["file", "path", "sha256", FIXED_BY].contains(&key.as_str())).map(|(key, value)| (key.clone(), value.clone())).collect())).unwrap_or(Value::Null) };
                    if strip(was) != strip(is) {
                        return Err(format!("its reference to {artifact} changed"));
                    }
                }
                None if kept != *is => return Err(format!("its {list} changed")),
                None => {}
            }
        }
    }
    Ok(())
}

/// Where a download's own record of its address, page, and licence is kept.
fn provenance_of(staged: &Path) -> PathBuf {
    let mut name = staged.as_os_str().to_os_string();
    name.push(".source.json");
    PathBuf::from(name)
}

/// Whether an address is this computer's or its local network's.
fn is_local(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, host)| host).unwrap_or(authority);
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("").to_lowercase()
    } else {
        authority.split(':').next().unwrap_or("").to_lowercase()
    };
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") || !host.contains('.') && host.parse::<std::net::Ipv6Addr>().is_err() {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

/// Every file a research spec files that was downloaded here is named in
/// its sources, with the address it came from, its page, and the use
/// allowed; and every file it files has a source with an address and a use.
fn sources_match(folder: &Path, spec: &Path) -> Result<(), String> {
    let spec: Value = std::fs::read_to_string(spec).ok().and_then(|text| serde_json::from_str(&text).ok()).ok_or("The research spec could not be read.")?;
    let sources = spec.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
    for output in spec.get("outputs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let name = Path::new(output).file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        let source = sources
            .iter()
            .find(|source| source.get("file").and_then(Value::as_str).and_then(|file| Path::new(file).file_name()).is_some_and(|file| file.to_string_lossy() == name))
            .ok_or(format!("{output} has no entry in the spec's sources, so where it came from would be lost. Add its address, page, and the use allowed."))?;
        let text = |key: &str| source.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text("address").is_empty() || text("use").is_empty() {
            return Err(format!("{output}'s source needs its address and the use allowed."));
        }
        let path = if Path::new(output).is_absolute() { PathBuf::from(output) } else { folder.join(output) };
        let staged = folder.join(protocol::PROJECT_DIR).join(STAGED);
        let found = std::fs::read_to_string(provenance_of(&path)).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        match found {
            Some(found) => {
                let downloaded = found.get("address").and_then(Value::as_str).unwrap_or("");
                if text("address") != downloaded {
                    return Err(format!("{output} was downloaded from {downloaded}, not from {}, as its source says.", text("address")));
                }
                // The use filed is the licence the user approved, or says more
                // of it; never another.
                let licence = found.get("licence").and_then(Value::as_str).unwrap_or("unknown").trim().to_string();
                if !licence.eq_ignore_ascii_case("unknown") && !text("use").to_lowercase().contains(&licence.to_lowercase()) {
                    return Err(format!("{output}'s use, \"{}\", is not the licence approved when it was downloaded, \"{licence}\".", text("use")));
                }
            }
            // A file waiting among downloads came through download_file, so
            // it has its record; one without was not downloaded here.
            None if path.starts_with(&staged) => return Err(format!("{output} has no record of where it was downloaded from, so it cannot be filed.")),
            None => {}
        }
    }
    Ok(())
}

/// The files a research spec files into `sources/`, each with where it
/// was found and the use allowed, for the card.
fn research_files(spec: &Path) -> Value {
    let spec: Value = std::fs::read_to_string(spec).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or(Value::Null);
    let sources = spec.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
    let outputs = spec.get("outputs").and_then(Value::as_array).cloned().unwrap_or_default();
    Value::Array(
        outputs
            .iter()
            .filter_map(Value::as_str)
            .map(|file| {
                let name = Path::new(file).file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
                let source = sources.iter().find(|source| source.get("file").and_then(Value::as_str).is_some_and(|named| Path::new(named).file_name().is_some_and(|named| named.to_string_lossy() == name)));
                let text = |key: &str| source.and_then(|source| source.get(key)).and_then(Value::as_str).unwrap_or("").to_string();
                json!({ "file": file, "address": text("address"), "page": text("page"), "use": text("use") })
            })
            .collect(),
    )
}

/// A plan, worked out offline by Skills' own script.
fn plan_with(argv: &[String], cwd: &Path, vars: &[(String, String)]) -> Result<Value, String> {
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .envs(vars.iter().map(|(name, value)| (name.as_str(), value.as_str())))
        .output()
        .map_err(|error| format!("Could not run {}: {error}", argv[0]))?;
    if !out.status.success() {
        return Err(format!("The plan could not be made: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|error| format!("The plan was not JSON: {error}"))
}

/// A plan and its hash. When the price of the model it names was not read
/// today, it is read, the setup file written again with it, and the plan
/// made again, so its estimate uses it. A stand-in's plan reads no price.
fn planned(argv: &[String], cwd: &Path, vars: &[(String, String)], stand_in: bool) -> Result<(Value, String), String> {
    let mut plan = plan_with(argv, cwd, vars)?;
    if !stand_in
        && let Some(data) = crate::prices::data_of(vars)
        && crate::prices::ensure(&data, &plan, vars)
    {
        write_setup(&data);
        plan = plan_with(argv, cwd, vars)?;
    }
    let (mut plan, hash) = bound(plan)?;
    // A price still unknown is estimated from what the project's earlier
    // requests were billed; the hash binds what is sent, not the estimate.
    // A listed price's note, what it leaves out, is shown beside it.
    if !stand_in {
        if let Some(data) = crate::prices::data_of(vars) {
            crate::prices::fill_from_token_prices(&data, cwd, &mut plan);
        }
        crate::prices::fill_from_history(cwd, &mut plan);
        // A model priced by its tokens with no prices and no estimate yet:
        // the card says where to set them.
        if let Some(data) = crate::prices::data_of(vars)
            && crate::prices::needs_prices(&data, &plan)
        {
            plan["price_hint"] = json!(crate::prices::SET_PRICES);
        }
        if let Some(note) = crate::prices::data_of(vars).and_then(|data| crate::prices::note_for(&data, &plan)) {
            plan["price_note"] = json!(note);
        }
    }
    Ok((plan, hash))
}

/// Whether an environment variable holds a secret, by its name, as
/// Spumcake-Skills' own `keyless_env` judges it.
fn secret(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    ["KEY", "KEY_ID", "SECRET", "TOKEN", "PASSWORD", "CREDENTIALS"].iter().any(|end| name.ends_with(end))
}

/// Takes every secret Studio's own process holds, such as a key loaded
/// from its `.env` at start, out of what a command inherits.
pub(crate) fn strip_keys(command: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        if secret(&name.to_string_lossy()) {
            command.env_remove(&name);
        }
    }
    command
}

/// What searches cost, for the card's title.
pub(crate) fn searches_cost(count: usize) -> String {
    format!("about ${:.3}", count as f64 * SEARCH_PRICE)
}

/// Where a download waits: a name from its address, kept apart from every
/// file already waiting, relative to the project.
fn staged_name(folder: &Path, url: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or("").rsplit('/').find(|part| !part.is_empty()).unwrap_or("download");
    let safe: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect();
    let safe = safe.trim_matches(['.', '-']).chars().take(80).collect::<String>();
    let safe = if safe.is_empty() { String::from("download") } else { safe };
    let (stem, extension) = match safe.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (safe.clone(), String::new()),
    };
    let dir = format!("{}/{STAGED}", protocol::PROJECT_DIR);
    let mut name = format!("{dir}/{stem}{extension}");
    let mut n = 2;
    while folder.join(&name).exists() {
        name = format!("{dir}/{stem}-{n}{extension}");
        n += 1;
    }
    name
}

/// One command running, or ended and not yet answered.
struct Job {
    call: String,
    tool: String,
    argv: Vec<String>,
    child: Child,
    output: Arc<Mutex<String>>,
    readers: Vec<std::thread::JoinHandle<()>>,
    shown: usize,
    /// The commands still to run after this one, as one approval of
    /// several requests runs them in turn.
    then: Vec<Vec<String>>,
    /// The output of those already run, for the answer.
    before: String,
    /// It was approved to spend: Stop never kills it, since what it sent
    /// may already be paid for.
    spends: bool,
}

/// Research running on a thread of its own: a page read, the approved
/// searches, or a download.
struct Task {
    call: String,
    done: std::thread::JoinHandle<ToolResult>,
    /// Set by Stop: searches not yet sent are not sent.
    stop: Arc<std::sync::atomic::AtomicBool>,
}

/// The runner: what waits for the user's yes, and what runs.
#[derive(Default)]
pub(crate) struct Runner {
    asking: Option<Asking>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    jobs: Vec<Job>,
    tasks: Vec<Task>,
    /// The configured checkout's scripts folder, whose scripts alone are
    /// given the keys.
    scripts: Option<PathBuf>,
    /// A plan being worked out, for its call.
    planning: Option<(String, std::thread::JoinHandle<Result<Asking, String>>)>,
    /// Brave Search's key while its searches wait for the user's yes, kept
    /// out of what is asked so it is never shown or printed.
    search_key: Option<String>,
    /// Commands left running at an endpoint after a stop, already paid
    /// for; each files its own results when it ends.
    detached: Vec<Child>,
    /// The project whose prerequisites were found, and what is missing.
    checked: Option<(PathBuf, Vec<String>)>,
    ctx: Option<eframe::egui::Context>,
}

/// What a call to a runner tool comes to at once.
pub(crate) enum Begun {
    /// It is answered now, as when it is refused.
    Answered(ToolResult),
    /// It waits for the user, then for the command.
    Waiting,
}

fn failed(call: &str, reason: impl Into<String>) -> ToolResult {
    ToolResult { call: call.to_string(), ok: false, output: json!({ "error": reason.into() }), images: Vec::new() }
}

impl Runner {
    pub(crate) fn asking(&self) -> Option<&Asking> {
        self.asking.as_ref()
    }

    /// Takes the programs a project's scripts need as found, for checks on a
    /// computer that may lack one the check does not run.
    #[cfg(test)]
    pub(crate) fn assume_prerequisites(&mut self, folder: &Path) {
        self.checked = Some((folder.to_path_buf(), Vec::new()));
    }

    /// The programs a project's scripts need, with any missing.
    fn missing(&mut self, folder: &Path) -> Vec<String> {
        if let Some((known, missing)) = &self.checked
            && known == folder
        {
            return missing.clone();
        }
        let works = |program: &str, args: &[&str]| {
            Command::new(program).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
        };
        let mut missing = Vec::new();
        if !works("python3", &["--version"]) {
            missing.push(String::from("Python 3"));
        } else {
            for (module, name) in [("PIL", "Pillow"), ("numpy", "numpy")] {
                if !works("python3", &["-c", &format!("import {module}")]) {
                    missing.push(name.to_string());
                }
            }
        }
        for (program, flag) in [("curl", "--version"), ("ffmpeg", "-version")] {
            if !works(program, &[flag]) {
                missing.push(program.to_string());
            }
        }
        self.checked = Some((folder.to_path_buf(), missing.clone()));
        missing
    }

    /// A call to one of the runner's tools. `tool` is its full key, and
    /// `name` its name within the group.
    pub(crate) fn begin(&mut self, folder: &Path, env: ScriptEnv, call: &str, tool: &str, name: &str, input: &Value) -> Begun {
        if self.asking.is_some() || self.planning.is_some() {
            return Begun::Answered(failed(call, "Another command is waiting for the user's yes. Ask for one at a time."));
        }
        if matches!(name, WEB_SEARCH | FETCH_PAGE | DOWNLOAD_FILE) {
            return self.research(folder, env, call, tool, name, input);
        }
        let missing = self.missing(folder);
        if !missing.is_empty() {
            return Begun::Answered(failed(
                call,
                format!("Running Spumcake-Skills' scripts needs Python 3 with Pillow and numpy, curl, and ffmpeg. Missing on this computer: {}.", missing.join(", ")),
            ));
        }
        self.cwd = folder.to_path_buf();
        self.env = env.vars.clone();
        self.scripts = env.scripts.clone();
        let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let stand_in = input.get("stand_in").and_then(Value::as_bool).unwrap_or(false);
        let asking = match name {
            RUN_COMMAND => {
                let command: Vec<String> = input.get("command").and_then(Value::as_array).map(|items| items.iter().filter_map(|item| item.as_str().map(String::from)).collect()).unwrap_or_default();
                let argv = match resolve(&command, &env) {
                    Ok(argv) => argv,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                if spends(&argv) {
                    return Begun::Answered(failed(call, "This command spends money, so it is never run as a plain command. Send a request with send_request and its spec, or a step with rerun_step, so the user approves its plan."));
                }
                if publishes(&argv) {
                    return Begun::Answered(failed(call, "Canon follows from the cells; nothing is published by command."));
                }
                // A command that only reads runs at once, with no question.
                if only_reads(&argv, env.scripts.as_deref(), folder) {
                    return match self.start(call, tool, argv, Vec::new(), String::new(), false) {
                        Ok(()) => Begun::Waiting,
                        Err(reason) => Begun::Answered(failed(call, reason)),
                    };
                }
                Asking::Command { call: call.to_string(), tool: tool.to_string(), argv, why: text("why") }
            }
            SEND_REQUEST => {
                let spec = text("spec");
                let argvs = resolve(&[String::from("request.py"), String::from("plan"), spec.clone(), String::from("--json")], &env)
                    .and_then(|argv| plans_offline(Path::new(&argv[1])).map(|()| argv))
                    .and_then(|planning| resolve(&[String::from("request.py"), String::from("run"), spec.clone()], &env).map(|run| (planning, run)));
                let (planning, mut run) = match argvs {
                    Ok(argvs) => argvs,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                let (cwd, vars, spec_path, call_name, tool_name) = (folder.to_path_buf(), env.vars.clone(), folder.join(&spec), call.to_string(), tool.to_string());
                return self.plan_later(call, move || {
                    let (mut plan, hash) = planned(&planning, &cwd, &vars, stand_in)?;
                    // A research request files what was found: the card shows
                    // each file with its address, page, and use, from the spec.
                    if plan.pointer("/endpoint/kind").and_then(Value::as_str) == Some("research") {
                        sources_match(&cwd, &spec_path)?;
                        plan["filed"] = research_files(&spec_path);
                    }
                    run.extend([String::from("--expect"), hash]);
                    if stand_in {
                        run.push(String::from("--stand-in"));
                    }
                    Ok(Asking::Plans { call: call_name, tool: tool_name, plans: vec![plan], runs: vec![run], stand_in, fix: None })
                });
            }
            RERUN_STEP => {
                let mut asked = vec![String::from("pipeline.py"), String::from("rerun"), text("pipeline")];
                for key in ["fact", "artifact"] {
                    if !text(key).is_empty() {
                        asked.push(format!("--{key}"));
                        asked.push(text(key));
                    }
                }
                // One named step, whose result ends its output as one line
                // of JSON naming the take.
                if !text("chain").is_empty() && !text("step").is_empty() {
                    asked.extend([String::from("--step"), text("chain"), text("step"), String::from("--json")]);
                }
                if stand_in {
                    asked.push(String::from("--stand-in"));
                }
                let mut planning = asked.clone();
                planning.extend([String::from("--plan"), String::from("--json")]);
                let argvs = resolve(&planning, &env)
                    .and_then(|argv| plans_offline(Path::new(&argv[1])).map(|()| argv))
                    .and_then(|planning| resolve(&asked, &env).map(|run| (planning, run)));
                let (planning, mut run) = match argvs {
                    Ok(argvs) => argvs,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                let (cwd, vars, call_name, tool_name) = (folder.to_path_buf(), env.vars.clone(), call.to_string(), tool.to_string());
                return self.plan_later(call, move || {
                    let (plan, hash) = planned(&planning, &cwd, &vars, stand_in)?;
                    run.extend([String::from("--expect"), hash]);
                    Ok(Asking::Plans { call: call_name, tool: tool_name, plans: vec![plan], runs: vec![run], stand_in, fix: None })
                });
            }
            FIX_CHAIN => {
                let pipeline = text("pipeline");
                let Some(scripts) = env.scripts.clone() else { return Begun::Answered(failed(call, crate::skills_checkout::MISSING)) };
                if let Err(reason) = plans_offline(&scripts.join("pipeline.py")) {
                    return Begun::Answered(failed(call, reason));
                }
                let (cwd, vars, call_name, tool_name) = (folder.to_path_buf(), env.vars.clone(), call.to_string(), tool.to_string());
                return self.plan_later(call, move || {
                    let found = steps_to_fix(&scripts, &cwd, &vars, &pipeline)?;
                    if found.is_empty() {
                        return Err(String::from("Nothing is out of date, so there is nothing to fix."));
                    }
                    let (mut plans, mut steps, mut fixed) = (Vec::new(), Vec::new(), Vec::new());
                    for (chain, step, artifact) in found {
                        let mut argv = skills_argv(&scripts, "pipeline.py", &["rerun", &pipeline, "--step", &chain, &step, "--plan", "--json"]);
                        if stand_in {
                            argv.push(String::from("--stand-in"));
                        }
                        let (mut plan, hash) = planned(&argv, &cwd, &vars, stand_in)?;
                        mark_fixed(&mut plan, &fixed);
                        plans.push(plan);
                        steps.push(FixStep { chain, step, artifact: artifact.clone(), hash });
                        fixed.push(artifact);
                    }
                    Ok(Asking::Plans { call: call_name, tool: tool_name, plans, runs: Vec::new(), stand_in, fix: Some(Fix { scripts, pipeline, steps }) })
                });
            }
            _ => return Begun::Answered(failed(call, "No such tool.")),
        };
        self.asking = Some(asking);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        Begun::Waiting
    }

    /// A research tool's call: a page is read at once; searches and a
    /// download wait for the user's yes.
    fn research(&mut self, folder: &Path, env: ScriptEnv, call: &str, tool: &str, name: &str, input: &Value) -> Begun {
        let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let web = |url: &str| -> Result<(), String> {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(format!("Not a web address: {url}"));
            }
            if is_local(url) {
                return Err(format!("{url} is on this computer or its local network, which research never reads."));
            }
            Ok(())
        };
        let asking = match name {
            FETCH_PAGE => {
                let url = text("url");
                if let Err(reason) = web(&url) {
                    return Begun::Answered(failed(call, reason));
                }
                let answered = call.to_string();
                self.spawn(call, move |_| match assistant::web::page_text(&url) {
                    Ok(page) => {
                        let cut = page.chars().count() > MOST_PAGE;
                        let page: String = page.chars().take(MOST_PAGE).collect();
                        ToolResult { call: answered, ok: true, output: json!({ "url": url, "text": page, "cut": cut }), images: Vec::new() }
                    }
                    Err(reason) => failed(&answered, reason),
                });
                return Begun::Waiting;
            }
            WEB_SEARCH => {
                let Some(key) = env.search_key.clone() else {
                    return Begun::Answered(failed(call, "Brave Search has no key, so nothing can be searched. The user adds it in Settings, as a Brave Search endpoint."));
                };
                let searches: Vec<(String, bool)> = input
                    .get("searches")
                    .and_then(Value::as_array)
                    .map(|searches| {
                        searches
                            .iter()
                            .filter_map(|search| {
                                let query = search.get("query").and_then(Value::as_str)?.trim().to_string();
                                (!query.is_empty()).then(|| (query, search.get("images").and_then(Value::as_bool).unwrap_or(false)))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if searches.is_empty() {
                    return Begun::Answered(failed(call, "No search was given."));
                }
                if searches.len() > MOST_SEARCHES {
                    return Begun::Answered(failed(call, format!("At most {MOST_SEARCHES} searches are asked for at once.")));
                }
                self.search_key = Some(key);
                Asking::Searches { call: call.to_string(), tool: tool.to_string(), searches, why: text("why") }
            }
            DOWNLOAD_FILE => {
                let url = text("url");
                if let Err(reason) = web(&url) {
                    return Begun::Answered(failed(call, reason));
                }
                let licence = if text("licence").is_empty() { String::from("unknown") } else { text("licence") };
                Asking::Download { call: call.to_string(), tool: tool.to_string(), to: staged_name(folder, &url), url, page: text("page"), licence }
            }
            _ => return Begun::Answered(failed(call, "No such tool.")),
        };
        self.cwd = folder.to_path_buf();
        self.asking = Some(asking);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        Begun::Waiting
    }

    /// Runs research on a thread of its own; its answer comes from `poll`.
    fn spawn(&mut self, call: &str, work: impl FnOnce(&std::sync::atomic::AtomicBool) -> ToolResult + Send + 'static) {
        let ctx = self.ctx.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let done = std::thread::spawn(move || {
            let answer = work(&stopped);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
            answer
        });
        self.tasks.push(Task { call: call.to_string(), done, stop });
    }

    /// The approved fix of the dependency chain, run step by step: each
    /// later step planned again and run only when it is the plan approved
    /// but for the files the fixes before it made; each result published as
    /// its artifact's next version with `--fixes`. Stop ends it before the
    /// next step.
    fn fix_chain(&mut self, call: &str, fix: Fix, approved: Vec<Value>, stand_in: bool) {
        let (answered, cwd, vars) = (call.to_string(), self.cwd.clone(), self.env.clone());
        self.spawn(call, move |stop| {
            let Fix { scripts, pipeline, steps } = fix;
            let mut published: HashMap<String, String> = HashMap::new();
            let mut done: Vec<Value> = Vec::new();
            let mut stopped: Option<String> = None;
            for (index, step) in steps.iter().enumerate() {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    stopped = Some(String::from("the user stopped the fix"));
                    break;
                }
                // A later step's plan as it is now, after the fixes before it.
                let hash = if index == 0 {
                    step.hash.clone()
                } else {
                    let mut argv = skills_argv(&scripts, "pipeline.py", &["rerun", &pipeline, "--step", &step.chain, &step.step, "--plan", "--json"]);
                    if stand_in {
                        argv.push(String::from("--stand-in"));
                    }
                    match planned(&argv, &cwd, &vars, stand_in).and_then(|(plan, hash)| same_but_fixed(&approved[index], &plan, &published).map(|()| hash)) {
                        Ok(hash) => hash,
                        Err(reason) => {
                            stopped = Some(format!("{} / {}: {reason}", step.chain, step.step));
                            break;
                        }
                    }
                };
                let mut argv = skills_argv(&scripts, "pipeline.py", &["rerun", &pipeline, "--step", &step.chain, &step.step, "--json", "--expect", &hash]);
                if stand_in {
                    argv.push(String::from("--stand-in"));
                }
                let made = match skills_run(&argv, &cwd, &vars) {
                    Ok(printed) => printed.lines().rev().find_map(|line| serde_json::from_str::<Value>(line.trim()).ok()).unwrap_or(Value::Null),
                    Err(reason) => {
                        stopped = Some(format!("{} / {}: {reason}", step.chain, step.step));
                        break;
                    }
                };
                if made.get("pending").is_some_and(|pending| !pending.is_null()) {
                    stopped = Some(format!("{} / {} is still being made at its endpoint: collect it with request.py collect, then fix again", step.chain, step.step));
                    break;
                }
                let Some(take) = made.get("files").and_then(Value::as_array).and_then(|files| files.first()).and_then(Value::as_str).map(String::from) else {
                    stopped = Some(format!("{} / {} made no file", step.chain, step.step));
                    break;
                };
                // Published at once as the artifact's next version, a fix,
                // with what its step refers to.
                let request = made.get("report").and_then(Value::as_str).and_then(|report| Path::new(report).file_stem()).map(|stem| stem.to_string_lossy().to_string());
                let query = format!("SELECT refs FROM step WHERE chain='{}' AND name='{}'", step.chain.replace('\'', "''"), step.step.replace('\'', "''"));
                let refs: Vec<Value> = skills_run(&skills_argv(&scripts, "pipeline.py", &["sql", &pipeline, &query, "--read-only"]), &cwd, &vars).ok().and_then(|printed| serde_json::from_str(printed.trim()).ok()).unwrap_or_default();
                let mut args: Vec<String> = ["publish", &pipeline, &step.artifact, "--file", &take, "--chain", &step.chain, "--fixes"].iter().map(|arg| arg.to_string()).collect();
                if let Some(request) = &request {
                    args.extend([String::from("--request"), request.clone()]);
                }
                for reference in &refs {
                    let text = |key: &str| reference.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                    if !text("artifact").is_empty() {
                        args.extend([String::from("--from"), text("artifact"), if text("role").is_empty() { String::from("input") } else { text("role") }]);
                    }
                }
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                if let Err(reason) = skills_run(&skills_argv(&scripts, "pipeline.py", &args), &cwd, &vars) {
                    stopped = Some(format!("{}'s fix could not be published: {reason}", step.artifact));
                    break;
                }
                published.insert(step.artifact.clone(), take.clone());
                done.push(json!({ "artifact": step.artifact, "chain": step.chain, "step": step.step, "file": take }));
            }
            let mut output = json!({ "fixed": done });
            if let Some(reason) = &stopped {
                let left: Vec<&str> = steps.iter().skip(published.len()).map(|step| step.artifact.as_str()).collect();
                output["stopped"] = json!(reason);
                output["not_fixed"] = json!(left);
                output["next"] = json!("Tell the user where it stopped and why. A new fix_dependency_chain plans the rest on a new card.");
            }
            ToolResult { call: answered, ok: stopped.is_none(), output, images: Vec::new() }
        });
    }

    /// The approved searches, run one after another.
    fn search(&mut self, call: &str, searches: Vec<(String, bool)>, key: String) {
        let answered = call.to_string();
        self.spawn(call, move |stop| {
            let mut results = Vec::new();
            for (query, images) in &searches {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    results.push(json!({ "query": query, "images": images, "error": "Not sent: the user stopped the research." }));
                    continue;
                }
                match assistant::web::brave_search(&key, query, *images) {
                    Ok(found) => results.push(json!({ "query": query, "images": images, "results": found })),
                    Err(reason) => results.push(json!({ "query": query, "images": images, "error": reason })),
                }
            }
            let ok = results.iter().any(|result| result.get("error").is_none());
            ToolResult { call: answered, ok, output: json!({ "searches": results, "cost": searches_cost(searches.len()) }), images: Vec::new() }
        });
    }

    /// The approved download, into Studio's own folder.
    fn download(&mut self, call: &str, url: String, page: String, licence: String, to: String) {
        let answered = call.to_string();
        let at = self.cwd.join(&to);
        self.spawn(call, move |_| match assistant::web::download(&url, &at, MOST_DOWNLOAD).and_then(|bytes| {
            // Where it came from, beside it: filing is checked against this.
            let found = json!({ "address": url, "page": page, "licence": licence });
            std::fs::write(provenance_of(&at), found.to_string()).map_err(|error| format!("Could not record where it came from: {error}"))?;
            Ok(bytes)
        }) {
            Ok(bytes) => ToolResult {
                call: answered,
                ok: true,
                output: json!({
                    "file": to, "bytes": bytes, "address": url, "page": page, "licence": licence,
                    "next": "Look at it with look_at_file. To keep it, file it into sources/ with send_request and a spec of kind research, whose outputs name this file and whose sources give its address, page, and the use allowed.",
                }),
                images: Vec::new(),
            },
            Err(reason) => failed(&answered, reason),
        });
    }

    /// Works out a plan on a thread of its own, so the window never waits
    /// on a script or a price list; the card shows when it is ready.
    fn plan_later(&mut self, call: &str, work: impl FnOnce() -> Result<Asking, String> + Send + 'static) -> Begun {
        let ctx = self.ctx.clone();
        let done = std::thread::spawn(move || {
            let asking = work();
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
            asking
        });
        self.planning = Some((call.to_string(), done));
        Begun::Waiting
    }

    /// The user's answer to what was asked. On their yes the command
    /// starts; otherwise the call is answered at once.
    pub(crate) fn answer(&mut self, approved: bool) -> Option<ToolResult> {
        let asking = self.asking.take()?;
        let (call, tool) = asking.call();
        let (call, tool) = (call.to_string(), tool.to_string());
        if !approved {
            // Declined, not failed: nothing ran, and nothing is to be tried
            // again or reported.
            self.search_key = None;
            return Some(ToolResult { call, ok: false, output: json!({ "error": "The user did not approve it, so nothing ran.", "declined": true }), images: Vec::new() });
        }
        let (mut runs, spending) = match asking {
            Asking::Command { argv, .. } => (vec![argv], false),
            // A step that runs a script, such as a builder, sends nothing to
            // a model, so Stop ends it as it ends any command.
            Asking::Plans { fix: Some(fix), plans, stand_in, .. } => {
                self.fix_chain(&call, fix, plans, stand_in);
                return None;
            }
            Asking::Plans { runs, stand_in, plans, .. } => (runs, !stand_in && plans.iter().any(|plan| plan.get("endpoint").is_some_and(|endpoint| !endpoint.is_null()))),
            Asking::Searches { searches, .. } => {
                let Some(key) = self.search_key.take() else { return Some(failed(&call, "Brave Search has no key.")) };
                self.search(&call, searches, key);
                return None;
            }
            Asking::Download { url, page, licence, to, .. } => {
                self.download(&call, url, page, licence, to);
                return None;
            }
        };
        let first = runs.remove(0);
        match self.start(&call, &tool, first, runs, String::new(), spending) {
            Ok(()) => None,
            Err(reason) => Some(failed(&call, reason)),
        }
    }

    fn start(&mut self, call: &str, tool: &str, argv: Vec<String>, then: Vec<Vec<String>>, before: String, spends: bool) -> Result<(), String> {
        // Keys go only to Skills' own scripts; any other command gets the
        // setup file alone.
        let skills = runs_skills(&argv, self.scripts.as_deref(), &self.cwd);
        let mut command = Command::new(&argv[0]);
        if !skills {
            strip_keys(&mut command);
        }
        let mut child = command
            .args(&argv[1..])
            .current_dir(&self.cwd)
            .envs(self.env.iter().filter(|(name, _)| skills || !name.ends_with("_KEY")).map(|(name, value)| (name.as_str(), value.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not run {}: {error}", argv[0]))?;
        let output = Arc::new(Mutex::new(String::new()));
        let mut readers = Vec::new();
        let streams: Vec<Box<dyn Read + Send>> = [child.stdout.take().map(|out| Box::new(out) as Box<dyn Read + Send>), child.stderr.take().map(|err| Box::new(err) as Box<dyn Read + Send>)]
            .into_iter()
            .flatten()
            .collect();
        for stream in streams {
            let output = output.clone();
            let ctx = self.ctx.clone();
            readers.push(std::thread::spawn(move || {
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Ok(mut output) = output.lock() {
                        output.push_str(&line);
                        output.push('\n');
                    }
                    if let Some(ctx) = &ctx {
                        ctx.request_repaint();
                    }
                }
            }));
        }
        self.jobs.push(Job { call: call.to_string(), tool: tool.to_string(), argv, child, output, readers, shown: 0, then, before, spends });
        Ok(())
    }

    /// Whether a command made for `call` is running.
    pub(crate) fn is_running(&self, call: &str) -> bool {
        self.jobs.iter().any(|job| job.call == call) || self.tasks.iter().any(|task| task.call == call) || self.planning.as_ref().is_some_and(|(planning, _)| planning == call)
    }

    /// Runs a command at once, with no question: the user's own action
    /// in a workspace, such as filing a file, which spends nothing. Its
    /// answer comes back from `poll` under `call`.
    pub(crate) fn run_now(&mut self, folder: &Path, env: ScriptEnv, call: &str, command: &[String]) -> Result<(), String> {
        let argv = resolve(command, &env)?;
        if spends(&argv) {
            return Err(String::from("This command spends money, so it is asked for with its plan first."));
        }
        self.cwd = folder.to_path_buf();
        self.scripts = env.scripts.clone();
        self.env = env.vars;
        self.start(call, "", argv, Vec::new(), String::new(), false)
    }

    /// Asks the user to approve a request from its spec, for a caller
    /// that is not the assistant, such as a cell on the Prompt page.
    pub(crate) fn ask_request(&mut self, folder: &Path, env: ScriptEnv, call: &str, spec: &str) -> Begun {
        self.begin(folder, env, call, "", SEND_REQUEST, &json!({ "spec": spec }))
    }

    /// The user stopped one caller's command, such as a cell's run. An
    /// approved spend runs on, as `stop` says; anything else is killed.
    pub(crate) fn stop_call(&mut self, call: &str) -> Vec<ToolResult> {
        let mut answers = Vec::new();
        if self.planning.as_ref().is_some_and(|(planning, _)| planning == call)
            && let Some((call, _)) = self.planning.take()
        {
            answers.push(failed(&call, "Stopped before it was asked, so nothing ran."));
        }
        let (stopping, kept): (Vec<Job>, Vec<Job>) = std::mem::take(&mut self.jobs).into_iter().partition(|job| job.call == call);
        let (stopping_tasks, kept_tasks): (Vec<Task>, Vec<Task>) = std::mem::take(&mut self.tasks).into_iter().partition(|task| task.call == call);
        self.jobs = stopping;
        self.tasks = stopping_tasks;
        answers.extend(self.stop_jobs());
        self.jobs = kept;
        self.tasks = kept_tasks;
        answers
    }

    /// The window to wake when a command's output arrives.
    pub(crate) fn wake_with(&mut self, ctx: &eframe::egui::Context) {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
    }

    /// Called each frame: the answers of commands that ended, and the
    /// output of those still running, as `(tool key, text)` for their rows.
    #[cfg(test)]
    pub(crate) fn poll(&mut self) -> (Vec<ToolResult>, Vec<(String, String)>) {
        let (answers, live, _) = self.poll_all();
        (answers, live)
    }

    /// As `poll`, with each command that ended well, as it ran.
    pub(crate) fn poll_all(&mut self) -> (Vec<ToolResult>, Vec<(String, String)>, Vec<Vec<String>>) {
        let mut finished = Vec::new();
        self.detached.retain_mut(|child| child.try_wait().ok().flatten().is_none());
        let mut answers = Vec::new();
        if self.planning.as_ref().is_some_and(|(_, done)| done.is_finished())
            && let Some((call, done)) = self.planning.take()
        {
            match done.join() {
                Ok(Ok(asking)) => self.asking = Some(asking),
                Ok(Err(reason)) => answers.push(failed(&call, reason)),
                Err(_) => answers.push(failed(&call, "The plan could not be made.")),
            }
        }
        for task in std::mem::take(&mut self.tasks) {
            if task.done.is_finished() {
                let call = task.call.clone();
                answers.push(task.done.join().unwrap_or_else(|_| failed(&call, "The research stopped unexpectedly.")));
            } else {
                self.tasks.push(task);
            }
        }
        let mut live = Vec::new();
        let mut next = Vec::new();
        let mut kept = Vec::new();
        for mut job in std::mem::take(&mut self.jobs) {
            let text = job.output.lock().map(|output| output.clone()).unwrap_or_default();
            match job.child.try_wait() {
                Ok(None) => {
                    if text.len() != job.shown {
                        job.shown = text.len();
                        live.push((job.tool.clone(), tail_lines(&text, LIVE_LINES)));
                    }
                    kept.push(job);
                }
                ended => {
                    for reader in job.readers.drain(..) {
                        let _ = reader.join();
                    }
                    let text = job.output.lock().map(|output| output.clone()).unwrap_or_default();
                    let code = ended.ok().flatten().and_then(|status| status.code());
                    let all = format!("{}{}", job.before, text);
                    live.push((job.tool.clone(), tail_lines(&all, LIVE_LINES)));
                    if code == Some(0) && !job.then.is_empty() {
                        let mut then = job.then;
                        next.push((job.call, job.tool, then.remove(0), then, all, job.spends));
                        continue;
                    }
                    if code == Some(0) {
                        finished.push(job.argv.clone());
                    }
                    answers.push(ToolResult {
                        call: job.call,
                        ok: code == Some(0),
                        output: json!({ "command": job.argv.join(" "), "exit": code, "output": tail_chars(&all, MOST_OUTPUT) }),
                        images: Vec::new(),
                    });
                }
            }
        }
        self.jobs = kept;
        for (call, tool, argv, then, before, spends) in next {
            if let Err(reason) = self.start(&call, &tool, argv, then, before, spends) {
                answers.push(failed(&call, reason));
            }
        }
        (answers, live, finished)
    }

    /// The user stopped the request. An approved spend is never killed:
    /// what it sent may already be paid for, so it runs on, its output
    /// still read, and files its results when it ends, or can be collected.
    /// Any other command is killed, since it spends nothing. A question
    /// waiting is answered as not approved.
    pub(crate) fn stop(&mut self) -> Vec<ToolResult> {
        let mut answers = Vec::new();
        if let Some(result) = self.answer(false) {
            answers.push(result);
        }
        if let Some((call, _)) = self.planning.take() {
            answers.push(failed(&call, "Stopped before it was asked, so nothing ran."));
        }
        answers.extend(self.stop_jobs());
        answers
    }

    fn stop_jobs(&mut self) -> Vec<ToolResult> {
        let mut answers = Vec::new();
        // Research on its own thread is let finish unheard: searches sent are
        // paid for, and a download only waits in Studio's own folder.
        for task in std::mem::take(&mut self.tasks) {
            task.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            answers.push(failed(&task.call, "Stopped. Searches already sent are paid for, and no more are sent; a download still arriving waits unfiled."));
        }
        for mut job in std::mem::take(&mut self.jobs) {
            let text = job.output.lock().map(|output| output.clone()).unwrap_or_default();
            let submitted = text.lines().find_map(|line| line.split(SUBMITTED).nth(1).map(|id| id.trim().to_string()));
            match (job.spends, submitted) {
                (true, submitted) => {
                    let at = submitted.map(|id| format!(" ({id})")).unwrap_or_default();
                    answers.push(failed(
                        &job.call,
                        format!("Stopped waiting. The request was approved and may already be sent and paid for{at}, so it runs on and files its results when it ends; if it does not, collect it later with request.py collect."),
                    ));
                    self.detached.push(job.child);
                }
                (false, _) => {
                    let _ = job.child.kill();
                    let _ = job.child.wait();
                    answers.push(failed(&job.call, "Stopped. It spent nothing."));
                }
            }
        }
        answers
    }
}

fn tail_lines(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn tail_chars(text: &str, most: usize) -> String {
    if text.len() <= most {
        return text.to_string();
    }
    let mut start = text.len() - most;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("[the first {start} characters are left out]\n{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    /// Waits for a plan worked out on its own thread to be asked.
    fn wait_until_asked(runner: &mut Runner) {
        let start = std::time::Instant::now();
        while runner.asking().is_none() {
            if let Some(answer) = runner.poll().0.pop() {
                panic!("answered before it was asked: {:?}", answer.output);
            }
            assert!(start.elapsed().as_secs() < 30, "the plan never came");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Catches a fix's later step run on a plan that changed in more than
    /// the file an earlier fix made, or one whose reference is not that
    /// fix's file, and a step marked when it uses nothing being fixed.
    #[test]
    fn a_later_step_runs_only_as_approved_but_for_the_fixes() {
        let mut approved = json!({
            "endpoint": { "kind": "fal", "model": "m" }, "prompt": "The diner.", "hash": "h1",
            "references": [{ "order": 1, "role": "image 1", "canon": "the diner: layout", "file": "library/layout.png", "sha256": "a" }, { "order": 2, "role": "image 2", "canon": "the cook: sheet", "file": "library/sheet.png", "sha256": "b" }],
        });
        mark_fixed(&mut approved, &[String::from("the diner: layout")]);
        assert_eq!(approved["references"][0][FIXED_BY], "the diner: layout");
        assert!(approved["references"][1].get(FIXED_BY).is_none(), "a reference nothing fixes was marked");
        let published = HashMap::from([(String::from("the diner: layout"), String::from("library/layout-2.png"))]);
        let mut now = json!({
            "endpoint": { "kind": "fal", "model": "m" }, "prompt": "The diner.", "hash": "h2",
            "references": [{ "order": 1, "role": "image 1", "canon": "the diner: layout", "file": "library/layout-2.png", "sha256": "c" }, { "order": 2, "role": "image 2", "canon": "the cook: sheet", "file": "library/sheet.png", "sha256": "b" }],
        });
        assert_eq!(same_but_fixed(&approved, &now, &published), Ok(()));
        now["prompt"] = json!("The diner at night.");
        assert!(same_but_fixed(&approved, &now, &published).is_err(), "a changed prompt ran");
        now["prompt"] = json!("The diner.");
        now["references"][1]["sha256"] = json!("d");
        assert!(same_but_fixed(&approved, &now, &published).is_err(), "a reference no fix made changed and ran");
        now["references"][1]["sha256"] = json!("b");
        now["references"][0]["file"] = json!("library/someone-else.png");
        assert!(same_but_fixed(&approved, &now, &published).is_err(), "a reference that is not the fix's file ran");
    }

    /// Catches a command that spends run as a plain command: the contract's
    /// list, and nothing else.
    #[test]
    fn only_run_and_rerun_spend() {
        assert!(spends(&argv(&["python3", "/s/scripts/request.py", "run", "agent/spec.json"])));
        assert!(spends(&argv(&["python3", "/s/scripts/pipeline.py", "rerun", "002"])));
        assert!(!spends(&argv(&["python3", "/s/scripts/request.py", "run", "spec.json", "--stand-in"])));
        assert!(!spends(&argv(&["python3", "/s/scripts/pipeline.py", "rerun", "002", "--plan", "--json"])));
        for read in [["request.py", "show"], ["request.py", "collect"], ["request.py", "cancel"], ["pipeline.py", "touches"]] {
            assert!(!spends(&argv(&["python3", read[0], read[1]])), "{read:?}");
        }
    }

    /// Catches a key written into the setup file, which is in the data
    /// folder and may be read by anyone with the folder.
    #[test]
    fn the_setup_file_holds_no_key() {
        let data = std::env::temp_dir().join(format!("runner-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let mut settings = Settings::default();
        settings.endpoints.push(crate::workspace::EndpointSetting { id: "fal".into(), name: "fal.ai".into(), kind: EndpointKind::Fal, address: "https://fal.ai".into(), description: String::new() });
        let env = script_env(&settings, &mut |_| Some(String::from("secret-key")), Some(&data), None);
        assert!(env.vars.contains(&(String::from("FAL_KEY"), String::from("secret-key"))));
        let setup = std::fs::read_to_string(data.join("skills-setup.json")).unwrap();
        assert!(!setup.contains("secret-key"), "{setup}");
        assert!(setup.contains("setup-v1"));
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Catches a read-only command asked about, or a command that writes
    /// run without asking: the owner's exact list, and nothing else.
    #[test]
    fn only_the_listed_reads_run_without_asking() {
        let folder = std::env::temp_dir().join(format!("runner-reads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let scripts = folder.join("checkout/scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::create_dir_all(folder.join("project/brief")).unwrap();
        for name in ["request.py", "pipeline.py"] {
            std::fs::write(scripts.join(name), "").unwrap();
        }
        let project = folder.join("project");
        let own = |name: &str| scripts.join(name).display().to_string();
        let reads = |words: &[&str]| only_reads(&argv(words), Some(&scripts), &project);
        for read in [["request.py", "show", "f.png"], ["request.py", "tell", "r1"], ["pipeline.py", "touches", "002"], ["pipeline.py", "history", "002"]] {
            assert!(reads(&["python3", &*own(read[0]), read[1], read[2]]), "{read:?}");
        }
        assert!(reads(&["python3", &*own("pipeline.py"), "sql", "002", "SELECT name FROM artifact", "--read-only"]));
        assert!(!reads(&["python3", &*own("pipeline.py"), "sql", "002", "DELETE FROM artifact"]));
        for write in [["request.py", "collect"], ["request.py", "resource"], ["pipeline.py", "publish"], ["pipeline.py", "keep"], ["pipeline.py", "fact"]] {
            assert!(!reads(&["python3", &*own(write[0]), write[1]]), "{write:?}");
        }
        assert!(!reads(&["sh", "-c", "python3 request.py show f.png"]));
        assert!(!runs_skills(&argv(&["sh", "-c", "env; python3 request.py"]), Some(&scripts), &project), "a shell string ending in a script's name got the keys");
        // A file the agent wrote, named like Skills' script, is not Skills'
        // own: it is asked about, and gets no key.
        std::fs::write(project.join("brief/request.py"), "").unwrap();
        assert!(!reads(&["python3", "brief/request.py", "show", "f.png"]), "a lookalike script ran without asking");
        assert!(!runs_skills(&argv(&["python3", "brief/request.py", "show"]), Some(&scripts), &project), "a lookalike script got the keys");
        // Nor is a program in the project named like Python.
        assert!(!runs_skills(&argv(&["agent/python3", &*own("request.py"), "show"]), Some(&scripts), &project), "a lookalike interpreter got the keys");
        assert!(secret("OPENROUTER_API_KEY") && secret("fal_key_id") && secret("GITHUB_TOKEN") && !secret("SKILLS_SETUP") && !secret("PATH"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a command that spends passed as a plain one through a
    /// shell, and a plan with no hash run unbound.
    #[test]
    fn a_spend_through_a_shell_and_a_plan_with_no_hash_are_refused() {
        assert!(publishes(&argv(&["python3", "/s/scripts/pipeline.py", "publish", "002", "the diner: layout"])), "a publish ran as a command");
        assert!(publishes(&argv(&["sh", "-c", "python3 pipeline.py publish 002 x"])));
        assert!(!publishes(&argv(&["python3", "/s/scripts/pipeline.py", "touches", "002"])));
        assert!(spends(&argv(&["sh", "-c", "python3 /s/scripts/request.py run agent/spec.json"])));
        assert!(spends(&argv(&["bash", "-c", "cd x; python3 pipeline.py rerun 002"])));
        assert!(!spends(&argv(&["sh", "-c", "python3 /s/scripts/request.py run agent/spec.json --stand-in"])));
        assert!(bound(json!({ "prompt": "p" })).is_err());
        assert!(bound(json!({ "prompt": "p", "hash": "" })).is_err());
        assert_eq!(bound(json!([{ "hash": "h1" }, { "hash": "h2" }])).unwrap().1, "h1");
    }

    /// Catches what only Spumcake-Skills' own request.py shows: a request
    /// planned by its `plan`, approved, and run on its `--stand-in` with
    /// the plan's hash leaves its report and its output. Runs when
    /// SPUMCAKE_SKILLS_SCRIPTS names the checkout's scripts folder.
    #[test]
    fn the_real_request_py_runs_an_approved_plan() {
        let Ok(scripts) = std::env::var("SPUMCAKE_SKILLS_SCRIPTS") else { return };
        let folder = std::env::temp_dir().join(format!("runner-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["library", "requests", "agent"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        std::fs::write(
            folder.join("agent/spec.json"),
            r#"{"name": "check", "skill": "generate-keyframe", "endpoint": {"kind": "fal", "model": "stand-in"}, "prompt": "a check"}"#,
        )
        .unwrap();
        let mut runner = Runner::default();
        runner.assume_prerequisites(&folder);
        let env = ScriptEnv { scripts: Some(PathBuf::from(scripts)), ..ScriptEnv::default() };
        let begun = runner.begin(&folder, env, "c1", "t", SEND_REQUEST, &json!({ "spec": "agent/spec.json", "stand_in": true }));
        if let Begun::Answered(result) = &begun {
            panic!("answered at once: {:?}", result.output);
        }
        wait_until_asked(&mut runner);
        assert!(runner.answer(true).is_none());
        let start = std::time::Instant::now();
        let answer = loop {
            if let Some(answer) = runner.poll().0.pop() {
                break answer;
            }
            assert!(start.elapsed().as_secs() < 60, "request.py never ended");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(answer.ok, "{:?}", answer.output);
        let count = |sub: &str| std::fs::read_dir(folder.join(sub)).unwrap().count();
        assert_eq!((count("library"), count("requests")), (1, 1), "{:?}", answer.output);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a search run before the user's yes or with no key, the key
    /// left in what is asked, a cancelled card taken for a failure, and a
    /// download kept outside Studio's own folder or over a file waiting.
    #[test]
    fn research_waits_for_the_users_yes() {
        let folder = std::env::temp_dir().join(format!("runner-research-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let mut runner = Runner::default();
        let input = json!({ "searches": [{ "query": "1939 diner counter", "images": true }, { "query": "1939 diner menu" }], "why": "The diner's references." });
        let refused = runner.begin(&folder, ScriptEnv::default(), "c1", "t", WEB_SEARCH, &input);
        assert!(matches!(refused, Begun::Answered(result) if !result.ok), "a search ran with no key");
        let env = ScriptEnv { search_key: Some(String::from("stand-in-key")), ..ScriptEnv::default() };
        assert!(matches!(runner.begin(&folder, env, "c2", "t", WEB_SEARCH, &input), Begun::Waiting));
        let asking = runner.asking().expect("nothing was asked");
        let Asking::Searches { searches, .. } = asking else { panic!("{asking:?}") };
        assert_eq!(searches.len(), 2);
        assert!(!format!("{asking:?}").contains("stand-in-key"), "the key is in what is asked");
        assert!(runner.tasks.is_empty(), "a search ran before the yes");
        let declined = runner.answer(false).unwrap();
        assert!(!declined.ok && declined.output["declined"] == true);
        assert!(runner.search_key.is_none() && runner.tasks.is_empty());
        assert_eq!(searches_cost(2), "about $0.010");
        let staged = staged_name(&folder, "https://example.org/files/Diner%20Counter.jpg?size=large");
        assert!(staged.starts_with(&format!("{}/{STAGED}/", protocol::PROJECT_DIR)) && staged.ends_with(".jpg"), "{staged}");
        std::fs::create_dir_all(folder.join(&staged).parent().unwrap()).unwrap();
        std::fs::write(folder.join(&staged), b"x").unwrap();
        assert_ne!(staged_name(&folder, "https://example.org/files/Diner%20Counter.jpg"), staged, "a download would overwrite one waiting");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches research filed into sources/ without where it came from, or
    /// with an address other than the one it was downloaded from, and a
    /// page read on this computer or its network.
    #[test]
    fn research_is_filed_only_with_where_it_came_from() {
        let folder = std::env::temp_dir().join(format!("runner-sources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let staged = folder.join(protocol::PROJECT_DIR).join(STAGED);
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("counter.jpg"), b"x").unwrap();
        std::fs::write(provenance_of(&staged.join("counter.jpg")), json!({ "address": "https://example.org/counter.jpg", "page": "https://example.org/page", "licence": "CC BY 4.0" }).to_string()).unwrap();
        let output = format!("{}/{STAGED}/counter.jpg", protocol::PROJECT_DIR);
        let check = |sources: Value| {
            let spec = folder.join("spec.json");
            std::fs::write(&spec, json!({ "endpoint": { "kind": "research" }, "outputs": [output], "sources": sources }).to_string()).unwrap();
            sources_match(&folder, &spec)
        };
        assert!(check(json!([])).is_err(), "a file with no source was filed");
        assert!(check(json!([{ "file": "counter.jpg", "address": "https://example.org/counter.jpg" }])).is_err(), "a source with no use was filed");
        assert!(check(json!([{ "file": "counter.jpg", "address": "https://example.org/other.jpg", "use": "CC BY 4.0" }])).is_err(), "a source with another address was filed");
        assert!(check(json!([{ "file": "counter.jpg", "address": "https://example.org/counter.jpg", "page": "https://example.org/page", "use": "Public domain" }])).is_err(), "a use other than the licence approved was filed");
        assert!(check(json!([{ "file": "counter.jpg", "address": "https://example.org/counter.jpg", "page": "https://example.org/page", "use": "CC BY 4.0" }])).is_ok());
        std::fs::write(staged.join("unrecorded.jpg"), b"x").unwrap();
        let spec = folder.join("spec.json");
        std::fs::write(&spec, json!({ "endpoint": { "kind": "research" }, "outputs": [format!("{}/{STAGED}/unrecorded.jpg", protocol::PROJECT_DIR)], "sources": [{ "file": "unrecorded.jpg", "address": "https://example.org/x.jpg", "use": "CC0" }] }).to_string()).unwrap();
        assert!(sources_match(&folder, &spec).is_err(), "a waiting file with no record of its download was filed");
        for local in ["http://localhost:8080/", "http://127.0.0.1/x", "https://192.168.1.4/", "http://[::1]/", "http://10.0.0.2", "http://router/", "http://printer.local/"] {
            assert!(is_local(local), "{local}");
        }
        for public in ["https://example.org/page", "https://en.wikipedia.org/wiki/Diner", "http://93.184.216.34/"] {
            assert!(!is_local(public), "{public}");
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a script step, such as a builder, left running after Stop:
    /// it sends nothing to a model, so it is ended as any command is.
    #[test]
    fn stop_ends_a_script_step() {
        let folder = std::env::temp_dir().join(format!("runner-script-step-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("scripts")).unwrap();
        std::fs::write(
            folder.join("scripts/pipeline.py"),
            "import json, sys, time\ndef plan():\n    print(json.dumps({'script': 'build.py', 'requests': 1, 'hash': 'h1'}))\nif '--plan' in sys.argv:\n    plan()\nelif '--expect' in sys.argv:\n    time.sleep(30)\n",
        )
        .unwrap();
        let mut runner = Runner::default();
        runner.assume_prerequisites(&folder);
        let env = ScriptEnv { scripts: Some(folder.join("scripts")), ..ScriptEnv::default() };
        if let Begun::Answered(result) = runner.begin(&folder, env, "c1", "t", RERUN_STEP, &json!({ "pipeline": "001" })) {
            panic!("answered at once: {:?}", result.output);
        }
        wait_until_asked(&mut runner);
        assert!(runner.answer(true).is_none());
        let stopped = runner.stop();
        assert_eq!(stopped.len(), 1);
        assert!(runner.jobs.is_empty() && runner.detached.is_empty(), "a script step was left running after Stop");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a chain that cannot move between Studio and a plain agentic
    /// harness: Studio reruns a step, the harness changes a fact and reruns
    /// with Skills' scripts alone, and Studio, with its own folder gone,
    /// reruns again from the fact as it now stands. Every file has its
    /// report, and `request.py check` passes. Runs when
    /// SPUMCAKE_SKILLS_SCRIPTS names the checkout's scripts folder.
    #[test]
    fn a_chain_moves_between_studio_and_a_plain_harness() {
        let Ok(scripts) = std::env::var("SPUMCAKE_SKILLS_SCRIPTS") else { return };
        let scripts = PathBuf::from(scripts);
        let folder = std::env::temp_dir().join(format!("runner-round-trip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["library", "requests"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        // The harness: Skills' scripts, run as any agent runs them.
        let harness = |script: &str, args: &[&str]| -> String {
            let out = Command::new("python3").arg(scripts.join(script)).args(args).current_dir(&folder).output().unwrap();
            assert!(out.status.success(), "{script} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        harness("pipeline.py", &["new", "001"]);
        harness("pipeline.py", &["element", "001", "the diner", "--kind", "place"]);
        harness("pipeline.py", &["chain", "001", "the line", "--order", "01"]);
        harness("pipeline.py", &["step", "001", "the line", "keyframe", "--order", "1", "--skill", "generate-keyframe", "--endpoint", r#"{"kind": "fal", "model": "stand-in"}"#, "--template", "The diner at {{the diner: time}}."]);
        harness("pipeline.py", &["fact", "001", "the diner: time", "dusk", "--artifact", "the diner: brief"]);
        harness("pipeline.py", &["step", "001", "the line", "keyframe", "--artifact", "the line: keyframe"]);
        let studio = |runner: &mut Runner| -> ToolResult {
            let env = ScriptEnv { scripts: Some(scripts.clone()), ..ScriptEnv::default() };
            let input = json!({ "pipeline": "001", "fact": "the diner: time", "stand_in": true });
            if let Begun::Answered(result) = runner.begin(&folder, env, "c1", "t", RERUN_STEP, &input) {
                panic!("answered at once: {:?}", result.output);
            }
            wait_until_asked(runner);
            assert!(runner.answer(true).is_none());
            let start = std::time::Instant::now();
            loop {
                if let Some(answer) = runner.poll().0.pop() {
                    break answer;
                }
                assert!(start.elapsed().as_secs() < 60, "pipeline.py rerun never ended");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        };
        let mut runner = Runner::default();
        runner.assume_prerequisites(&folder);
        let first = studio(&mut runner);
        assert!(first.ok, "{:?}", first.output);
        std::fs::create_dir_all(folder.join(protocol::PROJECT_DIR)).unwrap();
        std::fs::write(folder.join(protocol::PROJECT_DIR).join("project.json"), "{}").unwrap();
        // The harness changes the fact and reruns, with no Studio at all.
        harness("pipeline.py", &["fact", "001", "the diner: time", "night", "--artifact", "the diner: brief"]);
        let plan: Value = serde_json::from_str(&harness("pipeline.py", &["rerun", "001", "--fact", "the diner: time", "--stand-in", "--plan", "--json"])).unwrap();
        harness("pipeline.py", &["rerun", "001", "--fact", "the diner: time", "--stand-in", "--expect", plan["hash"].as_str().unwrap()]);
        // Back in Studio, with nothing of Studio's own left: the step's cell
        // makes its spec from canon as it now stands, as Render again does,
        // and sends it through the plan card on the stand-in.
        std::fs::remove_dir_all(folder.join(protocol::PROJECT_DIR)).unwrap();
        let mut runner = Runner::default();
        runner.assume_prerequisites(&folder);
        let cell: protocol::Cell = serde_json::from_value(json!({ "id": "c1", "kind": "image", "input": [], "state": "idle", "step": { "chain": "the line", "step": "keyframe" } })).unwrap();
        let at = protocol::ItemRef { path: String::from("work/Keyframes.ipx"), item: Some(String::from("c1")) };
        let mut page = crate::prompt_page::PromptPage::default();
        let spec = page.spec(&folder, &scripts, &[], &Settings::default(), &cell, &at).unwrap();
        let filled: Value = serde_json::from_str(&harness("pipeline.py", &["fill", "001", "the line", "keyframe"])).unwrap();
        assert_eq!(spec["prompt"], filled["prompt"], "Studio's spec is not the one pipeline.py fills");
        let file = crate::prompt_page::write_spec(&folder, "c1", &spec).unwrap();
        let env = ScriptEnv { scripts: Some(scripts.clone()), ..ScriptEnv::default() };
        if let Begun::Answered(result) = runner.begin(&folder, env, "cell:c1", "", SEND_REQUEST, &json!({ "spec": file, "stand_in": true })) {
            panic!("answered at once: {:?}", result.output);
        }
        wait_until_asked(&mut runner);
        assert!(runner.answer(true).is_none());
        let start = std::time::Instant::now();
        let last = loop {
            if let Some(answer) = runner.poll().0.pop() {
                break answer;
            }
            assert!(start.elapsed().as_secs() < 60, "request.py never ended");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(last.ok, "{:?}", last.output);
        let files: Vec<String> = std::fs::read_dir(folder.join("library")).unwrap().flatten().map(|entry| format!("library/{}", entry.file_name().to_string_lossy())).collect();
        assert_eq!(files.len(), 3, "{files:?}");
        for file in &files {
            harness("request.py", &["show", file]);
        }
        harness("request.py", &["check"]);
        let prompts: Vec<String> = std::fs::read_dir(folder.join("prompts")).unwrap().flatten().map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default()).collect();
        assert_eq!(prompts.iter().filter(|prompt| prompt.contains("at night")).count(), 2, "Studio's last rerun did not read the fact the harness set: {prompts:?}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a request whose plan is not asked for, or asked for with
    /// a run that does not carry the plan's hash.
    #[test]
    fn a_request_is_asked_with_its_plan() {
        let folder = std::env::temp_dir().join(format!("runner-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("scripts")).unwrap();
        // As Skills' own does, it plans offline, and runs only with --expect.
        std::fs::write(
            folder.join("scripts/request.py"),
            "import json, sys\ndef plan():\n    print(json.dumps({'prompt': 'p', 'requests': 1, 'hash': 'h1'}))\nif sys.argv[1] == 'plan':\n    plan()\nelif '--expect' not in sys.argv:\n    sys.exit('refused')\n",
        )
        .unwrap();
        std::fs::write(folder.join("spec.json"), "{}").unwrap();
        let mut runner = Runner::default();
        runner.assume_prerequisites(&folder);
        let env = ScriptEnv { scripts: Some(folder.join("scripts")), ..ScriptEnv::default() };
        let begun = runner.begin(&folder, env, "c1", "t", SEND_REQUEST, &json!({ "spec": "spec.json" }));
        if let Begun::Answered(result) = &begun {
            panic!("answered at once: {:?}", result.output);
        }
        wait_until_asked(&mut runner);
        match runner.asking() {
            Some(Asking::Plans { plans, runs, .. }) => {
                assert_eq!(plans[0]["hash"], "h1");
                assert!(runs[0].ends_with(&[String::from("--expect"), String::from("h1")]), "{runs:?}");
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a command that runs before the user's yes, or a spending
    /// command passed as a plain one; and catches output lost or a stopped
    /// command left running.
    #[test]
    fn a_command_runs_only_on_the_persons_yes() {
        let folder = std::env::temp_dir().join(format!("runner-yes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let mut runner = Runner::default();
        runner.checked = Some((folder.clone(), Vec::new()));
        let env = ScriptEnv::default();
        let input = json!({ "command": ["sh", "-c", "echo made > made.txt; echo done"], "why": "A check." });
        assert!(matches!(runner.begin(&folder, env.clone(), "c1", "t", RUN_COMMAND, &input), Begun::Waiting));
        assert!(!folder.join("made.txt").exists(), "it ran before the yes");
        let refused = runner.answer(false).unwrap();
        assert!(!refused.ok && !folder.join("made.txt").exists());
        assert!(matches!(runner.begin(&folder, env.clone(), "c2", "t", RUN_COMMAND, &input), Begun::Waiting));
        wait_until_asked(&mut runner);
        assert!(runner.answer(true).is_none());
        let start = std::time::Instant::now();
        let answer = loop {
            let (mut answers, _) = runner.poll();
            if let Some(answer) = answers.pop() {
                break answer;
            }
            assert!(start.elapsed().as_secs() < 10, "the command never ended");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(answer.ok && answer.output["output"].as_str().unwrap().contains("done"));
        assert!(folder.join("made.txt").exists());
        let spending = json!({ "command": ["python3", "/s/scripts/request.py", "run", "spec.json"], "why": "Spend." });
        assert!(matches!(runner.begin(&folder, env.clone(), "c3", "t", RUN_COMMAND, &spending), Begun::Answered(result) if !result.ok));
        let slow = json!({ "command": ["sleep", "30"], "why": "Wait." });
        assert!(matches!(runner.begin(&folder, env, "c4", "t", RUN_COMMAND, &slow), Begun::Waiting));
        runner.answer(true);
        let stopped = runner.stop();
        assert!(stopped.len() == 1 && !stopped[0].ok);
        assert!(runner.jobs.is_empty() && runner.detached.is_empty(), "a command not yet submitted was left running");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
