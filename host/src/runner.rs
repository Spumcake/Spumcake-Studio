//! The script runner: Spumcake-Skills' scripts, and any other command the
//! assistant asks for, run in a Skills project after the person approves
//! each one, as the Studio-as-a-setup contract says.
//!
//! A command that spends, `request.py run` or `pipeline.py rerun` without
//! `--stand-in`, never runs as a plain command: it is asked for with its
//! plan, which `request.py plan` or `pipeline.py rerun --plan` works out
//! offline, and runs only with `--expect` and the hash of the plan the
//! person approved. Every other command is shown in full and runs on their
//! yes. A command runs off the window's thread; its output streams into its
//! tool row, and its answer reaches the assistant when it ends. Keys reach
//! the scripts as environment variables only, never written anywhere.

use crate::workspace::{EndpointKind, Settings};
use protocol::{Tool, ToolResult};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub(crate) const RUN_COMMAND: &str = "run_command";
pub(crate) const SEND_REQUEST: &str = "send_request";
pub(crate) const RERUN_STEP: &str = "rerun_step";

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
            description: "Run a command in the project, such as a Spumcake-Skills script (request.py and pipeline.py may be named by their file name alone), a builder, or a script of the agent's own, after the person approves it. Its output is answered when it ends. A command that spends is refused: send it with send_request or rerun_step.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "array", "items": { "type": "string" }, "description": "The program and its arguments, one per item, such as [\"request.py\", \"show\", \"<file>\"]." },
                    "why": { "type": "string", "description": "One line for the person: what it does and why." }
                },
                "required": ["command", "why"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
        Tool {
            name: SEND_REQUEST.into(),
            description: "Send one request to a model with request.py, from its spec: its plan, worked out offline, is shown to the person with its cost, its prompt as sent, and every reference in order, and it runs only on their yes, exactly as approved. Answers with what request.py tell says.".into(),
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
            description: "Make a take of the first step of a pipeline that is out of date, or the first a change reaches, with pipeline.py rerun: its plan is shown to the person and runs only on their yes. It stops after that one step; publishing the take comes first.".into(),
            input: json!({
                "type": "object",
                "properties": {
                    "pipeline": { "type": "string", "description": "The pipeline's number, such as 002." },
                    "fact": { "type": "string", "description": "A changed fact, as \"<element>: <fact>\", to rerun what it reaches." },
                    "artifact": { "type": "string", "description": "A changed artifact, as \"<element>: <role>\", to rerun what it reaches." },
                    "stand_in": { "type": "boolean", "description": "Run it on the stand-in, which spends nothing." }
                },
                "required": ["pipeline"],
                "additionalProperties": false
            }),
            implements: Vec::new(),
        },
    ]
}

/// Whether `name` is one of the runner's tools.
pub(crate) fn is_runner_tool(name: &str) -> bool {
    matches!(name, RUN_COMMAND | SEND_REQUEST | RERUN_STEP)
}

/// What a script is run with: its folder, and its environment.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScriptEnv {
    /// The folder holding Spumcake-Skills' `request.py` and `pipeline.py`.
    pub scripts: Option<PathBuf>,
    /// The keys and `SKILLS_SETUP`, as NAME and value.
    pub vars: Vec<(String, String)>,
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
    for endpoint in &settings.endpoints {
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
    ScriptEnv { scripts, vars }
}

/// The setup file, `setup-v1`, with the keys `request.py` reads: its
/// prices, and Studio's `.env` when one is kept. It never holds a key.
fn write_setup(data: &Path) -> Option<PathBuf> {
    let path = data.join("skills-setup.json");
    let mut setup = json!({ "format": "setup-v1", "prices": {} });
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
        let folder = env.scripts.as_ref().ok_or("No Spumcake-Skills checkout is in the Skill folders, so its scripts cannot run. Add it in Settings.")?;
        Ok(folder.join(name).display().to_string())
    };
    Ok(match first.as_str() {
        "request.py" | "pipeline.py" => [String::from("python3"), script(first)?].into_iter().chain(command[1..].iter().cloned()).collect(),
        _ => command.to_vec(),
    })
}

