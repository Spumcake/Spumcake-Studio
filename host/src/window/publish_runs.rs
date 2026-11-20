//! The runner's side of publishing from a workspace,
//! as the Studio session manager laid it out on 2026-11-16: it takes what
//! `publishing.rs` keeps in `Publishing::asked`, fills the publish card or
//! answers the history, and carries out each card the user confirmed, all
//! through Spumcake-Skills' own scripts, one job at a time, off the
//! window's thread.
//!
//! - A publish checks first that every input it was made from is canon,
//!   and is refused with "Its inputs aren't canon." when one is not. Its
//!   card shows the version it writes, one after the artifact's last, and
//!   the run log's uncommitted lines a commit takes along.
//! - On the yes, a workspace's file is filed with `request.py resource`, a
//!   take is published with its request, and `pipeline.py publish` makes it
//!   canon; with Commit on, those paths alone are committed, never pushed.
//! - A revert publishes the earlier version's file again, with the request
//!   that made it, through the same card.
//! - History is `pipeline.py history <n> "<artifact>" --json`, newest
//!   first; a checkout without it answers "history unavailable".
//! - A workspace's `deliver` of a mixed film is checked as a publish is,
//!   from the film's own record, before the Deliver card; on its yes the
//!   film is published, the chain pages written, the film copied into
//!   `output/` with `pipeline.py deliver`, and what that wrote committed by
//!   path when the project is in git. The answer is `delivered`.
//! - A workspace's `run_step`, such as Storyboard's export, is the
//!   runner's `pipeline.py rerun <n> --step <chain> <step>`, through the plan
//!   card, answered `step_ran` with the take it made. It needs no assistant.

use super::HostWindow;
use super::publishing::{Asked, Card, Delivery};
#[cfg(test)]
use super::publishing::Asker;
use crate::publish_card::PublishCardView;
use crate::runner::ScriptEnv;
use eframe::egui;
use protocol::{CanonVersion, HostMessage};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

/// The sentence a publish is refused with when an input is not canon, as
/// the Studio session manager set on 2026-11-16.
const NOT_CANON: &str = "Its inputs aren't canon.";

/// What one job came to.
enum Done {
    /// A delivery checked, ready for the Deliver card, or refused before it.
    Checked(Delivery, Result<(), String>),
    /// A delivery carried out: the file in `output/`, or why not.
    Delivered(Delivery, Result<String, String>),
    Card(Asked, Result<PublishCardView, String>),
    History(Asked, Result<Vec<CanonVersion>, String>),
    Published(Asked, Result<(String, Option<u32>), String>),
}

/// A Save waiting on the warning `ig-dependency-warning` shows: the take,
/// the file saved, and the warning's words.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IterationAsk {
    app: usize,
    tab: String,
    cell: String,
    path: String,
    source: String,
    /// "Take 3 is in use as a dependency, …".
    pub warning: String,
    /// "2 takes in Shot 12 and Shot 13 were made from it."
    pub made_from: String,
}

/// What a take ask came to.
enum TakeDone {
    Used { app: usize, tab: String, cell: String, path: String, result: Result<String, String> },
    Saved { app: usize, tab: String, cell: String, path: String, result: Result<Option<String>, String> },
    Ignored { app: usize, tab: String, cell: String, path: String, result: Result<(), String> },
    /// The asset is in use and other work was made from it.
    Warn(IterationAsk),
}

/// The warning's title, as `ig-dependency-warning` shows it.
const SAVE_ITERATION_TITLE: &str = "Save a new iteration?";
const SAVE_ITERATION: &str = "Save iteration";
/// A take Use cannot put in use, since no recipe's step makes it.
const IN_NO_RECIPE: &str = "This take is in no recipe yet, so it cannot be put in use.";
/// A step that names no artifact to publish a take as.
const NO_ARTIFACT: &str = "Its step names no artifact to put the take in use as.";
/// A take that is no asset's, so it has nothing to iterate or ignore.
const NO_ASSET: &str = "This take is not one of the project's assets.";

/// The publishing job running, if any, and the delivery the Deliver card
/// shows.
#[derive(Default)]
pub(crate) struct Runs {
    working: Option<Receiver<Done>>,
    /// What workspaces asked of their takes, each with its workspace, the
    /// cell's id, and the cell's step, waiting their turn.
    take_queue: std::collections::VecDeque<(usize, String, String, Option<protocol::StepRef>, String, crate::process::TakeAsk)>,
    /// The take ask running, if any.
    taking: Option<Receiver<TakeDone>>,
    /// A Save on an asset other work was made from, waiting on the warning.
    pub iteration_warning: Option<IterationAsk>,
    /// Checked, and waiting for the user's answer on the Deliver card.
    pub deliver_card: Option<Delivery>,
    /// Confirmed on the Deliver card, to carry out.
    delivering: std::collections::VecDeque<Delivery>,
}

/// What a job runs with: the project, Skills' scripts, their environment,
/// and the living pipeline.
#[derive(Clone)]
struct Place {
    folder: PathBuf,
    scripts: PathBuf,
    vars: Vec<(String, String)>,
    pipeline: String,
}

impl Place {
    fn script(&self, name: &str, args: &[&str]) -> Result<String, String> {
        crate::prompt_page::script(&self.scripts, &self.folder, &self.vars, name, args)
    }

    /// A read of canon, each printed row's cells.
    fn rows(&self, query: &str) -> Result<Vec<Vec<String>>, String> {
        let printed = self.script("pipeline.py", &["sql", &self.pipeline, query, "--read-only"])?;
        Ok(printed.lines().filter(|line| !line.is_empty()).map(|line| line.split(" | ").map(String::from).collect()).collect())
    }
}

/// The runner's call for a workspace's step run, so its answer goes back.
fn step_call(app: usize, id: &str) -> String {
    format!("steprun:{app}:{id}")
}

/// The take a step's rerun made, from the one line of JSON its output ends
/// with: its first file, or why there is none yet.
fn step_take(printed: &str) -> (Option<String>, Option<String>) {
    let last = printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
    let Ok(result) = serde_json::from_str::<Value>(last) else {
        return (None, Some(String::from("The step ran, but its take was not named. Update the Spumcake-Skills checkout.")));
    };
    if let Some(pending) = result.get("pending").and_then(Value::as_str) {
        return (None, Some(format!("The request {pending} is still at its endpoint, already paid for; collect it with request.py collect {pending}.")));
    }
    match result.get("files").and_then(Value::as_array).and_then(|files| files.first()).and_then(Value::as_str) {
        Some(file) => (Some(file.to_string()), None),
        None => (None, Some(String::from("The step ran but made no file."))),
    }
}

/// A text as an SQL string.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Every version of an artifact, newest first, from Skills' own history.
fn versions(place: &Place, artifact: &str) -> Result<Vec<CanonVersion>, String> {
    let printed = match place.script("pipeline.py", &["history", &place.pipeline, artifact, "--json"]) {
        Ok(printed) => printed,
        // An artifact never published has no versions yet.
        Err(error) if error.contains("no artifact") => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let listed: Value = serde_json::from_str(&printed).map_err(|_| String::from("History unavailable: this Spumcake-Skills checkout cannot list an artifact's versions."))?;
    Ok(listed
        .as_array()
        .into_iter()
        .flatten()
        .map(|version| {
            let text = |key: &str| version.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            CanonVersion {
                file: text("file"),
                published: crate::prompt_page::epoch(&text("published")).unwrap_or(0),
                note: text("note"),
                canon: version.get("canon_now").and_then(Value::as_bool).unwrap_or(false),
                request: Some(text("request")).filter(|request| !request.is_empty()),
            }
        })
        .collect())
}

/// A git command in the project, never with a key.
fn git(folder: &Path, args: &[&str]) -> Result<String, String> {
    let out = crate::runner::strip_keys(&mut Command::new("git")).args(args).current_dir(folder).output().map_err(|error| format!("Could not run git: {error}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Whether the project is kept in git, so a publish can be committed: its
/// repository tracks a file in the project's folder, as spumcake-benchmarks
/// tracks Last order's. A repository that only happens to hold the folder,
/// ignoring it or never having added it, is not the project's.
fn in_git(folder: &Path) -> bool {
    git(folder, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out.trim() == "true")
        && git(folder, &["ls-files", "--", "."]).is_ok_and(|tracked| !tracked.trim().is_empty())
}

/// The run log's lines not yet committed, which a commit takes along, each
/// as a line the card shows.
fn ride_along(folder: &Path, pipeline: &str) -> Vec<String> {
    let log = format!("pipeline/{pipeline}/runs.jsonl");
    let tracked = git(folder, &["ls-files", "--error-unmatch", &log]).is_ok();
    let added: Vec<String> = if tracked {
        git(folder, &["diff", "--no-color", "-U0", "HEAD", "--", &log])
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .map(|line| line[1..].to_string())
            .collect()
    } else {
        std::fs::read_to_string(folder.join(&log)).unwrap_or_default().lines().map(String::from).collect()
    };
    added
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|event| {
            let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            let what = [text("artifact"), text("step"), text("label")].into_iter().find(|what| !what.is_empty()).unwrap_or_default();
            let file = [text("file")].into_iter().chain(event.get("files").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from)).find(|file| !file.is_empty());
            match file {
                Some(file) => format!("{} {what}, {file}", text("event")),
                None => format!("{} {what}", text("event")),
            }
        })
        .collect()
}

/// The card for a publish or a revert, or why it is refused before it.
fn prepare(place: &Place, asked: &Asked) -> Result<PublishCardView, String> {
    let (artifact, file, revert) = match asked {
        Asked::Publish { artifact, source, from, .. } => {
            // Every input it was made from must be canon; a workspace's own
            // file in work/ is where it came from, never an input.
            let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline)?;
            if from.iter().filter(|input| !input.starts_with("work/")).any(|input| !canon.published.contains(input)) {
                return Err(String::from(NOT_CANON));
            }
            (artifact, source, false)
        }
        Asked::Revert { artifact, file, .. } => (artifact, file, true),
        Asked::History { .. } => return Err(String::from("Nothing to publish.")),
    };
    let listed = versions(place, artifact)?;
    // Skills' history lists each file once, at its latest publish: a file
    // already listed, as a revert's is, becomes the newest of the same
    // count, and a new file the next.
    let version = if listed.iter().any(|version| &version.file == file) { listed.len() as u32 } else { listed.len() as u32 + 1 };
    let commit = in_git(&place.folder);
    Ok(PublishCardView {
        file: file.clone(),
        artifact: artifact.clone(),
        version,
        ride_along: if commit { ride_along(&place.folder, &place.pipeline) } else { Vec::new() },
        commit,
        revert,
    })
}

/// The files a file made without a model was made from, as its own record
/// names them, outside `work/`; None when it has no record.
fn record_inputs(place: &Place, file: &str) -> Option<Vec<String>> {
    let record: Value = serde_json::from_str(&place.script("request.py", &["show", file]).ok()?).ok()?;
    let lines = match record {
        Value::Array(lines) => lines,
        line => vec![line],
    };
    Some(
        lines
            .iter()
            .flat_map(|line| line.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default())
            .filter_map(|input| input.get("file").and_then(Value::as_str).map(String::from))
            .filter(|input| !input.starts_with("work/"))
            .collect(),
    )
}

/// Whether a delivery may be shown on its card: its film's record names
/// what it was mixed from, and each of those is published canon.
fn check_delivery(place: &Place, delivery: &Delivery) -> Result<(), String> {
    let inputs = record_inputs(place, &delivery.file).ok_or_else(|| String::from(NO_MIX_RECORD))?;
    let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline)?;
    if inputs.iter().any(|input| !canon.published.contains(input)) {
        return Err(String::from(NOT_CANON));
    }
    Ok(())
}

