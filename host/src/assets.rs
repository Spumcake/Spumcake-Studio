//! A project's takes grouped into reference assets, as the owner's data
//! model guide sets them out: each asset is a take and the iterations made
//! from it, its latest iteration canon; it belongs to the cell at a chain's
//! place whose step makes it, and is in use while it is that step's
//! artifact; and it is outlined while Spumcake-Skills finds it out of date,
//! or unverified while Skills has no record to compare. A cell's rounds and
//! the project panel's Library both read it.
//!
//! Its iterations are read the way the storyboard's timeline reads them, a
//! file filed with `request.py resource --iterates` and a version published
//! as an edit of the artifact itself or to fix it, until that reading moves
//! to `protocol/src/assets.rs`.

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

/// Each file's iterations, by the file it iterates, oldest first, each as
/// when it was filed or published, its file, and whether it was a fix: from
/// `prompts/resources/records.jsonl` and every pipeline's `runs.jsonl`.
fn iterations_of(folder: &Path) -> HashMap<String, Vec<(String, String, bool)>> {
    let lines = |path: &Path| std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).collect::<Vec<_>>();
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).map(String::from);
    let mut next: HashMap<String, Vec<(String, String, bool)>> = HashMap::new();
    let mut add = |of: String, file: String, at: String, fix: bool| {
        if of != file {
            next.entry(of).or_default().push((at, file, fix));
        }
    };
    for record in lines(&folder.join("prompts/resources/records.jsonl")) {
        let of = record.get("iterates").and_then(|iterates| text(iterates, "file").or_else(|| iterates.as_str().map(String::from)));
        if let (Some(of), Some(file)) = (of, text(&record, "file")) {
            add(of, file, text(&record, "at").unwrap_or_default(), false);
        }
    }
    for pipeline in std::fs::read_dir(folder.join("pipeline")).into_iter().flatten().flatten() {
        for event in lines(&pipeline.path().join("runs.jsonl")) {
            if text(&event, "event").as_deref() != Some("publish") {
                continue;
            }
            let Some(file) = text(&event, "file") else { continue };
            let at = text(&event, "at").unwrap_or_default();
            if let Some(of) = text(&event, "edited_from") {
                add(of, file.clone(), at.clone(), false);
            }
            if let Some(of) = text(&event, "fixes") {
                add(of, file.clone(), at, true);
            }
        }
    }
    for later in next.values_mut() {
        later.sort();
        // A file filed and then published as an iteration is one iteration,
        // a fix when either says so.
        let mut kept: Vec<(String, String, bool)> = Vec::new();
        for (at, file, fix) in later.drain(..) {
            match kept.iter_mut().find(|(_, known, _)| *known == file) {
                Some(known) => known.2 |= fix,
                None => kept.push((at, file, fix)),
            }
        }
        *later = kept;
    }
    next
}

/// The project's assets, from its reports, its records, and its canon,
/// with `touches`, `pipeline.py touches <n> --json` as printed, for what is
/// out of date and unverified. Takes appear in the order their reports
/// started, then the files only canon names.
pub(crate) fn assets(folder: &Path, canon: &Canon, touches: &Value) -> Vec<Asset> {
    let next = iterations_of(folder);
    let iterated: HashSet<&str> = next.values().flatten().map(|(_, file, _)| file.as_str()).collect();
    // Each take, with the step its report names.
    let mut takes: Vec<(String, String, Option<(String, String)>)> = Vec::new();
    for entry in std::fs::read_dir(folder.join("requests")).into_iter().flatten().flatten() {
        if entry.path().extension().and_then(|extension| extension.to_str()) != Some("html") {
            continue;
        }
        let Some(record) = std::fs::read_to_string(entry.path()).ok().as_deref().and_then(crate::skills_setup::record_of) else { continue };
        let text = |key: &str| record.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).map(String::from);
        let step = text("chain").zip(text("step"));
        for output in record.get("outputs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            // A file an asset's own iteration is never an asset of its own,
            // and a written document is no reference asset.
            if !iterated.contains(output) && is_reference(output) {
                takes.push((text("started").unwrap_or_default(), output.to_string(), step.clone()));
            }
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
                if !iterated.contains(file) && is_reference(file) {
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
    let mut edited: Vec<&String> = next.keys().filter(|file| is_reference(file) && !iterated.contains(file.as_str())).collect();
    edited.sort();
    for file in edited {
        if seen.insert(file.clone()) {
            takes.push((String::new(), file.clone(), None));
        }
    }
    // And so is a file filed with nothing it was made from, an import or an
    // export, that no report or step made and canon does not name: an asset
    // outside any cell. A file filed from others, such as a mix of its shot,
    // is that one's finishing, and a file nothing records stays out.
    let records = std::fs::read_to_string(folder.join("prompts/resources/records.jsonl")).unwrap_or_default();
    let mut filed: Vec<String> = records
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record.get("inputs").and_then(Value::as_array).is_none_or(Vec::is_empty))
        .filter_map(|record| record.get("file").and_then(Value::as_str).map(String::from))
        .filter(|file| is_reference(file) && !iterated.contains(file.as_str()) && !ran.contains(file))
        .collect();
    filed.sort();
    filed.dedup();
    for file in filed {
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
            while let Some((when, newer, fix)) = next.get(&at).and_then(|later| later.last()) {
                if !walked.insert(newer.clone()) {
                    break;
                }
                iterations.push(protocol::Iteration { file: newer.clone(), published: crate::prompt_page::epoch(when), fix: *fix });
                at = newer.clone();
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
        assert_eq!(firsts, ["library/bell.png", "library/bell-alt.png", "prompts/resources/previs-render.png", "prompts/resources/layout.png", "prompts/resources/animatic-export-1.mp4"], "an iteration became an asset, a document, a stray file, or a mix was taken for one, or a filed export or a script step's take was left out");
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
        let _ = std::fs::remove_dir_all(&folder);
    }
}
