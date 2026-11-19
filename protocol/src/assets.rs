//! A reference asset's iterations, read from a project's own plain files the
//! same way for every asset, in canon or not, as the data model guide and
//! the 0.15 plan's rule "The storyboard's timeline" set out:
//! - a version published as an edit of the artifact itself, `edited_from` on
//!   a `publish` event in a pipeline's `runs.jsonl`, or made again to fix it,
//!   `fixes`;
//! - a file filed with Spumcake-Skills' `request.py resource --iterates`,
//!   whose line in `prompts/resources/records.jsonl` names the file it
//!   iterates as `"iterates": {"file", "sha256"}`.
//!
//! A file filed only `--from` others, such as a drawn hold over a keyframe, is
//! made from them and never their iteration; another take put in use for a
//! step, a publish that only `replaced`, is another asset. Pure: the callers
//! read the files and pass their text, so a workspace and the host read
//! iterations alike.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// The field a filed file's record names the file it iterates in.
pub const ITERATES: &str = "iterates";

/// Where a project keeps the records of what it filed, relative to it.
pub const RESOURCE_RECORDS: &str = "prompts/resources/records.jsonl";

/// Each file, by the files filed or published as its iterations, oldest
/// first, and each iteration by the file it iterates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Iterations {
    next: HashMap<String, Vec<(String, String)>>,
    before: HashMap<String, String>,
}

impl Iterations {
    /// From the text of `records.jsonl` and of each pipeline's `runs.jsonl`.
    /// Lines that are not JSON, or name no iteration, are passed over.
    pub fn from_text(records: &str, run_logs: &[&str]) -> Self {
        let mut found = Self::default();
        for record in json_lines(records) {
            // The iterated file, as Skills writes it, or a plain path.
            let of = record.get(ITERATES).and_then(|iterates| text(iterates, "file").or_else(|| iterates.as_str()));
            if let (Some(of), Some(file)) = (of, text(&record, "file")) {
                found.add(of, file, text(&record, "at").unwrap_or_default());
            }
        }
        for event in run_logs.iter().flat_map(|log| json_lines(log)) {
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
        for later in found.next.values_mut() {
            later.sort();
            later.dedup();
        }
        found
    }

    fn add(&mut self, of: &str, file: &str, at: &str) {
        if of != file {
            self.next.entry(of.to_string()).or_default().push((at.to_string(), file.to_string()));
            self.before.insert(file.to_string(), of.to_string());
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

    /// The asset's first file, the one `file` is an iteration of, through
    /// every iteration before it, or `file` itself.
    pub fn first(&self, file: &str) -> String {
        let mut at = file.to_string();
        let mut seen = HashSet::new();
        while seen.insert(at.clone()) {
            match self.before.get(&at) {
                Some(older) => at = older.clone(),
                None => break,
            }
        }
        at
    }

    /// The asset `file` belongs to, as its stack: its first file, then each
    /// iteration after it, oldest first, to the latest.
    pub fn stack(&self, file: &str) -> Vec<String> {
        let mut stack = vec![self.first(file)];
        let mut seen: HashSet<String> = stack.iter().cloned().collect();
        let mut at = stack[0].clone();
        while let Some((_, newer)) = self.next.get(&at).and_then(|later| later.last()) {
            if !seen.insert(newer.clone()) {
                break;
            }
            stack.push(newer.clone());
            at = newer.clone();
        }
        stack
    }
}

fn json_lines(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok())
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(values: &[Value]) -> String {
        values.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn iterations_are_read_alike_in_canon_or_not() {
        let records = lines(&[
            // A take no step uses, edited in its workspace twice.
            serde_json::json!({ "at": "2026-11-19T10:00:00Z", "file": "prompts/resources/take-2.png", "inputs": [{ "file": "library/take.png" }], ITERATES: { "file": "library/take.png", "sha256": "a1" } }),
            serde_json::json!({ "at": "2026-11-19T11:00:00Z", "file": "prompts/resources/take-3.png", "inputs": [{ "file": "prompts/resources/take-2.png" }], ITERATES: { "file": "prompts/resources/take-2.png", "sha256": "a2" } }),
            // A drawn hold over it: made from it, never its iteration.
            serde_json::json!({ "at": "2026-11-19T12:00:00Z", "file": "prompts/resources/shot-2-hold-1.png", "inputs": [{ "file": "prompts/resources/take-3.png" }] }),
        ]);
        let runs = lines(&[
            // The bell's keyframe edited, then fixed.
            serde_json::json!({ "event": "publish", "at": "2026-11-19T10:00:00Z", "artifact": "Last order: keyframe, the bell", "file": "prompts/resources/bell-2.png", "edited_from": "library/bell.png" }),
            serde_json::json!({ "event": "publish", "at": "2026-11-19T11:00:00Z", "artifact": "Last order: keyframe, the bell", "file": "library/bell-3.png", "fixes": "prompts/resources/bell-2.png" }),
            // Another take put in use: another asset, not an iteration.
            serde_json::json!({ "event": "publish", "at": "2026-11-19T12:00:00Z", "artifact": "Last order: keyframe, the line", "file": "library/line-take-4.png", "replaced": "library/line.png" }),
        ]);
        let found = Iterations::from_text(&records, &[&runs, "not json"]);
        assert_eq!(found.latest("library/take.png"), "prompts/resources/take-3.png");
        assert_eq!(found.latest("prompts/resources/take-2.png"), "prompts/resources/take-3.png");
        assert_eq!(found.latest("library/bell.png"), "library/bell-3.png");
        assert_eq!(found.latest("library/line.png"), "library/line.png");
        assert_eq!(found.latest("prompts/resources/shot-2-hold-1.png"), "prompts/resources/shot-2-hold-1.png");
        assert_eq!(found.first("prompts/resources/take-3.png"), "library/take.png");
        assert_eq!(found.stack("prompts/resources/take-2.png"), ["library/take.png", "prompts/resources/take-2.png", "prompts/resources/take-3.png"]);
        assert_eq!(found.stack("library/line-take-4.png"), ["library/line-take-4.png"]);
    }
}
