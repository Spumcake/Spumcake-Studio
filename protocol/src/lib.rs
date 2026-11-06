//! Messages between the host and a sub-application.
//!
//! JSON Lines: one JSON object per line, ended by `\n`. A trailing `\r` is
//! stripped. Every object has a `type` field naming the message. Strings are
//! JSON strings, so a label may hold spaces, quotes, or any other text.

use serde::{Deserialize, Serialize};

mod cell;
mod declare;
mod items;
mod raster;
mod tools;
pub use cell::*;
pub use declare::*;
pub use items::*;
pub use raster::*;
pub use tools::*;

/// The version this crate speaks. A sub-application's first message says
/// which version it speaks; the host accepts only this one.
pub const PROTOCOL_VERSION: u32 = 1;

/// What a sub-application writes to the host, on its standard output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppMessage {
    /// Always first. Names the protocol version the sub-application speaks.
    Hello { protocol: u32 },
    /// One document the sub-application has open. `label` is the name of the
    /// file it manages or, when it manages no file, the sub-application's name.
    Tab { id: String, label: String },
    /// What the host draws in that tab's contents area.
    Contents { tab: String, kind: ContentsKind },
    /// Words for the host to put on the system clipboard, such as a text's
    /// selection copied.
    Clipboard { text: String },
    /// This application edits files of these media types, such as
    /// `image/png`: the host may offer to edit one there, with `open_as_new`.
    Edits { media_types: Vec<String> },
    /// That tab uses the host's properties panel, on this side.
    Panel { tab: String, side: PanelSide },
    /// The whole drawing after a change. `camera` is present when a drawing
    /// is opened or created.
    Document {
        tab: String,
        drawing: serde_json::Value,
        selection: Vec<u64>,
        active_canvas: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        camera: Option<DocumentCamera>,
    },
    /// A file dialog for the host to open.
    File {
        id: String,
        kind: FileKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<FileFilter>,
    },
    /// The declaration is complete.
    End,
    /// Every shortcut, replacing the ones declared before.
    Keys(Keys),
    /// The selected tab's toolbar.
    Toolbar(Toolbar),
    /// The hamburger menu of this sub-application's tabs.
    Menu { rows: Vec<MenuEntry> },
    /// The selected tab's properties panel.
    Properties(Properties),
    /// The tab list, which replaces the tabs reported before.
    Tabs(Tabs),
    /// The drawing surface's current tool and the text being typed.
    Canvas(Canvas),
    /// A raster tab's image, tool, selection, crop box, and layer box.
    Raster(RasterView),
    /// A question for the host to show. The host answers once.
    Ask(Ask),
    /// Every unsaved document has been resolved after `quit`. The host may exit.
    QuitReady,
    /// The answer to `ping`.
    Pong,
    /// Every tool group the sub-application offers the assistant, replacing
    /// those sent before. Before the hierarchy of 2026-11-04 this message was
    /// `skills`, with its groups under `skills`: the stored spelling, still read.
    #[serde(alias = "skills")]
    Tools {
        #[serde(alias = "skills")]
        groups: Vec<ToolGroup>,
    },
    /// The answer to one `invoke`, after any message the call caused.
    #[serde(rename = "result")]
    ToolResult(ToolResult),
    /// The answer to `keep`: the file that tab now has, or none when the
    /// tab is untouched, or `error` when the change could not be written.
    Kept {
        tab: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The whole cell list of a tab that keeps cells, after any change.
    Cells {
        tab: String,
        cells: Vec<Cell>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<String>,
    },
    /// Asks the host to run that cell, as the toolbar's Run does. With
    /// `content` and `at`, the host runs the cell given, which lives at `at`
    /// in a file no tab has open, and answers `cell_result` with `at`.
    RunCell {
        tab: String,
        cell: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Cell>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<ItemRef>,
    },
    /// Asks the host to stop that cell. The host keeps what had arrived.
    StopCell { tab: String, cell: String },
    /// Whether this application can open `path` as its own work.
    Claimed { id: String, yours: bool },
    /// Every item of one file, replacing what was declared for it before.
    /// Sent for each of its files after `project`, and again whenever a
    /// file's items change. An empty list says the file is gone.
    Items { path: String, items: Vec<Item> },
    /// The answer to `render`: the file written, inside the project, and the
    /// item's fingerprint as declared in `items`, not a hash of the file; or
    /// `error`, a sentence.
    Rendered {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fingerprint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Asks for another item as a file of `media_type`, for this
    /// application's own work. The host answers `provided`.
    Need { id: String, item: ItemRef, media_type: String },
    /// Asks the host to let the person choose an item of one of these media
    /// types. The host answers `picked`.
    Pick { id: String, media_types: Vec<String> },
}

/// The cell, and its image's path relative to the project, that new work
/// was opened from, and where the cell lives: a conversation session, or a
/// recipe's file relative to the project. Files written before 0.14 call
/// these by their stored spelling, `picture` and `notebook`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenedFrom {
    pub cell: String,
    #[serde(alias = "picture")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, alias = "notebook", skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
}

