//! What a placed file's iterations are, read from the project's own plain files,
//! the same way for every file, in canon or not, as the 0.15 plan's rule
//! "The storyboard's timeline" sets out: a version published as an edit of
//! the artifact itself (`edited_from` in the run log) or made again to fix
//! it (`fixes`), and a file filed with `request.py resource` naming the file
//! it iterates. A file filed only `--from` others, such as a drawn hold over
//! a keyframe, is made from them and never their iteration; a different take
//! put in use for a step is never an iteration.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The field a filed file's record names the file it iterates in, as
/// `{"file", "sha256"}`: what Spumcake-Skills' `request.py resource
/// --iterates` writes.
pub const ITERATES: &str = "iterates";

/// Each file, by the files filed or published as its iterations, oldest
/// first, from `prompts/resources/records.jsonl` and every pipeline's
/// `runs.jsonl`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Iterations {
    next: HashMap<String, Vec<(String, String)>>,
}

impl Iterations {
    /// Reads the project's records. Lines that are not JSON, or name no
    /// iteration, are passed over.
    pub fn read(project: &Path) -> Self {
        let mut found = Self::default();
        let lines = |path: &Path| std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).collect::<Vec<_>>();
        for record in lines(&project.join("prompts/resources/records.jsonl")) {
            // The iterated file, as Skills writes it, or a plain path.
            let of = record.get(ITERATES).and_then(|iterates| text(iterates, "file").or_else(|| iterates.as_str()));
            if let (Some(of), Some(file)) = (of, text(&record, "file")) {
                found.add(of, file, text(&record, "at").unwrap_or_default());
            }
        }
        let pipelines = std::fs::read_dir(project.join("pipeline")).into_iter().flatten().flatten();
        for pipeline in pipelines {
            for event in lines(&pipeline.path().join("runs.jsonl")) {
                if text(&event, "event") != Some("publish") {
                    continue;
                }
                let Some(file) = text(&event, "file") else {
                    continue;
                };
                for of in [text(&event, "edited_from"), text(&event, "fixes")].into_iter().flatten() {
                    found.add(of, file, text(&event, "at").unwrap_or_default());
                }
            }
        }
        for later in found.next.values_mut() {
            later.sort();
            later.dedup();
        }
        found
    }

    fn add(&mut self, of: &str, file: &str, at: &str) {
        if of != file {
            self.next.entry(of.to_string()).or_default().push((at.to_string(), file.to_string()));
        }
    }

    /// The latest iteration of `file`: its newest iteration's latest, and
    /// so on, or `file` itself when it has none.
    pub fn latest(&self, file: &str) -> String {
        let mut at = file.to_string();
        let mut seen = HashSet::new();
        while seen.insert(at.clone()) {
            match self.next.get(&at).and_then(|later| later.last()) {
                Some((_, newer)) => at = newer.clone(),
                None => break,
            }
        }
        at
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterations_are_read_alike_in_canon_or_not() {
        let dir = std::env::temp_dir().join(format!("storyboard-iterations-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("prompts/resources")).unwrap();
        std::fs::create_dir_all(dir.join("pipeline/002")).unwrap();
        let records = [
            // A take no step uses, edited in its workspace twice.
            serde_json::json!({ "at": "2026-11-19T10:00:00Z", "file": "prompts/resources/take-2.png", "inputs": [{ "file": "library/take.png" }], ITERATES: { "file": "library/take.png", "sha256": "a1" } }),
            serde_json::json!({ "at": "2026-11-19T11:00:00Z", "file": "prompts/resources/take-3.png", "inputs": [{ "file": "prompts/resources/take-2.png" }], ITERATES: "prompts/resources/take-2.png" }),
            // A drawn hold over it: made from it, never its iteration.
            serde_json::json!({ "at": "2026-11-19T12:00:00Z", "file": "prompts/resources/shot-2-hold-1.png", "inputs": [{ "file": "prompts/resources/take-3.png" }] }),
        ];
        std::fs::write(dir.join("prompts/resources/records.jsonl"), records.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let events = [
            // The bell's keyframe edited, then fixed.
            serde_json::json!({ "event": "publish", "at": "2026-11-19T10:00:00Z", "artifact": "Last order: keyframe, the bell", "file": "prompts/resources/bell-2.png", "edited_from": "library/bell.png" }),
            serde_json::json!({ "event": "publish", "at": "2026-11-19T11:00:00Z", "artifact": "Last order: keyframe, the bell", "file": "library/bell-3.png", "fixes": "prompts/resources/bell-2.png" }),
            // Another take put in use: not an iteration.
            serde_json::json!({ "event": "publish", "at": "2026-11-19T12:00:00Z", "artifact": "Last order: keyframe, the line", "file": "library/line-take-4.png", "replaced": "library/line.png" }),
        ];
        std::fs::write(dir.join("pipeline/002/runs.jsonl"), events.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let found = Iterations::read(&dir);
        assert_eq!(found.latest("library/take.png"), "prompts/resources/take-3.png");
        assert_eq!(found.latest("prompts/resources/take-2.png"), "prompts/resources/take-3.png");
        assert_eq!(found.latest("library/bell.png"), "library/bell-3.png");
        assert_eq!(found.latest("library/line.png"), "library/line.png");
        assert_eq!(found.latest("prompts/resources/shot-2-hold-1.png"), "prompts/resources/shot-2-hold-1.png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
