//! The coverage check: everything a person can do to a storyboard's contents by
//! hand has a tool the assistant can call.
//!
//! It reads Storyboard Generator's own hand surface: its command list
//! (`registry::commands`), every action its toolbar, menu, keys, panel,
//! outliner, and canvas declare, the properties its panel shows on every
//! object and color page, the drawing surface's gesture kinds, and the
//! select tool's operations. Each one that changes a storyboard's contents must
//! map to a tool, and an argument, that the declared skills offer. The rest
//! are named below as left out, each with the reason from systems' "Left
//! out": undo and redo; opening, saving, and closing files; recent files;
//! zoom, fit, and the window; the panel; and the drawing tools' own
//! settings. A new command, action, property, gesture, or selection
//! operation fails this check until it gains a tool or joins that list on
//! purpose. The matches over commands, properties, gestures, and selection
//! operations have no catch-all, so a new one does not compile until it is
//! placed.

use draw::PropId;
use draw::document::{ColorId, Dynamics, NodeId, ShapeKind, TextBox};
use draw::gesture::GestureKind;
use protocol::{AppMessage, HostMessage, SelectOp, ToolGroup};
use serde_json::{Value, json};
use storyboard_generator::registry::{self, CommandId};
use storyboard_generator::serve::Server;
use std::collections::{BTreeMap, BTreeSet};

/// Where one part of the hand surface went.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// The tool `skill__tool`, and the input field and value that select
    /// this part, when the tool covers several.
    Tool(&'static str, Option<(&'static str, &'static str)>),
    /// Several tools between them.
    Tools(&'static [&'static str]),
    /// Left out, and why: not a storyboard's contents, or the person's own. The
    /// reason is for the people reading this list.
    Left(#[allow(dead_code)] &'static str),
}

const UNDO: &str = "undo and redo stay with the person";
const FILES: &str = "opening, saving, and closing files";
const RECENT: &str = "recent files";
const VIEW: &str = "zoom, fit, and the window";
const PANEL: &str = "the panel: which page it shows";
const TOOLS: &str = "the drawing tools' own settings";
const LAYOUT: &str = "the host's text layout report, not a hand action";
const PLAYBACK: &str = "playing the film, which changes nothing in the storyboard";
const NOTICE: &str = "closing a card, which changes nothing in the storyboard";

const TEXT_STYLE: Place = Place::Tool("style__set_object_properties", None);

/// Every command of `registry::commands`.
fn command(id: CommandId) -> Place {
    use Place::{Left, Tool};
    match id {
        CommandId::NewStoryboard | CommandId::Open | CommandId::Save | CommandId::SaveAs | CommandId::SaveCopy | CommandId::Close => Left(FILES),
        CommandId::ClearRecent => Left(RECENT),
        CommandId::Undo | CommandId::Redo => Left(UNDO),
        CommandId::ZoomIn | CommandId::ZoomOut | CommandId::Fit | CommandId::ZoomToSelection | CommandId::Maximize => Left(VIEW),
        CommandId::TogglePanel => Left(PANEL),
        // On the text being worked on, the Size keys change its font size,
        // which set_object_properties sets; otherwise they set the tool's size.
        CommandId::SizeDown | CommandId::SizeUp => TEXT_STYLE,
        CommandId::Cut => Tool("edit__clipboard", Some(("action", "cut"))),
        CommandId::Copy => Tool("edit__clipboard", Some(("action", "copy"))),
        CommandId::Paste => Tool("edit__clipboard", Some(("action", "paste"))),
        CommandId::Delete => Tool("edit__delete", None),
        CommandId::NudgeLeft => Tool("edit__nudge", Some(("direction", "left"))),
        CommandId::NudgeRight => Tool("edit__nudge", Some(("direction", "right"))),
        CommandId::NudgeUp => Tool("edit__nudge", Some(("direction", "up"))),
        CommandId::NudgeDown => Tool("edit__nudge", Some(("direction", "down"))),
        CommandId::Group => Tool("edit__group", None),
        CommandId::Ungroup => Tool("edit__ungroup", None),
        CommandId::BringForward => Tool("edit__arrange", Some(("order", "bring_forward"))),
        CommandId::SendBackward => Tool("edit__arrange", Some(("order", "send_backward"))),
        CommandId::BringToFront => Tool("edit__arrange", Some(("order", "bring_to_front"))),
        CommandId::SendToBack => Tool("edit__arrange", Some(("order", "send_to_back"))),
        CommandId::AlignLeft => Tool("edit__align", Some(("edge", "left"))),
        CommandId::AlignCenter => Tool("edit__align", Some(("edge", "center"))),
        CommandId::AlignRight => Tool("edit__align", Some(("edge", "right"))),
        CommandId::RotateClockwise => Tool("edit__turn", Some(("turn", "rotate_clockwise"))),
        CommandId::RotateCounterclockwise => Tool("edit__turn", Some(("turn", "rotate_counterclockwise"))),
        CommandId::RotateHalf => Tool("edit__turn", Some(("turn", "rotate_half"))),
        CommandId::FlipHorizontal => Tool("edit__turn", Some(("turn", "flip_horizontal"))),
        CommandId::FlipVertical => Tool("edit__turn", Some(("turn", "flip_vertical"))),
    }
}

