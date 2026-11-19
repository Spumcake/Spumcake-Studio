//! A project's chains and steps as `read_canon` reads them from `canon.sql`,
//! and Skills' `touches --json` answer as `read_touches` gives it: pure
//! helpers any workspace uses alike, never messages. A chain opens as a
//! recipe, and each of its cells is the chain's steps at one order, one step
//! per asset, as the 0.15 plan's rule "How publishing maps onto Skills" sets
//! out. Ignore keeps an artifact with `crate::IGNORED_REASON`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Every chain, in order: one `json_object` per row, `{name, order, title}`.
pub const CHAINS: &str = "SELECT json_object('name', name, 'order', ord, 'title', coalesce(title, '')) FROM chain ORDER BY ord, name";

/// A chain of a project's living pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chain {
    pub name: String,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub title: String,
}

/// One step of a chain, with the artifact it makes and that artifact's
/// current file, its latest canon version, when it has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub chain: String,
    #[serde(default)]
    pub order: u32,
    pub name: String,
    #[serde(default)]
    pub skill: String,
    #[serde(default)]
    pub artifact: String,
    /// The endpoint's kind, such as "openrouter", "fal", or "script".
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub file: String,
}

/// The steps of `chain`, in order: one `json_object` per row.
pub fn chain_steps(chain: &str) -> String {
    format!(
        "SELECT json_object('chain', s.chain, 'order', s.ord, 'name', s.name, 'skill', coalesce(s.skill, ''), 'artifact', coalesce(s.artifact, ''), \
         'endpoint', coalesce(CASE WHEN json_valid(s.endpoint) THEN json_extract(s.endpoint, '$.kind') END, ''), 'file', coalesce(a.file, '')) \
         FROM step s LEFT JOIN artifact a ON a.id = s.artifact WHERE s.chain = {} ORDER BY s.ord, s.name",
        quoted(chain)
    )
}

/// The living pipeline among a project's `pipeline/` folder names: the
/// highest-numbered, as the owner set on 2026-11-16.
pub fn living_pipeline<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    names.into_iter().filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_digit())).max_by_key(|name| name.parse::<u32>().unwrap_or(0)).map(str::to_string)
}

/// Where a chain's recipe is kept as a tab, relative to the project:
/// `pipeline/<n>/canon.sql#<chain>`, so it comes back after a restart as
/// every tab does, read again from `canon.sql`.
pub fn chain_path(pipeline: &str, chain: &str) -> String {
    format!("pipeline/{pipeline}/canon.sql#{chain}")
}

/// A chain's recipe's path, split into the `canon.sql` file and the chain.
pub fn chain_of_path(path: &str) -> Option<(&str, &str)> {
    let (file, chain) = path.split_once('#')?;
    (file.ends_with("canon.sql") && !chain.is_empty()).then_some((file, chain))
}

/// A text as an SQL string.
pub fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The chains in a `canon_rows` answer to [`CHAINS`].
pub fn chains_of(rows: &[Value]) -> Vec<Chain> {
    rows.iter().filter_map(|row| serde_json::from_value(row.clone()).ok()).collect()
}

/// The steps in a `canon_rows` answer to [`chain_steps`].
pub fn steps_of(rows: &[Value]) -> Vec<Step> {
    rows.iter().filter_map(|row| serde_json::from_value(row.clone()).ok()).collect()
}

/// The cells of a chain: its steps grouped by their order, each group one
/// cell whose steps are its assets, the first step in name order its place.
pub fn cells_of(steps: &[Step]) -> Vec<Vec<Step>> {
    let mut by_order: BTreeMap<u32, Vec<Step>> = BTreeMap::new();
    for step in steps {
        by_order.entry(step.order).or_default().push(step.clone());
    }
    by_order.into_values().collect()
}

/// The cell kind a step shows as: a script step is a script; otherwise the
/// kind of its artifact's file, an image, a video, a sound, or a written
/// document; text when it has none yet.
pub fn kind_of(step: &Step) -> &'static str {
    if step.endpoint == "script" {
        return SCRIPT_KIND;
    }
    let extension = step.file.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" => crate::IMAGE_KIND,
        "mp4" | "mov" | "webm" => crate::VIDEO_KIND,
        "wav" | "mp3" | "flac" | "ogg" | "m4a" => crate::SOUND_KIND,
        "md" | "txt" | "json" | "html" => DOCUMENT_KIND,
        _ => crate::TEXT_KIND,
    }
}