/// Carries out a delivery the user said yes to: the film published as its
/// artifact with what it was made from, the chain pages written, the film
/// copied into `output/`, and, in git, what that wrote committed by path.
/// Returns the file in `output/`.
fn deliver(place: &Place, delivery: &Delivery) -> Result<String, String> {
    // The mix is an asset of its own, made from the shot, as finishing is
    // and Last order's mixes are: "<shot>, mixed", made from the shot in a
    // role that says how its picture was finished, as Last order's 60 fps
    // mix records it. The shot's own asset in use is never replaced.
    let (shot, file) = (delivery.artifact.as_str(), delivery.file.as_str());
    let artifact = format!("{shot}, mixed");
    let artifact = artifact.as_str();
    let role = mix_source_role(place, shot, file);
    let made_from = sources_of(place, artifact, None, file, &[(shot.to_string(), role)])?;
    let chain = chain_of(place, shot)?;
    let mut args: Vec<&str> = vec!["publish", place.pipeline.as_str(), artifact, "--file", file];
    if let Some(chain) = &chain {
        args.extend(["--chain", chain.as_str()]);
    }
    for (source, role) in &made_from {
        args.extend(["--from", source.as_str(), role.as_str()]);
    }
    place.script("pipeline.py", &args)?;
    let _ = place.script("pipeline.py", &["render", &place.pipeline]);
    let printed = place.script("pipeline.py", &["deliver", &place.pipeline, file])?;
    // Studio commits nothing by itself: committing is the user's.
    printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).map(String::from).ok_or_else(|| String::from("The film was published, but where it was delivered was not given."))
}

/// The role a mix names its shot in, as Last order records it, so Claude
/// Code and the finishing rule find Studio's mixes as they find its: the
/// shot as it is, or how it was finished first.
const MIX_SOURCE: &str = "the picture"; // Skills' stored spelling
const UNTOUCHED: &str = "untouched";

/// The role the mix at `file` names `shot` in: from the video its record
/// says it was mixed from, back through each finishing request that made
/// it, to the shot's own file, each finishing in turn as Last order words
/// it, "retimed to 60 fps", "upscaled to 4K"; or "untouched".
fn mix_source_role(place: &Place, shot: &str, file: &str) -> String {
    let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline).unwrap_or_default();
    let shot_file = canon.files.get(shot).cloned().unwrap_or_default();
    let picture = record_inputs(place, file).unwrap_or_default().into_iter().find(|input| crate::prompt_page::media_type(input).starts_with("video/"));
    let mut finished: Vec<String> = Vec::new();
    let mut at = picture.unwrap_or_default();
    for _ in 0..8 {
        if at.is_empty() || at == shot_file {
            break;
        }
        let Some(record) = place.script("request.py", &["show", &at]).ok().and_then(|shown| serde_json::from_str::<Value>(&shown).ok()) else { break };
        let settings = record.pointer("/endpoint/settings").cloned().unwrap_or(Value::Null);
        if let Some(fps) = settings.get("target_fps").and_then(Value::as_f64) {
            finished.push(format!("retimed to {fps:.0} fps"));
        }
        if settings.get("upscale_factor").is_some() {
            finished.push(String::from("upscaled to 4K"));
        }
        let Some(source) = record.get("references").and_then(Value::as_array).and_then(|references| references.iter().find(|reference| reference.get("field").and_then(Value::as_str) == Some("video_url"))).and_then(|reference| reference.get("file").and_then(Value::as_str)) else { break };
        at = source.to_string();
    }
    finished.reverse();
    if finished.is_empty() { format!("{MIX_SOURCE}, {UNTOUCHED}") } else { format!("{MIX_SOURCE}, {}", finished.join(", then ")) }
}

/// The chain an artifact belongs to: its step's, or its own row's.
fn chain_of(place: &Place, artifact: &str) -> Result<Option<String>, String> {
    let query = format!("SELECT chain FROM step WHERE artifact={0} UNION ALL SELECT chain FROM artifact WHERE id={0}", quoted(artifact));
    Ok(place.rows(&query)?.into_iter().filter_map(|row| row.into_iter().next()).find(|chain| !chain.trim().is_empty()))
}

/// The request whose report made a file, from its record.
fn request_of(place: &Place, file: &str) -> Option<String> {
    let shown: Value = serde_json::from_str(&place.script("request.py", &["show", file]).ok()?).ok()?;
    ["id", "request"].iter().find_map(|key| shown.get(*key).and_then(Value::as_str).map(String::from))
}

/// What a take was made from, as Skills' method publishes it: each canon
/// artifact the step that makes `artifact` refers to, with its role, and
/// each canon reference its request's report names. `pipeline.py publish`
/// checks the take against these and records them, so `touches` and
/// `rerun` reach it from them in any harness.
fn sources_of(place: &Place, artifact: &str, request: Option<&str>, file: &str, named: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut add = |name: &str, role: &str| {
        if !name.is_empty() && !sources.iter().any(|(known, _)| known == name) {
            sources.push((name.to_string(), if role.trim().is_empty() { String::from("input") } else { role.to_string() }));
        }
    };
    // What the asker names comes first, with its own role.
    for (name, role) in named {
        add(name, role);
    }
    let query = format!("SELECT json_object('refs', refs) FROM step WHERE artifact={}", quoted(artifact));
    let printed = place.script("pipeline.py", &["sql", &place.pipeline, &query, "--read-only"])?;
    for line in printed.lines() {
        let refs: Vec<Value> = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|row| row.get("refs").and_then(Value::as_str).and_then(|refs| serde_json::from_str(refs).ok()))
            .unwrap_or_default();
        for reference in refs {
            let text = |key: &str| reference.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            add(&text("artifact"), &text("role"));
        }
    }
    if let Some(request) = request
        && let Some(record) = place.script("request.py", &["show", request]).ok().and_then(|shown| serde_json::from_str::<Value>(&shown).ok())
    {
        for reference in record.get("references").and_then(Value::as_array).into_iter().flatten() {
            let text = |key: &str| reference.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            add(&text("canon"), &text("role"));
        }
    }
    // A file made without a model: the canon artifacts whose files its
    // record names as inputs.
    if request.is_none()
        && let Some(record) = place.script("request.py", &["show", file]).ok().and_then(|shown| serde_json::from_str::<Value>(&shown).ok())
    {
        let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline)?;
        let lines = match record {
            Value::Array(lines) => lines,
            line => vec![line],
        };
        for input in lines.iter().flat_map(|line| line.get("inputs").and_then(Value::as_array).cloned().unwrap_or_default()) {
            let Some(path) = input.get("file").and_then(Value::as_str) else { continue };
            if let Some((name, _)) = canon.files.iter().find(|(_, published)| published.as_str() == path) {
                add(name, "input");
            }
        }
    }
    Ok(sources)
}

/// The sources a version had, with the roles it really had, as Skills'
/// own history works them out, from the run log or canon.sql as it stood
/// at that version's commit. None when no record holds them.
fn version_sources(place: &Place, artifact: &str, file: &str) -> Option<Vec<(String, String)>> {
    let printed = place.script("pipeline.py", &["history", &place.pipeline, artifact, "--json"]).ok()?;
    let listed: Value = serde_json::from_str(&printed).ok()?;
    let version = listed.as_array()?.iter().find(|version| version.get("file").and_then(Value::as_str) == Some(file))?;
    let sources = version.get("made_from")?.as_array()?;
    Some(
        sources
            .iter()
            .filter_map(|source| {
                let name = source.get("source").and_then(Value::as_str)?.to_string();
                let role = source.get("role").and_then(Value::as_str).filter(|role| !role.trim().is_empty()).unwrap_or("input").to_string();
                Some((name, role))
            })
            .collect(),
    )
}