/// Every declared action that is not a command's own id.
fn action(id: &str) -> Option<Place> {
    use Place::{Left, Tool, Tools};
    Some(match id {
        "select_click" => Tool("edit__select", None),
        "rename_node" | "toggle_node" => Tool("style__set_object_properties", None),
        "rename_color" | "toggle_color" => Tool("style__set_color_properties", None),
        "layer_action" => Tools(&["edit__duplicate_colour", "edit__delete_colour"]),
        "reorder" => Tool("edit__move_object", None),
        "sort_color" => Tool("edit__move_colour", None),
        "edit_text" | "text_edit" | "commit_text" => Tools(&["draw__add_text", "style__set_text"]),
        "draft_box" => Tool("draw__add_text", None),
        // On the text being worked on, these toolbar controls set its size,
        // alignment, padding, radius, and list; otherwise the tool's.
        "set_size" | "set_bar" | "bar_choice" | "toggle_list" | "choose_list" => TEXT_STYLE,
        "select_tool" | "swatch_click" | "swatch_action" | "sort_brush" | "make_palette_default" | "finish_edit" => Left(TOOLS),
        "object_properties" | "object_action" | "tool_action" | "open_palette_color" | "open_color" | "select_color" | "go_back" => Left(PANEL),
        "text_metrics" => Left(LAYOUT),
        "open_recent" => Left(RECENT),
        "select_tab" | "close_tab" | "close" => Left(FILES),
        "canvas.zoom_in" | "canvas.zoom_out" | "canvas.fit" | "canvas.zoom_to_selection" | "window.maximize" => Left(VIEW),
        // The film's timeline, its menus, and its header.
        "new_clip" => Tool("clips__add_clip", None),
        "edit_clip" => Tool("clips__edit_clip", None),
        "leave_clip" => Tool("clips__leave_clip", None),
        "rename_clip" => Tool("clips__set_clip", None),
        "duplicate_clip" => Tool("clips__duplicate_clip", None),
        "delete_clip" => Tool("clips__delete_clip", None),
        "step_back" | "step_forward" => Tool("clips__go_to_frame", None),
        "toggle_onion" => Tool("clips__set_onion_skin", None),
        "insert_frame" => Tool("clips__insert_frames", None),
        "insert_keyframe" | "insert_blank_keyframe" => Tool("clips__insert_keyframe", None),
        "clear_keyframe" => Tool("clips__clear_keyframe", None),
        "remove_frames" => Tool("clips__remove_frames", None),
        "copy_frames" => Tool("clips__copy_frames", None),
        "paste_frames" => Tool("clips__paste_frames", None),
        "reverse_frames" => Tool("clips__reverse_frames", None),
        "add_mark" => Tool("clips__add_mark", None),
        "new_layer" | "new_folder" => Tool("clips__add_layer", None),
        "delete_layer" => Tool("clips__delete_layer", None),
        "layer_flag" => Tool("clips__set_layer", None),
        // The film's sound, its tracks, and the animatic.
        "import_sound" => Tool("clips__import_sound", None),
        "insert_image_key" => Tool("clips__insert_image_key", None),
        "publish_hold" => Tool("clips__publish_hold", None),
        "save_timing" => Tool("clips__save_timing", None),
        "follow_retime" => Tool("clips__follow_retime", None),
        "new_sound_track" => Tool("clips__add_sound_track", None),
        "rename_sound_track" => Tool("clips__rename_sound_track", None),
        "delete_sound_track" => Tool("clips__delete_sound_track", None),
        "delete_sound" => Tool("clips__delete_sound", None),
        "track_flag" => Tool("clips__set_track", None),
        "export_animatic" => Tool("clips__export_animatic", None),
        "close_notice" => Left(NOTICE),
        "play" | "toggle_loop" => Left(PLAYBACK),
        "set_volume" | "finish_volume" => Tool("clips__set_listening_volume", None),
        "timeline_page" => Left(PANEL),
        _ => return None,
    })
}