/// What the host writes to a sub-application, on its standard input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    /// A menu row, toolbar button, shortcut, the plus, a tab's close mark,
    /// selecting a tab, or an outliner action. `new_storyboard` creates a storyboard.
    Command {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg: Option<serde_json::Value>,
        /// The object a canvas menu row was chosen on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<u64>,
        viewport: [f64; 2],
    },
    /// One finished gesture.
    Gesture {
        tab: String,
        gesture: serde_json::Value,
    },
    /// One finished gesture on a raster tab.
    RasterGesture { tab: String, gesture: RasterGesture },
    /// A click, a marquee, or a cancelled drag on the drawing surface. The
    /// host shows the selection the next `document` states.
    Select { tab: String, op: SelectOp },
    /// The tab's camera after a pan, zoom, or fit. Not a history step.
    Camera {
        tab: String,
        origin: [f64; 2],
        zoom: f64,
    },
    /// The contents area's size, once it is first known.
    Viewport { viewport: [f64; 2] },
    /// The button the person chose on an `ask`.
    Answer { id: String, button: String },
    /// A panel or toolbar edit. `preview` is true while a slider is held.
    Property {
        prop: serde_json::Value,
        value: serde_json::Value,
        preview: bool,
    },
    /// The path the host's file dialog returned. Absent when the person cancelled.
    FileResult {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// The window's close was pressed. The window stays until `quit_ready`.
    Quit,
    /// Sent every second. The answer is `pong`.
    Ping,
    /// Run one tool of a declared tool group on that tab. `tool` is the
    /// tool's key, `<application id>__<group id>__<tool name>`. Every change made
    /// under one `group`, the assistant's request, is one undo step.
    Invoke {
        call: String,
        tab: String,
        tool: String,
        input: serde_json::Value,
        group: String,
    },
    /// Open that file in a new tab or, with no path, the tab the
    /// sub-application opens of its own at start. Used to restore a workspace.
    Open {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// The keyboard, while a raster tab's text is being typed in.
    RasterText { tab: String, input: TextInput },
    /// Open that file as new work, as `edits` declared: a new tab called
    /// `name`, never the file itself. `from` says what it came from.
    OpenAsNew {
        path: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<OpenedFrom>,
    },
    /// Make sure that tab's file is written. The answer is `kept`.
    Keep { tab: String },
    /// The project's folder, as an absolute path. New activities are created
    /// in it, and paths outside it are refused.
    Project { folder: String },
    /// The window is closing. Exit.
    Shutdown,
    /// The person's edit of a cell's prompt, when it loses focus and before
    /// a run. Never per keystroke. One undo step for the cell's owner.
    EditCell {
        tab: String,
        cell: String,
        input: Vec<CellPart>,
    },
    /// A run finished, failed, or was stopped. The owner keeps it as a new
    /// version, made from `input`, the prompt that actually ran. A run is
    /// never an undo step. A Markdown cell's run has no input or output: it
    /// shows the cell rendered and makes no version.
    CellResult {
        tab: String,
        cell: String,
        #[serde(default)]
        input: Vec<CellPart>,
        endpoint: String,
        model: String,
        /// The reasoning effort it ran at, if one was set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
        state: CellState,
        output: Vec<CellPart>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// The settings it ran with.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settings: Option<serde_json::Value>,
        /// Each item part, with the fingerprint it resolved to.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        used: Vec<UsedItem>,
        /// Where the cell lives, for a run asked for with `at`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<ItemRef>,
    },
    /// A dragged cell, already given a new id and `from` by the host, or,
    /// with no cell, an empty one the owner makes, of the kind it would add.
    InsertCell {
        tab: String,
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<Cell>,
    },
    /// The cell the person selected.
    SelectCell { tab: String, cell: String },
    /// Make that version of the cell current. One undo step.
    SetVersion { tab: String, cell: String, index: usize },
    /// Move the cell to that position. One undo step.
    MoveCell { tab: String, cell: String, to: usize },
    /// Delete the cell. One undo step. Deleting the last cell leaves an
    /// empty one.
    DeleteCell { tab: String, cell: String },
    /// A Markdown cell shows its source again. Not an undo step.
    ShowSource { tab: String, cell: String },
    /// Asks whether `path` is this application's own work.
    Claim { id: String, path: String },
    /// Render that item of one of this application's files as a file of
    /// `media_type`, under `render_path`. For a cell, `use` says whether its
    /// prompt or its result. The answer is `rendered`.
    Render {
        id: String,
        item: ItemRef,
        media_type: String,
        #[serde(default, rename = "use", skip_serializing_if = "Option::is_none")]
        part: Option<CellUse>,
    },
    /// The answer to `need`: the file, and the item's fingerprint; or
    /// `error`, a sentence.
    Provided {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fingerprint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The items of that file that are out of date, and which items they
    /// were made from changed. Sent whenever that set changes; an empty list
    /// when none are.
    Stale { path: String, items: Vec<StaleItem> },
    /// The answer to `pick`: the item the person chose, and which of the
    /// media types asked for it is, or none when they cancelled.
    Picked {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
}

/// What a selection message does, as the drawing surface's select tool does it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SelectOp {
    /// A click on an object. `add` toggles it in the selection.
    Click { node: u64, add: bool },
    /// A click on nothing.
    Clear,
    /// Everything a marquee touches, added to `base` when `add`.
    Marquee {
        min: [f64; 2],
        max: [f64; 2],
        add: bool,
        base: Vec<u64>,
    },
    /// Puts a selection back, as Escape during a drag does.
    Replace { nodes: Vec<u64> },
}

/// The camera sent with a document when a drawing is opened or created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentCamera {
    pub origin: [f64; 2],
    pub zoom: f64,
}

/// Which file dialog the host should open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Open,
    Save,
}

