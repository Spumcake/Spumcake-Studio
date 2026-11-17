//! A cell: one promptable task, its prompt and its result.
//!
//! The host draws every cell. A sub-application that keeps cells stores them
//! and never draws them. `running` is not a stored state: only the host draws
//! it, while a run is in progress.

use serde::{Deserialize, Serialize};

use crate::{CellUse, ItemRef, UsedItem};

/// One cell. A kind this build does not know still parses: `kind` is a string,
/// and parts this build does not show stay in `input` and `output`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// Unique everywhere, so a later version can name any cell in any recipe.
    pub id: String,
    /// When the cell was dragged in, the id of the cell it was copied from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `"text"` in this version. Any other string is kept and drawn generically.
    pub kind: String,
    /// The prompt. A text cell has one text part. Later kinds add path parts.
    #[serde(default)]
    pub input: Vec<CellPart>,
    /// Which endpoint ran it. Set when it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// That endpoint's model id. Set when it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The reasoning effort it ran at, such as `"medium"`, if one was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The result. Empty until a run finishes, fails, or is stopped.
    #[serde(default)]
    pub output: Vec<CellPart>,
    pub state: CellState,
    /// A sentence when the cell failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Every run, oldest first. A run always exists: nothing removes one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<CellVersion>,
    /// What the task is run with besides its prompt, such as a video's
    /// duration or a seed. Sent with the task and kept with each version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    /// The version the cell shows. The top-level `endpoint`, `model`, `effort`,
    /// `output`, `state`, and `error` always equal it; only `input` may
    /// differ, as an edit not yet run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<usize>,
    /// In a Skills project, the chain step this cell is. The step lives in
    /// `canon.sql` alone, so a cell with a step has empty `input`,
    /// `versions`, and `output`, and no `settings`, `endpoint`, or `model`:
    /// the host reads the step and never sends it to the workspace. Without
    /// one, the cell is a draft that carries its own prompt until it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<StepRef>,
}

/// One chain step in a Skills project's `canon.sql`, by its chain and step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRef {
    pub chain: String,
    pub step: String,
}

/// One run of a cell, kept for good.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellVersion {
    /// Unique everywhere, so a later version can name one run as context.
    pub id: String,
    /// The prompt that ran.
    #[serde(default)]
    pub input: Vec<CellPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub output: Vec<CellPart>,
    /// Done, failed, or stopped.
    pub state: CellState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Seconds since the Unix epoch.
    #[serde(default)]
    pub time: u64,
    /// The settings it ran with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    /// Each item part of its prompt, with the fingerprint it resolved to
    /// when it ran: what the run was made from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used: Vec<UsedItem>,
}

/// The kind of a cell that sends its prompt to a language model.
pub const TEXT_KIND: &str = "text";
/// The kind of a cell that makes a image from its prompt.
pub const IMAGE_KIND: &str = "image";
/// The kind of a cell of Markdown, which runs by showing its source as rich
/// text, calls nothing, and has no versions.
pub const MARKDOWN_KIND: &str = "markdown";
/// The kind of a cell that makes a video from its prompt and frames: its
/// first frame and, optionally, its last, in that order.
pub const VIDEO_KIND: &str = "video";

/// One part of a cell's prompt or result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellPart {
    Text { text: String },
    /// A file. As a reference, `role` is what it is
    /// for, as "<menu word>, <the user's words>", such as "Start frame, the
    /// keeper", and `field` the model's input it fills, which the role's
    /// menu word picked.
    Path {
        path: String,
        media_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field: Option<String>,
    },
    /// An item of the project, resolved into a file of `media_type` when the
    /// cell runs. For a cell, `use` says whether its prompt or its result.
    /// `role` and `field` are as a path's.
    Item {
        item: ItemRef,
        media_type: String,
        #[serde(default, rename = "use", skip_serializing_if = "Option::is_none")]
        part: Option<CellUse>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        field: Option<String>,
    },
    /// A part of a shape a later version adds. It is kept as it was, so a
    /// cells message or a recipe holding one never fails to read.
    Other(serde_json::Value),
}

/// What a sub-application stores. The host draws `running` itself and never
/// sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellState {
    Idle,
    Done,
    Failed,
    Stopped,
    /// Only the host draws a cell as running. A sub-application that sends
    /// it is refused when its message is read.
    Running,
    /// A state a later version adds, read as a state this build cannot name.
    #[serde(other)]
    Other,
}