/// Every property. An object's and a color's are set by their own tools.
fn property(id: PropId) -> Place {
    use Place::{Left, Tool, Tools};
    const BOTH: &[&str] = &["style__set_object_properties", "style__set_color_properties"];
    match id {
        PropId::Name | PropId::Visible | PropId::Locked => Tools(BOTH),
        PropId::Padding
        | PropId::CornerRadius
        | PropId::Family
        | PropId::FontSize
        | PropId::Bold
        | PropId::Italic
        | PropId::Underline
        | PropId::Align
        | PropId::VAlign
        | PropId::List
        | PropId::TextPadding
        | PropId::TextRadius
        | PropId::LineHeight
        | PropId::LetterSpacing
        | PropId::AllCaps
        | PropId::AutoSize
        | PropId::WrapText => Tool("style__set_object_properties", None),
        PropId::Hex | PropId::Rgb | PropId::Opacity | PropId::ColorSize | PropId::Dash | PropId::Cap => {
            Tool("style__set_color_properties", None)
        }
        PropId::Preset
        | PropId::PressureSize
        | PropId::Curve
        | PropId::Smoothing
        | PropId::Stabilization
        | PropId::Streamline
        | PropId::EraserSize
        | PropId::LockSize
        | PropId::Constrain
        | PropId::Snap => Left(TOOLS),
        // A list of the object's colors; each is set with set_color_properties.
        PropId::Palette => Left(PANEL),
    }
}

/// Every gesture kind of the drawing surface.
fn gesture(kind: &GestureKind) -> Place {
    use Place::Tool;
    match kind {
        GestureKind::Brush { .. } => Tool("draw__draw_stroke", None),
        GestureKind::Erase { .. } => Tool("draw__erase", None),
        GestureKind::Shape { kind: ShapeKind::Rectangle, .. } => Tool("draw__add_shape", Some(("kind", "rectangle"))),
        GestureKind::Shape { kind: ShapeKind::Ellipse, .. } => Tool("draw__add_shape", Some(("kind", "ellipse"))),
        GestureKind::Shape { kind: ShapeKind::Line, .. } => Tool("draw__add_shape", Some(("kind", "line"))),
        GestureKind::Text { .. } => Tool("draw__add_text", None),
        GestureKind::Transform { .. } => Tool("draw__transform", None),
        GestureKind::TextFrame { .. } => Tool("draw__set_text_box", None),
    }
}

/// The kinds of row a timeline gesture can land on: the film's Shots track,
/// a Sound track, or a layer inside a clip.
#[derive(Clone, Copy, Debug)]
enum RowKind {
    Shots,
    Sound,
    Layer,
}

impl RowKind {
    /// The kind of a declared row, by its id: `Some(None)` for the clip's
    /// read-only Film sound row, which takes no gesture, and `None` for a
    /// row of a kind this check does not know yet.
    fn of(id: &str) -> Option<Option<Self>> {
        Some(match id {
            "shots" => Some(RowKind::Shots),
            "film-sound" => None,
            _ if id.starts_with("sound-") => Some(RowKind::Sound),
            _ if id.parse::<u64>().is_ok() => Some(RowKind::Layer),
            _ => return None,
        })
    }

    /// A row id of this kind, as the Storyboard Generator declares them.
    fn row(self) -> &'static str {
        match self {
            RowKind::Shots => "shots",
            RowKind::Sound => "sound-1",
            RowKind::Layer => "1",
        }
    }
}

/// Every gesture on the timeline, by what it acts on and the kind of row it
/// is on: there is no catch-all, so a new kind of gesture or row does not
/// compile until it is placed.
fn timeline_gesture(gesture: &protocol::TimelineGesture, row: RowKind) -> Place {
    use Place::{Tool, Tools};
    use protocol::TimelineGesture as G;
    match (gesture, row) {
        (G::Select { .. }, RowKind::Shots) => Tool("clips__select_clips", None),
        (G::Select { .. }, RowKind::Sound) => Tool("clips__select_sounds", None),
        (G::Select { .. }, RowKind::Layer) => Tools(&["clips__select_layer", "clips__go_to_frame", "clips__select_frames"]),
        (G::SelectCells { .. }, _) => Tool("clips__select_frames", None),
        (G::Move { push: false, .. }, RowKind::Shots) | (G::Move { push: true, .. }, RowKind::Shots) => Tool("clips__move_clips", None),
        (G::Move { push: false, .. }, RowKind::Sound) | (G::Move { push: true, .. }, RowKind::Sound) => Tool("clips__move_sound", None),
        (G::Move { push: false, .. }, RowKind::Layer) | (G::Move { push: true, .. }, RowKind::Layer) => Tool("clips__move_frames", None),
        (G::Resize { .. }, RowKind::Shots) => Tool("clips__retime_clip", None),
        (G::Resize { .. }, RowKind::Sound) => Tool("clips__trim_sound", None),
        (G::Resize { .. }, RowKind::Layer) => Tool("clips__stretch_hold", None),
        (G::Range { .. }, _) => Tool("clips__set_onion_skin", None),
    }
}

