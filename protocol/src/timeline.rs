//! The timeline, the stage, and notices: what a workspace declares so the
//! host can draw a timeline under its contents, a decoded frame and other
//! drawings on its stage, and progress cards, without knowing the workspace.
//!
//! A timeline is a shared component, as the properties panel is: a tab that
//! declares one gets it docked under its contents, whatever their kind. The
//! host previews each gesture from the declaration and sends one
//! `timeline_gesture` when it ends; dragging the playhead sends `seek`
//! while it is held, as a held slider sends `preview`. Menus, toggles, and
//! buttons send their actions as `command`. The contract is the lifecycle's
//! `systems.md` in the team repository.

use serde::{Deserialize, Serialize};

use crate::{Action, MenuEntry, Thumbnail, ToolItem};

fn is_false(value: &bool) -> bool {
    !*value
}

/// A tab's timeline, replaced whole on each change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub tab: String,
    /// Items above the rows, as the toolbar draws them: play and pause,
    /// step, the timecode as a label, volume, snapping, zoom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header: Vec<ToolItem>,
    /// Underlined page tabs, such as Timeline and Dope Sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<TimelinePage>,
    /// A way back out, with its breadcrumb, such as from inside a clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<TimelineBack>,
    pub ruler: Ruler,
    pub rows: Vec<TimelineRow>,
    /// The frame shown.
    pub playhead: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<TimelineRange>,
    /// The button that adds a row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_row: Option<Action>,
    /// Muted words drawn when there are no rows or nothing on them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub empty_hint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelinePage {
    pub label: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineBack {
    /// From the outermost level to the one shown, such as ["Film", "Shot 12"].
    pub path: Vec<String>,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ruler {
    pub fps: f64,
    /// Frames from the start to the end.
    pub length: u32,
    pub unit: RulerUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulerUnit {
    Frames,
    Timecode,
}

/// One row: a track of the film, a layer of a clip, or a property of the
/// dope sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineRow {
    pub id: String,
    pub name: String,
    pub color: [u8; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub indent: u8,
    /// Open or closed, for a row with rows under it; none for one without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// What the disclosure sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disclose: Option<Action>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    /// Lock, eye, mute, outline, and the like.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toggles: Vec<RowToggle>,
    /// Shown, never edited, such as the film's sound inside a clip.
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// The right-click menu on the row's empty space.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menu: Vec<MenuEntry>,
    /// What a drop onto items already on this row does; Shift does the other.
    #[serde(default)]
    pub drop: DropRule,
    #[serde(flatten)]
    pub content: RowContent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowToggle {
    pub icon: String,
    pub tip: String,
    pub on: bool,
    pub action: Action,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropRule {
    /// The dropped frames replace those under them.
    #[default]
    Replace,
    /// Later items move later to make room.
    Push,
}

/// What a row holds: spans, as a track's clips, or frame cells, as a layer's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "content", rename_all = "snake_case")]
pub enum RowContent {
    Items { items: Vec<TimelineItem> },
    Cells {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        marks: Vec<CellMark>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        spans: Vec<CellSpan>,
    },
}

/// One span on a row, such as a clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: String,
    pub start: u32,
    pub length: u32,
    pub fill: [u8; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(default)]
    pub style: ItemStyle,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    /// A small image, such as a drawing's or a rendered shot's first frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Thumbnail>,
    /// A sound's peaks, 0 to 1, spread over the item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waveform: Vec<f32>,
    /// Work under way on it, 0 to 1, drawn with a spinner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    /// An icon at its end, such as out of date, with its tooltip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_mark: Option<EndMark>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tip: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menu: Vec<MenuEntry>,
    /// What a double-click sends, such as opening a clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<Action>,
    /// Whether it can be dragged and its edges moved.
    #[serde(default = "yes")]
    pub movable: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub rgb: [u8; 3],
    pub width: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStyle {
    #[default]
    Solid,
    Dashed,
    /// A lighter copy, such as a loop's repeats.
    Ghost,
    /// An empty stretch, hatched gray.
    Gap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndMark {
    pub icon: String,
    pub tip: String,
}

/// A mark at one frame of a layer's or a property's row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellMark {
    pub frame: u32,
    #[serde(flatten)]
    pub kind: MarkKind,
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mark", rename_all = "snake_case")]
pub enum MarkKind {
    /// A filled dot.
    Key,
    /// An empty circle.
    BlankKey,
    /// The rectangle at a hold's last frame.
    HoldEnd,
    /// A flag with words.
    Label { text: String },
    /// `//` with words.
    Comment { text: String },
    /// A dope sheet's diamond.
    Diamond,
}

/// A run of frames on a layer's or a property's row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellSpan {
    pub start: u32,
    pub length: u32,
    pub kind: SpanKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    Hold,
    /// An arrow from key to key.
    Tween,
    /// A tween that cannot be made, as a dashed line.
    BrokenTween,
    /// A dope sheet's bar joining equal keys.
    HoldBar,
}

/// A range on the ruler with a handle at each end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineRange {
    pub id: String,
    pub kind: RangeKind,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeKind {
    /// Onion skin, its earlier handle red and its later green.
    OnionSkin,
    /// A loop, as an orange bar.
    Loop,
}

/// One item, or one row's frame, a gesture acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelinePick {
    pub row: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u32>,
}

/// One finished gesture on a timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "gesture", rename_all = "snake_case")]
pub enum TimelineGesture {
    /// A click on items or marks; `add` with Shift, `toggle` with Ctrl.
    Select {
        picks: Vec<TimelinePick>,
        #[serde(default, skip_serializing_if = "is_false")]
        add: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        toggle: bool,
    },
    /// A block of cells across rows, from frame `start` to `end`.
    SelectCells { rows: Vec<String>, start: u32, end: u32 },
    /// The selected items or cells moved by `frames`, and to `row` when it
    /// changed. `push` when the row's other rule was chosen with Shift.
    Move {
        picks: Vec<TimelinePick>,
        frames: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        push: bool,
    },
    /// One edge of an item moved by `frames`.
    Resize { pick: TimelinePick, edge: Edge, frames: i64 },
    /// A range's ends.
    Range { id: String, start: u32, end: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Start,
    End,
}

/// What a canvas tab's stage shows besides its drawing, replaced whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub tab: String,
    /// A frame the workspace decoded, drawn in the field, as raster tabs
    /// write theirs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Thumbnail>,
    /// A plain colour in the field instead, such as black for a gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<[u8; 3]>,
    /// `[x, y, width, height]` in the drawing's units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<[f64; 4]>,
    /// What lies outside the field is drawn under a gray veil.
    #[serde(default, skip_serializing_if = "is_false")]
    pub veil: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub underlays: Vec<Underlay>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<Overlay>,
}

/// Another drawing under the active one, never edited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Underlay {
    /// A drawing, as `document` carries one.
    pub drawing: serde_json::Value,
    pub tint: [u8; 3],
    pub opacity: f32,
    #[serde(default)]
    pub offset: [f64; 2],
    /// Degrees.
    #[serde(default)]
    pub rotation: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// A mark on the stage that is not part of the drawing, in its units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "overlay", rename_all = "snake_case")]
pub enum Overlay {
    Rect {
        min: [f64; 2],
        max: [f64; 2],
        #[serde(default, skip_serializing_if = "is_false")]
        dashed: bool,
    },
    /// A dotted path, with a dot at each point.
    Path { points: Vec<[f64; 2]> },
    Label { at: [f64; 2], text: String },
    /// Everything outside this rectangle dimmed.
    Dim { min: [f64; 2], max: [f64; 2] },
}

/// A card at the stage's bottom right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    pub state: NoticeState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NoticeAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeState {
    Working,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoticeAction {
    pub label: String,
    pub action: Action,
}
