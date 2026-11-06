//! The tools a sub-application offers the assistant, and how the host asks
//! it to use one.
//!
//! A sub-application declares its tools as data with `tools`, in tool
//! groups. Each group has instructions for using its tools, and each tool a
//! JSON Schema for its input. A tool group is never a skill: a skill is an
//! agent skill, a `SKILL.md`, as the conventions guide says. The host offers every running sub-application's tools to the
//! assistant, and when the assistant calls one, sends `invoke`. The
//! sub-application carries it out through the same path its hand actions
//! take, sends any `document` or `tabs` the call caused, then answers with
//! `result`. The host and the assistant never know a tool group or tool by name.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tools a sub-application declares together for the assistant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolGroup {
    /// Lowercase letters, digits, and single underscores, unique in the
    /// sub-application. With the application id and a tool name it must fit
    /// a model's 64-character function name.
    pub id: String,
    pub name: String,
    /// One or two sentences on what the group's tools are for.
    pub description: String,
    /// How to use its tools well: units, coordinates, what to read first.
    #[serde(default)]
    pub instructions: String,
    pub tools: Vec<Tool>,
}

/// One operation of a tool group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    /// Lowercase letters, digits, and underscores, unique in the group.
    pub name: String,
    pub description: String,
    /// A JSON Schema for the tool's input, an object. The host adds `tab`.
    pub input: Value,
    /// The Spumcake-Agent general skills this tool carries out, by name, as
    /// an execution skill's `implements` does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<String>,
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
/// `<application id>__<group id>__<tool name>`.
pub fn tool_key(app: &str, group: &str, tool: &str) -> String {
    format!("{app}__{group}__{tool}")
}

/// Splits a key made by [`tool_key`]. An application id holds hyphens, never
/// two underscores in a row, so the first `__` ends it.
pub fn split_tool_key(key: &str) -> Option<(&str, &str, &str)> {
    let (app, rest) = key.split_once("__")?;
    let (group, tool) = rest.split_once("__")?;
    if app.is_empty() || group.is_empty() || tool.is_empty() {
        return None;
    }
    Some((app, group, tool))
}