/// The sources a file was published with before, from the run log's last
/// publish of it as this artifact, with their roles as the log records
/// them, or, in an older log, as canon gives them now or "input": a revert
/// keeps what was named when it was first published.
fn logged_sources(place: &Place, artifact: &str, file: &str) -> Vec<(String, String)> {
    let log = std::fs::read_to_string(place.folder.join("pipeline").join(&place.pipeline).join("runs.jsonl")).unwrap_or_default();
    let Some(event) = log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event.get("event").and_then(Value::as_str) == Some("publish") && event.get("artifact").and_then(Value::as_str) == Some(artifact) && event.get("file").and_then(Value::as_str) == Some(file))
        .last()
    else {
        return Vec::new();
    };
    // Skills writes each source with its role from 07f09ad on.
    if let Some(sources) = event.get("sources").and_then(Value::as_array) {
        return sources
            .iter()
            .filter_map(|source| {
                let name = source.get("source").and_then(Value::as_str)?.to_string();
                let role = source.get("role").and_then(Value::as_str).filter(|role| !role.trim().is_empty()).unwrap_or("input").to_string();
                Some((name, role))
            })
            .collect();
    }
    // An older log names its sources only: their roles are canon's now.
    let names: Vec<String> = event.get("made_from").and_then(Value::as_array).map(|names| names.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
    let query = format!("SELECT json_array(source, role) FROM made_from WHERE artifact={}", quoted(artifact));
    let roles: Vec<(String, String)> = place
        .script("pipeline.py", &["sql", &place.pipeline, &query, "--read-only"])
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Vec<String>>(line).ok())
        .filter_map(|pair| match pair.as_slice() {
            [source, role] => Some((source.clone(), role.clone())),
            _ => None,
        })
        .collect();
    names
        .into_iter()
        .map(|name| {
            let role = roles.iter().find(|(source, _)| *source == name).map(|(_, role)| role.clone()).unwrap_or_else(|| String::from("input"));
            (name, role)
        })
        .collect()
}

/// The files of the takes published together with this one, each
/// "<artifact>=<request id>": their outputs, reports, and prompts.
fn with_paths(place: &Place, with: &[String]) -> Vec<String> {
    let mut paths = Vec::new();
    for together in with {
        let Some((_, request)) = together.split_once('=') else { continue };
        paths.extend([format!("requests/{request}.html"), format!("prompts/{request}.txt")]);
        if let Some(record) = place.script("request.py", &["show", request]).ok().and_then(|shown| serde_json::from_str::<Value>(&shown).ok()) {
            paths.extend(record.get("outputs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from));
        }
    }
    paths
}

/// The version a file is of an artifact, counted from its first: its place
/// in the history, which lists each file once, newest first.
fn version_of(versions: &[CanonVersion], file: &str) -> Option<u32> {
    versions.iter().position(|version| version.file == file).map(|at| (versions.len() - at) as u32)
}

/// Carries out a confirmed card: the file filed when it is a workspace's,
/// published with what it was made from, the chain pages written again,
/// and, with Commit on, the paths it wrote committed. Returns the file
/// published and its version.
fn carry(place: &Place, card: &Card, commit: bool) -> Result<(String, Option<u32>), String> {
    let mut paths: Vec<String> = vec![format!("pipeline/{}", place.pipeline)];
    let (artifact, file, request, made_from, with) = match &card.asked {
        Asked::Publish { artifact, source, from, with, sources, .. } => {
            if source.starts_with("work/") {
                let stem = Path::new(source).file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_else(|| String::from("published"));
                // request.py names a resource by the minute and its name, so
                // a second publish within the minute takes the next name.
                let mut printed = Err(String::new());
                for n in 1..10 {
                    let name = if n == 1 { stem.clone() } else { format!("{stem}-{n}") };
                    let mut args: Vec<&str> = vec!["resource", source.as_str(), "--name", name.as_str()];
                    for input in from {
                        args.extend(["--from", input.as_str()]);
                    }
                    printed = place.script("request.py", &args);
                    if !printed.as_ref().is_err_and(|error| error.contains("refusing to overwrite")) {
                        break;
                    }
                }
                let printed = printed.map_err(|error| format!("The file could not be filed: {error}"))?;
                let filed = printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).map(String::from).ok_or("The file was filed, but its new name was not given.")?;
                paths.extend([filed.clone(), String::from("prompts/resources/records.jsonl")]);
                // A workspace's file was made from the canon files it links,
                // each with the role it played there.
                let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline)?;
                let mut made_from: Vec<(String, String)> = from
                    .iter()
                    .filter(|input| !input.starts_with("work/"))
                    .filter_map(|input| canon.files.iter().find(|(_, published)| *published == input).map(|(name, _)| (name.clone(), String::from("input"))))
                    .collect();
                for (name, role) in sources.iter().rev() {
                    made_from.retain(|(known, _)| known != name);
                    made_from.insert(0, (name.clone(), role.clone()));
                }
                made_from.dedup();
                (artifact.clone(), filed, None, made_from, with.clone())
            } else {
                // A take: its request, and what its step and its report say
                // it was made from, as the method publishes it by hand.
                let request = request_of(place, source);
                match &request {
                    Some(request) => paths.extend([format!("requests/{request}.html"), format!("prompts/{request}.txt")]),
                    None => paths.push(String::from("prompts/resources/records.jsonl")),
                }
                paths.push(source.clone());
                let made_from = sources_of(place, artifact, request.as_deref(), source, sources)?;
                (artifact.clone(), source.clone(), request, made_from, with.clone())
            }
        }
        Asked::Revert { artifact, file, .. } => {
            let request = versions(place, artifact)?.into_iter().find(|version| &version.file == file).and_then(|version| version.request);
            // The roles that version really had, from Skills' history,
            // or else the run log.
            let named = version_sources(place, artifact, file).unwrap_or_else(|| logged_sources(place, artifact, file));
            let made_from = sources_of(place, artifact, request.as_deref(), file, &named)?;
            (artifact.clone(), file.clone(), request, made_from, Vec::new())
        }
        Asked::History { .. } => return Err(String::from("Nothing to publish.")),
    };
    paths.extend(with_paths(place, &with));
    let chain = chain_of(place, &artifact)?;
    let mut args: Vec<&str> = vec!["publish", place.pipeline.as_str(), artifact.as_str(), "--file", file.as_str()];
    if let Some(chain) = &chain {
        args.extend(["--chain", chain.as_str()]);
    }
    if let Some(request) = &request {
        args.extend(["--request", request.as_str()]);
    }
    for (source, role) in &made_from {
        args.extend(["--from", source.as_str(), role.as_str()]);
    }
    for together in &with {
        args.extend(["--with", together.as_str()]);
    }
    place.script("pipeline.py", &args)?;
    // The chain pages say what is canon now, as the method's next step does.
    let _ = place.script("pipeline.py", &["render", &place.pipeline]);
    let version = versions(place, &artifact).ok().and_then(|versions| version_of(&versions, &file)).unwrap_or(card.view.version);
    if commit {
        let mut existing: Vec<&str> = paths.iter().map(String::as_str).filter(|path| place.folder.join(path).exists()).collect();
        existing.dedup();
        let mut add = vec!["add", "--"];
        add.extend(&existing);
        git(&place.folder, &add).map_err(|error| format!("Published, but not committed: {error}"))?;
        // The message names the run log's earlier lines the commit takes.
        let mut message = format!("Publish {artifact}, version {version}.");
        if !card.view.ride_along.is_empty() {
            message.push_str("\n\nAlso in the run log:\n");
            for line in &card.view.ride_along {
                message.push_str(&format!("- {line}\n"));
            }
        }
        let mut committing = vec!["commit", "-m", message.as_str(), "--"];
        committing.extend(&existing);
        git(&place.folder, &committing).map_err(|error| format!("Published, but not committed: {error}"))?;
    }
    Ok((file, Some(version)))
}

/// The take's number in its round, as the filmstrip counts it: its place
/// among its report's outputs.
fn take_number(place: &Place, path: &str) -> Option<usize> {
    let shown: Value = serde_json::from_str(&place.script("request.py", &["show", path]).ok()?).ok()?;
    let outputs = shown.get("outputs").and_then(Value::as_array)?;
    outputs.iter().position(|output| output.as_str() == Some(path)).map(|index| index + 1)
}

/// The project's assets, read through Skills' own scripts.
fn assets_of(place: &Place) -> Result<Vec<crate::assets::Asset>, String> {
    crate::assets::read_assets(&place.scripts, &place.folder, &place.vars, &place.pipeline)
}

/// The asset whose take, or one of whose iterations, is at `path`.
fn asset_at(assets: Vec<crate::assets::Asset>, path: &str) -> Result<crate::assets::Asset, String> {
    assets.into_iter().find(|asset| asset.files().any(|file| file == path)).ok_or_else(|| String::from(NO_ASSET))
}

/// Ignore: the take's artifact kept as it is, with Skills' own `keep`, so
/// Claude Code reads it as kept too.
fn ignore_take(place: &Place, path: &str) -> Result<(), String> {
    let asset = asset_at(assets_of(place)?, path)?;
    let Some(artifact) = asset.artifact.filter(|_| asset.in_use) else { return Ok(()) };
    place.script("pipeline.py", &["keep", &place.pipeline, &artifact, "--reason", protocol::IGNORED_REASON]).map(|_| ())
}

