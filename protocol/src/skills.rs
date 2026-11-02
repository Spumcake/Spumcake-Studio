//! What a sub-application tells the assistant it can do, and how the host
//! asks it to do one thing.
//!
//! A sub-application declares its skills as data with `skills`. Each skill
//! has instructions for the assistant and tools, each with a JSON Schema for
//! its input. The host offers every running sub-application's tools to the
//! assistant, and when the assistant calls one, sends `invoke`. The
//! sub-application carries it out through the same path its hand actions
//! take, sends any `document` or `tabs` the call caused, then answers with
//! `result`. The host and the assistant never know a skill or tool by name.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One thing a sub-application can do for the assistant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// Lowercase letters, digits, and single underscores, unique in the
    /// sub-application. With the application id and a tool name it must fit
    /// a model's 64-character function name.
    pub id: String,
    pub name: String,
    /// One or two sentences on what the skill is for.
    pub description: String,
    /// How to use the skill well: units, coordinates, what to read first.
    #[serde(default)]
    pub instructions: String,
    pub tools: Vec<Tool>,
}

/// One operation of a skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    /// Lowercase letters, digits, and underscores, unique in the skill.
    pub name: String,
    pub description: String,
    /// A JSON Schema for the tool's input, an object. The host adds `tab`.
    pub input: Value,
}

/// The answer to one `invoke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The `call` of the `invoke` this answers.
    pub call: String,
    /// False when nothing changed, and `output` says why.
    pub ok: bool,
    /// What was found, or what was made and its ids, for the model to read.
    pub output: Value,
    /// Images for the model to look at, each PNG data in standard base64.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// The flag the host passes when it starts a sub-application to restore a
/// workspace: it opens no document of its own, and waits for `open`.
pub const RESTORE_FLAG: &str = "--restore";

/// A tool's id as the host and the assistant use it:
/// `<application id>__<skill id>__<tool name>`.
pub fn tool_key(app: &str, skill: &str, tool: &str) -> String {
    format!("{app}__{skill}__{tool}")
}

/// Splits a key made by [`tool_key`]. An application id holds hyphens, never
/// two underscores in a row, so the first `__` ends it.
pub fn split_tool_key(key: &str) -> Option<(&str, &str, &str)> {
    let (app, rest) = key.split_once("__")?;
    let (skill, tool) = rest.split_once("__")?;
    if app.is_empty() || skill.is_empty() || tool.is_empty() {
        return None;
    }
    Some((app, skill, tool))
}