/// One of each gesture on a row of `kind`: within it and onto another
/// row, with and without Shift, and both edges.
fn timeline_gestures(kind: RowKind) -> Vec<protocol::TimelineGesture> {
    use protocol::{Edge, TimelineGesture as G, TimelinePick};
    let row = kind.row();
    let pick = TimelinePick { row: row.into(), item: Some("1".into()), frame: Some(0) };
    let mut out = vec![G::Select { picks: vec![pick.clone()], add: false, toggle: false }];
    for push in [false, true] {
        out.push(G::Move { picks: vec![pick.clone()], frames: 1, row: None, push });
        out.push(G::Move { picks: vec![pick.clone()], frames: 1, row: Some(row.into()), push });
    }
    out.push(G::Resize { pick: pick.clone(), edge: Edge::Start, frames: 1 });
    out.push(G::Resize { pick, edge: Edge::End, frames: 1 });
    if let RowKind::Layer = kind {
        out.push(G::SelectCells { rows: vec![row.into()], start: 0, end: 1 });
        out.push(G::Range { id: "onion".into(), start: 0, end: 1 });
    }
    out
}

/// That every timeline gesture on every kind of row has a tool.
fn gestures_offered(tools: &BTreeMap<String, Value>) -> Result<(), String> {
    for kind in [RowKind::Shots, RowKind::Sound, RowKind::Layer] {
        for gesture in timeline_gestures(kind) {
            offered(tools, &format!("the timeline gesture {gesture:?} on a {kind:?} row"), timeline_gesture(&gesture, kind))?;
        }
    }
    Ok(())
}

/// Where each row of the film's pages and the onion skin's settings is set:
/// the tool, and the input key that takes its value.
fn film_row(kind: &str, field: &str) -> Option<(&'static str, &'static str)> {
    Some(match (kind, field) {
        ("clip", "name") => ("clips__set_clip", "name"),
        ("clip", "shot") => ("clips__set_clip", "shot"),
        ("clip", "action") => ("clips__set_clip", "action"),
        ("clip", "dialogue") => ("clips__set_clip", "dialogue"),
        ("clip", "camera") => ("clips__set_clip", "camera"),
        ("clip", "animator") => ("clips__set_clip", "animator"),
        ("clip", "length") => ("clips__retime_clip", "frames"),
        ("storyboard", "title") => ("clips__set_storyboard", "title"),
        ("storyboard", "scene") => ("clips__set_storyboard", "scene"),
        ("storyboard", "width") => ("clips__set_storyboard", "width"),
        ("storyboard", "height") => ("clips__set_storyboard", "height"),
        ("storyboard", "paper") => ("clips__set_storyboard", "paper"),
        ("layer", "name") => ("clips__set_layer", "name"),
        ("layer", "color") => ("clips__set_layer", "color"),
        ("layer", "opacity") => ("clips__set_layer", "opacity"),
        ("onion", "before") => ("clips__set_onion_skin", "before"),
        ("onion", "after") => ("clips__set_onion_skin", "after"),
        ("onion", "opacity") => ("clips__set_onion_skin", "opacity"),
        ("onion", "fade") => ("clips__set_onion_skin", "fade"),
        ("onion", "keys_only") => ("clips__set_onion_skin", "keys_only"),
        ("sound", "volume") => ("clips__set_sound_volume", "volume"),
        ("sound", "at") => ("clips__trim_sound", "at"),
        ("sound", "from") => ("clips__trim_sound", "from"),
        ("track", "name") => ("clips__rename_sound_track", "name"),
        _ => return None,
    })
}

/// Every operation of the select tool.
fn select(op: &SelectOp) -> Place {
    use Place::Tool;
    match op {
        SelectOp::Click { add: false, .. } | SelectOp::Replace { .. } => Tool("edit__select", Some(("mode", "replace"))),
        SelectOp::Click { add: true, .. } => Tool("edit__select", Some(("mode", "add"))),
        SelectOp::Clear => Tool("edit__select", Some(("mode", "clear"))),
        SelectOp::Marquee { .. } => Tool("edit__select", Some(("mode", "replace"))),
    }
}

