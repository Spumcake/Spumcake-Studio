//! A project's takes grouped into reference assets, as the owner's data
//! model guide sets them out: each asset is a take and the iterations made
//! from it, its latest iteration canon; it belongs to the cell at a chain's
//! place whose step makes it, and is in use while it is that step's
//! artifact; and it is outlined while Spumcake-Skills finds it out of date,
//! or unverified while Skills has no record to compare. A cell's rounds and
//! the project panel's Library both read it.
//!
//! Its iterations are read with `protocol::Iterations`, the reader every
//! workspace shares: a file filed with `request.py resource --iterates`, and
//! a version published as an edit of the artifact itself or to fix it.

use crate::prompt_page::{Canon, media_type};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// One reference asset.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Asset {
    /// The take it started as, relative to the project.
    pub first: String,
    /// Its iterations after the first, oldest first, each with when it was
    /// published and whether it was a fix. The last, or the first when there
    /// are none, is canon.
    pub iterations: Vec<protocol::Iteration>,
    /// The chain and the place in it of the cell whose step makes it, or
    /// none for a take no step makes.
    pub cell: Option<(String, i64)>,
    /// The artifact of its own step: the step its report names, or the one
    /// whose artifact it is.
    pub artifact: Option<String>,
    /// Whether it is that artifact as published now.
    pub in_use: bool,
    /// Whether Skills finds that artifact out of date, so it is outlined.
    pub outlined: bool,
    /// Whether Skills has no record to compare that artifact against.
    pub unverified: bool,
}

impl Asset {
    /// Its canonical iteration: its latest.
    pub(crate) fn canonical(&self) -> &str {
        self.iterations.last().map_or(self.first.as_str(), |iteration| iteration.file.as_str())
    }

    /// Every file it holds, the first take first.
    pub(crate) fn files(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.first).chain(self.iterations.iter().map(|iteration| &iteration.file))
    }
}

/// Each file's iterations, read with the reader every workspace shares,
/// `protocol::Iterations`: from `prompts/resources/records.jsonl` and every
/// pipeline's `runs.jsonl`.
fn read_iterations(folder: &Path) -> protocol::Iterations {
    let records = std::fs::read_to_string(folder.join(protocol::RESOURCE_RECORDS)).unwrap_or_default();
    let mut logs: Vec<String> = Vec::new();
    for pipeline in std::fs::read_dir(folder.join("pipeline")).into_iter().flatten().flatten() {
        logs.push(std::fs::read_to_string(pipeline.path().join("runs.jsonl")).unwrap_or_default());
    }
    let logs: Vec<&str> = logs.iter().map(String::as_str).collect();
    protocol::Iterations::from_text(&records, &logs)
}