/// A chain step run by a script, such as `build-animatic`'s.
pub const SCRIPT_KIND: &str = "script";
/// A chain step whose artifact is a written document or a builder's input.
pub const DOCUMENT_KIND: &str = "document";

/// What Skills' `touches --json` says is out of date and what it cannot
/// verify, by artifact, with its words kept as written: shown, never read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Touches {
    pub out_of_date: BTreeMap<String, Vec<String>>,
    pub unverified: BTreeMap<String, String>,
}

impl Touches {
    /// From a `touches_read` answer's `touches`.
    pub fn from_value(touches: &Value) -> Self {
        let mut found = Self::default();
        for entry in touches.get("out_of_date").and_then(Value::as_array).into_iter().flatten() {
            let Some(artifact) = entry.get("artifact").and_then(Value::as_str) else { continue };
            let why = match entry.get("why") {
                Some(Value::Array(sentences)) => sentences.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                Some(Value::String(sentence)) => vec![sentence.clone()],
                _ => Vec::new(),
            };
            found.out_of_date.insert(artifact.to_string(), why);
        }
        for entry in touches.get("unverified").and_then(Value::as_array).into_iter().flatten() {
            let Some(artifact) = entry.get("artifact").and_then(Value::as_str) else { continue };
            let why = entry.get("why").and_then(Value::as_str).unwrap_or_default();
            found.unverified.insert(artifact.to_string(), why.to_string());
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_chain_reads_as_cells_by_order() {
        let chains = chains_of(&[json!({ "name": "Otis", "order": 3, "title": "" }), json!("not a chain")]);
        assert_eq!(chains, [Chain { name: "Otis".into(), order: 3, title: String::new() }]);
        let rows = [
            json!({ "chain": "Otis", "order": 1, "name": "model sheet", "skill": "generate-model-sheet", "artifact": "Otis: model sheet", "endpoint": "openai-image-edit", "file": "library/otis.png" }),
            json!({ "chain": "Otis", "order": 1, "name": "model sheet, 2", "skill": "generate-model-sheet", "artifact": "Otis: model sheet, 2", "endpoint": "openai-image-edit", "file": "" }),
            json!({ "chain": "Otis", "order": 2, "name": "the line, voice take", "skill": "generate-voice", "artifact": "Otis: the line, voice take", "endpoint": "fal", "file": "library/line.wav" }),
            json!({ "chain": "Otis", "order": 3, "name": "turnaround", "skill": "build-previs", "artifact": "Otis: turnaround", "endpoint": "script", "file": "prompts/resources/turnaround.png" }),
        ];
        let cells = cells_of(&steps_of(&rows));
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].iter().map(|step| step.name.as_str()).collect::<Vec<_>>(), ["model sheet", "model sheet, 2"]);
        assert_eq!([kind_of(&cells[0][0]), kind_of(&cells[1][0]), kind_of(&cells[2][0])], [crate::IMAGE_KIND, crate::SOUND_KIND, SCRIPT_KIND]);
        assert!(chain_steps("Otis's").contains("'Otis''s'"));
        assert_eq!(living_pipeline(["001", "002", "notes", ""]), Some("002".to_string()));
        assert_eq!(chain_path("002", "the animatic"), "pipeline/002/canon.sql#the animatic");
        assert_eq!(chain_of_path("/p/pipeline/002/canon.sql#the animatic"), Some(("/p/pipeline/002/canon.sql", "the animatic")));
        assert_eq!(chain_of_path("/p/Untitled Recipe.smx"), None);
    }

    #[test]
    fn touches_keeps_skills_words_as_written() {
        let touches = Touches::from_value(&json!({
            "out_of_date": [{ "artifact": "Otis: expressions", "why": ["Otis: model sheet was published again"] }],
            "unverified": [{ "artifact": "Otis: turnaround", "why": "no record of what made it" }],
        }));
        assert_eq!(touches.out_of_date["Otis: expressions"], ["Otis: model sheet was published again"]);
        assert_eq!(touches.unverified["Otis: turnaround"], "no record of what made it");
    }
}