fn gesture_kinds() -> Vec<GestureKind> {
    let frame = TextBox { origin: [0.0, 0.0], width: None, height: None };
    let mut kinds = vec![
        GestureKind::Brush { canvas: None, source: ColorId::new(1), layer: None, size: None, dynamics: Dynamics::round() },
        GestureKind::Erase { canvas: NodeId::new(1), size: 1, dynamics: Dynamics::round() },
        GestureKind::Text { origin: [0.0, 0.0], pointer: [0.0, 0.0], hit: None },
        GestureKind::Transform { before: Vec::new(), after: Vec::new() },
        GestureKind::TextFrame { node: NodeId::new(1), before: frame, after: frame },
    ];
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Line] {
        kinds.push(GestureKind::Shape { kind, origin: [0.0, 0.0], pointer: [0.0, 0.0], shift: false });
    }
    kinds
}

fn select_ops() -> Vec<SelectOp> {
    vec![
        SelectOp::Click { node: 1, add: false },
        SelectOp::Click { node: 1, add: true },
        SelectOp::Clear,
        SelectOp::Marquee { min: [0.0, 0.0], max: [1.0, 1.0], add: false, base: Vec::new() },
        SelectOp::Replace { nodes: Vec::new() },
    ]
}

/// The declared tools, by `skill__tool`, with their input schemas.
fn declared(skills: &[ToolGroup]) -> BTreeMap<String, Value> {
    skills
        .iter()
        .flat_map(|skill| skill.tools.iter().map(move |tool| (format!("{}__{}", skill.id, tool.name), tool.input.clone())))
        .collect()
}

/// That `place` names declared tools, with the input value it needs.
fn offered(tools: &BTreeMap<String, Value>, what: &str, place: Place) -> Result<(), String> {
    let names: Vec<(&str, Option<(&str, &str)>)> = match place {
        Place::Left(_) => return Ok(()),
        Place::Tool(name, field) => vec![(name, field)],
        Place::Tools(names) => names.iter().map(|name| (*name, None)).collect(),
    };
    for (name, field) in names {
        let input = tools.get(name).ok_or_else(|| format!("{what} has no tool: {name} is not declared"))?;
        if let Some((key, value)) = field {
            let options = input["properties"][key]["enum"].as_array().cloned().unwrap_or_default();
            if !options.contains(&json!(value)) {
                return Err(format!("{what} has no tool: {name} does not take {key} {value:?}"));
            }
        }
    }
    Ok(())
}

/// That `tool`'s `properties` input takes `key`.
fn takes(tools: &BTreeMap<String, Value>, tool: &str, key: &str) -> bool {
    tools
        .get(tool)
        .and_then(|input| input["properties"]["properties"]["properties"].as_object())
        .is_some_and(|keys| keys.contains_key(key))
}

/// Every action id in a message: an object with a text `id` and at most an `arg`.
fn actions_in(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(id)) = object.get("id")
                && object.keys().all(|key| key == "id" || key == "arg")
            {
                out.insert(id.clone());
            }
            for child in object.values() {
                actions_in(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| actions_in(item, out)),
        _ => {}
    }
}

/// Every row of the film's pages and the onion skin: a `prop` naming `film`.
fn film_rows_in(value: &Value, out: &mut BTreeSet<(String, String)>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::Object(prop)) = object.get("prop")
                && let Some(Value::String(kind)) = prop.get("film")
                && let Some(Value::String(field)) = prop.get("field")
            {
                out.insert((kind.clone(), field.clone()));
            }
            for child in object.values() {
                film_rows_in(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| film_rows_in(item, out)),
        _ => {}
    }
}

/// What the person sees, gathered from an in-process Storyboard Generator put
/// through every page: actions, and the panel's properties on object pages
/// and on object colors' pages.
struct Seen {
    actions: BTreeSet<String>,
    /// The film's page rows and onion skin rows, as (page, field); a note
    /// row, which edits nothing, has no field.
    film_rows: BTreeSet<(String, String)>,
    object_props: BTreeSet<String>,
    color_props: BTreeSet<String>,
    /// The id of every timeline row declared.
    rows: BTreeSet<String>,
}

struct Walk {
    server: Server,
    seen: Seen,
    tab: String,
    calls: u64,
}

impl Walk {
    /// Sends a message and notes what came back. `page` names the panel
    /// page it opens: "object", "color", or none.
    fn send(&mut self, message: HostMessage, page: Option<&str>) -> Vec<AppMessage> {
        let replies = self.server.handle(message).unwrap_or_default();
        self.note(&replies, page);
        replies
    }