/// Use this take: the asset of the take at `path`, at its latest
/// iteration, published as its own step's artifact, beside the cell's
/// assets already in use. A take whose step's
/// artifact holds another take already gets a step of its own at the same
/// place, the next alternative, as Generate again makes one. Returns the
/// artifact.
fn use_take(place: &Place, app: usize, tab: &str, cell_step: Option<protocol::StepRef>, path: &str) -> Result<String, String> {
    let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline)?;
    // The step its report names, or else the cell's.
    let record = request_of(place, path).and_then(|request| place.script("request.py", &["show", &request]).ok()).and_then(|shown| serde_json::from_str::<Value>(&shown).ok());
    let named = record.as_ref().and_then(|record| Some(protocol::StepRef { chain: record.get("chain")?.as_str()?.to_string(), step: record.get("step")?.as_str()?.to_string() }));
    let step = named.or(cell_step).ok_or_else(|| String::from(IN_NO_RECIPE))?;
    let artifact = canon.step_artifacts.get(&(step.chain.clone(), step.step.clone())).cloned().ok_or_else(|| String::from(NO_ARTIFACT))?;
    // The asset is put in use at its latest iteration, canon as the guide
    // has it; one already in use stays as it is.
    let asset = asset_at(assets_of(place)?, path)?;
    if let Some(in_use) = asset.artifact.clone().filter(|_| asset.in_use) {
        return Ok(in_use);
    }
    let latest = asset.canonical().to_string();
    let publish = |artifact: &str| {
        let asked = Asked::Publish { asker: super::publishing::Asker::Workspace { app, tab: tab.to_string() }, artifact: artifact.to_string(), source: latest.clone(), from: Vec::new(), with: Vec::new(), sources: Vec::new() };
        let view = prepare(place, &asked)?;
        carry(place, &Card { asked, view }, false).map(|_| ())
    };
    // Its own step's artifact, while that holds nothing yet and the step is
    // still as the take was made.
    let holds = canon.files.get(&artifact).filter(|file| canon.published.contains(*file));
    if holds.is_none() {
        match publish(&artifact) {
            Ok(()) => return Ok(artifact),
            Err(error) if !error.contains(STEP_EDITED) => return Err(error),
            Err(_) => {}
        }
    }
    // Else an asset of its own beside those in use, the next alternative at
    // the step's place. The take is published first, with no step yet, so a
    // refusal leaves nothing behind; its step is then written as the take
    // was made, from its own record.
    let order = canon.step_orders.get(&(step.chain.clone(), step.step.clone())).copied().ok_or("The step has no place in its chain.")?;
    let base = crate::prompt_page::alternative_of(&step.step).0.to_string();
    let first = crate::prompt_page::alternative_of(&artifact).0.to_string();
    let n = (2..).find(|n| !canon.step_orders.contains_key(&(step.chain.clone(), crate::prompt_page::alternative_name(&base, *n))) && !canon.files.contains_key(&crate::prompt_page::alternative_name(&first, *n))).unwrap_or(2);
    let (name, alternative) = (crate::prompt_page::alternative_name(&base, n), crate::prompt_page::alternative_name(&first, n));
    publish(&alternative)?;
    let made = record.or_else(|| request_of(place, &asset.first).and_then(|request| place.script("request.py", &["show", &request]).ok()).and_then(|shown| serde_json::from_str::<Value>(&shown).ok()));
    step_as_made(place, &canon, &step, &name, order, &alternative, made.as_ref())?;
    Ok(alternative)
}

/// What Skills says when a take's step was edited after the take was made.
const STEP_EDITED: &str = "its step was edited since this take was made";

/// The alternative step `name` at `order`, holding the prompt, references,
/// and settings its take was made with, as its request record keeps them,
/// so Skills reads the take as its step's own. A reference whose artifact
/// still holds the file it sent names the artifact; one published again
/// since names the file sent. With no record, the step is copied as it is.
fn step_as_made(place: &Place, canon: &crate::prompt_page::Canon, step: &protocol::StepRef, name: &str, order: i64, artifact: &str, record: Option<&Value>) -> Result<(), String> {
    let read: Value = serde_json::from_str(&place.script("pipeline.py", &["step", &place.pipeline, &step.chain, &step.step, "--show"])?).map_err(|error| format!("The step could not be read: {error}"))?;
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let (template, refs, endpoint, skill) = match record {
        Some(record) => {
            let refs: Vec<Value> = record
                .get("references")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|reference| {
                    let (file, canon_name) = (text(reference, "file"), text(reference, "canon"));
                    let mut made = if !canon_name.is_empty() && canon.files.get(&canon_name) == Some(&file) { serde_json::json!({ "artifact": canon_name }) } else { serde_json::json!({ "file": file }) };
                    for key in ["role", "field"] {
                        if !text(reference, key).is_empty() {
                            made[key] = serde_json::json!(text(reference, key));
                        }
                    }
                    made
                })
                .collect();
            let endpoint = record.get("endpoint").filter(|endpoint| endpoint.is_object()).map(Value::to_string).unwrap_or_else(|| text(&read, "endpoint"));
            let skill = if text(record, "skill").is_empty() { text(&read, "skill") } else { text(record, "skill") };
            (text(record, "prompt"), serde_json::to_string(&refs).unwrap_or_default(), endpoint, skill)
        }
        None => (text(&read, "template"), text(&read, "refs"), text(&read, "endpoint"), text(&read, "skill")),
    };
    let template = if template.starts_with('@') { format!("@{}", crate::prompt_page::write_template(&place.folder, &template)?) } else { template };
    let order = order.to_string();
    let mut args: Vec<&str> = vec!["step", &place.pipeline, &step.chain, name, "--order", &order, "--artifact", artifact];
    for (flag, value) in [("--skill", &skill), ("--endpoint", &endpoint), ("--template", &template), ("--refs", &refs)] {
        if !value.is_empty() {
            args.extend([flag, value.as_str()]);
        }
    }
    place.script("pipeline.py", &args).map(|_| ())
}

/// A refusal from Skills' scripts as the user reads it: one of Studio's own
/// sentences as it is, Skills' known refusals as sentences, and anything
/// else as `otherwise`, never the script's own words.
fn plain_refusal(error: &str, otherwise: &str) -> String {
    let own = [IN_NO_RECIPE, NO_ARTIFACT, NO_ASSET, NOT_CANON, NO_MIX_RECORD, "This project has no pipeline yet."];
    if own.contains(&error) {
        return error.to_string();
    }
    let known = [
        (STEP_EDITED, TAKE_STEP_CHANGED),
        ("has been published again since this take was made", TAKE_INPUT_CHANGED),
        ("that was never published", ITERATION_NEVER_IN_USE),
        ("has no record of what made it", NO_RECORD),
        ("its record does not hold", NO_RECORD),
    ];
    known.iter().find(|(said, _)| error.contains(said)).map_or_else(|| otherwise.to_string(), |(_, sentence)| sentence.to_string())
}

const TAKE_STEP_CHANGED: &str = "This take's step was changed after it was made, so it can't be put in use as it is.";
const TAKE_INPUT_CHANGED: &str = "An image this take was made from has changed since, so it can't be put in use as it is.";
const ITERATION_NEVER_IN_USE: &str = "This was made from a version that was never in use, so it can't be saved as its next iteration.";
const NO_RECORD: &str = "Nothing records what this was made from, so it can't be put in use.";
const NOT_USED: &str = "The take could not be put in use.";
const NOT_SAVED: &str = "The iteration could not be saved.";
const NOT_IGNORED: &str = "The take's outline could not be cleared.";
const NOT_DELIVERED: &str = "The film could not be delivered.";
const NO_MIX_RECORD: &str = "This film has no record of what it was mixed from, so it cannot be delivered.";

/// Whether a Save on the asset at `path` waits on the warning: the asset is
/// in use and other work in use was made from it. Returns the warning's
/// words when it does.
fn iteration_warning(place: &Place, path: &str) -> Result<Option<(String, String)>, String> {
    let assets = assets_of(place)?;
    let asset = asset_at(assets.clone(), path)?;
    let Some(artifact) = asset.artifact.clone().filter(|_| asset.in_use) else { return Ok(None) };
    let reached: Value = serde_json::from_str(&place.script("pipeline.py", &["touches", &place.pipeline, "--artifact", &artifact, "--json"])?).map_err(|error| format!("What uses it could not be read: {error}"))?;
    let reached: Vec<&str> = reached.get("artifacts").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter(|reached| *reached != artifact).collect();
    let made: Vec<&crate::assets::Asset> = assets.iter().filter(|other| other.in_use && other.artifact.as_deref().is_some_and(|other| reached.contains(&other))).collect();
    if made.is_empty() {
        return Ok(None);
    }
    // Each cell once, by its chain and place, named as its recipe shows it.
    let titles: HashMap<String, String> = place.rows("SELECT name, title FROM chain")?.into_iter().filter_map(|row| Some((row.first()?.clone(), row.get(1).cloned().unwrap_or_default()))).collect();
    let steps: Vec<Vec<String>> = place.rows("SELECT chain, ord, name FROM step")?;
    let mut cells: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<(String, i64)> = Vec::new();
    for other in &made {
        let Some((chain, order)) = other.cell.clone() else {
            // A cell with no place in a chain, by its artifact's own name.
            if let Some(name) = other.artifact.as_deref().and_then(|artifact| artifact.split_once(": ").map(|(_, role)| crate::prompt_page::alternative_of(role).0.to_string())) {
                if !cells.iter().any(|(_, known)| *known == name) {
                    cells.push((String::new(), name));
                }
            }
            continue;
        };
        if seen.contains(&(chain.clone(), order)) {
            continue;
        }
        seen.push((chain.clone(), order));
        let step = steps.iter().filter(|row| row.first() == Some(&chain) && row.get(1).and_then(|at| at.trim().parse::<i64>().ok()) == Some(order)).filter_map(|row| row.get(2)).map(|name| crate::prompt_page::alternative_of(name).0.to_string()).min().unwrap_or_default();
        let title = titles.get(&chain).filter(|title| !title.trim().is_empty()).cloned().unwrap_or(chain);
        cells.push((title, step));
    }
    let take = take_number(place, &asset.first).map_or_else(|| String::from("This take"), |number| format!("Take {number}"));
    let warning = format!("{take} is in use as a dependency, and new iterations would break canonical consistency.");
    let cells = cell_names(&cells);
    let made_from = if made.len() == 1 { format!("1 take in {cells} was made from it.") } else { format!("{} takes in {cells} were made from it.", made.len()) };
    Ok(Some((warning, made_from)))
}

/// The cells a warning names, as `ig-dependency-warning` does: each by its
/// recipe's title, with its step only when the recipe has more than one of
/// them here, joined with commas and "and", the first three and then
/// "and N more". Each is `(recipe's title, step)`.
fn cell_names(cells: &[(String, String)]) -> String {
    let named: Vec<String> = cells
        .iter()
        .map(|(title, step)| match (title.is_empty(), cells.iter().filter(|(other, _)| other == title).count() > 1) {
            (true, _) => step.clone(),
            (false, true) if !step.is_empty() => format!("{title}: {step}"),
            (false, _) => title.clone(),
        })
        .collect();
    match named.as_slice() {
        [] => String::from("other cells"),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [first, second, third] => format!("{first}, {second} and {third}"),
        [first, second, third, rest @ ..] => format!("{first}, {second}, {third} and {} more", rest.len()),
    }
}