/// Which of Skills' scripts a command runs, and its subcommand.
/// Only a script Python runs directly counts, as `resolve` makes it, so a
/// shell string that merely ends in a script's name is never taken for one.
fn script_of(argv: &[String]) -> Option<(&'static str, String)> {
    let program = Path::new(argv.first()?).file_name()?.to_string_lossy().to_string();
    if !program.starts_with("python") {
        return None;
    }
    let script = argv.get(1)?;
    let name = if script.ends_with("request.py") {
        "request.py"
    } else if script.ends_with("pipeline.py") {
        "pipeline.py"
    } else {
        return None;
    };
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

/// Whether a command only reads, and so runs without asking, as the owner
/// approved on 2026-11-16: the contract's exact list of Skills' commands
/// that render, download, spend, and change nothing. `pipeline.py sql`
/// counts only with `--read-only`. Commands run with no shell, so a command
/// given to one names no script of its own and never counts.
pub(crate) fn only_reads(argv: &[String]) -> bool {
    match script_of(argv) {
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
fn runs_skills(argv: &[String]) -> bool {
    script_of(argv).is_some()
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

/// What the person is asked to approve.
#[derive(Debug, Clone)]
pub(crate) enum Asking {
    /// A command shown in full.
    Command { call: String, tool: String, argv: Vec<String>, why: String },
    /// One or more requests, each with its plan, each run with its own
    /// `--expect`.
    Plans { call: String, tool: String, plans: Vec<Value>, runs: Vec<Vec<String>>, stand_in: bool },
}

impl Asking {
    fn call(&self) -> (&str, &str) {
        match self {
            Asking::Command { call, tool, .. } | Asking::Plans { call, tool, .. } => (call, tool),
        }
    }
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

/// The runner: what waits for the person's yes, and what runs.
#[derive(Default)]
pub(crate) struct Runner {
    asking: Option<Asking>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    jobs: Vec<Job>,
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
    /// It waits for the person, then for the command.
    Waiting,
}

fn failed(call: &str, reason: impl Into<String>) -> ToolResult {
    ToolResult { call: call.to_string(), ok: false, output: json!({ "error": reason.into() }), images: Vec::new() }
}

impl Runner {
    pub(crate) fn asking(&self) -> Option<&Asking> {
        self.asking.as_ref()
    }

    /// Takes the programs a Skills project needs as found, for checks on a
    /// computer that may lack one the check does not run.
    #[cfg(test)]
    pub(crate) fn assume_prerequisites(&mut self, folder: &Path) {
        self.checked = Some((folder.to_path_buf(), Vec::new()));
    }

    /// The programs a Skills project's scripts need, with any missing.
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
        if self.asking.is_some() {
            return Begun::Answered(failed(call, "Another command is waiting for the person's yes. Ask for one at a time."));
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
                    return Begun::Answered(failed(call, "This command spends money, so it is never run as a plain command. Send a request with send_request and its spec, or a step with rerun_step, so the person approves its plan."));
                }
                // A command that only reads runs at once, with no question.
                if only_reads(&argv) {
                    return match self.start(call, tool, argv, Vec::new(), String::new(), false) {
                        Ok(()) => Begun::Waiting,
                        Err(reason) => Begun::Answered(failed(call, reason)),
                    };
                }
                Asking::Command { call: call.to_string(), tool: tool.to_string(), argv, why: text("why") }
            }
            SEND_REQUEST => {
                let spec = text("spec");
                let planned = resolve(&[String::from("request.py"), String::from("plan"), spec.clone(), String::from("--json")], &env)
                    .and_then(|argv| plans_offline(Path::new(&argv[1])).map(|()| argv))
                    .and_then(|argv| self.plan(&argv))
                    .and_then(bound);
                let (plan, hash) = match planned {
                    Ok(planned) => planned,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                let mut run = vec![String::from("request.py"), String::from("run"), spec, String::from("--expect"), hash];
                if stand_in {
                    run.push(String::from("--stand-in"));
                }
                let run = match resolve(&run, &env) {
                    Ok(run) => run,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                Asking::Plans { call: call.to_string(), tool: tool.to_string(), plans: vec![plan], runs: vec![run], stand_in }
            }
            RERUN_STEP => {
                let mut asked = vec![String::from("pipeline.py"), String::from("rerun"), text("pipeline")];
                for key in ["fact", "artifact"] {
                    if !text(key).is_empty() {
                        asked.push(format!("--{key}"));
                        asked.push(text(key));
                    }
                }
                if stand_in {
                    asked.push(String::from("--stand-in"));
                }
                let mut planning = asked.clone();
                planning.extend([String::from("--plan"), String::from("--json")]);
                let planned = resolve(&planning, &env)
                    .and_then(|argv| plans_offline(Path::new(&argv[1])).map(|()| argv))
                    .and_then(|argv| self.plan(&argv))
                    .and_then(bound);
                let (plan, hash) = match planned {
                    Ok(planned) => planned,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                asked.extend([String::from("--expect"), hash]);
                let run = match resolve(&asked, &env) {
                    Ok(run) => run,
                    Err(reason) => return Begun::Answered(failed(call, reason)),
                };
                Asking::Plans { call: call.to_string(), tool: tool.to_string(), plans: vec![plan], runs: vec![run], stand_in }
            }
            _ => return Begun::Answered(failed(call, "No such tool.")),
        };
        self.asking = Some(asking);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        Begun::Waiting
    }

    /// A plan, worked out offline by Skills' own script.
    fn plan(&self, argv: &[String]) -> Result<Value, String> {
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(&self.cwd)
            .envs(self.env.iter().map(|(name, value)| (name.as_str(), value.as_str())))
            .output()
            .map_err(|error| format!("Could not run {}: {error}", argv[0]))?;
        if !out.status.success() {
            return Err(format!("The plan could not be made: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        serde_json::from_slice(&out.stdout).map_err(|error| format!("The plan was not JSON: {error}"))
    }

    /// The person's answer to what was asked. On their yes the command
    /// starts; otherwise the call is answered at once.
    pub(crate) fn answer(&mut self, approved: bool) -> Option<ToolResult> {
        let asking = self.asking.take()?;
        let (call, tool) = asking.call();
        let (call, tool) = (call.to_string(), tool.to_string());
        if !approved {
            return Some(failed(&call, "The person did not approve it, so nothing ran."));
        }
        let (mut runs, spending) = match asking {
            Asking::Command { argv, .. } => (vec![argv], false),
            Asking::Plans { runs, stand_in, .. } => (runs, !stand_in),
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
        let skills = runs_skills(&argv);
        let mut child = Command::new(&argv[0])
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
        self.jobs.iter().any(|job| job.call == call)
    }

    /// Runs a command at once, with no question: the person's own action
    /// in a workspace, such as filing a file, which spends nothing. Its
    /// answer comes back from `poll` under `call`.
    pub(crate) fn run_now(&mut self, folder: &Path, env: ScriptEnv, call: &str, command: &[String]) -> Result<(), String> {
        let argv = resolve(command, &env)?;
        if spends(&argv) {
            return Err(String::from("This command spends money, so it is asked for with its plan first."));
        }
        self.cwd = folder.to_path_buf();
        self.env = env.vars;
        self.start(call, "", argv, Vec::new(), String::new(), false)
    }

    /// Asks the person to approve a request from its spec, for a caller
    /// that is not the assistant, such as a cell on the Prompt page.
    pub(crate) fn ask_request(&mut self, folder: &Path, env: ScriptEnv, call: &str, spec: &str) -> Begun {
        self.begin(folder, env, call, "", SEND_REQUEST, &json!({ "spec": spec }))
    }

    /// The person stopped one caller's command, such as a cell's run. An
    /// approved spend runs on, as `stop` says; anything else is killed.
    pub(crate) fn stop_call(&mut self, call: &str) -> Vec<ToolResult> {
        let (stopping, kept): (Vec<Job>, Vec<Job>) = std::mem::take(&mut self.jobs).into_iter().partition(|job| job.call == call);
        self.jobs = stopping;
        let answers = self.stop_jobs();
        self.jobs = kept;
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
    pub(crate) fn poll(&mut self) -> (Vec<ToolResult>, Vec<(String, String)>) {
        let (answers, live, _) = self.poll_all();
        (answers, live)
    }

    /// As `poll`, with each command that ended well, as it ran.
    pub(crate) fn poll_all(&mut self) -> (Vec<ToolResult>, Vec<(String, String)>, Vec<Vec<String>>) {
        let mut finished = Vec::new();
        self.detached.retain_mut(|child| child.try_wait().ok().flatten().is_none());
        let mut answers = Vec::new();
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

    /// The person stopped the request. An approved spend is never killed:
    /// what it sent may already be paid for, so it runs on, its output
    /// still read, and files its results when it ends, or can be collected.
    /// Any other command is killed, since it spends nothing. A question
    /// waiting is answered as not approved.
    pub(crate) fn stop(&mut self) -> Vec<ToolResult> {
        let mut answers = Vec::new();
        if let Some(result) = self.answer(false) {
            answers.push(result);
        }
        answers.extend(self.stop_jobs());
        answers
    }

    fn stop_jobs(&mut self) -> Vec<ToolResult> {
        let mut answers = Vec::new();
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
        settings.endpoints.push(crate::workspace::EndpointSetting { id: "fal".into(), name: "fal.ai".into(), kind: EndpointKind::Fal, address: "https://fal.ai".into() });
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
        for read in [["request.py", "show", "f.png"], ["request.py", "tell", "r1"], ["pipeline.py", "touches", "002"], ["pipeline.py", "history", "002"]] {
            assert!(only_reads(&argv(&["python3", read[0], read[1], read[2]])), "{read:?}");
        }
        assert!(only_reads(&argv(&["python3", "/s/pipeline.py", "sql", "002", "SELECT name FROM artifact", "--read-only"])));
        assert!(!only_reads(&argv(&["python3", "/s/pipeline.py", "sql", "002", "DELETE FROM artifact"])));
        for write in [["request.py", "collect"], ["request.py", "resource"], ["pipeline.py", "publish"], ["pipeline.py", "keep"], ["pipeline.py", "fact"]] {
            assert!(!only_reads(&argv(&["python3", write[0], write[1]])), "{write:?}");
        }
        assert!(!only_reads(&argv(&["sh", "-c", "python3 request.py show f.png"])));
        assert!(!runs_skills(&argv(&["sh", "-c", "env; python3 request.py"])), "a shell string ending in a script's name got the keys");
    }

    /// Catches a command that spends passed as a plain one through a
    /// shell, and a plan with no hash run unbound.
    #[test]
    fn a_spend_through_a_shell_and_a_plan_with_no_hash_are_refused() {
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
        let env = ScriptEnv { scripts: Some(PathBuf::from(scripts)), vars: Vec::new() };
        let begun = runner.begin(&folder, env, "c1", "t", SEND_REQUEST, &json!({ "spec": "agent/spec.json", "stand_in": true }));
        if let Begun::Answered(result) = &begun {
            panic!("answered at once: {:?}", result.output);
        }
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
        let env = ScriptEnv { scripts: Some(folder.join("scripts")), vars: Vec::new() };
        let begun = runner.begin(&folder, env, "c1", "t", SEND_REQUEST, &json!({ "spec": "spec.json" }));
        if let Begun::Answered(result) = &begun {
            panic!("answered at once: {:?}", result.output);
        }
        match runner.asking() {
            Some(Asking::Plans { plans, runs, .. }) => {
                assert_eq!(plans[0]["hash"], "h1");
                assert!(runs[0].ends_with(&[String::from("--expect"), String::from("h1")]), "{runs:?}");
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a command that runs before the person's yes, or a spending
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