    fn note(&mut self, replies: &[AppMessage], page: Option<&str>) {
        for reply in replies {
            if let Ok(value) = serde_json::to_value(reply) {
                actions_in(&value, &mut self.seen.actions);
                film_rows_in(&value, &mut self.seen.film_rows);
            }
            if let AppMessage::Timeline(timeline) = reply {
                self.seen.rows.extend(timeline.rows.iter().map(|row| row.id.clone()));
            }
            if let (AppMessage::Properties(properties), Some(page)) = (reply, page) {
                let target = if page == "object" { &mut self.seen.object_props } else { &mut self.seen.color_props };
                for section in &properties.sections {
                    for row in &section.rows {
                        if let Some(key) = row.prop.as_str() {
                            target.insert(key.to_string());
                        }
                    }
                }
            }
        }
    }

    fn command(&mut self, id: &str, arg: Option<Value>, page: Option<&str>) {
        let tab = self.tab.clone();
        self.send(HostMessage::Command { id: id.into(), tab: Some(tab), arg, node: None, viewport: crate::VIEWPORT }, page);
    }

    fn invoke(&mut self, tool: &str, input: Value) -> Value {
        self.calls += 1;
        let (skill, name) = tool.split_once("__").unwrap_or(("", tool));
        let replies = self.send(
            HostMessage::Invoke {
                call: format!("walk-{}", self.calls),
                tab: self.tab.clone(),
                tool: protocol::tool_key("storyboard-generator", skill, name),
                input,
                group: format!("walk-{}", self.calls),
            },
            None,
        );
        replies
            .iter()
            .find_map(|reply| match reply {
                AppMessage::ToolResult(result) => Some(result.output.clone()),
                _ => None,
            })
            .unwrap_or(Value::Null)
    }
}