/// The project's assets, from its reports, its records, and its canon,
/// with `touches`, `pipeline.py touches <n> --json` as printed, for what is
/// out of date and unverified. Takes appear in the order their reports
/// started, then the files only canon names.
pub(crate) fn assets(folder: &Path, canon: &Canon, touches: &Value) -> Vec<Asset> {
    let next = read_iterations(folder);
    let iterated: HashSet<&str> = next.iterated().map(|(file, _)| file).collect();
    // Each take, with the step its report names; and every file a report
    // names as a reference it sent, an asset too, outside any cell. Research
    // files its reports beside its sources.
    let mut takes: Vec<(String, String, Option<(String, String)>)> = Vec::new();
    let mut named: HashSet<String> = HashSet::new();
    for reports in ["requests", "sources"] {
        for entry in std::fs::read_dir(folder.join(reports)).into_iter().flatten().flatten() {
            if entry.path().extension().and_then(|extension| extension.to_str()) != Some("html") {
                continue;
            }
            let Some(record) = std::fs::read_to_string(entry.path()).ok().as_deref().and_then(crate::skills_setup::record_of) else { continue };
            let text = |key: &str| record.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).map(String::from);
            let step = text("chain").zip(text("step"));
            for output in files_named(record.get("outputs").unwrap_or(&Value::Null)) {
                // A file an asset's own iteration is never an asset of its
                // own, a written document is no reference asset, and a lost
                // output, which the project no longer holds, shows nothing.
                if !iterated.contains(output.as_str()) && is_reference(&output) && held(folder, &output) {
                    takes.push((text("started").unwrap_or_default(), output, step.clone()));
                }
            }
            named.extend(files_named(record.get("references").unwrap_or(&Value::Null)));
        }
    }
    // A script step's take, such as a pre-vis render, is filed with a
    // `run` event naming its step: its step's own, as a request's take is.
    let mut ran: HashSet<String> = HashSet::new();
    for pipeline in std::fs::read_dir(folder.join("pipeline")).into_iter().flatten().flatten() {
        for event in std::fs::read_to_string(pipeline.path().join("runs.jsonl")).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
            if event.get("event").and_then(Value::as_str) != Some("run") {
                continue;
            }
            let text = |key: &str| event.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).map(String::from);
            let step = text("chain").zip(text("step"));
            for file in event.get("files").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                ran.insert(file.to_string());
                if !iterated.contains(file) && is_reference(file) && held(folder, file) {
                    takes.push((text("at").unwrap_or_default(), file.to_string(), step.clone()));
                }
            }
        }
    }
    takes.sort();
    takes.dedup_by(|later, earlier| later.1 == earlier.1);
    // A published file no report made, such as a filed resource, is an
    // asset too.
    let mut seen: HashSet<String> = takes.iter().map(|(_, file, _)| file.clone()).collect();
    let mut published: Vec<&String> = canon.files.values().filter(|file| is_reference(file) && !iterated.contains(file.as_str())).collect();
    published.sort();
    for file in published {
        if seen.insert(file.clone()) {
            takes.push((String::new(), file.clone(), None));
        }
    }
    // So is a file iterated since, such as a filed layout edited in its
    // workspace, though canon now names its iteration.
    let mut edited: Vec<&str> = next.with_iterations().filter(|file| is_reference(file) && !iterated.contains(file)).collect();
    edited.sort();
    for file in edited {
        if seen.insert(file.to_string()) {
            takes.push((String::new(), file.to_string(), None));
        }
    }
    // And so is every other file the project's records name, a filed
    // resource, what one was made from, or a reference a report sent, as
    // the plan has it since 2026-11-20: outside any cell. Only a file the
    // project holds counts, and a file nothing records stays out.
    let _ = &ran;
    for records in records_files(folder) {
        for line in std::fs::read_to_string(&records).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()) {
            named.extend(files_within(&line));
        }
    }
    let mut others: Vec<String> = named.into_iter().filter(|file| is_reference(file) && !iterated.contains(file.as_str()) && held(folder, file)).collect();
    others.sort();
    for file in others {
        if seen.insert(file.clone()) {
            takes.push((String::new(), file, None));
        }
    }
    let listed = |key: &str| -> HashSet<String> { touches.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(|entry| entry.get("artifact").and_then(Value::as_str).map(String::from)).collect() };
    let (out_of_date, unverified) = (listed("out_of_date"), listed("unverified"));
    let step_of_artifact: HashMap<&String, &(String, String)> = canon.step_artifacts.iter().map(|(step, artifact)| (artifact, step)).collect();
    takes
        .into_iter()
        .map(|(_, first, named)| {
            let mut iterations = Vec::new();
            let mut at = first.clone();
            let mut walked = HashSet::from([first.clone()]);
            while let Some(newer) = next.next(&at).last() {
                if !walked.insert(newer.file.clone()) {
                    break;
                }
                iterations.push(protocol::Iteration { file: newer.file.clone(), published: crate::prompt_page::epoch(&newer.at), fix: newer.fix });
                at = newer.file.clone();
            }
            let mut asset = Asset { first, iterations, ..Asset::default() };
            // In use: one of its files is an artifact's file as published now.
            let in_use = canon.files.iter().find(|(_, file)| asset.files().any(|own| own == *file)).map(|(artifact, _)| artifact.clone());
            let step = named.or_else(|| in_use.as_ref().and_then(|artifact| step_of_artifact.get(artifact).map(|step| (*step).clone())));
            asset.artifact = in_use.clone().or_else(|| step.as_ref().and_then(|step| canon.step_artifacts.get(step).cloned()));
            asset.in_use = in_use.is_some();
            asset.cell = step.as_ref().and_then(|step| canon.step_orders.get(step).map(|order| (step.0.clone(), *order)));
            if let Some(artifact) = asset.artifact.as_ref().filter(|_| asset.in_use) {
                asset.outlined = out_of_date.contains(artifact);
                asset.unverified = unverified.contains(artifact);
            }
            asset
        })
        .collect()
}

