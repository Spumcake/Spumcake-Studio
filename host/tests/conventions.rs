//! The conventions guide's names, held to across the workspace: each
//! workspace's id and name, one rule for naming new items, and no retired
//! word left in code, documents, or what the assistant is told.

use std::path::{Path, PathBuf};

/// The workspaces the conventions guide names: id, name, and whether it is
/// hidden.
const WORKSPACES: &[(&str, &str, bool)] = &[
    ("image-generator", "Image Generator", false),
    ("recipe-creation", "Recipe Creation", false),
    ("storyboard-generator", "Storyboard Generator", false),
    ("template", "Template", true),
    ("video-generator", "Video Generator", false),
];

/// Words the guide retires, as a word or part of an identifier.
const RETIRED_WORDS: &[&str] = &["notebook", "book", "sketch", "picture", "asset", "strategy", "strategies"];

/// Names the guide retires, matched with spaces, hyphens, and underscores
/// left out, so "image-editor", "ImageEditor", and "mission_control" count.
const RETIRED_NAMES: &[&str] = &["simplesketch", "imageeditor", "missioncontrol"];

/// Lines that may keep a retired word: what reads an earlier spelling from
/// disk, a manifest's former ids, the reference the shell came from, and
/// a crate's `assets/` folder of fonts and icons.
const KEPT: &[&str] = &["assets/", "rename = \"", "alias = \"", "formerly", "stored spelling", "simple sketch 1.0.1"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "target" | "fixtures" | "vendor" | "assets") {
                files(&path, found);
            }
        } else if matches!(path.extension().and_then(|ext| ext.to_str()), Some("rs" | "md" | "json" | "toml")) {
            found.push(path);
        }
    }
}

/// A line's words, split at anything not a letter or digit and at each
/// lower-to-upper step, lowercased: "RasterPicture_list" gives "raster",
/// "picture", "list".
fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for c in line.chars() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = false;
            continue;
        }
        if c.is_uppercase() && previous_lower && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        previous_lower = c.is_lowercase();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn retired(word: &str) -> bool {
    RETIRED_WORDS.iter().any(|retired| {
        word == *retired || word.strip_suffix('s') == Some(retired) || word.strip_suffix("es") == Some(retired)
    })
}

/// Catches a workspace renamed in its manifest alone: its folder, Cargo
/// package, program, manifest `executable`, and id are one, and its id,
/// name, and hiding are the guide's.
#[test]
fn every_workspace_is_named_as_the_guide_says_through_and_through() {
    let apps = root().join("apps");
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(&apps).unwrap().flatten() {
        let folder = entry.file_name().to_string_lossy().into_owned();
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(entry.path().join("manifest.json")).unwrap()).unwrap();
        let cargo = std::fs::read_to_string(entry.path().join("Cargo.toml")).unwrap();
        let package = cargo
            .lines()
            .find_map(|line| line.strip_prefix("name = \"").and_then(|rest| rest.strip_suffix('"')))
            .unwrap_or_default();
        let id = manifest["id"].as_str().unwrap_or_default();
        assert_eq!(
            [id, manifest["executable"].as_str().unwrap_or_default(), package],
            [folder.as_str(); 3],
            "{folder}: its id, program, and package are not its folder's name"
        );
        let hidden = manifest["hidden"].as_bool().unwrap_or(false);
        let named = WORKSPACES.iter().find(|(guide_id, ..)| *guide_id == id);
        assert_eq!(
            named.map(|(_, name, hidden)| (*name, *hidden)),
            Some((manifest["name"].as_str().unwrap_or_default(), hidden)),
            "{folder} is not the guide's"
        );
        seen.push(folder);
    }
    seen.sort();
    let guide: Vec<&str> = WORKSPACES.iter().map(|(id, ..)| *id).collect();
    assert_eq!(seen, guide, "the workspaces are not the guide's");
}

/// Catches a workspace that names its new items by a rule of its own: no
/// naming function is defined outside the protocol.
#[test]
fn only_the_protocol_names_new_items() {
    let mut found = Vec::new();
    let mut all = Vec::new();
    for dir in ["apps", "host/src", "client/src", "draw/src", "assistant/src"] {
        files(&root().join(dir), &mut all);
    }
    for path in all.iter().filter(|path| path.extension().is_some_and(|ext| ext == "rs")) {
        let text = std::fs::read_to_string(path).unwrap();
        for function in ["fn free_names", "fn free_name(", "fn untitled_name"] {
            if text.contains(function) {
                found.push(format!("{}: {function}", path.display()));
            }
        }
    }
    assert!(found.is_empty(), "named outside the protocol:\n{}", found.join("\n"));
}

/// Catches a rename of labels alone: no retired word is left anywhere in
/// the code, the documents, or the skills and tools the assistant reads,
/// except where an earlier spelling is read from disk, a manifest names its
/// former ids, or the shell's reference is named.
#[test]
fn no_retired_word_is_left() {
    let mut all = Vec::new();
    for dir in ["apps", "host", "protocol", "draw", "media", "assistant", "client", "docs", "skills"] {
        files(&root().join(dir), &mut all);
    }
    all.push(root().join("AGENTS.md"));
    all.push(root().join("README.md"));
    all.push(root().join("Cargo.toml"));
    let this = Path::new(file!()).file_name().unwrap().to_owned();
    let mut found = Vec::new();
    for path in all {
        if path.file_name() == Some(this.as_os_str()) || path.ends_with("tests/boundaries.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            if KEPT.iter().any(|kept| lower.contains(kept)) {
                continue;
            }
            let joined: String = lower.chars().filter(|c| !matches!(c, ' ' | '-' | '_')).collect();
            let hit = RETIRED_NAMES.iter().find(|name| joined.contains(*name)).map(|name| name.to_string())
                .or_else(|| words(line).into_iter().find(|word| retired(word)));
            if let Some(word) = hit {
                found.push(format!("{}:{}: {word:?} in {}", path.strip_prefix(root()).unwrap_or(&path).display(), number + 1, line.trim()));
            }
        }
    }
    assert!(found.is_empty(), "{} retired words left:\n{}", found.len(), found.join("\n"));
}
