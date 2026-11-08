//! The catalogue of every tool the assistant can be offered, written to
//! `docs/tools.md` from what the host and each workspace actually declare,
//! so it can be referenced and audited and never drifts from the code.
//! `UPDATE_TOOL_CATALOGUE=1 cargo test -p inpainter tool_catalogue` writes
//! it again after a change; the check fails while it is out of date.

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use protocol::{AppMessage, ToolGroup};
    use serde_json::Value;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    /// Every workspace a person sees, from its manifest, with the tool
    /// groups its built program declares. The hidden template is left out.
    fn workspaces() -> Vec<(String, String, Vec<ToolGroup>)> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(root().join("apps")).expect("apps").flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path().join("manifest.json")) else {
                continue;
            };
            let manifest = protocol::parse_manifest(&text).expect("manifest");
            if manifest.hidden {
                continue;
            }
            let program = root().join("target/debug").join(&manifest.executable);
            assert!(program.exists(), "{} is missing: run `cargo build` first", program.display());
            let mut child = Command::new(program).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("start");
            let _ = child.stdin.take().expect("stdin").write_all(b"{\"type\":\"shutdown\"}\n");
            let mut groups = Vec::new();
            for line in BufReader::new(child.stdout.take().expect("stdout")).lines().map_while(Result::ok) {
                if let Ok(AppMessage::Tools { groups: declared }) = protocol::parse_app_message(&line) {
                    groups = declared;
                }
            }
            let _ = child.wait();
            found.push((manifest.id, manifest.name, groups));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    /// A tool's inputs, as "`name` (type)", required ones first, marked.
    fn inputs(schema: &Value) -> String {
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return String::from("nothing");
        };
        if properties.is_empty() {
            return String::from("nothing");
        }
        let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let kind = |value: &Value| match value.get("type") {
            Some(Value::String(kind)) => kind.clone(),
            Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" or "),
            _ if value.get("enum").is_some() => String::from("choice"),
            _ => String::from("any"),
        };
        let mut names: Vec<(&String, &Value)> = properties.iter().collect();
        names.sort_by_key(|(name, _)| !required.contains(&name.as_str()));
        names
            .into_iter()
            .map(|(name, value)| {
                let mark = if required.contains(&name.as_str()) { ", required" } else { "" };
                format!("`{name}` ({}{mark})", kind(value))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn section(out: &mut String, app: &str, title: &str, groups: &[ToolGroup]) {
        out.push_str(&format!("\n## {title}\n"));
        if groups.is_empty() {
            out.push_str("\nNo tools.\n");
        }
        for group in groups {
            out.push_str(&format!("\n### {} (`{}`)\n\n{}\n\n", group.name, group.id, group.description.trim()));
            for tool in &group.tools {
                out.push_str(&format!(
                    "- **`{}`**: {} Takes {}.",
                    protocol::tool_key(app, &group.id, &tool.name),
                    tool.description.trim(),
                    inputs(&tool.input)
                ));
                if !tool.implements.is_empty() {
                    out.push_str(&format!(" Carries out {}.", tool.implements.join(", ")));
                }
                out.push('\n');
            }
        }
    }

    fn catalogue() -> String {
        let workspaces = workspaces();
        let count: usize = workspaces.iter().flat_map(|(_, _, groups)| groups).map(|group| group.tools.len()).sum::<usize>() + crate::route::host_tools().tools.len();
        let mut out = String::from(
            "# Tool catalogue\n\nEvery tool Spumcake Studio's assistant can be offered, as the host and each workspace declare them. This file is written from the code by `UPDATE_TOOL_CATALOGUE=1 cargo test -p inpainter tool_catalogue`, and a check fails while it is out of date; do not edit it by hand. A workspace's tools also take `tab`, the tab they act on, which the host adds. Besides the tools below, each skill a person makes in Settings is offered as a tool of its own, in the host's \"Your skills\" group.\n",
        );
        out.push_str(&format!("\n{count} tools.\n"));
        section(&mut out, crate::route::HOST_ID, "The host", &[crate::route::host_tools()]);
        for (id, name, groups) in &workspaces {
            section(&mut out, id, name, groups);
        }
        out
    }

    /// Catches a tool added, removed, or changed without the catalogue that
    /// lists it: the file is what the code declares, exactly.
    #[test]
    fn the_tool_catalogue_matches_the_code() {
        let path = root().join("docs/tools.md");
        let made = catalogue();
        if std::env::var_os("UPDATE_TOOL_CATALOGUE").is_some() {
            std::fs::write(&path, &made).expect("write docs/tools.md");
            return;
        }
        let kept = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(kept == made, "docs/tools.md is out of date: run `UPDATE_TOOL_CATALOGUE=1 cargo test -p inpainter tool_catalogue`");
    }
}