/// Every file a record names, at any depth: a path ending in a known
/// image, video, or sound extension.
fn files_named(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) if is_reference(text) => vec![text.clone()],
        Value::Array(values) => values.iter().flat_map(files_named).collect(),
        Value::Object(fields) => fields.values().flat_map(files_named).collect(),
        _ => Vec::new(),
    }
}

/// Every `records.jsonl` in the project, outside Studio's and the agent's
/// own folders and its working files.
/// Every file a record names, as `files_named`, and every path inside a
/// longer string, such as an export command a record keeps
/// (`--reference far-end-wall=prompts/resources/…door-swing.mp4`), as the
/// port's inventory reads them.
fn files_within(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => text
            .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '/' || c == '-'))
            .filter(|word| is_reference(word))
            .map(String::from)
            .collect(),
        Value::Array(values) => values.iter().flat_map(files_within).collect(),
        Value::Object(fields) => fields.values().flat_map(files_within).collect(),
        _ => Vec::new(),
    }
}

fn records_files(folder: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut waiting = vec![folder.to_path_buf()];
    while let Some(at) = waiting.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !(at == folder && ["agent", ".git", "work", ".spumcake", "target"].contains(&name.as_str())) {
                    waiting.push(path);
                }
            } else if name == "records.jsonl" {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Whether the project holds a file where Skills files media: in
/// `library/`, `prompts/resources/`, or `sources/`.
fn held(folder: &Path, file: &str) -> bool {
    ["library/", "prompts/resources/", "sources/"].iter().any(|place| file.starts_with(place)) && folder.join(file).is_file()
}

/// Whether a file is a reference asset's: an image, a video, or a sound.
fn is_reference(file: &str) -> bool {
    let media = media_type(file);
    media.starts_with("image/") || media.starts_with("video/") || media.starts_with("audio/")
}

/// The project's assets, read through Skills' own scripts: its canon, and
/// what `touches` finds out of date.
pub(crate) fn read_assets(scripts: &Path, folder: &Path, setup: &[(String, String)], pipeline: &str) -> Result<Vec<Asset>, String> {
    let canon = crate::prompt_page::read_canon(scripts, folder, setup, pipeline)?;
    let printed = crate::prompt_page::script(scripts, folder, setup, "pipeline.py", &["touches", pipeline, "--json"])?;
    let touches: Value = serde_json::from_str(&printed).map_err(|error| format!("What is out of date could not be read: {error}"))?;
    Ok(assets(folder, &canon, &touches))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Catches Library leaving out a file Last order's reports or records
    /// name, or showing one nothing records, as the plan's rule of
    /// 2026-11-20 and the port's inventory count them: 616 assets, a path
    /// inside a record's export command among them, and 3 stray files
    /// nothing records. It only reads: set `LAST_ORDER` and
    /// `SPUMCAKE_SKILLS_SCRIPTS`.
    #[test]
    fn last_orders_assets_are_every_file_its_records_name() {
        let (Some(project), Some(scripts)) = (std::env::var_os("LAST_ORDER"), std::env::var_os("SPUMCAKE_SKILLS_SCRIPTS")) else { return };
        let (project, scripts) = (std::path::PathBuf::from(project), std::path::PathBuf::from(scripts));
        let pipeline = crate::prompt_page::living_pipeline(&project).unwrap();
        let found = read_assets(&scripts, &project, &[], &pipeline).unwrap();
        let covered: HashSet<String> = found.iter().flat_map(|asset| asset.files().cloned().collect::<Vec<_>>()).collect();
        assert_eq!(covered.len(), 616, "Library covers {} files, where the port's inventory counts 616", covered.len());
        fn walk(at: &Path, root: &Path, into: &mut Vec<String>) {
            for entry in std::fs::read_dir(at).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, into);
                } else {
                    into.push(path.strip_prefix(root).unwrap().display().to_string());
                }
            }
        }
        let mut held_files = Vec::new();
        for folder in ["library", "prompts/resources", "sources"] {
            walk(&project.join(folder), &project, &mut held_files);
        }
        let mut stray: Vec<String> = held_files.into_iter().filter(|file| is_reference(file) && !covered.contains(file)).collect();
        stray.sort();
        assert_eq!(
            stray,
            [
                "prompts/resources/2026-11-12_21-22-last-order-animatic-4.mp4",
                "prompts/resources/2026-11-14_21-07-soundtrack-door-cue.wav",
                "prompts/resources/2026-11-14_21-16-final-shot-mixed-1.wav",
            ],
            "Library's strays are not the port's inventory's"
        );
    }

    /// Catches an edit or a fix shown as an asset of its own rather than an
    /// iteration, another take put in use counted as an iteration, a take
    /// in use not marked so, an outline on an asset not in use, and a
    /// written document taken for an asset.
    #[test]
    fn takes_are_grouped_into_assets_with_their_iterations() {
        let folder = std::env::temp_dir().join(format!("assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["requests", "prompts/resources", "pipeline/002"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        let report = |id: &str, started: &str, step: &str, outputs: Value| {
            let record = json!({ "id": id, "started": started, "chain": "the animatic", "step": step, "outputs": outputs });
            std::fs::write(folder.join(format!("requests/{id}.html")), format!("<script type=\"application/json\" id=\"request\">{record}</script>")).unwrap();
        };
        report("bell", "2026-11-19T09:00:00Z", "keyframe, the bell", json!(["library/bell.png"]));
        report("bell-2", "2026-11-19T09:30:00Z", "keyframe, the bell (2)", json!(["library/bell-alt.png"]));
        report("bell-fix", "2026-11-19T11:00:00Z", "keyframe, the bell", json!(["library/bell-3.png"]));
        report("notes", "2026-11-19T08:00:00Z", "story", json!(["brief/notes.md"]));
        std::fs::write(
            folder.join("prompts/resources/records.jsonl"),
            json!({ "at": "2026-11-19T10:00:00Z", "file": "prompts/resources/bell-edit.png", "iterates": { "file": "library/bell.png", "sha256": "a1" } }).to_string(),
        )
        .unwrap();
        std::fs::write(
            folder.join("pipeline/002/runs.jsonl"),
            [
                json!({ "event": "publish", "at": "2026-11-19T10:00:01Z", "artifact": "Last order: keyframe, the bell", "file": "prompts/resources/bell-edit.png", "edited_from": "library/bell.png" }),
                json!({ "event": "publish", "at": "2026-11-19T11:00:01Z", "artifact": "Last order: keyframe, the bell", "file": "library/bell-3.png", "fixes": "prompts/resources/bell-edit.png" }),
            ]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
        )
        .unwrap();
        // An earlier export, filed but neither made by a request nor canon,
        // and a stray file nothing records.
        let mut records = std::fs::read_to_string(folder.join("prompts/resources/records.jsonl")).unwrap();
        records.push('\n');
        records.push_str(&json!({ "at": "2026-11-19T12:00:00Z", "file": "prompts/resources/animatic-export-1.mp4", "inputs": [] }).to_string());
        // A pre-vis render a script step filed, and a mix filed from a take.
        for (file, input) in [("prompts/resources/previs-render.png", "prompts/resources/layout.png"), ("prompts/resources/shot-mixed.mp4", "library/bell.png")] {
            records.push('\n');
            records.push_str(&json!({ "at": "2026-11-19T12:30:00Z", "file": file, "inputs": [{ "file": input, "sha256": "x" }] }).to_string());
        }
        let mut log = std::fs::read_to_string(folder.join("pipeline/002/runs.jsonl")).unwrap();
        log.push('\n');
        log.push_str(&json!({ "event": "run", "at": "2026-11-19T12:30:00Z", "chain": "the animatic", "step": "pre-vis, the bell", "files": ["prompts/resources/previs-render.png"] }).to_string());
        std::fs::write(folder.join("pipeline/002/runs.jsonl"), log).unwrap();
        std::fs::write(folder.join("prompts/resources/records.jsonl"), records).unwrap();
        std::fs::write(folder.join("prompts/resources/stray.png"), b"stray").unwrap();
        for held in ["prompts/resources/animatic-export-1.mp4", "prompts/resources/shot-mixed.mp4", "prompts/resources/previs-render.png", "library/bell.png", "library/bell-alt.png"] {
            std::fs::create_dir_all(folder.join(held).parent().unwrap()).unwrap();
            std::fs::write(folder.join(held), b"held").unwrap();
        }
        // A report's output the project no longer holds shows nothing.
        let lost = json!({ "id": "lost", "started": "2026-11-19T07:00:00Z", "chain": "the animatic", "step": "keyframe, the bell", "outputs": ["library/lost.png"] });
        std::fs::write(folder.join("requests/lost.html"), format!("<script type=\"application/json\" id=\"request\">{lost}</script>")).unwrap();
        let mut canon = Canon::default();
        for (step, artifact, order) in [("keyframe, the bell", "Last order: keyframe, the bell", 3), ("keyframe, the bell (2)", "Last order: keyframe, the bell (2)", 3), ("pre-vis, the bell", "Last order: pre-vis, the bell", 2)] {
            let key = (String::from("the animatic"), step.to_string());
            canon.step_artifacts.insert(key.clone(), artifact.to_string());
            canon.step_orders.insert(key, order);
        }
        canon.files.insert(String::from("Last order: keyframe, the bell"), String::from("library/bell-3.png"));
        canon.files.insert(String::from("Last order: the diner, layout"), String::from("prompts/resources/layout.png"));
        let touches = json!({ "out_of_date": [{ "artifact": "Last order: keyframe, the bell", "why": ["x"] }, { "artifact": "Last order: keyframe, the bell (2)", "why": ["x"] }], "unverified": [] });
        let found = assets(&folder, &canon, &touches);
        let firsts: Vec<&str> = found.iter().map(|asset| asset.first.as_str()).collect();
        assert_eq!(firsts, ["library/bell.png", "library/bell-alt.png", "prompts/resources/previs-render.png", "prompts/resources/layout.png", "prompts/resources/animatic-export-1.mp4", "prompts/resources/shot-mixed.mp4"], "an iteration became an asset, a document or a stray file was taken for one, or a filed file, a script step's take, or a mix was left out");
        assert_eq!(found[2].cell, Some((String::from("the animatic"), 2)), "a script step's take was left outside its step's cell");
        let bell = &found[0];
        let files: Vec<&str> = bell.iterations.iter().map(|iteration| iteration.file.as_str()).collect();
        assert_eq!(files, ["prompts/resources/bell-edit.png", "library/bell-3.png"]);
        assert_eq!(bell.iterations.iter().map(|iteration| iteration.fix).collect::<Vec<_>>(), [false, true], "the fix was not marked as one");
        assert_eq!(bell.canonical(), "library/bell-3.png");
        assert!(bell.in_use && bell.outlined, "the bell's keyframe in use was not outlined: {bell:?}");
        assert_eq!(bell.cell, Some((String::from("the animatic"), 3)));
        let alternative = &found[1];
        assert!(!alternative.in_use && !alternative.outlined, "an asset not in use was outlined: {alternative:?}");
        assert_eq!(alternative.artifact.as_deref(), Some("Last order: keyframe, the bell (2)"));
        assert_eq!(alternative.cell, Some((String::from("the animatic"), 3)), "the second asset lost its cell");
        assert!(found[3].in_use && found[3].cell.is_none(), "a published resource no step makes was given a cell");
        assert!(!found[4].in_use && found[4].cell.is_none(), "a filed export was put in a cell, or in use");
        assert!(found[5].cell.is_none(), "a mix filed from a take was put in a cell");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Catches a file lost from the Library when the file filed as its
    /// iteration is later published to fix another.
    #[test]
    fn a_file_iterated_by_a_fix_of_another_stays_an_asset() {
        let folder = std::env::temp_dir().join(format!("assets-both-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for sub in ["prompts/resources", "pipeline/002", "library"] {
            std::fs::create_dir_all(folder.join(sub)).unwrap();
        }
        std::fs::write(
            folder.join("prompts/resources/records.jsonl"),
            json!({ "at": "2026-11-19T10:00:00Z", "file": "prompts/resources/F.png", "iterates": { "file": "library/A.png", "sha256": "a1" } }).to_string(),
        )
        .unwrap();
        std::fs::write(
            folder.join("pipeline/002/runs.jsonl"),
            json!({ "event": "publish", "at": "2026-11-19T11:00:00Z", "artifact": "Last order: keyframe, the bell", "file": "prompts/resources/F.png", "fixes": "library/B.png" }).to_string(),
        )
        .unwrap();
        for held in ["library/A.png", "library/B.png", "prompts/resources/F.png"] {
            std::fs::write(folder.join(held), b"held").unwrap();
        }
        let found = assets(&folder, &Canon::default(), &json!({}));
        let firsts: Vec<&str> = found.iter().map(|asset| asset.first.as_str()).collect();
        assert_eq!(firsts, ["library/A.png", "library/B.png"], "a file with iterations was left out, or its iteration became an asset");
        assert!(found.iter().all(|asset| asset.canonical() == "prompts/resources/F.png"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
