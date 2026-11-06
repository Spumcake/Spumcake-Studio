//! Items: the pieces of work inside a project's files, what each was made
//! from, and how one workspace asks for another's.
//!
//! Each workspace declares the items of its files with `items`: an id, a
//! name, what it can be rendered as, a fingerprint of its content, and what
//! it was made from. The host keeps an index of what was declared and
//! nothing else. It marks an item out of date with `stale` when an item it
//! was made from no longer has the fingerprint recorded, and resolves an item
//! into a file with `render` when a cell that names it runs or a workspace
//! sends `need`. The host knows items only as paths, ids, names, media
//! types, and fingerprints. The contract is the lifecycle's `systems.md` in
//! the team repository.

use serde::{Deserialize, Serialize};

/// One item of a project: a file, relative to the project with `/` between
/// folders, and the owner's own id for a piece inside it, or none for the
/// whole file.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ItemRef {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

/// One item as its owner declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// The owner's id for it, unique in its file.
    pub id: String,
    /// What a person reads for it, such as an image's name.
    pub name: String,
    /// What it can be rendered as, such as `image/png`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_types: Vec<String>,
    /// [`fingerprint`] of its content.
    pub fingerprint: String,
    /// What it was made from, when it was made from other items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<Made>,
}

/// What an item was made from. The owner keeps it in its own file and
/// declares it; the host stores none of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Made {
    /// The cell that made it, when a cell did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<ItemRef>,
    /// The id of that cell's run, its `CellVersion.id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The items that run resolved, copied from its version's `used`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used: Vec<UsedItem>,
    /// What a person put in it by hand, such as a reference placed on a board.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<UsedItem>,
}

/// An item, and the fingerprint it had when it was used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsedItem {
    pub item: ItemRef,
    pub fingerprint: String,
}

/// One out-of-date item of a file, and which items it was made from changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleItem {
    pub id: String,
    pub changed: Vec<ItemRef>,
}

/// Which part of a cell an item part takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellUse {
    Prompt,
    Result,
}

/// The fingerprint of an item's content: FNV-1a, 64 bits, as lowercase hex.
/// Written here by hand, so it stays the same across Rust releases and every
/// workspace computes it alike. The same bytes always give the same string.
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Where the owner writes an item rendered as a file for another workspace,
/// relative to the project: named by the item, the part of a cell it takes,
/// and its fingerprint, and never overwritten, so an older run's inputs still
/// match what ran, and a cell's prompt and result are never one file.
pub fn render_path(owner: &str, item: &ItemRef, part: Option<CellUse>, fingerprint: &str, extension: &str) -> String {
    let stem = item.item.as_deref().unwrap_or("file");
    let safe: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let part = match part {
        None => "",
        Some(CellUse::Prompt) => "-prompt",
        Some(CellUse::Result) => "-result",
    };
    format!(".inpainter/renders/{owner}/{safe}{part}-{fingerprint}.{extension}")
}

/// The fingerprint of a cell: its current prompt and its current result,
/// and nothing else, so running it again to the same result, or reading it
/// back from its file, changes nothing, and editing its prompt does. Every
/// owner of cells uses this one.
pub fn cell_fingerprint(cell: &crate::Cell) -> String {
    let content = serde_json::json!({ "input": cell.input, "output": cell.output });
    fingerprint(content.to_string().as_bytes())
}

/// The answer to `render` from a workspace that declares no items yet.
pub fn no_items(id: String) -> crate::AppMessage {
    crate::AppMessage::Rendered {
        id,
        path: None,
        fingerprint: None,
        error: Some(String::from("This workspace declares no items yet.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a fingerprint that depends on anything but the bytes, such as
    /// a hasher seeded per run: these values are fixed for good.
    #[test]
    fn a_fingerprint_is_the_same_for_the_same_bytes_forever() {
        assert_eq!(fingerprint(b""), "cbf29ce484222325");
        assert_eq!(fingerprint(b"a"), "af63dc4c8601ec8c");
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
    }

    #[test]
    fn a_render_is_named_by_its_item_and_fingerprint() {
        let item = ItemRef { path: "Beach.ipx".into(), item: Some("img 2".into()) };
        assert_eq!(render_path("image-generator", &item, None, "00ff", "png"), ".inpainter/renders/image-generator/img_2-00ff.png");
        let whole = ItemRef { path: "Beach.ipx".into(), item: None };
        assert_eq!(render_path("image-generator", &whole, None, "00ff", "png"), ".inpainter/renders/image-generator/file-00ff.png");
    }

    /// Catches a cell's prompt and result rendered to one file, where the
    /// second would silently be the first.
    #[test]
    fn a_cells_prompt_and_result_are_rendered_apart() {
        let cell = ItemRef { path: "Pilot.smx".into(), item: Some("c1".into()) };
        let prompt = render_path("recipe-creation", &cell, Some(CellUse::Prompt), "ab", "txt");
        let result = render_path("recipe-creation", &cell, Some(CellUse::Result), "ab", "txt");
        assert_ne!(prompt, result);
    }

    /// Catches a cell's fingerprint taken from its whole record: a run that
    /// gives the same result, or the cell read back, is the same content.
    #[test]
    fn a_cells_fingerprint_is_its_prompt_and_result() {
        let mut cell = crate::Cell::text("c1", "A wave.");
        cell.output = vec![crate::CellPart::Text { text: "Beat one.".into() }];
        let before = cell_fingerprint(&cell);
        let run = crate::CellVersion {
            id: "v2".into(),
            input: cell.input.clone(),
            endpoint: Some("e".into()),
            model: Some("m".into()),
            effort: None,
            output: cell.output.clone(),
            state: crate::CellState::Done,
            error: None,
            time: 1,
            settings: None,
            used: Vec::new(),
        };
        cell.versions.push(run);
        let read: crate::Cell = serde_json::from_str(&serde_json::to_string(&cell).unwrap()).unwrap();
        assert_eq!(cell_fingerprint(&cell), before);
        assert_eq!(cell_fingerprint(&read), before);
        cell.input = vec![crate::CellPart::Text { text: "A storm.".into() }];
        assert_ne!(cell_fingerprint(&cell), before);
    }
}