fn gather() -> Result<(Vec<ToolGroup>, Seen), String> {
    let mut walk = Walk {
        server: Server::new(None),
        seen: Seen { actions: BTreeSet::new(), film_rows: BTreeSet::new(), object_props: BTreeSet::new(), color_props: BTreeSet::new(), rows: BTreeSet::new() },
        tab: String::new(),
        calls: 0,
    };
    let greeting = walk.server.greeting();
    walk.note(&greeting, None);
    let skills = greeting
        .iter()
        .find_map(|message| match message {
            AppMessage::Tools { groups } => Some(groups.clone()),
            _ => None,
        })
        .ok_or("storyboard-generator declared no tools")?;
    walk.tab = crate::document_of(&greeting).ok_or("no document")?.0;
    walk.send(HostMessage::Viewport { viewport: crate::VIEWPORT }, None);
    // A project of its own, so sound is imported into it and the animatic
    // exported there.
    let project = std::env::temp_dir().join(format!("storyboard-client-coverage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(&project).map_err(|error| error.to_string())?;
    walk.send(HostMessage::Project { folder: project.display().to_string() }, None);

    // The film: its storyboard page, two clips, a clip's page and menus.
    walk.command("toggle_panel", None, None);
    let first = walk.invoke("clips__add_clip", json!({}))["made"].as_u64().ok_or("no clip")?;
    walk.invoke("clips__add_clip", json!({}));
    walk.invoke("clips__select_clips", json!({"clips": [first]}));
    walk.command("select_tab", None, None);
    walk.invoke("clips__select_clips", json!({"clips": []}));
    walk.command("select_tab", None, None);
    // The film's sound: two Sound tracks, a sound's page, a track's page,
    // the workspace menu's rows, and the export's cards.
    let sound_file = project.join("line.wav");
    std::fs::write(&sound_file, storyboard_generator::sounds::wav_bytes(storyboard_generator::sounds::RATE, &storyboard_generator::sounds::clicks(48, &[6])))
        .map_err(|error| error.to_string())?;
    let imported = walk.invoke("clips__import_sound", json!({"path": sound_file.display().to_string(), "frame": 1}));
    let sound = imported["made"]["sound"].as_u64().ok_or(format!("no sound was imported: {imported}"))?;
    walk.invoke("clips__add_sound_track", json!({}));
    walk.invoke("clips__select_sounds", json!({"sounds": [sound]}));
    let track = walk.invoke("clips__read_film", json!({}))["sound_tracks"][0]["id"].as_u64().ok_or("no Sound track")?;
    walk.command("rename_sound_track", Some(json!(track)), None);
    walk.invoke("clips__select_clips", json!({"clips": []}));
    // Its card while it runs holds Cancel; once done, a close.
    walk.command("export_animatic", None, None);
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let replies = walk.server.tick();
        walk.note(&replies, None);
        if walk.seen.actions.contains("close_notice") {
            break;
        }
    }
    // Inside the first clip, where drawing happens: a folder, a layer, keys,
    // a label, the onion skin, and the layer's page.
    walk.invoke("clips__edit_clip", json!({"clip": first}));
    walk.invoke("clips__add_layer", json!({"folder": true}));
    let made = walk.invoke("clips__add_layer", json!({}));
    let added = made["made"].as_u64().ok_or("no layer")?;
    walk.invoke("clips__insert_keyframe", json!({"layer": added, "frame": 6}));
    walk.invoke("clips__add_mark", json!({"layer": added, "frame": 3, "kind": "label", "text": "radio on"}));
    walk.invoke("clips__set_onion_skin", json!({"on": true}));
    walk.invoke("clips__copy_frames", json!({"start": 1, "end": 4}));
    let tab = walk.tab.clone();
    walk.send(
        HostMessage::TimelineGesture {
            tab,
            gesture: protocol::TimelineGesture::Select {
                picks: vec![protocol::TimelinePick { row: added.to_string(), item: None, frame: Some(0) }],
                add: false,
                toggle: false,
            },
        },
        None,
    );
    walk.invoke("clips__go_to_frame", json!({"frame": 1}));

    // One of each object, so every page and menu exists.
    let read = walk.invoke("look__read_storyboard", json!({"points": false}));
    let canvas = read["active_canvas"].as_u64().ok_or("no canvas")?;
    let layer = read["objects_front_to_back"][0]["layers_top_to_bottom"][0]["id"].as_u64().ok_or("no layer")?;
    let mut objects = Vec::new();
    for (kind, from, to) in [("rectangle", [0, 0], [40, 40]), ("ellipse", [50, 0], [90, 40]), ("line", [0, 60], [90, 60])] {
        let made = walk.invoke("draw__add_shape", json!({"kind": kind, "from": from, "to": to}));
        objects.push(made["made"][0].clone());
    }
    let text = walk.invoke("draw__add_text", json!({"text": "Words", "at": [0, 100]}));
    objects.push(text["made"][0].clone());
    walk.invoke("draw__draw_stroke", json!({"points": [[0, 200], [50, 220]], "canvas": canvas}));
    let ids: Vec<u64> = objects.iter().filter_map(|object| object["id"].as_u64()).collect();
    walk.invoke("edit__group", json!({"objects": [ids[0], ids[1]]}));
    walk.invoke("edit__ungroup", json!({}));

    // Every tool's toolbar, page, and colors.
    for tool in registry::tools() {
        let key = format!("{:?}", tool.id).to_lowercase();
        walk.command("select_tool", Some(json!(key)), None);
        walk.command("tool_action", Some(json!({"tool": key, "action": "properties"})), None);
    }
    // The select tool with a selection, and each object's page and colors.
    walk.command("select_tool", Some(json!("select")), None);
    for object in objects.iter().chain([&json!({"id": canvas, "palette": [{"id": layer}]})]) {
        let Some(id) = object["id"].as_u64() else { continue };
        walk.command("select_click", Some(json!({"node": id})), None);
        walk.command("object_properties", Some(json!(id.to_string())), Some("object"));
        for color in object["palette"].as_array().into_iter().flatten() {
            if let Some(color) = color["id"].as_u64() {
                if id == canvas {
                    walk.command("select_color", Some(json!({"owner": id, "color": color})), Some("color"));
                } else {
                    walk.command("object_properties", Some(json!(id.to_string())), Some("object"));
                    walk.command("open_palette_color", Some(json!({"color": color})), Some("color"));
                }
            }
        }
        walk.command("go_back", None, None);
    }
    // Text being edited, with the text tool's bar on it.
    walk.command("select_tool", Some(json!("text")), None);
    let words = ids[3];
    let tab = walk.tab.clone();
    walk.send(HostMessage::Command { id: "edit_text".into(), tab: Some(tab), arg: None, node: Some(words), viewport: crate::VIEWPORT }, Some("object"));
    walk.command("commit_text", None, None);
    walk.command("select_click", Some(json!({"node": words})), Some("object"));
    // Something on the clipboard, for paste.
    walk.command("select_tool", Some(json!("select")), None);
    walk.command("copy", None, None);
    Ok((skills, walk.seen))
}

pub(crate) fn check() -> Result<(), String> {
    let (skills, seen) = gather()?;
    let tools = declared(&skills);

    for definition in registry::commands() {
        let what = format!("the command {:?}", definition.id);
        offered(&tools, &what, command(definition.id))?;
    }
    let command_keys: BTreeSet<String> = registry::commands()
        .iter()
        .map(|definition| snake(&format!("{:?}", definition.id)))
        .collect();
    for id in &seen.actions {
        if command_keys.contains(id) {
            continue;
        }
        let place = action(id).ok_or_else(|| format!("the declared action {id:?} is neither a tool's nor left out on purpose"))?;
        offered(&tools, &format!("the action {id:?}"), place)?;
    }
    if seen.actions.len() < 20 {
        return Err("the walk saw too few of Storyboard Generator's actions to be a check".into());
    }

    for (seen_props, tool) in [(&seen.object_props, "style__set_object_properties"), (&seen.color_props, "style__set_color_properties")] {
        if seen_props.is_empty() {
            return Err(format!("the walk saw no panel rows for {tool}"));
        }
        for key in seen_props {
            let id = PropId::from_key(key).ok_or_else(|| format!("the panel shows an unknown property {key:?}"))?;
            match property(id) {
                Place::Left(_) => {}
                place => {
                    offered(&tools, &format!("the property {key:?}"), place)?;
                    if !takes(&tools, tool, key) {
                        return Err(format!("the panel edits {key:?} on this page, and {tool} does not take it"));
                    }
                }
            }
        }
    }
    // Every property the session knows, through its keys.
    for key in all_prop_keys() {
        let id = PropId::from_key(key).ok_or_else(|| format!("{key:?} is not a property"))?;
        let place = property(id);
        offered(&tools, &format!("the property {key:?}"), place)?;
        if let Place::Tool(tool, _) = place
            && !takes(&tools, tool, key)
        {
            return Err(format!("{tool} does not take the property {key:?}"));
        }
        if let Place::Tools(names) = place
            && !names.iter().all(|tool| takes(&tools, tool, key))
        {
            return Err(format!("not every tool for {key:?} takes it"));
        }
    }

    if seen.film_rows.len() < 15 {
        return Err(format!("the walk saw too few of the film's rows to be a check: {:?}", seen.film_rows));
    }
    for (kind, field) in &seen.film_rows {
        let (tool, key) = film_row(kind, field)
            .ok_or_else(|| format!("the film's row {kind}.{field} is neither a tool's nor left out on purpose"))?;
        let input = tools.get(tool).ok_or_else(|| format!("the film's row {kind}.{field} has no tool: {tool} is not declared"))?;
        if input["properties"][key].is_null() {
            return Err(format!("the film's row {kind}.{field} is set by {tool}, which does not take {key}"));
        }
    }
    gestures_offered(&tools)?;
    // Every row the walk saw declared is of a kind the gestures above
    // cover, and every kind was seen.
    let mut kinds = BTreeSet::new();
    for row in &seen.rows {
        let kind = RowKind::of(row).ok_or_else(|| format!("the timeline row {row:?} is of a kind coverage does not place"))?;
        kinds.insert(format!("{kind:?}"));
    }
    for kind in [Some(RowKind::Shots), Some(RowKind::Sound), Some(RowKind::Layer)] {
        if !kinds.contains(&format!("{kind:?}")) {
            return Err(format!("the walk never saw a {kind:?} row"));
        }
    }
    for kind in gesture_kinds() {
        offered(&tools, &format!("the gesture {kind:?}"), gesture(&kind))?;
    }
    for op in select_ops() {
        offered(&tools, &format!("the selection {op:?}"), select(&op))?;
    }
    let modes = tools.get("edit__select").map(|input| input["properties"]["mode"]["enum"].clone()).unwrap_or_default();
    for mode in ["remove", "add", "replace", "clear"] {
        if !modes.as_array().is_some_and(|modes| modes.contains(&json!(mode))) {
            return Err(format!("edit__select has no mode {mode}"));
        }
    }
    Ok(())
}

/// Every property key, found by asking `PropId` for each word its keys
/// could be; `property` above has a place for every variant.
fn all_prop_keys() -> Vec<&'static str> {
    const KEYS: &[&str] = &[
        "preset", "pressure_size", "curve", "smoothing", "stabilization", "streamline", "eraser_size", "name", "hex",
        "rgb", "opacity", "color_size", "dash", "cap", "visible", "locked", "padding", "lock_size", "palette",
        "corner_radius", "constrain", "snap", "family", "font_size", "bold", "italic", "underline", "align", "valign",
        "list", "text_padding", "text_radius", "line_height", "letter_spacing", "all_caps", "auto_size", "wrap_text",
    ];
    KEYS.to_vec()
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Coverage shown failing once: with no tool to move a sound, a move on
    /// a Sound track fails it, though the Shots track's moves are covered.
    #[test]
    fn a_move_on_a_sound_track_needs_its_own_tool() {
        let (skills, _) = gather().unwrap();
        let mut tools = declared(&skills);
        assert!(gestures_offered(&tools).is_ok());
        tools.remove("clips__move_sound");
        let error = gestures_offered(&tools).unwrap_err();
        assert!(error.contains("Sound row") && error.contains("clips__move_sound"), "{error}");
        assert!(tools.contains_key("clips__move_clips"));
    }
}
