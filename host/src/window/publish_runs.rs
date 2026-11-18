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
use protocol::{CanonVersion, HostMessage};
use serde_json::Value;
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

/// The publishing job running, if any, and the delivery the Deliver card
/// shows.
#[derive(Default)]
pub(crate) struct Runs {
    working: Option<Receiver<Done>>,
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
    let inputs = record_inputs(place, &delivery.file).ok_or_else(|| String::from("This film has no record of what it was mixed from, so it cannot be delivered."))?;
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
    let (artifact, file) = (delivery.artifact.as_str(), delivery.file.as_str());
    let made_from = sources_of(place, artifact, None, file, &[])?;
    let chain = chain_of(place, artifact)?;
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
    let output = printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).map(String::from).ok_or("The film was published, but where it was delivered was not given.")?;
    if in_git(&place.folder) {
        let paths = [format!("pipeline/{}", place.pipeline), file.to_string(), String::from("prompts/resources/records.jsonl"), output.clone()];
        let existing: Vec<&str> = paths.iter().map(String::as_str).filter(|path| place.folder.join(path).exists()).collect();
        let mut add = vec!["add", "--"];
        add.extend(&existing);
        git(&place.folder, &add).map_err(|error| format!("Delivered, but not committed: {error}"))?;
        let message = format!("Deliver {artifact}: {output}.");
        let mut committing = vec!["commit", "-m", message.as_str(), "--"];
        committing.extend(&existing);
        git(&place.folder, &committing).map_err(|error| format!("Delivered, but not committed: {error}"))?;
    }
    Ok(output)
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
    pub(super) fn run_publishing(&mut self) {
        self.take_step_runs();
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
                self.send(delivery.app, HostMessage::Delivered { tab: delivery.tab, file: String::new(), error: Some(sentence) });
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
        let delivery = |file: &str| Delivery { app: 0, tab: String::from("1"), artifact: String::from("the line: final shot"), file: file.into(), uses: String::from("Take 1 · with Voice"), size: String::from("1280 × 720"), frame_rate: String::from("24 fps"), format: String::from("H.264 with AAC sound") };
        check_delivery(&place, &delivery(&mix)).unwrap();
        let output = deliver(&place, &delivery(&mix)).unwrap();
        assert!(output.starts_with("output/") && folder.join(&output).is_file(), "{output}");
        let canon = crate::prompt_page::read_canon(&scripts, &folder, &[], "001").unwrap();
        assert!(canon.published.contains(&mix), "the film is not canon");
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

    /// Gives the scratch repository an identity of its own, so a commit
    /// never needs the computer's.
    fn give_git_identity(folder: &Path) {
        for (key, value) in [("user.name", "Check"), ("user.email", "check@example.org")] {
            assert!(Command::new("git").args(["config", key, value]).current_dir(folder).status().unwrap().success());
        }
    }
}