/// Save in a workspace: `source` filed as the next iteration of the asset at
/// `path`, from its latest iteration and with what that iteration was made
/// from, and, while the asset is in use, published as its artifact's next
/// version, naming the artifact itself, so Skills records it as an edit.
/// Studio commits nothing. Returns the file filed.
fn save_iteration(place: &Place, path: &str, source: &str) -> Result<Option<String>, String> {
    let asset = asset_at(assets_of(place)?, path)?;
    let latest = asset.canonical().to_string();
    let in_use = asset.artifact.clone().filter(|_| asset.in_use);
    let canon = crate::prompt_page::read_canon(&place.scripts, &place.folder, &place.vars, &place.pipeline)?;
    // What the latest iteration was made from, so the new one keeps it: its
    // version's sources while it is in use, or else what its own step and
    // its take's report name, so a later Use can publish it.
    let sources: Vec<(String, String)> = match (&in_use, &asset.artifact) {
        (Some(artifact), _) => version_sources(place, artifact, &latest).unwrap_or_default(),
        (None, Some(artifact)) => sources_of(place, artifact, request_of(place, &asset.first).as_deref(), &asset.first, &[]).unwrap_or_default(),
        (None, None) => Vec::new(),
    }
    .into_iter()
    .filter(|(name, _)| canon.files.contains_key(name))
    .collect();
    let stem = Path::new(source).file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_else(|| String::from("iteration"));
    let mut printed = Err(String::new());
    for n in 1..10 {
        let name = if n == 1 { stem.clone() } else { format!("{stem}-{n}") };
        let mut args: Vec<&str> = vec!["resource", source, "--name", name.as_str(), "--iterates", latest.as_str()];
        for (name, _) in &sources {
            args.extend(["--from", canon.files[name].as_str()]);
        }
        printed = place.script("request.py", &args);
        if !printed.as_ref().is_err_and(|error| error.contains("refusing to overwrite")) {
            break;
        }
    }
    let printed = printed.map_err(|error| format!("The iteration could not be filed: {error}"))?;
    let filed = printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).map(String::from).ok_or("The iteration was filed, but its new name was not given.")?;
    if let Some(artifact) = &in_use {
        let chain = chain_of(place, artifact)?.unwrap_or_default();
        let mut args: Vec<&str> = vec!["publish", place.pipeline.as_str(), artifact.as_str(), "--file", filed.as_str(), "--chain", chain.as_str(), "--from", artifact.as_str(), "iteration"];
        for (name, role) in &sources {
            args.extend(["--from", name.as_str(), role.as_str()]);
        }
        place.script("pipeline.py", &args)?;
    }
    Ok(Some(filed))
}

impl HostWindow {
    /// Where publishing runs: the open project's folder, scripts,
    /// and living pipeline.
    fn publish_place(&mut self) -> Option<Place> {
        let (folder, env): (PathBuf, ScriptEnv) = self.project_scripts()?;
        let pipeline = crate::prompt_page::living_pipeline(&folder)?;
        Some(Place { scripts: env.scripts?, vars: env.vars, folder, pipeline })
    }

    /// The user's answer on the Deliver card: a yes carries it out, a
    /// Cancel answers the workspace with no file and no error.
    pub(crate) fn answer_delivery(&mut self, yes: bool) {
        let Some(delivery) = self.publish_runs.deliver_card.take() else { return };
        if yes {
            self.publish_runs.delivering.push_back(delivery);
        } else {
            self.send(delivery.app, HostMessage::Delivered { tab: delivery.tab, file: String::new(), error: None });
        }
    }

    /// Workspaces' step runs, each a rerun of that one step through the
    /// plan card.
    fn take_step_runs(&mut self) {
        let mut runs = Vec::new();
        for (app, started) in self.apps.iter_mut().enumerate() {
            runs.extend(started.running.declared.step_runs.drain(..).map(|run| (app, run)));
        }
        for (app, (id, _tab, step)) in runs {
            let Some(place) = self.publish_place() else {
                self.send(app, HostMessage::StepRan { id, take: None, error: Some(String::from("This project has no pipeline yet, so it has no step to run.")) });
                continue;
            };
            let Some((folder, env)) = self.project_scripts() else { continue };
            let input = serde_json::json!({ "pipeline": place.pipeline, "chain": step.chain, "step": step.step });
            if let crate::runner::Begun::Answered(answer) = self.runner.begin(&folder, env, &step_call(app, &id), "", crate::runner::RERUN_STEP, &input) {
                self.answer_step_run(answer);
            }
        }
    }

    /// The runner's answer to a workspace's step run, sent to it. Returns
    /// false when the answer is not a step run's.
    pub(super) fn answer_step_run(&mut self, answer: protocol::ToolResult) -> bool {
        let Some((app, id)) = answer.call.strip_prefix("steprun:").and_then(|rest| rest.split_once(':')) else {
            return false;
        };
        let Ok(app) = app.parse::<usize>() else { return false };
        let printed = answer.output.get("output").and_then(Value::as_str).unwrap_or("");
        let (take, error) = if answer.ok {
            step_take(printed)
        } else if answer.output.get("declined").and_then(Value::as_bool) == Some(true) {
            (None, Some(String::from("The user did not approve it, so nothing ran.")))
        } else {
            (None, Some(answer.output.get("error").and_then(Value::as_str).unwrap_or(printed).to_string()))
        };
        // Canon may have changed, and the take is new: the Prompt page reads
        // them again.
        self.prompt_page.forget();
        self.send(app, HostMessage::StepRan { id: id.to_string(), take, error });
        true
    }