impl Cell {
    /// A text cell that has not run, with one prompt part.
    pub fn text(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            from: None,
            kind: String::from("text"),
            input: vec![CellPart::Text { text: prompt.into() }],
            endpoint: None,
            model: None,
            effort: None,
            output: Vec::new(),
            state: CellState::Idle,
            error: None,
            versions: Vec::new(),
            settings: None,
            current: None,
            step: None,
        }
    }

    /// A Markdown cell showing its source.
    pub fn markdown(id: impl Into<String>, source: impl Into<String>) -> Self {
        let mut cell = Self::text(id, source);
        cell.kind = String::from(MARKDOWN_KIND);
        cell
    }

    /// Makes the top level agree with the current version, as the rule
    /// between the two copies says. A current index past the last version
    /// points at the last one.
    pub fn agree(&mut self) {
        if self.versions.is_empty() {
            self.current = None;
            return;
        }
        let index = self.current.unwrap_or(self.versions.len() - 1).min(self.versions.len() - 1);
        self.current = Some(index);
        let version = &self.versions[index];
        self.endpoint = version.endpoint.clone();
        self.model = version.model.clone();
        self.effort = version.effort.clone();
        self.output = version.output.clone();
        self.state = version.state;
        self.error = version.error.clone();
    }

    /// Makes version `index` current: its prompt and result both show,
    /// replacing an edit not yet run.
    pub fn show_version(&mut self, index: usize) -> bool {
        let Some(version) = self.versions.get(index) else {
            return false;
        };
        self.input = version.input.clone();
        self.current = Some(index);
        self.agree();
        true
    }

    /// A cell with a step that also carries a copy of the step: a prompt,
    /// versions, a result, settings, an endpoint, or a model. Such a cell
    /// is refused when its message is read.
    pub fn copies_its_step(&self) -> bool {
        self.step.is_some()
            && (!self.input.is_empty()
                || !self.versions.is_empty()
                || !self.output.is_empty()
                || self.settings.is_some()
                || self.endpoint.is_some()
                || self.model.is_some())
    }

    /// The text parts of the prompt, joined.
    pub fn prompt(&self) -> String {
        parts_text(&self.input)
    }

    /// The text parts of the result, joined.
    pub fn result_text(&self) -> String {
        parts_text(&self.output)
    }
}

fn parts_text(parts: &[CellPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            CellPart::Text { text } => Some(text.as_str()),
            CellPart::Path { .. } | CellPart::Item { .. } | CellPart::Other(_) => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// A new cell id. It is a UUID, so two cells never share one.
pub fn cell_id() -> String {
    let mut bytes = [0u8; 16];
    let mut file = std::fs::File::open("/dev/urandom").expect("urandom");
    std::io::Read::read_exact(&mut file, &mut bytes).expect("urandom");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8],
        bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// One request a cell made, in a Skills project, read from `requests/` and
/// its pending records. The host sends a cell's rounds with `takes`; the
/// workspace shows them on its filmstrip and never stores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Round {
    /// The request's id.
    pub request: String,
    /// Counted from 1 among the cell's rounds, as "Round 14" shows it.
    pub number: u32,
    pub model: String,
    /// The model's name as it is read, such as "GPT Image 2.5", the name
    /// the card shows; empty when the host does not know it, and the
    /// workspace shows `model`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_name: String,
    /// The prompt as it was sent.
    #[serde(default)]
    pub prompt: String,
    /// The references in the order they were sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<SentReference>,
    /// References the step lists that the request did not send, when its
    /// report records them: the references sheet's "Not sent".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<SentReference>,
    pub state: RoundState,
    /// When it started, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
    /// While it runs, how many takes it makes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<TakeFile>,
    /// A sentence when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Its report, relative to the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// The canon artifact its step makes, "<element>: <role>", when the
    /// step names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

/// One reference of a round: its file, relative to the project, and its role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentReference {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundState {
    /// Being made.
    Running,
    /// Waiting at the endpoint, to be collected or cancelled.
    Pending,
    Done,
    Failed,
    Cancelled,
}

/// One take a round made: its file, relative to the project, and whether
/// `canon.sql` names it as canon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TakeFile {
    pub path: String,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub canon: bool,
}

/// One version of a canon artifact, as `history` lists them: its file,
/// relative to the project, when it was published as `canon.sql` records
/// it, in seconds since the epoch as `Round.started` is, its note, whether it is the version canon names now, and the request
/// that made it, so its report can be opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonVersion {
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub published: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub canon: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}