/// A shared component the host draws in a contents area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentsKind {
    Grid,
    /// The drawing surface: the grid, the document, and its tools.
    Canvas,
    /// A list of cells. The host draws them; the sub-application keeps them.
    Cells,
    /// An image of pixels the sub-application composites: see `RasterView`.
    Raster,
}

/// Where the properties panel sits. Only the right side has a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelSide {
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabReport {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsReport {
    pub tab: String,
    pub kind: ContentsKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelReport {
    pub tab: String,
    pub side: PanelSide,
}

/// Everything a sub-application has declared. The host draws only a
/// declaration that `accept` has returned `Ready` for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub tabs: Vec<TabReport>,
    pub contents: Vec<ContentsReport>,
    pub panels: Vec<PanelReport>,
    greeted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept {
    More,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: String,
}

impl Declaration {
    pub fn empty() -> Self {
        Self {
            tabs: Vec::new(),
            contents: Vec::new(),
            panels: Vec::new(),
            greeted: false,
        }
    }

    fn knows_tab(&self, id: &str) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    /// Adds one message. `Hello` with this crate's version must come first,
    /// and only once. A tab id is new; contents and a panel name a known tab,
    /// at most one of each per tab.
    pub fn accept(&mut self, message: AppMessage) -> Accept {
        if !self.greeted {
            return match message {
                AppMessage::Hello { protocol } if protocol == PROTOCOL_VERSION => {
                    self.greeted = true;
                    Accept::More
                }
                _ => Accept::Failed,
            };
        }
        match message {
            AppMessage::Hello { .. } => Accept::Failed,
            AppMessage::Tab { id, label } => {
                if self.knows_tab(&id) {
                    Accept::Failed
                } else {
                    self.tabs.push(TabReport { id, label });
                    Accept::More
                }
            }
            AppMessage::Contents { tab, kind } => {
                let vacant = self.contents.iter().all(|contents| contents.tab != tab);
                if self.knows_tab(&tab) && vacant {
                    self.contents.push(ContentsReport { tab, kind });
                    Accept::More
                } else {
                    Accept::Failed
                }
            }
            AppMessage::Panel { tab, side } => {
                let vacant = self.panels.iter().all(|panel| panel.tab != tab);
                if self.knows_tab(&tab) && vacant {
                    self.panels.push(PanelReport { tab, side });
                    Accept::More
                } else {
                    Accept::Failed
                }
            }
            // Later messages. They do not change the declaration the window draws.
            AppMessage::Document { .. }
            | AppMessage::File { .. }
            | AppMessage::Keys(_)
            | AppMessage::Toolbar(_)
            | AppMessage::Menu { .. }
            | AppMessage::Properties(_)
            | AppMessage::Tabs(_)
            | AppMessage::Canvas(_)
            | AppMessage::Raster(_)
            | AppMessage::Ask(_)
            | AppMessage::QuitReady
            | AppMessage::Pong
            | AppMessage::Tools { .. }
            | AppMessage::ToolResult(_)
            | AppMessage::Kept { .. }
            | AppMessage::Cells { .. }
            | AppMessage::RunCell { .. }
            | AppMessage::StopCell { .. }
            | AppMessage::Claimed { .. }
            | AppMessage::Items { .. }
            | AppMessage::Rendered { .. }
            | AppMessage::Need { .. }
            | AppMessage::Pick { .. }
            | AppMessage::Edits { .. }
            | AppMessage::Clipboard { .. } => Accept::More,
            AppMessage::End => Accept::Ready,
        }
    }
}

fn strip(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

pub fn parse_app_message(line: &str) -> Result<AppMessage, ParseError> {
    let line = strip(line);
    let refused = || ParseError {
        line: line.to_string(),
    };
    let message: AppMessage = serde_json::from_str(line).map_err(|_| refused())?;
    // A running cell is the host's to draw, never a sub-application's to say.
    if let AppMessage::Cells { cells, .. } = &message
        && cells.iter().any(|cell| cell.state == CellState::Running)
    {
        return Err(refused());
    }
    Ok(message)
}

pub fn parse_host_message(line: &str) -> Result<HostMessage, ParseError> {
    let line = strip(line);
    serde_json::from_str(line).map_err(|_| ParseError {
        line: line.to_string(),
    })
}

/// One line, including the trailing newline. Compact JSON escapes any
/// newline inside a string, so the message stays on one line.
pub fn encode_app_message(message: &AppMessage) -> String {
    let mut line = serde_json::to_string(message).expect("an app message always encodes");
    line.push('\n');
    line
}

pub fn encode_host_message(message: &HostMessage) -> String {
    let mut line = serde_json::to_string(message).expect("a host message always encodes");
    line.push('\n');
    line
}

/// The file that registers a sub-application with the host. Each one lives
/// in its own folder under `apps/`: `apps/<id>/manifest.json`.
pub const MANIFEST_FILE: &str = "manifest.json";

/// A new item's name, as Studio's conventions guide sets it: "Untitled" and
/// the item's word, such as "Untitled Image".
pub fn untitled(word: &str) -> String {
    format!("Untitled {word}")
}

/// `name`, then the names after it: "Untitled Image" gives "Untitled Image",
/// "Untitled Image 2", and on; "Beach 3" gives "Beach 3", "Beach 4". The
/// first is never numbered.
pub fn free_names(name: &str) -> impl Iterator<Item = String> {
    let (stem, first) = match name.rsplit_once(' ') {
        Some((stem, number)) if !stem.is_empty() => match number.parse::<u32>() {
            Ok(number) => (stem.to_string(), Some(number)),
            Err(_) => (name.to_string(), None),
        },
        _ => (name.to_string(), None),
    };
    let start = first.unwrap_or(1);
    (start..start + 10_000).map(move |number| match first {
        None if number == 1 => stem.clone(),
        _ => format!("{stem} {number}"),
    })
}

/// The first of `free_names(name)` that `taken` does not hold, such as among
/// a workspace's open tabs and the files in the project's folder.
pub fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    free_names(name).find(|candidate| !taken(candidate)).unwrap_or_else(|| name.to_string())
}

/// What the host reads about a sub-application before it starts the program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Lowercase letters, digits, and hyphens. It is the folder name under
    /// `apps/` and the value the host's `--app` option takes.
    pub id: String,
    /// The sub-application's name as people see it.
    pub name: String,
    /// The program's file name, found beside the host's executable.
    pub executable: String,
    /// The protocol version the program speaks. Must be `PROTOCOL_VERSION`.
    pub protocol: u32,
    /// Left out of the new-tab menu and not started with the others; only
    /// `--app` or a session's tab of it starts it. A workspace copied from
    /// the template drops it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The ids this sub-application had before, so what was stored under
    /// them, such as a session's tabs or its data folder, is its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formerly: Vec<String>,
    /// A folder of agent skills it ships, each a `SKILL.md` in a folder of
    /// its own, relative to the manifest. The host loads them beside its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError(pub String);

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reads a manifest and checks the rules the host relies on.
pub fn parse_manifest(text: &str) -> Result<Manifest, ManifestError> {
    let manifest: Manifest =
        serde_json::from_str(text).map_err(|error| ManifestError(error.to_string()))?;
    let id_ok = !manifest.id.is_empty()
        && manifest
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_ok {
        return Err(ManifestError(format!(
            "id {:?} must be lowercase letters, digits, and hyphens",
            manifest.id
        )));
    }
    if manifest.executable.is_empty() || manifest.executable.contains(['/', '\\']) {
        return Err(ManifestError(format!(
            "executable {:?} must be a file name, not a path",
            manifest.executable
        )));
    }
    if manifest.formerly.iter().any(|old| *old == manifest.id) {
        return Err(ManifestError(format!("formerly {:?} is its own id", manifest.id)));
    }
    if manifest.protocol != PROTOCOL_VERSION {
        return Err(ManifestError(format!(
            "protocol {} is not this host's version {PROTOCOL_VERSION}",
            manifest.protocol
        )));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a naming rule that numbers the first item, or skips a number:
    /// the conventions guide's "Untitled Image", then "Untitled Image 2".
    #[test]
    fn the_first_new_item_is_not_numbered_and_the_next_are() {
        let names: Vec<String> = free_names(&untitled("Image")).take(3).collect();
        assert_eq!(names, ["Untitled Image", "Untitled Image 2", "Untitled Image 3"]);
        let names: Vec<String> = free_names("Beach 3").take(2).collect();
        assert_eq!(names, ["Beach 3", "Beach 4"]);
        let taken = ["Untitled Recipe", "Untitled Recipe 2"];
        assert_eq!(free_name(&untitled("Recipe"), |name| taken.contains(&name)), "Untitled Recipe 3");
    }

    fn manifest_text(id: &str, executable: &str, protocol: u32) -> String {
        serde_json::json!({
            "id": id,
            "name": "Storyboard",
            "executable": executable,
            "protocol": protocol,
        })
        .to_string()
    }

    /// Catches a framing that splits on spaces and drops the rest of the label.
    #[test]
    fn tab_label_with_spaces_round_trips() {
        let label = "Canvas 1".to_string();
        let message = AppMessage::Tab {
            id: "canvas-1".to_string(),
            label: label.clone(),
        };
        let encoded = encode_app_message(&message);
        let line = encoded
            .strip_suffix('\n')
            .expect("encoded message ends with one newline");
        let parsed = parse_app_message(line).expect("label with spaces parses as one message");
        assert_eq!(parsed, message);

        let mut declaration = Declaration::empty();
        assert_eq!(
            declaration.accept(AppMessage::Hello { protocol: 1 }),
            Accept::More
        );
        assert_eq!(declaration.accept(parsed), Accept::More);
        assert_eq!(declaration.tabs[0].label, label);
    }

    /// Catches a declaration the host would draw without agreeing the version.
    #[test]
    fn declaration_fails_without_hello_version_1() {
        let mut declaration = Declaration::empty();
        assert_eq!(
            declaration.accept(AppMessage::Tab {
                id: "canvas-1".to_string(),
                label: "Canvas 1".to_string(),
            }),
            Accept::Failed
        );

        let mut declaration = Declaration::empty();
        assert_eq!(
            declaration.accept(AppMessage::Hello { protocol: 2 }),
            Accept::Failed
        );
    }

    /// Catches an id that is not lowercase letters, digits, and hyphens.
    #[test]
    fn manifest_refuses_id_outside_lowercase_digits_and_hyphens() {
        let text = manifest_text("Storyboard", "storyboard", 1);
        assert!(parse_manifest(&text).is_err(), "uppercase id was accepted");
    }

    /// Catches an executable that is a path.
    #[test]
    fn manifest_refuses_executable_path() {
        for executable in ["bin/storyboard", "bin\\storyboard"] {
            let text = manifest_text("storyboard", executable, 1);
            assert!(
                parse_manifest(&text).is_err(),
                "path executable {executable:?} was accepted"
            );
        }
    }

    /// Catches a protocol version other than 1.
    #[test]
    fn manifest_refuses_protocol_other_than_1() {
        let text = manifest_text("storyboard", "storyboard", 2);
        assert!(parse_manifest(&text).is_err(), "protocol 2 was accepted");
    }

    /// Catches a result sent under another type name than `result`, or a
    /// tool key that splits wrongly at an application id's hyphen.
    #[test]
    fn invoke_and_result_round_trip() {
        let key = tool_key("storyboard-generator", "make", "add_shape");
        assert_eq!(split_tool_key(&key), Some(("storyboard-generator", "make", "add_shape")));
        assert_eq!(split_tool_key("storyboard-generator__make"), None);

        let invoke = HostMessage::Invoke {
            call: "c1".to_string(),
            tab: "7".to_string(),
            tool: key,
            input: serde_json::json!({"kind": "ellipse"}),
            group: "r1".to_string(),
        };
        let line = encode_host_message(&invoke);
        assert_eq!(parse_host_message(&line).unwrap(), invoke);

        let result = AppMessage::ToolResult(ToolResult {
            call: "c1".to_string(),
            ok: true,
            output: serde_json::json!({"id": 12}),
            images: Vec::new(),
        });
        let line = encode_app_message(&result);
        assert!(line.contains("\"type\":\"result\""), "{line}");
        assert_eq!(parse_app_message(&line).unwrap(), result);
    }

    /// Catches a tab list from before 0.3, without `path` or `saves`, that
    /// no longer parses.
    #[test]
    fn tabs_without_workspace_fields_still_parse() {
        let line = r#"{"type":"tabs","tabs":[{"id":"1","label":"Untitled 1"}],"active":"1"}"#;
        let AppMessage::Tabs(tabs) = parse_app_message(line).unwrap() else {
            panic!("not a tab list");
        };
        assert_eq!(tabs.tabs[0].path, None);
        assert!(!tabs.tabs[0].saves);
    }

    /// Catches `open` without a path being refused: a workspace reopens a
    /// tab that has no file this way.
    #[test]
    fn open_without_path_parses() {
        assert_eq!(
            parse_host_message(r#"{"type":"open"}"#).unwrap(),
            HostMessage::Open { path: None }
        );
    }

    fn round_trip_app(message: AppMessage) {
        let line = encode_app_message(&message);
        assert_eq!(parse_app_message(&line).unwrap(), message);
    }

    fn round_trip_host(message: HostMessage) {
        let line = encode_host_message(&message);
        assert_eq!(parse_host_message(&line).unwrap(), message);
    }

    /// A cell whose prompt and result hold a file, and one of a kind this
    /// build does not run, both survive a round trip. Dropping either would
    /// lose a later kind.
    /// Catches a cells message from a later version refused whole: a part
    /// and a state this build does not know are kept, and the rest reads.
    #[test]
    fn a_later_part_and_state_still_read() {
        let line = r#"{"type":"cells","tab":"recipe","cells":[{"id":"x","kind":"text","input":[{"text":"north"},{"image_ref":3}],"state":"queued"}]}"#;
        let AppMessage::Cells { cells, .. } = parse_app_message(line).expect("a later cell was refused") else {
            panic!("not a cells message");
        };
        assert_eq!(cells[0].state, CellState::Other);
        assert_eq!(cells[0].input.len(), 2);
        assert!(matches!(cells[0].input[1], CellPart::Other(_)));
        assert_eq!(cells[0].prompt(), "north");
    }

    #[test]
    fn a_cell_with_a_path_and_an_unknown_kind_round_trip() {
        let path = Cell {
            id: "11111111-1111-4111-8111-111111111111".into(),
            from: Some("00000000-0000-4000-8000-000000000000".into()),
            kind: "text".into(),
            input: vec![
                CellPart::Text { text: "Describe this.".into() },
                CellPart::Path {
                    path: "/tmp/still.png".into(),
                    media_type: "image/png".into(),
                },
            ],
            endpoint: Some("openrouter".into()),
            model: Some("stand-in/model".into()),
            effort: Some("low".into()),
            output: vec![CellPart::Path {
                path: "/tmp/out.png".into(),
                media_type: "image/png".into(),
            }],
            state: CellState::Done,
            error: None,
            versions: vec![CellVersion {
                id: "33333333-3333-4333-8333-333333333333".into(),
                input: vec![CellPart::Text { text: "Describe this.".into() }],
                endpoint: Some("openrouter".into()),
                model: Some("stand-in/model".into()),
                effort: Some("low".into()),
                output: vec![CellPart::Path { path: "/tmp/out.png".into(), media_type: "image/png".into() }],
                state: CellState::Done,
                error: None,
                time: 1_793_146_000,
                settings: None,
                used: Vec::new(),
            }],
            settings: None,
            current: Some(0),
        };
        let unknown = Cell {
            id: "22222222-2222-4222-8222-222222222222".into(),
            from: None,
            kind: "plot".into(),
            input: vec![CellPart::Text { text: "a rising line".into() }],
            endpoint: None,
            model: None,
            effort: None,
            output: Vec::new(),
            state: CellState::Idle,
            error: None,
            versions: Vec::new(),
            settings: None,
            current: None,
        };
        round_trip_app(AppMessage::Cells {
            tab: "recipe".into(),
            cells: vec![path.clone(), unknown.clone()],
            selected: Some(unknown.id.clone()),
        });
        round_trip_host(HostMessage::InsertCell {
            tab: "recipe".into(),
            index: 1,
            cell: Some(unknown),
        });
        round_trip_host(HostMessage::InsertCell { tab: "recipe".into(), index: 2, cell: None });
        round_trip_host(HostMessage::SetVersion { tab: "recipe".into(), cell: path.id.clone(), index: 0 });
        round_trip_host(HostMessage::MoveCell { tab: "recipe".into(), cell: path.id.clone(), to: 1 });
        round_trip_host(HostMessage::DeleteCell { tab: "recipe".into(), cell: path.id.clone() });
        round_trip_host(HostMessage::ShowSource { tab: "recipe".into(), cell: path.id.clone() });
        round_trip_host(HostMessage::CellResult {
            tab: "recipe".into(),
            cell: path.id.clone(),
            input: path.input.clone(),
            endpoint: "openrouter".into(),
            model: "stand-in/model".into(),
            effort: Some("high".into()),
            state: CellState::Failed,
            output: path.output.clone(),
            error: Some("The model failed.".into()),
            settings: Some(serde_json::json!({"seconds": 4})),
            used: vec![UsedItem { item: ItemRef { path: "Beach.ipx".into(), item: Some("2".into()) }, fingerprint: "00ff".into() }],
            at: Some(ItemRef { path: "Shots.vid".into(), item: Some("c1".into()) }),
        });
        assert!(parse_app_message(r#"{"type":"cells","tab":"recipe","cells":[{"id":"x","kind":"plot","state":"running"}]}"#).is_err());
    }

    /// Catches the rename of 2026-11-05 stranding a sub-application or a
    /// stored line that still says `skills`: it reads as `tools`.
    #[test]
    fn the_stored_spelling_of_tools_still_reads() {
        let old = r#"{"type":"skills","skills":[{"id":"quill","name":"Quill","description":"Stamps.","tools":[{"name":"stamp","description":"Stamp.","input":{"type":"object"}}]}]}"#;
        let Ok(AppMessage::Tools { groups }) = parse_app_message(old) else {
            panic!("the stored spelling did not read");
        };
        assert_eq!(groups[0].tools[0].name, "stamp");
        assert!(groups[0].tools[0].implements.is_empty());
        round_trip_app(AppMessage::Tools { groups });
    }

    /// Every message of the lifecycle's contract round-trips, so a field
    /// cannot be dropped quietly.
    #[test]
    fn item_messages_round_trip() {
        let board = ItemRef { path: "Pilot.smx".into(), item: Some("b3".into()) };
        let reference = ItemRef { path: "Cast.ipx".into(), item: Some("7".into()) };
        round_trip_app(AppMessage::Items {
            path: "Shots.vid".into(),
            items: vec![Item {
                id: "s1".into(),
                name: "Shot 1".into(),
                media_types: vec!["video/mp4".into()],
                fingerprint: "a1".into(),
                made: Some(Made {
                    cell: Some(ItemRef { path: "Shots.vid".into(), item: Some("c1".into()) }),
                    version: Some("v1".into()),
                    used: vec![UsedItem { item: board.clone(), fingerprint: "b1".into() }],
                    sources: vec![UsedItem { item: reference.clone(), fingerprint: "r1".into() }],
                }),
            }],
        });
        round_trip_app(AppMessage::Items { path: "Gone.vid".into(), items: Vec::new() });
        round_trip_app(AppMessage::Rendered { id: "r".into(), path: Some(".inpainter/renders/x/b3-b1.png".into()), fingerprint: Some("b1".into()), error: None });
        round_trip_app(AppMessage::Rendered { id: "r".into(), path: None, fingerprint: None, error: Some("No such board.".into()) });
        round_trip_app(AppMessage::Need { id: "n".into(), item: board.clone(), media_type: "image/png".into() });
        round_trip_app(AppMessage::Pick { id: "p".into(), media_types: vec!["image/png".into()] });
        round_trip_app(AppMessage::RunCell {
            tab: String::new(),
            cell: "c1".into(),
            content: Some(Cell::text("c1", "A wave.")),
            at: Some(ItemRef { path: "Shots.vid".into(), item: Some("c1".into()) }),
        });
        round_trip_host(HostMessage::Render { id: "r".into(), item: board.clone(), media_type: "image/png".into(), part: None });
        round_trip_host(HostMessage::Render { id: "r".into(), item: board.clone(), media_type: "text/plain".into(), part: Some(CellUse::Prompt) });
        round_trip_host(HostMessage::Provided { id: "n".into(), path: Some("x.png".into()), fingerprint: Some("b1".into()), error: None });
        round_trip_host(HostMessage::Stale { path: "Shots.vid".into(), items: vec![StaleItem { id: "s1".into(), changed: vec![reference.clone()] }] });
        round_trip_host(HostMessage::Stale { path: "Shots.vid".into(), items: Vec::new() });
        round_trip_host(HostMessage::Picked { id: "p".into(), item: Some(reference), media_type: Some("image/png".into()) });
        round_trip_host(HostMessage::Picked { id: "p".into(), item: None, media_type: None });
        // An item part reads as an item, not as a path or an unknown part.
        let part: CellPart = serde_json::from_str(r#"{"item":{"path":"Pilot.smx","item":"b3"},"media_type":"image/png","use":"result"}"#).unwrap();
        assert_eq!(part, CellPart::Item { item: board, media_type: "image/png".into(), part: Some(CellUse::Result) });
    }

    /// Every cell message round-trips, so a field cannot be dropped quietly.
    #[test]
    fn cell_messages_round_trip() {
        round_trip_app(AppMessage::RunCell {
            tab: "recipe".into(),
            cell: "c1".into(),
            content: None,
            at: None,
        });
        round_trip_app(AppMessage::StopCell {
            tab: "recipe".into(),
            cell: "c1".into(),
        });
        round_trip_app(AppMessage::Edits { media_types: vec!["image/png".into(), "image/jpeg".into()] });
        round_trip_host(HostMessage::RasterText { tab: "1".into(), input: TextInput::Insert { text: "Hi".into() } });
        round_trip_host(HostMessage::RasterText { tab: "1".into(), input: TextInput::Key { key: TextKey::Left, shift: true, word: false } });
        round_trip_host(HostMessage::RasterText { tab: "1".into(), input: TextInput::Place { at: [1.0, 2.0], extend: true } });
        round_trip_app(AppMessage::Clipboard { text: "Hi".into() });
        round_trip_host(HostMessage::OpenAsNew {
            path: "/p/images/a.png".into(),
            name: "A girl on a beach".into(),
            from: Some(OpenedFrom { cell: "c1".into(), image: "images/a.png".into(), session: Some("s1".into()), recipe: None }),
        });
        round_trip_app(AppMessage::Claimed {
            id: "claim-1".into(),
            yours: true,
        });
        round_trip_host(HostMessage::EditCell {
            tab: "recipe".into(),
            cell: "c1".into(),
            input: vec![CellPart::Text { text: "A lighthouse.".into() }],
        });
        round_trip_host(HostMessage::SelectCell {
            tab: "recipe".into(),
            cell: "c1".into(),
        });
        round_trip_host(HostMessage::Claim {
            id: "claim-1".into(),
            path: "/tmp/recipe.smx".into(),
        });
    }

    /// `project` and `kept`, including a failed `keep`, survive a round trip.
    #[test]
    fn project_and_kept_round_trip() {
        round_trip_host(HostMessage::Project {
            folder: "/home/someone/Inpainter/Lighthouse".into(),
        });
        round_trip_app(AppMessage::Kept {
            tab: "1".into(),
            path: Some("/home/someone/Inpainter/Lighthouse/Untitled 1.smx".into()),
            error: None,
        });
        round_trip_app(AppMessage::Kept {
            tab: "1".into(),
            path: None,
            error: None,
        });
        round_trip_app(AppMessage::Kept {
            tab: "1".into(),
            path: Some("/home/someone/Inpainter/Lighthouse/Untitled 1.smx".into()),
            error: Some("This location is read-only.".into()),
        });
    }
}