    /// Called each frame: a job's answer when it ends, and the next job when
    /// none runs and no card waits for the user.
    /// Takes each workspace's asks of its takes into the queue, each with
    /// its cell's step, and runs the next when none is running.
    fn run_take_asks(&mut self) {
        let mut asked = Vec::new();
        for (app, started) in self.apps.iter_mut().enumerate() {
            asked.extend(started.running.declared.take_asks.drain(..).map(|ask| (app, ask)));
        }
        for (app, (tab, cell, path, ask)) in asked {
            let id = cell.item.clone().unwrap_or_default();
            let step = self.panel_cell(app, &tab, &id).and_then(|cell| cell.step);
            self.publish_runs.take_queue.push_back((app, tab, id, step, path, ask));
        }
        if let Some(taking) = &self.publish_runs.taking {
            match taking.try_recv() {
                Ok(done) => {
                    self.publish_runs.taking = None;
                    self.take_done(done);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.publish_runs.taking = None,
            }
        }
        if self.publish_runs.iteration_warning.is_some() {
            return;
        }
        let Some((app, tab, cell, step, path, ask)) = self.publish_runs.take_queue.pop_front() else { return };
        let Some(place) = self.publish_place() else {
            let error = Some(String::from("This project has no pipeline yet."));
            self.send(app, match ask {
                crate::process::TakeAsk::Use => HostMessage::TakeUsed { tab, path, artifact: None, error },
                crate::process::TakeAsk::SaveIteration { .. } => HostMessage::IterationSaved { tab, path, file: None, error },
                crate::process::TakeAsk::Ignore => HostMessage::TakeIgnored { tab, path, error },
            });
            return;
        };
        self.start_take(place, app, tab, cell, step, path, ask, false);
    }

    /// Runs one take ask on its own thread. A Save already past its warning
    /// is `warned`.
    #[allow(clippy::too_many_arguments)]
    fn start_take(&mut self, place: Place, app: usize, tab: String, cell: String, step: Option<protocol::StepRef>, path: String, ask: crate::process::TakeAsk, warned: bool) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let done = match ask {
                crate::process::TakeAsk::Use => {
                    let result = use_take(&place, app, &tab, step, &path);
                    TakeDone::Used { app, tab, cell, path, result }
                }
                crate::process::TakeAsk::Ignore => {
                    let result = ignore_take(&place, &path);
                    TakeDone::Ignored { app, tab, cell, path, result }
                }
                crate::process::TakeAsk::SaveIteration { source } => match (warned, iteration_warning(&place, &path)) {
                    (false, Ok(Some((warning, made_from)))) => TakeDone::Warn(IterationAsk { app, tab, cell, path, source, warning, made_from }),
                    (false, Err(error)) => TakeDone::Saved { app, tab, cell, path, result: Err(error) },
                    _ => {
                        let result = save_iteration(&place, &path, &source);
                        TakeDone::Saved { app, tab, cell, path, result }
                    }
                },
            };
            let _ = sender.send(done);
            ctx.request_repaint();
        });
        self.publish_runs.taking = Some(receiver);
    }

    /// A take ask's answer to its workspace, then the cell's takes as they
    /// now stand.
    fn take_done(&mut self, done: TakeDone) {
        let cell = match done {
            TakeDone::Used { app, tab, cell, path, result } => {
                let (artifact, error) = match result {
                    Ok(artifact) => (Some(artifact), None),
                    Err(error) => (None, Some(plain_refusal(&error, NOT_USED))),
                };
                self.send(app, HostMessage::TakeUsed { tab, path, artifact, error });
                cell
            }
            TakeDone::Saved { app, tab, cell, path, result } => {
                let (file, error) = match result {
                    Ok(file) => (file, None),
                    Err(error) => (None, Some(plain_refusal(&error, NOT_SAVED))),
                };
                self.send(app, HostMessage::IterationSaved { tab, path, file, error });
                cell
            }
            TakeDone::Ignored { app, tab, cell, path, result } => {
                self.send(app, HostMessage::TakeIgnored { tab, path, error: result.err().map(|error| plain_refusal(&error, NOT_IGNORED)) });
                cell
            }
            TakeDone::Warn(ask) => {
                self.publish_runs.iteration_warning = Some(ask);
                return;
            }
        };
        self.send_rounds(&cell);
    }

    /// The warning before a Save on an asset other work was made from, as
    /// `ig-dependency-warning` shows it: Save iteration goes ahead, Cancel
    /// answers with no file and no error.
    pub(super) fn show_iteration_warning(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.publish_runs.iteration_warning.clone() else { return };
        let Some(answer) = crate::plan_card::ask(ctx, SAVE_ITERATION_TITLE, &[ask.warning.clone(), ask.made_from.clone()], SAVE_ITERATION) else { return };
        self.publish_runs.iteration_warning = None;
        if !answer {
            self.send(ask.app, HostMessage::IterationSaved { tab: ask.tab, path: ask.path, file: None, error: None });
            return;
        }
        match self.publish_place() {
            Some(place) => {
                let IterationAsk { app, tab, cell, path, source, .. } = ask;
                self.start_take(place, app, tab, cell, None, path, crate::process::TakeAsk::SaveIteration { source }, true);
            }
            None => self.send(ask.app, HostMessage::IterationSaved { tab: ask.tab, path: ask.path, file: None, error: Some(String::from("This project has no pipeline yet.")) }),
        }
    }

    pub(super) fn run_publishing(&mut self) {
        self.take_step_runs();
        self.run_take_asks();
        if let Some(working) = &self.publish_runs.working {
            match working.try_recv() {
                Ok(done) => {
                    self.publish_runs.working = None;
                    self.publish_done(done);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.publish_runs.working = None,
            }
        }
        let card_open = self.publishing.card.is_some() || self.publish_runs.deliver_card.is_some();
        let nothing = self.publishing.confirmed.is_empty() && self.publishing.asked.is_empty() && self.publishing.delivering.is_empty() && self.publish_runs.delivering.is_empty();
        if card_open || nothing {
            return;
        }
        let Some(place) = self.publish_place() else {
            // With no pipeline nothing can be published or listed: each
            // asker hears so at once.
            let sentence = String::from("This project has no pipeline yet.");
            for (card, _) in std::mem::take(&mut self.publishing.confirmed) {
                self.publish_done(Done::Published(card.asked, Err(sentence.clone())));
            }
            for asked in std::mem::take(&mut self.publishing.asked) {
                let done = match asked {
                    asked @ Asked::History { .. } => Done::History(asked, Err(sentence.clone())),
                    asked => Done::Card(asked, Err(sentence.clone())),
                };
                self.publish_done(done);
            }
            let deliveries: Vec<Delivery> = std::mem::take(&mut self.publishing.delivering).into_iter().chain(std::mem::take(&mut self.publish_runs.delivering)).collect();
            for delivery in deliveries {
                self.publish_done(Done::Delivered(delivery, Err(sentence.clone())));
            }
            return;
        };
        let job: Box<dyn FnOnce(&Place) -> Done + Send> = if let Some(delivery) = self.publish_runs.delivering.pop_front() {
            Box::new(move |place| {
                let outcome = deliver(place, &delivery);
                Done::Delivered(delivery, outcome)
            })
        } else if let Some(delivery) = self.publishing.delivering.pop_front() {
            Box::new(move |place| {
                let checked = check_delivery(place, &delivery);
                Done::Checked(delivery, checked)
            })
        } else if let Some((card, commit)) = self.publishing.confirmed.pop_front() {
            Box::new(move |place| {
                let outcome = carry(place, &card, commit);
                Done::Published(card.asked, outcome)
            })
        } else if let Some(asked) = self.publishing.asked.pop_front() {
            Box::new(move |place| match &asked {
                Asked::History { artifact, .. } => {
                    let versions = versions(place, artifact);
                    Done::History(asked, versions)
                }
                _ => {
                    let card = prepare(place, &asked);
                    Done::Card(asked, card)
                }
            })
        } else {
            return;
        };
        let (send, receive) = channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(job(&place));
            ctx.request_repaint();
        });
        self.publish_runs.working = Some(receive);
    }

    /// A job's answer: a card to show, a refusal, the history, or what was
    /// published.
    fn publish_done(&mut self, done: Done) {
        match done {
            Done::Checked(delivery, Ok(())) => self.publish_runs.deliver_card = Some(delivery),
            Done::Checked(delivery, Err(sentence)) | Done::Delivered(delivery, Err(sentence)) => {
                self.send(delivery.app, HostMessage::Delivered { tab: delivery.tab, file: String::new(), error: Some(plain_refusal(&sentence, NOT_DELIVERED)) });
            }
            Done::Delivered(delivery, Ok(output)) => {
                // Canon changed: the Prompt page reads it again.
                self.prompt_page.forget();
                self.send(delivery.app, HostMessage::Delivered { tab: delivery.tab, file: output, error: None });
            }
            Done::Card(asked, Ok(view)) => self.publishing.card = Some(Card { asked, view }),
            Done::Card(asked, Err(sentence)) | Done::Published(asked, Err(sentence)) => {
                if let Asked::Publish { asker, artifact, source: file, .. } | Asked::Revert { asker, artifact, file } = asked {
                    self.answer_published(asker, artifact, file, None, Some(sentence));
                }
            }
            Done::Published(asked, Ok((file, version))) => {
                if let Asked::Publish { asker, artifact, .. } | Asked::Revert { asker, artifact, .. } = asked {
                    // Canon changed: the Prompt page reads it again.
                    self.prompt_page.forget();
                    self.answer_published(asker, artifact, file, version, None);
                }
            }
            Done::History(asked, versions) => {
                if let Asked::History { app, tab, artifact } = asked {
                    match versions {
                        Ok(versions) => self.answer_history(app, tab, artifact, versions),
                        Err(sentence) => {
                            self.tell(sentence);
                            self.send(app, HostMessage::History { tab, artifact, versions: Vec::new() });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a mixed film delivered past an input that is no longer canon,
    /// and a delivery that leaves no file in output/ or does not make the
    /// film canon, with the real scripts. Runs when SPUMCAKE_SKILLS_SCRIPTS
    /// names the checkout's scripts folder.
    #[test]
    fn a_film_is_checked_published_and_delivered_with_the_real_scripts() {
        let Ok(scripts) = std::env::var("SPUMCAKE_SKILLS_SCRIPTS") else { return };
        let scripts = PathBuf::from(scripts);
        let folder = std::env::temp_dir().join(format!("deliver-runs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["library", "requests", "agent"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        let skills = |script: &str, args: &[&str]| {
            let out = Command::new("python3").arg(scripts.join(script)).args(args).current_dir(&folder).output().unwrap();
            assert!(out.status.success(), "{script} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        let last = |printed: String| printed.lines().last().unwrap().trim().to_string();
        skills("pipeline.py", &["new", "001"]);
        skills("pipeline.py", &["element", "001", "the line", "--kind", "story"]);
        skills("pipeline.py", &["chain", "001", "the line", "--order", "01"]);
        skills("pipeline.py", &["step", "001", "the line", "take", "--order", "1", "--skill", "generate-shot", "--artifact", "the line: take"]);
        skills("pipeline.py", &["step", "001", "the line", "final", "--order", "2", "--skill", "mix", "--refs", r#"[{"artifact": "the line: take", "role": "the shot"}]"#, "--artifact", "the line: final shot"]);
        std::fs::write(folder.join("agent/take.wav"), b"RIFF take").unwrap();
        let take = last(skills("request.py", &["resource", "agent/take.wav", "--name", "take"]));
        skills("pipeline.py", &["publish", "001", "the line: take", "--file", &take, "--chain", "the line"]);
        std::fs::write(folder.join("agent/mix.wav"), b"RIFF mix").unwrap();
        let mix = last(skills("request.py", &["resource", "agent/mix.wav", "--name", "mix", "--from", &take]));
        let place = Place { folder: folder.clone(), scripts: scripts.clone(), vars: Vec::new(), pipeline: String::from("001") };
        let delivery = |file: &str| Delivery { app: 0, tab: String::from("1"), artifact: String::from("the line: take"), file: file.into(), uses: String::from("Take 1 · with Voice"), size: String::from("1280 × 720"), frame_rate: String::from("24 fps"), format: String::from("H.264 with AAC sound") };
        check_delivery(&place, &delivery(&mix)).unwrap();
        let output = deliver(&place, &delivery(&mix)).unwrap();
        assert!(output.starts_with("output/") && folder.join(&output).is_file(), "{output}");
        // The mix is an asset of its own, made from the shot in the role Last
        // order records, and the shot in use is left as it was.
        let canon = crate::prompt_page::read_canon(&scripts, &folder, &[], "001").unwrap();
        assert_eq!(canon.files.get("the line: take, mixed"), Some(&mix), "the film is not an asset of its own");
        assert_eq!(canon.files.get("the line: take"), Some(&take), "the delivery replaced the shot in use");
        let role = skills("pipeline.py", &["sql", "001", "SELECT role FROM made_from WHERE artifact='the line: take, mixed' AND source='the line: take'", "--read-only"]);
        assert_eq!(role.trim(), format!("{MIX_SOURCE}, {UNTOUCHED}"), "the mix of a shot as it is did not name it untouched");
        // A mix of a take published again since is refused before its card.
        std::fs::write(folder.join("agent/take2.wav"), b"RIFF take again").unwrap();
        let take2 = last(skills("request.py", &["resource", "agent/take2.wav", "--name", "take-again"]));
        skills("pipeline.py", &["publish", "001", "the line: take", "--file", &take2, "--chain", "the line"]);
        std::fs::write(folder.join("agent/old-mix.wav"), b"RIFF old mix").unwrap();
        let old_mix = last(skills("request.py", &["resource", "agent/old-mix.wav", "--name", "old-mix", "--from", &take]));
        assert_eq!(check_delivery(&place, &delivery(&old_mix)), Err(String::from(NOT_CANON)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a commit made into a repository that only happens to hold the
    /// project: one that has never added it, or ignores it, is not its own.
    #[test]
    fn only_a_repository_that_keeps_the_project_is_committed_to() {
        let outer = std::env::temp_dir().join(format!("in-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outer);
        std::fs::create_dir_all(outer.join("proj")).unwrap();
        std::fs::write(outer.join("proj/brief.md"), "a brief").unwrap();
        let run = |args: &[&str]| assert!(Command::new("git").args(args).current_dir(&outer).output().unwrap().status.success(), "git {args:?}");
        run(&["init", "-q"]);
        assert!(!in_git(&outer.join("proj")), "a folder its repository never added was taken as kept in git");
        run(&["add", "proj/brief.md"]);
        assert!(in_git(&outer.join("proj")), "a tracked project was not taken as kept in git");
        let _ = std::fs::remove_dir_all(&outer);
    }

    /// Catches a step run answered with the wrong file, or with none when a
    /// request still waits at its endpoint.
    #[test]
    fn a_step_runs_take_is_its_last_lines_first_file() {
        let printed = "--- tell the user, word for word ---\n**r1**\n{\"chain\": \"the animatic\", \"step\": \"animatic\", \"files\": [\"prompts/resources/a.mp4\"], \"report\": null, \"pending\": null, \"stand_in\": false}\n";
        assert_eq!(step_take(printed), (Some(String::from("prompts/resources/a.mp4")), None));
        let (take, error) = step_take("{\"files\": [], \"pending\": \"r9\"}");
        assert!(take.is_none() && error.unwrap().contains("r9"));
        assert!(step_take("take: x.png").1.is_some());
    }

    /// Catches an artifact's name breaking out of its quotes in a read.
    #[test]
    fn an_artifact_is_quoted_whole() {
        assert_eq!(quoted("the cook's hands: sheet"), "'the cook''s hands: sheet'");
    }

    /// Catches a publish of a take made from inputs that are not canon run
    /// past its card, and a card whose version or ride-along is wrong,
    /// with the real scripts. Runs when SPUMCAKE_SKILLS_SCRIPTS names the
    /// checkout's scripts folder.
    #[test]
    fn a_publish_is_checked_carried_and_committed_with_the_real_scripts() {
        let Ok(scripts) = std::env::var("SPUMCAKE_SKILLS_SCRIPTS") else { return };
        let scripts = PathBuf::from(scripts);
        let folder = std::env::temp_dir().join(format!("publish-runs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["library", "requests", "work"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        let run = |program: &str, args: &[&str]| {
            let out = Command::new(program).args(args).current_dir(&folder).output().unwrap();
            assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        let skills = |script: &str, args: &[&str]| {
            let mut all = vec![scripts.join(script).display().to_string()];
            all.extend(args.iter().map(|arg| arg.to_string()));
            let all: Vec<&str> = all.iter().map(String::as_str).collect();
            run("python3", &all)
        };
        run("git", &["init", "-q"]);
        std::fs::write(folder.join("README.md"), "A scratch project.").unwrap();
        run("git", &["add", "README.md"]);
        run("git", &["-c", "user.name=Check", "-c", "user.email=check@example.org", "commit", "-q", "-m", "start"]);
        skills("pipeline.py", &["new", "001"]);
        skills("pipeline.py", &["element", "001", "the diner", "--kind", "place"]);
        skills("pipeline.py", &["chain", "001", "the diner", "--order", "01"]);
        skills("pipeline.py", &["step", "001", "the diner", "layout", "--order", "1", "--skill", "build-previs", "--artifact", "the diner: layout"]);
        std::fs::write(folder.join("work/layout.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(folder.join("work/Layout.ipx"), b"IPX1\n{}\n").unwrap();
        let place = Place { folder: folder.clone(), scripts: scripts.clone(), vars: Vec::new(), pipeline: String::from("001") };
        let asked = |from: Vec<String>| Asked::Publish { asker: Asker::Workspace { app: 0, tab: String::from("1") }, artifact: String::from("the diner: layout"), source: String::from("work/layout.png"), from, with: Vec::new(), sources: Vec::new() };
        assert_eq!(prepare(&place, &asked(vec![String::from("library/never-published.png")])), Err(String::from(NOT_CANON)));
        let view = prepare(&place, &asked(vec![String::from("work/Layout.ipx")])).unwrap();
        assert_eq!((view.version, view.commit, view.revert), (1, true, false));
        let card = Card { asked: asked(vec![String::from("work/Layout.ipx")]), view };
        // The commit is the user's own, by their git identity.
        give_git_identity(&folder);
        let (file, version) = carry(&place, &card, true).unwrap();
        assert!(file.starts_with("prompts/resources/") && version == Some(1), "{file} {version:?}");
        let canon = crate::prompt_page::read_canon(&scripts, &folder, &[], "001").unwrap();
        assert!(canon.published.contains(&file), "the filed file is not canon");
        assert!(run("git", &["status", "--porcelain", "--", "pipeline", file.as_str()]).trim().is_empty(), "the publish was not committed");
        assert_eq!(versions(&place, "the diner: layout").unwrap().len(), 1);

        // A take of a step that refers to the layout records
        // it, with its role, as publishing it by hand does.
        skills("pipeline.py", &["step", "001", "the diner", "keyframe", "--order", "2", "--skill", "generate-keyframe", "--endpoint", r#"{"kind": "fal", "model": "stand-in"}"#, "--template", "The diner.", "--refs", r#"[{"artifact": "the diner: layout", "role": "image 1"}]"#, "--artifact", "the diner: keyframe"]);
        std::fs::create_dir_all(folder.join("agent")).unwrap();
        std::fs::write(folder.join("agent/keyframe.json"), skills("pipeline.py", &["fill", "001", "the diner", "keyframe"])).unwrap();
        skills("request.py", &["run", "agent/keyframe.json", "--stand-in"]);
        let take = std::fs::read_dir(folder.join("library")).unwrap().flatten().map(|entry| format!("library/{}", entry.file_name().to_string_lossy())).next().unwrap();
        let publish_as = |artifact: &str, source: &str| {
            let asked = Asked::Publish { asker: Asker::Workspace { app: 0, tab: String::from("1") }, artifact: artifact.into(), source: source.into(), from: Vec::new(), with: Vec::new(), sources: Vec::new() };
            let view = prepare(&place, &asked).unwrap();
            carry(&place, &Card { asked, view }, false)
        };
        publish_as("the diner: keyframe", &take).unwrap();
        let recorded = skills("pipeline.py", &["sql", "001", "SELECT role FROM made_from WHERE artifact='the diner: keyframe' AND source='the diner: layout'", "--read-only"]);
        assert_eq!(recorded.trim(), "image 1", "the take's made_from was lost");

        // A take whose step lists nothing records the sources named with it,
        // as the skill has the agent pass it by hand.
        skills("pipeline.py", &["step", "001", "the diner", "sound bed", "--order", "4", "--skill", "generate-sound", "--artifact", "the diner: sound bed"]);
        std::fs::write(folder.join("agent/bed.wav"), b"RIFF").unwrap();
        let bed = skills("request.py", &["resource", "agent/bed.wav", "--name", "bed", "--from", &take]).lines().last().unwrap().trim().to_string();
        let bed_asked = Asked::Publish {
            asker: Asker::Workspace { app: 0, tab: String::from("1") },
            artifact: String::from("the diner: sound bed"),
            source: bed,
            from: Vec::new(),
            with: Vec::new(),
            sources: vec![(String::from("the diner: keyframe"), String::from("the beat it scores"))],
        };
        let view = prepare(&place, &bed_asked).unwrap();
        carry(&place, &Card { asked: bed_asked, view }, false).unwrap();
        let named = skills("pipeline.py", &["sql", "001", "SELECT role FROM made_from WHERE artifact='the diner: sound bed' AND source='the diner: keyframe'", "--read-only"]);
        assert_eq!(named.trim(), "the beat it scores", "a source named with it was lost");
        // A revert to the bed keeps the source named when it was published.
        let revert = Asked::Revert { asker: Asker::Workspace { app: 0, tab: String::from("1") }, artifact: String::from("the diner: sound bed"), file: skills("pipeline.py", &["sql", "001", "SELECT file FROM artifact WHERE id='the diner: sound bed'", "--read-only"]).trim().to_string() };
        let view = prepare(&place, &revert).unwrap();
        carry(&place, &Card { asked: revert, view }, false).unwrap();
        let kept = skills("pipeline.py", &["sql", "001", "SELECT role FROM made_from WHERE artifact='the diner: sound bed' AND source='the diner: keyframe'", "--read-only"]);
        assert_eq!(kept.trim(), "the beat it scores", "a revert dropped a source named at its publish");

        // A built take whose input has been published again since is
        // refused, as the method refuses it.
        skills("pipeline.py", &["step", "001", "the diner", "floor plan", "--order", "3", "--skill", "build-previs", "--refs", r#"[{"artifact": "the diner: layout", "role": "the room"}]"#, "--artifact", "the diner: floor plan"]);
        std::fs::write(folder.join("agent/plan.png"), b"\x89PNG\r\n\x1a\nplan").unwrap();
        let plan = skills("request.py", &["resource", "agent/plan.png", "--name", "plan", "--from", &file]).lines().last().unwrap().trim().to_string();
        std::fs::write(folder.join("work/layout.png"), b"\x89PNG\r\n\x1a\nagain").unwrap();
        let view = prepare(&place, &asked(vec![String::from("work/Layout.ipx")])).unwrap();
        assert_eq!(view.version, 2);
        carry(&place, &Card { asked: asked(vec![String::from("work/Layout.ipx")]), view }, false).unwrap();
        let refused = publish_as("the diner: floor plan", &plan);
        assert!(refused.as_ref().is_err_and(|error| error.contains("refused")), "a take made from an old input was published: {refused:?}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches the take answers drifting from Skills' own scripts: a take
    /// put in use not published as its step's artifact; a second take of
    /// the same step replacing the first rather than standing beside it; a
    /// Save on an asset other work was made from going ahead without the
    /// warning, or filed as anything but its next iteration; what was made
    /// from it not outlined after; and Ignore not clearing the outline.
    #[test]
    fn takes_are_used_iterated_and_ignored_with_the_real_scripts() {
        let Ok(scripts) = std::env::var("SPUMCAKE_SKILLS_SCRIPTS") else { return };
        let scripts = PathBuf::from(scripts);
        let folder = std::env::temp_dir().join(format!("take-runs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["library", "requests", "work", "agent"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        let skills = |script: &str, args: &[&str]| {
            let out = Command::new("python3").arg(scripts.join(script)).args(args).current_dir(&folder).output().unwrap();
            assert!(out.status.success(), "{script} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        skills("pipeline.py", &["new", "001"]);
        skills("pipeline.py", &["element", "001", "the diner", "--kind", "place"]);
        skills("pipeline.py", &["chain", "001", "the diner", "--order", "01", "--title", "Shot 12"]);
        skills("pipeline.py", &["step", "001", "the diner", "layout", "--order", "1", "--skill", "build-previs", "--artifact", "the diner: layout"]);
        skills("pipeline.py", &["step", "001", "the diner", "keyframe", "--order", "2", "--skill", "generate-keyframe", "--endpoint", r#"{"kind": "fal", "model": "stand-in"}"#, "--template", "The diner.", "--refs", r#"[{"artifact": "the diner: layout", "role": "image 1"}]"#, "--artifact", "the diner: keyframe"]);
        let place = Place { folder: folder.clone(), scripts: scripts.clone(), vars: Vec::new(), pipeline: String::from("001") };
        // The layout, filed and published from a workspace's file.
        std::fs::write(folder.join("work/layout.png"), b"\x89PNG\r\n\x1a\nlayout").unwrap();
        let asked = Asked::Publish { asker: Asker::Workspace { app: 0, tab: String::from("1") }, artifact: String::from("the diner: layout"), source: String::from("work/layout.png"), from: Vec::new(), with: Vec::new(), sources: Vec::new() };
        let view = prepare(&place, &asked).unwrap();
        let (layout, _) = carry(&place, &Card { asked, view }, false).unwrap();
        // Two takes of the keyframe's step, made with the stand-in.
        let take = |known: &[String]| {
            let name = format!("keyframe-{}", known.len() + 1);
            std::fs::write(folder.join("agent/keyframe.json"), skills("pipeline.py", &["fill", "001", "the diner", "keyframe", "--name", &name])).unwrap();
            skills("request.py", &["run", "agent/keyframe.json", "--stand-in"]);
            std::fs::read_dir(folder.join("library")).unwrap().flatten().map(|entry| format!("library/{}", entry.file_name().to_string_lossy())).find(|file| !known.contains(file)).unwrap()
        };
        let first = take(&[]);
        let step = protocol::StepRef { chain: String::from("the diner"), step: String::from("keyframe") };
        assert_eq!(use_take(&place, 0, "1", Some(step.clone()), &first), Ok(String::from("the diner: keyframe")));
        let second = take(std::slice::from_ref(&first));
        assert_eq!(use_take(&place, 0, "1", Some(step), &second), Ok(String::from("the diner: keyframe (2)")), "a second take replaced the first");
        let canon = crate::prompt_page::read_canon(&scripts, &folder, &[], "001").unwrap();
        assert_eq!(canon.files.get("the diner: keyframe"), Some(&first), "the first take left use");
        assert_eq!(canon.files.get("the diner: keyframe (2)"), Some(&second));
        // Skills reads the alternative as the keyframe's own, verified.
        let touches: Value = serde_json::from_str(&skills("pipeline.py", &["touches", "001", "--json"])).unwrap();
        let unverified: Vec<&str> = touches["unverified"].as_array().unwrap().iter().filter_map(|entry| entry["artifact"].as_str()).collect();
        assert!(!unverified.contains(&"the diner: keyframe (2)"), "the alternative reads as unverified: {touches}");
        // A Save on the layout, which both keyframes were made from.
        let (warning, made_from) = iteration_warning(&place, &layout).unwrap().expect("a Save on a dependency gave no warning");
        assert!(warning.ends_with("is in use as a dependency, and new iterations would break canonical consistency."), "{warning}");
        assert_eq!(made_from, "2 takes in Shot 12 were made from it.", "the two takes of one cell were not named once by their recipe's title");
        std::fs::write(folder.join("work/layout-edit.png"), b"\x89PNG\r\n\x1a\nedited").unwrap();
        let filed = save_iteration(&place, &layout, "work/layout-edit.png").unwrap().expect("nothing was filed");
        let assets = assets_of(&place).unwrap();
        let layout_asset = assets.iter().find(|asset| asset.first == layout).expect("the layout lost its asset");
        assert_eq!(layout_asset.canonical(), filed, "the Save was not the layout's next iteration");
        let history = skills("pipeline.py", &["history", "001", "the diner: layout", "--json"]);
        assert!(history.contains(&filed), "the iteration was not published as the layout's next version");
        let keyframe = assets.iter().find(|asset| asset.first == first).unwrap();
        assert!(keyframe.outlined, "what was made from the layout was not outlined: {keyframe:?}");
        // A third take, edited before it is used: Use puts in use its edit,
        // its latest iteration, never the take as first made; and Use again
        // on an asset in use makes no new step.
        let third = take(&[first.clone(), second.clone()]);
        std::fs::write(folder.join("work/third-edit.png"), b"\x89PNG\r\n\x1a\nthird edit").unwrap();
        let edit = save_iteration(&place, &third, "work/third-edit.png").unwrap().unwrap();
        let used = use_take(&place, 0, "1", Some(protocol::StepRef { chain: String::from("the diner"), step: String::from("keyframe") }), &third).unwrap();
        let canon = crate::prompt_page::read_canon(&scripts, &folder, &[], "001").unwrap();
        assert_eq!(canon.files.get(&used), Some(&edit), "Use left the take's edit out of canon");
        let steps = skills("pipeline.py", &["sql", "001", "SELECT count(*) FROM step WHERE chain='the diner'", "--read-only"]);
        assert_eq!(use_take(&place, 0, "1", None, &third), Ok(used.clone()));
        assert_eq!(skills("pipeline.py", &["sql", "001", "SELECT count(*) FROM step WHERE chain='the diner'", "--read-only"]), steps, "Use on an asset in use made a new step");
        // A take made before its step was edited goes into use as made: its
        // alternative step holds the prompt it was made with.
        let fourth = take(&[first.clone(), second.clone(), third.clone()]);
        skills("pipeline.py", &["step", "001", "the diner", "keyframe", "--template", "The diner at night."]);
        let made = use_take(&place, 0, "1", None, &fourth).expect("a take whose step was edited since was refused");
        let template = skills("pipeline.py", &["sql", "001", &format!("SELECT template FROM step WHERE artifact='{made}'"), "--read-only"]);
        assert_eq!(template.trim(), "The diner.", "the alternative step holds the step as edited, not as the take was made");
        let touches: Value = serde_json::from_str(&skills("pipeline.py", &["touches", "001", "--json"])).unwrap();
        let outlined: Vec<&str> = touches["out_of_date"].as_array().unwrap().iter().filter_map(|entry| entry["artifact"].as_str()).collect();
        assert!(!outlined.contains(&made.as_str()), "the alternative step reads as out of date: {touches}");
        // A take made from an image changed since is refused in plain words,
        // and leaves no step behind.
        let fifth = take(&[first.clone(), second.clone(), third.clone(), fourth.clone()]);
        std::fs::write(folder.join("work/layout-again.png"), b"\x89PNG\r\n\x1a\nagain").unwrap();
        save_iteration(&place, &layout, "work/layout-again.png").unwrap();
        let steps = skills("pipeline.py", &["sql", "001", "SELECT count(*) FROM step", "--read-only"]);
        let refused = use_take(&place, 0, "1", None, &fifth).expect_err("a take made from an image changed since was put in use");
        assert_eq!(plain_refusal(&refused, NOT_USED), TAKE_INPUT_CHANGED, "{refused}");
        assert_eq!(skills("pipeline.py", &["sql", "001", "SELECT count(*) FROM step", "--read-only"]), steps, "a refused Use left a step behind");
        // Ignore clears the first keyframe's outline, and not the second's.
        ignore_take(&place, &first).unwrap();
        let assets = assets_of(&place).unwrap();
        assert!(!assets.iter().find(|asset| asset.first == first).unwrap().outlined, "Ignore left the outline");
        assert!(assets.iter().find(|asset| asset.first == second).unwrap().outlined, "Ignore cleared another take's outline");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches the warning naming cells by their steps' lowercase names, as
    /// "start frame, from the pre-vis, final shot", rather than as their
    /// recipes show them, "Shot 12 and Shot 13", or listing them all.
    #[test]
    fn a_warning_names_cells_as_their_recipes_do() {
        let cell = |title: &str, step: &str| (title.to_string(), step.to_string());
        assert_eq!(cell_names(&[cell("Shot 12", "keyframe"), cell("Shot 13", "keyframe")]), "Shot 12 and Shot 13");
        assert_eq!(cell_names(&[cell("Shot 12", "keyframe"), cell("Shot 12", "final shot"), cell("Shot 13", "keyframe")]), "Shot 12: keyframe, Shot 12: final shot and Shot 13");
        assert_eq!(cell_names(&[cell("A", ""), cell("B", ""), cell("C", ""), cell("D", ""), cell("E", "")]), "A, B, C and 2 more");
        assert_eq!(cell_names(&[cell("", "the mix")]), "the mix");
    }

    /// Gives the scratch repository an identity of its own, so a commit
    /// never needs the computer's.
    fn give_git_identity(folder: &Path) {
        for (key, value) in [("user.name", "Check"), ("user.email", "check@example.org")] {
            assert!(Command::new("git").args(["config", key, value]).current_dir(folder).status().unwrap().success());
        }
    }
}
