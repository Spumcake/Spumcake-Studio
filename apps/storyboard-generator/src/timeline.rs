//! The timeline this workspace declares under its stage, as the approved
//! views show it: the film's Shots track of clips, and inside a clip its
//! layers of keyframes and holds, in Adobe Animate's look. The host draws it
//! and previews its gestures; every change comes back here as a command or a
//! `timeline_gesture`, and runs the session call a tool runs too.

use crate::board::{self, Boards};
use crate::film::{Clip, FPS, Layer, NEW_CLIP_LENGTH};
use crate::look::Looker;
use crate::session::Session;
use crate::session::film::{ClipText, LayerFlag};
use crate::soundtrack::{SoundTrack, TrackFlag, TrackRef};
use crate::sounds::State;
use protocol::{
    Action, CellMark, CellSpan, DropRule, MarkKind, MenuEntry, PropValue, Row, RowContent, RowKind, RowToggle,
    Ruler, RulerUnit, Section, SpanKind, Timeline, TimelineBack, TimelineGesture, TimelineItem, TimelinePage,
    TimelinePick, TimelineRange, ToolItem,
};
use serde_json::{Value, json};

/// A clip's fill and its selected border, as the approved film views show them.
const CLIP_FILL: [u8; 3] = [219, 234, 254];
const CLIP_BORDER: [u8; 3] = [147, 197, 253];
const SELECTED_BORDER: [u8; 3] = [37, 99, 235];
const SHOTS_COLOR: [u8; 3] = [96, 165, 250];
/// A sound clip's pale green, its border, and its track's colour, as the
/// approved film-clip-kinds shows them.
const SOUND_FILL: [u8; 3] = [209, 250, 229];
const SOUND_BORDER: [u8; 3] = [110, 231, 183];
const SOUND_COLOR: [u8; 3] = [16, 185, 129];
/// The kinds of item the film's tracks take: a clip lands only on Shots,
/// and a sound only on a Sound track.
const CLIP_KIND: &str = "clip";
const SOUND_KIND: &str = "sound";
/// A sound whose file is missing: pale red.
const MISSING_FILL: [u8; 3] = [254, 226, 226];
const MISSING_BORDER: [u8; 3] = [248, 113, 113];

fn action(id: &str) -> Action {
    Action::new(id)
}

fn action_with(id: &str, arg: Value) -> Action {
    Action::with_arg(id, arg)
}

fn entry(label: &str, action: Action, enabled: bool) -> MenuEntry {
    MenuEntry {
        tip: None,
        label: label.to_string(),
        icon: None,
        enabled,
        divider: false,
        action: Some(action),
        submenu: Vec::new(),
        rename: false,
    }
}

/// The host draws a row's `divider` as a line above it, so a divider marks
/// the row after it rather than standing as an empty row of its own.
fn ruled(rows: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut out: Vec<MenuEntry> = Vec::new();
    let mut rule = false;
    for row in rows {
        if row.divider && row.label.is_empty() {
            rule = !out.is_empty();
            continue;
        }
        out.push(MenuEntry { divider: rule, ..row });
        rule = false;
    }
    out
}

fn divider() -> MenuEntry {
    MenuEntry {
        tip: None,
        label: String::new(),
        icon: None,
        enabled: true,
        divider: true,
        action: None,
        submenu: Vec::new(),
        rename: false,
    }
}

fn button(icon: &str, tip: &str, id: &str, enabled: bool, selected: bool) -> ToolItem {
    ToolItem::Button {
        icon: icon.to_string(),
        tip: tip.to_string(),
        enabled,
        selected,
        action: Some(action(id)),
        menu: Vec::new(),
    }
}

/// A timecode as HH:MM:SS:FF, or fewer parts from the right, such as
/// "2:10", or a bare number of frames.
pub fn parse_timecode(text: &str) -> Option<u32> {
    let parts: Vec<u32> = text.trim().split(':').map(|part| part.trim().parse().ok()).collect::<Option<_>>()?;
    let (frames, rest) = parts.split_last()?;
    let mut seconds = 0;
    for part in rest {
        seconds = seconds * 60 + part;
    }
    Some(seconds * FPS + frames)
}

/// HH:MM:SS:FF, as the film's ruler and timecode read.
pub fn timecode(frame: u32) -> String {
    let seconds = frame / FPS;
    format!("{:02}:{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60, frame % FPS)
}

/// The timeline of the storyboard being shown.
pub fn declare(session: &Session, boards: &mut Boards, looker: &mut Looker, tab: String) -> Timeline {
    let record = session.active_storyboard();
    let view = record.view();
    match view.clip.and_then(|clip| record.film().clip(clip)) {
        Some(clip) => declare_clip(session, clip, tab),
        None => declare_film(session, boards, looker, tab),
    }
}

fn declare_film(session: &Session, boards: &mut Boards, looker: &mut Looker, tab: String) -> Timeline {
    let record = session.active_storyboard();
    let film = record.film();
    let view = record.view();
    let picked = session.picked();
    let length = film.length();
    let header = vec![
        play_button(session, length > 0),
        button("skip-prev", "Step back (,)", "step_back", true, false),
        button("skip-next", "Step forward (.)", "step_forward", true, false),
        ToolItem::Label { text: timecode(view.film_frame), strong: true },
        ToolItem::Label { text: format!("/ {}", timecode(length)), strong: false },
        volume_slider(session),
    ];
    let items = film
        .clips
        .iter()
        .map(|clip| {
            let selected = picked.clips.contains(&clip.id);
            TimelineItem {
                id: clip.id.to_string(),
                start: clip.start,
                length: clip.length,
                fill: CLIP_FILL,
                border: Some(protocol::Border {
                    rgb: if selected { SELECTED_BORDER } else { CLIP_BORDER },
                    width: if selected { 2.0 } else { 1.0 },
                }),
                style: protocol::ItemStyle::Solid,
                label: clip.name.clone(),
                icon: None,
                selected,
                thumbnail: board::thumbnail(boards, looker, record, clip),
                waveform: Vec::new(),
                progress: None,
                end_mark: None,
                tip: String::new(),
                menu: clip_menu(clip.id, !film.shots_locked),
                open: Some(action_with("edit_clip", json!(clip.id))),
                movable: !film.shots_locked,
                kind: Some(CLIP_KIND.to_string()),
                earliest_start: None,
                latest_end: None,
            }
        })
        .collect();
    let shots = protocol::TimelineRow {
        id: "shots".to_string(),
        name: "Shots".to_string(),
        color: SHOTS_COLOR,
        icon: Some("edit-pencil".to_string()),
        indent: 0,
        open: None,
        disclose: None,
        selected: false,
        toggles: vec![
            lock_toggle(TrackRef::Shots, film.shots_locked),
            RowToggle {
                icon: if film.shots_visible { "eye" } else { "eye-closed" }.to_string(),
                tip: if film.shots_visible { "Hide" } else { "Show" }.to_string(),
                on: film.shots_visible,
                action: action_with("track_flag", json!({ "track": "shots", "flag": "visible" })),
            },
        ],
        read_only: false,
        menu: vec![entry("New clip", action("new_clip"), !film.shots_locked)],
        drop: DropRule::Push,
        accepts: Some(CLIP_KIND.to_string()),
        content: RowContent::Items { items },
    };
    let mut rows = vec![shots];
    rows.extend(film.sound_tracks.iter().map(|track| sound_row(session, track, &picked.sounds)));
    Timeline {
        tab,
        header,
        pages: Vec::new(),
        back: None,
        // A short film still shows the ruler's first seconds.
        ruler: Ruler { fps: f64::from(FPS), length: length.max(24 * FPS), unit: RulerUnit::Timecode },
        rows,
        playhead: view.film_frame,
        ranges: Vec::new(),
        add_row: Some(action("new_sound_track")),
        empty_hint: if film.clips.is_empty() { "Right-click to add a clip".to_string() } else { String::new() },
    }
}

fn clip_menu(clip: u64, unlocked: bool) -> Vec<MenuEntry> {
    ruled(vec![
        entry("Edit clip", action_with("edit_clip", json!(clip)), true),
        divider(),
        entry("Rename", action_with("rename_clip", json!(clip)), unlocked),
        entry("Duplicate", action_with("duplicate_clip", json!(clip)), unlocked),
        entry("Delete", action_with("delete_clip", json!(clip)), unlocked),
    ])
}

/// Play, or Pause while playing, as one button.
fn play_button(session: &Session, enabled: bool) -> ToolItem {
    if session.is_playing() {
        button("pause", "Pause", "play", true, true)
    } else {
        button("play", "Play", "play", enabled, false)
    }
}

/// How loud the film plays here: never in the export.
fn volume_slider(session: &Session) -> ToolItem {
    ToolItem::Slider {
        caption: "Volume".to_string(),
        width: 80.0,
        value: (session.listening() * 100.0).round() as i32,
        min: 0,
        max: 100,
        tint: None,
        set: action("set_volume"),
        finish: Some(action("finish_volume")),
        suffix: "%".to_string(),
    }
}

fn lock_toggle(track: TrackRef, locked: bool) -> RowToggle {
    RowToggle {
        icon: "lock".to_string(),
        tip: if locked { "Unlock" } else { "Lock" }.to_string(),
        on: locked,
        action: action_with("track_flag", json!({ "track": track.row(), "flag": "locked" })),
    }
}

/// The film's sound under a clip's layers, for lip-sync: every unmuted
/// Sound track mixed over the clip's span, read only.
fn film_sound_row(session: &Session, clip: &Clip) -> protocol::TimelineRow {
    let waveform = crate::sounds::span_peaks(&session.film_sound_peaks(clip.start, clip.length), 0, clip.length);
    protocol::TimelineRow {
        id: "film-sound".to_string(),
        name: "Film sound".to_string(),
        color: SOUND_COLOR,
        icon: Some("sound-high".to_string()),
        indent: 0,
        open: None,
        disclose: None,
        selected: false,
        toggles: Vec::new(),
        read_only: true,
        menu: Vec::new(),
        drop: DropRule::Push,
        accepts: Some(SOUND_KIND.to_string()),
        content: RowContent::Items {
            items: vec![TimelineItem {
                id: "film-sound".to_string(),
                start: 0,
                length: clip.length,
                fill: SOUND_FILL,
                border: Some(protocol::Border { rgb: SOUND_BORDER, width: 1.0 }),
                style: protocol::ItemStyle::Solid,
                label: String::new(),
                icon: None,
                selected: false,
                thumbnail: None,
                waveform,
                progress: None,
                end_mark: None,
                tip: String::new(),
                menu: Vec::new(),
                open: None,
                movable: false,
                kind: Some(SOUND_KIND.to_string()),
                earliest_start: Some(0),
                latest_end: Some(clip.length),
            }],
        },
    }
}

/// Where the sound before `clip` on its track ends.
fn before(track: &SoundTrack, clip: &crate::soundtrack::SoundClip) -> u32 {
    track.clips.iter().filter(|other| other.id != clip.id && other.at < clip.at).map(crate::soundtrack::SoundClip::end).max().unwrap_or(0)
}

/// Where the sound after `clip` on its track starts.
fn after(track: &SoundTrack, clip: &crate::soundtrack::SoundClip) -> Option<u32> {
    track.clips.iter().filter(|other| other.id != clip.id && other.at >= clip.end()).map(|other| other.at).min()
}

/// A Sound track: its lock and speaker, and its sounds with their waveforms.
fn sound_row(session: &Session, track: &SoundTrack, picked: &[u64]) -> protocol::TimelineRow {
    let items = track
        .clips
        .iter()
        .map(|clip| {
            let selected = picked.contains(&clip.id);
            let state = session.peek_sound(&clip.file);
            let missing = matches!(state, Some(State::Missing | State::Unreadable(_)));
            let waveform = match state {
                Some(State::Loaded(loaded)) => crate::sounds::span_peaks(&loaded.peaks, clip.from, clip.length),
                _ => Vec::new(),
            };
            let tip = match state {
                Some(State::Unreadable(reason)) => reason.clone(),
                Some(State::Missing) => format!("{} is missing from the sounds folder.", clip.name),
                _ => String::new(),
            };
            TimelineItem {
                id: clip.id.to_string(),
                start: clip.at,
                length: clip.length,
                fill: if missing { MISSING_FILL } else { SOUND_FILL },
                border: Some(protocol::Border {
                    rgb: if selected { SELECTED_BORDER } else if missing { MISSING_BORDER } else { SOUND_BORDER },
                    width: if selected { 2.0 } else { 1.0 },
                }),
                style: if missing { protocol::ItemStyle::Dashed } else { protocol::ItemStyle::Solid },
                label: if missing { "Missing".to_string() } else { clip.name.clone() },
                icon: Some("music-double-note".to_string()),
                selected,
                thumbnail: None,
                waveform,
                progress: None,
                end_mark: None,
                tip,
                menu: vec![entry("Delete", action_with("delete_sound", json!(clip.id)), !track.locked)],
                open: None,
                movable: !track.locked,
                kind: Some(SOUND_KIND.to_string()),
                // An edge stops at the sound's own ends, and at its neighbours.
                earliest_start: Some(clip.at.saturating_sub(clip.from).max(before(track, clip))),
                latest_end: Some({
                    // Where the sound's own end falls on the film.
                    let own = (i64::from(clip.at) - i64::from(clip.from) + i64::from(clip.source)).max(i64::from(clip.end())) as u32;
                    after(track, clip).map_or(own, |next| next.min(own))
                }),
            }
        })
        .collect();
    protocol::TimelineRow {
        id: TrackRef::Sound(track.id).row(),
        name: track.name.clone(),
        color: SOUND_COLOR,
        icon: Some("sound-high".to_string()),
        indent: 0,
        open: None,
        disclose: None,
        selected: false,
        toggles: vec![
            lock_toggle(TrackRef::Sound(track.id), track.locked),
            RowToggle {
                icon: if track.muted { "sound-off" } else { "sound-high" }.to_string(),
                tip: if track.muted { "Unmute" } else { "Mute" }.to_string(),
                on: track.muted,
                action: action_with("track_flag", json!({ "track": TrackRef::Sound(track.id).row(), "flag": "muted" })),
            },
        ],
        read_only: false,
        menu: ruled(vec![
            entry("Rename", action_with("rename_sound_track", json!(track.id)), true),
            entry("Delete track", action_with("delete_sound_track", json!(track.id)), true),
        ]),
        drop: DropRule::Push,
        accepts: Some(SOUND_KIND.to_string()),
        content: RowContent::Items { items },
    }
}

/// The frame menu's rows, as the approved clip-frame-menu shows them with
/// Animate's keys. Tweens come in a later slice.
fn frame_menu(session: &Session) -> Vec<MenuEntry> {
    let mut rows = vec![
        entry("Insert frame\tF5", action("insert_frame"), true),
        entry("Insert keyframe\tF6", action("insert_keyframe"), true),
        entry("Insert blank keyframe\tF7", action("insert_blank_keyframe"), true),
        entry("Insert image key…", action("insert_image_key"), true),
        entry("Publish hold", action("publish_hold"), session.is_skills()),
        entry("Clear keyframe\t⇧F6", action("clear_keyframe"), true),
        entry("Remove frames\t⇧F5", action("remove_frames"), true),
        divider(),
        entry("Copy frames", action("copy_frames"), true),
        entry("Paste frames", action("paste_frames"), session.has_copied_frames()),
        divider(),
        entry("Reverse frames", action("reverse_frames"), true),
        divider(),
        entry("Add label", action_with("add_mark", json!({ "comment": false })), true),
        entry("Add comment", action_with("add_mark", json!({ "comment": true })), true),
        divider(),
        entry("New layer", action("new_layer"), true),
        entry("New folder", action("new_folder"), true),
        entry("Delete layer", action("delete_layer"), true),
    ];
    rows.retain(|row| !row.label.is_empty() || row.divider);
    ruled(rows)
}

fn declare_clip(session: &Session, clip: &Clip, tab: String) -> Timeline {
    let record = session.active_storyboard();
    let view = record.view();
    let onion = record.onion();
    let picked = session.picked();
    let header = vec![
        ToolItem::Button {
            icon: "copy".to_string(),
            tip: "Onion skin".to_string(),
            enabled: true,
            selected: onion.on,
            action: Some(action("toggle_onion")),
            menu: Vec::new(),
        },
        ToolItem::Popover { icon: "nav-arrow-down".to_string(), tip: "Onion skin settings".to_string(), rows: onion_rows(session), action: None, selected: false, enabled: true },
        ToolItem::Button {
            icon: "plus-circle".to_string(),
            tip: "Insert keyframe (F6)".to_string(),
            enabled: true,
            selected: false,
            action: Some(action("insert_keyframe")),
            menu: vec![
                entry("Keyframe", action("insert_keyframe"), true),
                entry("Blank keyframe", action("insert_blank_keyframe"), true),
                entry("Frame", action("insert_frame"), true),
            ],
        },
        ToolItem::Separator,
        button("skip-prev", "Step back (,)", "step_back", true, false),
        play_button(session, true),
        button("skip-next", "Step forward (.)", "step_forward", true, false),
        button("repeat", "Loop", "toggle_loop", true, session.looping()),
        ToolItem::Separator,
        ToolItem::Label { text: format!("{FPS} fps"), strong: false },
        ToolItem::Separator,
        ToolItem::Label { text: format!("{:.1} s", f64::from(view.frame) / f64::from(FPS)), strong: false },
    ];
    let menu = frame_menu(session);
    let mut rows: Vec<protocol::TimelineRow> = clip
        .layers
        .iter()
        .filter(|layer| !clip.folded(layer))
        .map(|layer| layer_row(session, clip, layer, view.layer == Some(layer.id), &picked, &menu))
        .collect();
    if !session.film().sound_tracks.is_empty() {
        rows.push(film_sound_row(session, clip));
    }
    let mut ranges = Vec::new();
    if onion.on
        && let Some(layer) = view.layer.and_then(|layer| clip.layer(layer))
    {
        let (start, end) = onion_range(layer, view.frame, onion.before, onion.after, clip.length);
        ranges.push(TimelineRange { id: "onion".to_string(), kind: protocol::RangeKind::OnionSkin, start, end });
    }
    Timeline {
        tab,
        header,
        pages: vec![TimelinePage { label: "Timeline".to_string(), selected: true, action: action("timeline_page") }],
        back: Some(TimelineBack { path: vec!["Film".to_string(), clip.name.clone()], action: action("leave_clip") }),
        ruler: Ruler { fps: f64::from(FPS), length: clip.length, unit: RulerUnit::Frames },
        rows,
        playhead: view.frame,
        ranges,
        add_row: Some(action("new_layer")),
        empty_hint: String::new(),
    }
}

/// The frames the onion skin covers around `frame`: from the earliest key it
/// shows to the end of the latest key's hold.
fn onion_range(layer: &Layer, frame: u32, before: u8, after: u8, length: u32) -> (u32, u32) {
    let index = layer.keys.iter().rposition(|key| key.frame <= frame).unwrap_or(0);
    let start = layer.keys[index.saturating_sub(usize::from(before))].frame;
    let last = (index + usize::from(after)).min(layer.keys.len().saturating_sub(1));
    let end = layer.keys.get(last + 1).map_or(length, |key| key.frame).saturating_sub(1).max(frame);
    (start.min(frame), end)
}

fn layer_row(
    session: &Session,
    clip: &Clip,
    layer: &Layer,
    active: bool,
    picked: &crate::session::film::Picked,
    menu: &[MenuEntry],
) -> protocol::TimelineRow {
    let record = session.active_storyboard();
    let toggles = vec![
        RowToggle {
            icon: if layer.visible { "eye" } else { "eye-closed" }.to_string(),
            tip: "Show".to_string(),
            on: layer.visible,
            action: action_with("layer_flag", json!({ "layer": layer.id, "flag": "visible" })),
        },
        RowToggle {
            icon: "lock".to_string(),
            tip: "Lock".to_string(),
            on: layer.locked,
            action: action_with("layer_flag", json!({ "layer": layer.id, "flag": "locked" })),
        },
        RowToggle {
            icon: "square".to_string(),
            tip: "Outline".to_string(),
            on: layer.outline,
            action: action_with("layer_flag", json!({ "layer": layer.id, "flag": "outline" })),
        },
    ];
    let selected_cells = picked
        .cells
        .as_ref()
        .filter(|cells| cells.layers.contains(&layer.id))
        .map(|cells| (cells.start, cells.end));
    let in_block = |frame: u32| selected_cells.is_some_and(|(start, end)| frame >= start && frame <= end);
    let mut marks = Vec::new();
    let mut spans = Vec::new();
    for (index, key) in layer.keys.iter().enumerate() {
        if key.frame >= clip.length {
            continue;
        }
        let end = layer.keys.get(index + 1).map_or(clip.length, |next| next.frame.min(clip.length));
        let blank = record.drawing_of(key.drawing).is_none_or(|drawing| drawing.model_is_blank());
        marks.push(CellMark {
            frame: key.frame,
            kind: if blank { MarkKind::BlankKey } else { MarkKind::Key },
            selected: in_block(key.frame),
        });
        if end > key.frame + 1 {
            if !blank {
                spans.push(CellSpan { start: key.frame, length: end - key.frame, kind: SpanKind::Hold });
            }
            marks.push(CellMark { frame: end - 1, kind: MarkKind::HoldEnd, selected: in_block(end - 1) });
        }
    }
    for mark in &layer.marks {
        marks.push(CellMark {
            frame: mark.frame,
            kind: if mark.comment {
                MarkKind::Comment { text: mark.text.clone() }
            } else {
                MarkKind::Label { text: mark.text.clone() }
            },
            selected: false,
        });
    }
    protocol::TimelineRow {
        id: layer.id.to_string(),
        name: layer.name.clone(),
        color: layer.color,
        icon: layer.folder.then(|| "folder".to_string()),
        indent: clip.depth(layer),
        open: layer.folder.then_some(layer.open),
        disclose: layer.folder.then(|| action_with("layer_flag", json!({ "layer": layer.id, "flag": "open" }))),
        selected: active,
        toggles,
        read_only: false,
        menu: menu.to_vec(),
        drop: DropRule::Replace,
        accepts: None,
        content: if layer.folder {
            RowContent::Cells { marks: Vec::new(), spans: Vec::new() }
        } else {
            RowContent::Cells { marks, spans }
        },
    }
}

/// The onion skin's settings, as the approved clip-onion-settings shows them.
fn onion_rows(session: &Session) -> Vec<Row> {
    let onion = session.onion();
    let prop = |field: &str| json!({ "film": "onion", "field": field });
    vec![
        Row {
            prop: prop("before"),
            label: "Before".to_string(),
            kind: RowKind::Number { min: 0.0, max: 9.0, percent: false, places: 0, centered: false },
            value: PropValue::Number(f64::from(onion.before)),
        },
        Row {
            prop: prop("after"),
            label: "After".to_string(),
            kind: RowKind::Number { min: 0.0, max: 9.0, percent: false, places: 0, centered: false },
            value: PropValue::Number(f64::from(onion.after)),
        },
        Row {
            prop: prop("opacity"),
            label: "Starting opacity".to_string(),
            kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
            value: PropValue::Number(onion.opacity),
        },
        Row {
            prop: prop("fade"),
            label: "Fade by".to_string(),
            kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
            value: PropValue::Number(onion.fade),
        },
        Row {
            prop: prop("keys_only"),
            label: "Keyframes only".to_string(),
            kind: RowKind::Toggle,
            value: PropValue::Bool(onion.keys_only),
        },
    ]
}

/// The panel's page for the film, when one shows: a clip's at the film's
/// level when one is selected, the storyboard's when none is, and a layer's
/// inside a clip after its row was chosen.
pub fn panel_sections(session: &Session) -> Option<Vec<Section>> {
    let record = session.active_storyboard();
    let film = record.film();
    let view = record.view();
    let text_row = |prop: Value, label: &str, value: &str, lines: u8| Row {
        prop,
        label: label.to_string(),
        kind: if lines > 1 { RowKind::Paragraph { lines } } else { RowKind::Text },
        value: PropValue::Text(value.to_string()),
    };
    match view.clip {
        None => {
            let picked = session.picked();
            if let Some(track) = picked.track.and_then(|track| film.sound_track(track)) {
                let prop = json!({ "film": "track", "track": track.id, "field": "name" });
                return Some(vec![Section { title: "Track".to_string(), rows: vec![text_row(prop, "Name", &track.name, 1)], icon: None }]);
            }
            if let Some((_, sound)) = picked.sounds.first().and_then(|sound| film.sound(*sound)) {
                let prop = |field: &str| json!({ "film": "sound", "sound": sound.id, "field": field });
                return Some(vec![Section {
                    title: "Sound".to_string(),
                    rows: vec![
                        Row {
                            prop: json!({ "film": "note" }),
                            label: format!("File: {}", sound.name),
                            kind: RowKind::Note,
                            value: PropValue::Idle,
                        },
                        Row {
                            prop: prop("volume"),
                            label: "Volume".to_string(),
                            kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
                            value: PropValue::Number(sound.volume),
                        },
                    ]
                    .into_iter()
                    // A gain above 0 dB, which 0–100% cannot show, plays at
                    // 100% and is written back as read until it is changed.
                    .chain(session.kept_gain(sound.id).map(|gain_db| Row {
                        prop: json!({ "film": "note" }),
                        label: format!("+{} dB, kept as read", crate::assembly::said_gain(gain_db).trim_end_matches(" dB")),
                        kind: RowKind::Note,
                        value: PropValue::Idle,
                    }))
                    .chain([
                        text_row(prop("at"), "Starts at", &timecode(sound.at), 1),
                        text_row(prop("from"), "From", &timecode(sound.from), 1),
                    ])
                    .collect(),
                    icon: None,
                }]);
            }
            if let Some(clip) = picked.clips.first().and_then(|clip| film.clip(*clip)) {
                let prop = |field: &str| json!({ "film": "clip", "clip": clip.id, "field": field });
                let length = format!("{} s {} f", clip.length / FPS, clip.length % FPS);
                return Some(vec![
                    Section {
                        title: "Clip".to_string(),
                        rows: vec![
                            text_row(prop("name"), "Name", &clip.name, 1),
                            text_row(prop("shot"), "Shot", &clip.shot, 1),
                            Row {
                                prop: prop("length"),
                                label: "Length".to_string(),
                                kind: RowKind::Number { min: 1.0, max: 100_000.0, percent: false, places: 0, centered: false },
                                value: PropValue::Number(f64::from(clip.length)),
                            },
                            Row {
                                prop: json!({ "film": "note" }),
                                label: length,
                                kind: RowKind::Note,
                                value: PropValue::Idle,
                            },
                        ],
                        icon: None,
                    },
                    Section {
                        title: "Notes".to_string(),
                        rows: vec![
                            text_row(prop("action"), "Action", &clip.action, 3),
                            text_row(prop("dialogue"), "Dialogue", &clip.dialogue, 3),
                            text_row(prop("camera"), "Camera", &clip.camera, 3),
                            text_row(prop("animator"), "Animator", &clip.animator, 1),
                        ],
                        icon: None,
                    },
                ]);
            }
            let prop = |field: &str| json!({ "film": "storyboard", "field": field });
            Some(vec![
                Section {
                    title: "Storyboard".to_string(),
                    rows: vec![text_row(prop("title"), "Title", &film.title, 1), text_row(prop("scene"), "Scene", &film.scene, 1)],
                    icon: None,
                },
                Section {
                    title: "Film".to_string(),
                    rows: vec![
                        Row {
                            prop: json!({ "film": "note" }),
                            label: "Frame rate".to_string(),
                            kind: RowKind::Note,
                            value: PropValue::Idle,
                        },
                        Row {
                            prop: prop("width"),
                            label: "Frame width".to_string(),
                            kind: RowKind::Number { min: 16.0, max: 16384.0, percent: false, places: 0, centered: false },
                            value: PropValue::Number(f64::from(film.size[0])),
                        },
                        Row {
                            prop: prop("height"),
                            label: "Frame height".to_string(),
                            kind: RowKind::Number { min: 16.0, max: 16384.0, percent: false, places: 0, centered: false },
                            value: PropValue::Number(f64::from(film.size[1])),
                        },
                        Row {
                            prop: prop("paper"),
                            label: "Paper".to_string(),
                            kind: RowKind::Choice {
                                options: crate::film::PAPER_NAMES.iter().map(|name| name.to_string()).collect(),
                                segmented: false,
                                preview: protocol::ChoicePreview::Label,
                            },
                            value: PropValue::Index(film.paper),
                        },
                    ],
                    icon: None,
                },
            ])
            .map(|mut sections| {
                // The frame rate is fixed at 24 for now: it reads as a note.
                if let Some(row) = sections[1].rows.first_mut() {
                    row.label = format!("Frame rate: {FPS} fps");
                }
                sections
            })
        }
        Some(clip) if session.layer_page() => {
            let layer = view.layer.and_then(|layer| film.clip(clip)?.layer(layer).cloned())?;
            let prop = |field: &str| json!({ "film": "layer", "layer": layer.id, "field": field });
            Some(vec![Section {
                title: if layer.folder { "Folder" } else { "Layer" }.to_string(),
                rows: vec![
                    text_row(prop("name"), "Name", &layer.name, 1),
                    Row {
                        prop: prop("color"),
                        label: "Colour".to_string(),
                        kind: RowKind::Rgb { quick: crate::film::LAYER_COLORS.to_vec() },
                        value: PropValue::Rgb(layer.color),
                    },
                    Row {
                        prop: prop("opacity"),
                        label: "Opacity".to_string(),
                        kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
                        value: PropValue::Number(layer.opacity),
                    },
                ],
                icon: None,
            }])
        }
        Some(_) => None,
    }
}

/// A panel or popover row's edit. Returns whether the row was the film's.
pub fn property(session: &mut Session, prop: &Value, value: &Value, preview: bool) -> bool {
    let Some(kind) = prop.get("film").and_then(Value::as_str) else {
        return false;
    };
    let field = prop.get("field").and_then(Value::as_str).unwrap_or_default();
    let text = value.as_str().map(str::to_string).or_else(|| value.get("text").and_then(Value::as_str).map(str::to_string));
    let number = value.as_f64().or_else(|| value.get("number").and_then(Value::as_f64));
    // Typing and dragging preview nothing: the edit lands on release.
    if preview {
        return true;
    }
    match kind {
        "clip" => {
            let Some(clip) = prop.get("clip").and_then(Value::as_u64) else {
                return true;
            };
            let which = match field {
                "name" => Some(ClipText::Name),
                "shot" => Some(ClipText::Shot),
                "action" => Some(ClipText::Action),
                "dialogue" => Some(ClipText::Dialogue),
                "camera" => Some(ClipText::Camera),
                "animator" => Some(ClipText::Animator),
                _ => None,
            };
            if let (Some(which), Some(text)) = (which, text) {
                session.set_clip_text(clip, which, &text);
            } else if field == "length"
                && let Some(number) = number
                && let Some(current) = session.film().clip(clip).map(|clip| clip.length)
            {
                session.resize_clip(clip, true, number.round() as i64 - i64::from(current));
            }
        }
        "storyboard" => match field {
            "title" => {
                session.set_film_settings(text.as_deref(), None, None, None);
            }
            "scene" => {
                session.set_film_settings(None, text.as_deref(), None, None);
            }
            "width" | "height" => {
                if let Some(number) = number {
                    let mut size = session.film().size;
                    size[usize::from(field == "height")] = number.round().max(16.0) as u32;
                    session.set_film_settings(None, None, Some(size), None);
                }
            }
            "paper" => {
                if let Some(index) = value.as_u64().or_else(|| value.get("index").and_then(Value::as_u64)) {
                    session.set_film_settings(None, None, None, Some(index as u8));
                }
            }
            _ => {}
        },
        "layer" => {
            let Some(layer) = prop.get("layer").and_then(Value::as_u64) else {
                return true;
            };
            match field {
                "name" => {
                    if let Some(text) = text {
                        session.rename_layer(layer, &text);
                    }
                }
                "opacity" => {
                    if let Some(number) = number {
                        session.set_layer_opacity(layer, number);
                    }
                }
                "color" => {
                    if let Some(rgb) = rgb_of(value) {
                        session.set_layer_color(layer, rgb);
                    }
                }
                _ => {}
            }
        }
        "sound" => {
            let Some(sound) = prop.get("sound").and_then(Value::as_u64) else {
                return true;
            };
            match field {
                "volume" => {
                    if let Some(number) = number {
                        session.set_sound_volume(sound, number);
                    }
                }
                "at" | "from" => {
                    if let Some(frame) = text.as_deref().and_then(parse_timecode).or_else(|| number.map(|number| number.max(0.0).round() as u32)) {
                        if field == "at" {
                            session.set_sound_span(sound, Some(frame), None, None);
                        } else {
                            session.set_sound_span(sound, None, Some(frame), None);
                        }
                    }
                }
                _ => {}
            }
        }
        "track" => {
            if let (Some(track), Some(text)) = (prop.get("track").and_then(Value::as_u64), text) {
                session.rename_sound_track(track, &text);
            }
        }
        "onion" => {
            session.set_onion(|onion| match field {
                "before" => onion.before = number.unwrap_or(f64::from(onion.before)).round() as u8,
                "after" => onion.after = number.unwrap_or(f64::from(onion.after)).round() as u8,
                "opacity" => onion.opacity = number.unwrap_or(onion.opacity),
                "fade" => onion.fade = number.unwrap_or(onion.fade),
                "keys_only" => onion.keys_only = value.as_bool().or_else(|| value.get("bool").and_then(Value::as_bool)).unwrap_or(onion.keys_only),
                _ => {}
            });
        }
        _ => {}
    }
    true
}

fn rgb_of(value: &Value) -> Option<[u8; 3]> {
    let array = value.as_array().or_else(|| value.get("rgb").and_then(Value::as_array))?;
    let channel = |index: usize| array.get(index).and_then(Value::as_u64).map(|value| value.min(255) as u8);
    Some([channel(0)?, channel(1)?, channel(2)?])
}

fn id_of(arg: Option<&Value>) -> Option<u64> {
    let arg = arg?;
    arg.as_u64()
        .or_else(|| arg.as_str().and_then(|text| text.parse().ok()))
        .or_else(|| arg.get("arg").and_then(Value::as_u64))
}

/// The cells a frame command acts on: the selected block, or the active
/// layer at the playhead.
fn acting_range(session: &Session) -> (Vec<u64>, u32, u32) {
    match session.picked().cells {
        Some(cells) if !cells.layers.is_empty() => (cells.layers, cells.start, cells.end),
        _ => {
            let frame = session.view().frame;
            (session.acting_layers(), frame, frame)
        }
    }
}

/// A timeline command. Returns whether it was one.
pub fn command(session: &mut Session, id: &str, arg: Option<&Value>) -> bool {
    match id {
        "new_clip" => {
            session.add_clip(None, NEW_CLIP_LENGTH);
        }
        "edit_clip" => {
            if let Some(clip) = id_of(arg) {
                session.enter_clip(clip);
            }
        }
        "leave_clip" => {
            session.leave_clip();
        }
        "rename_clip" => {
            if let Some(clip) = id_of(arg) {
                session.select_clips(vec![clip]);
                session.open_panel();
            }
        }
        "duplicate_clip" => {
            if let Some(clip) = id_of(arg) {
                session.duplicate_clip(clip);
            }
        }
        "delete_clip" => {
            if let Some(clip) = id_of(arg) {
                session.delete_clip(clip);
            }
        }
        "step_back" | "step_forward" => {
            let back = id == "step_back";
            let view = session.view();
            if view.clip.is_some() {
                session.seek_clip(if back { view.frame.saturating_sub(1) } else { view.frame + 1 });
            } else {
                session.seek_film(if back { view.film_frame.saturating_sub(1) } else { view.film_frame + 1 });
            }
            // A step by hand plays that frame's slice of sound, to find a
            // mouth by ear.
            session.hear_playhead();
        }
        "play" => session.toggle_play(),
        "toggle_loop" => session.toggle_loop(),
        "set_volume" | "finish_volume" => {
            let value = arg.and_then(|arg| arg.get("value")).and_then(Value::as_f64).or_else(|| arg.and_then(Value::as_f64));
            if let Some(value) = value {
                session.set_listening(value / 100.0);
            }
        }
        "timeline_page" => {}
        "import_sound" => session.ask_import_sound(),
        "insert_image_key" => session.ask_image_key(),
        "publish_hold" if session.open_clip().is_some() => {
            if let Err(reason) = session.publish_hold() {
                session.failed_by_hand(Some("Hold not published"), &reason);
            }
        }
        "open_animatic" => {
            if let Err(reason) = session.open_animatic(None) {
                session.failed_by_hand(Some("Animatic not opened"), &reason);
            }
        }
        "save_timing" => {
            if let Err(reason) = session.save_timing() {
                session.timing_refused(&reason);
            }
        }
        "follow_retime" => {
            if let Err(reason) = session.follow_retime() {
                session.failed_by_hand(None, &reason);
            }
        }
        "export_animatic" => {
            let _ = session.start_export();
        }
        "cancel_export" => {
            session.cancel_export();
        }
        "close_notice" => {
            if let Some(id) = arg.and_then(Value::as_str).or_else(|| arg.and_then(|arg| arg.get("arg")).and_then(Value::as_str)) {
                session.close_notice(id);
            }
        }
        "new_sound_track" => {
            session.add_sound_track();
        }
        "rename_sound_track" => {
            if let Some(track) = id_of(arg) {
                session.pick_track(track);
            }
        }
        "delete_sound_track" => {
            if let Some(track) = id_of(arg) {
                session.delete_sound_track(track);
            }
        }
        "delete_sound" => {
            if let Some(sound) = id_of(arg) {
                session.delete_sound(sound);
            }
        }
        "track_flag" => {
            let track = arg.and_then(|arg| arg.get("track")).and_then(Value::as_str).and_then(TrackRef::from_row);
            let flag = match arg.and_then(|arg| arg.get("flag")).and_then(Value::as_str) {
                Some("locked") => Some(TrackFlag::Locked),
                Some("visible") => Some(TrackFlag::Visible),
                Some("muted") => Some(TrackFlag::Muted),
                _ => None,
            };
            if let (Some(track), Some(flag)) = (track, flag) {
                let film = session.film();
                let on = match (track, flag) {
                    (TrackRef::Shots, TrackFlag::Locked) => film.shots_locked,
                    (TrackRef::Shots, _) => film.shots_visible,
                    (TrackRef::Sound(id), TrackFlag::Locked) => film.sound_track(id).is_some_and(|track| track.locked),
                    (TrackRef::Sound(id), _) => film.sound_track(id).is_some_and(|track| track.muted),
                };
                session.set_track_flag(track, flag, !on);
            }
        }
        "toggle_onion" => {
            session.set_onion(|onion| onion.on = !onion.on);
        }
        "insert_frame" => {
            let (layers, frame, end) = acting_range(session);
            session.insert_frames(&layers, frame, end - frame + 1);
        }
        "insert_keyframe" | "insert_blank_keyframe" => {
            let blank = id == "insert_blank_keyframe";
            let (layers, frame, _) = acting_range(session);
            if let Some(layer) = layers.first() {
                session.insert_key(*layer, frame, blank);
            }
        }
        "clear_keyframe" => {
            let (layers, frame, _) = acting_range(session);
            for layer in layers {
                session.clear_key(layer, frame);
            }
        }
        "remove_frames" => {
            let (layers, frame, end) = acting_range(session);
            session.remove_frames(&layers, frame, end - frame + 1);
        }
        "copy_frames" => {
            let (layers, start, end) = acting_range(session);
            session.copy_frames(&layers, start, end);
        }
        "paste_frames" => {
            let (layers, frame, _) = acting_range(session);
            session.paste_frames(&layers, frame);
        }
        "reverse_frames" => {
            let (layers, start, end) = acting_range(session);
            session.reverse_frames(&layers, start, end);
        }
        "add_mark" => {
            let comment = arg.and_then(|arg| arg.get("comment")).and_then(Value::as_bool).unwrap_or(false);
            let text = arg
                .and_then(|arg| arg.get("value").or_else(|| arg.get("text")))
                .and_then(Value::as_str)
                .unwrap_or(if comment { "Comment" } else { "Label" })
                .to_string();
            let (layers, frame, _) = acting_range(session);
            if let Some(layer) = layers.first() {
                session.add_mark(*layer, frame, comment, &text);
            }
        }
        "new_layer" => {
            session.add_layer(false);
        }
        "new_folder" => {
            session.add_layer(true);
        }
        "delete_layer" => {
            if let Some(layer) = session.view().layer {
                session.delete_layer(layer);
            }
        }
        "layer_flag" => {
            let layer = arg.and_then(|arg| arg.get("layer")).and_then(Value::as_u64);
            let flag = match arg.and_then(|arg| arg.get("flag")).and_then(Value::as_str) {
                Some("visible") => Some(LayerFlag::Visible),
                Some("locked") => Some(LayerFlag::Locked),
                Some("outline") => Some(LayerFlag::Outline),
                Some("open") => Some(LayerFlag::Open),
                _ => None,
            };
            if let (Some(layer), Some(flag)) = (layer, flag)
                && let Some(current) = session.open_clip().and_then(|clip| session.film().clip(clip)?.layer(layer).cloned())
            {
                let on = match flag {
                    LayerFlag::Visible => current.visible,
                    LayerFlag::Locked => current.locked,
                    LayerFlag::Outline => current.outline,
                    LayerFlag::Open => current.open,
                };
                session.set_layer_flag(layer, flag, !on);
            }
        }
        _ => return false,
    }
    true
}

fn row_layer(pick: &TimelinePick) -> Option<u64> {
    pick.row.parse().ok()
}

/// A finished gesture on the timeline.
pub fn gesture(session: &mut Session, gesture: TimelineGesture) {
    let in_clip = session.open_clip().is_some();
    match gesture {
        TimelineGesture::Select { picks, add, toggle } => {
            if in_clip {
                let Some(pick) = picks.first() else {
                    return;
                };
                if let Some(layer) = row_layer(pick) {
                    session.select_layer(layer);
                    session.show_layer_page(true);
                }
                if let Some(frame) = pick.frame {
                    session.seek_clip(frame);
                    let layers: Vec<u64> = picks.iter().filter_map(row_layer).collect();
                    session.select_cells(layers, frame, frame);
                }
                return;
            }
            if picks.iter().any(|pick| matches!(TrackRef::from_row(&pick.row), Some(TrackRef::Sound(_)))) {
                let clicked: Vec<u64> = picks.iter().filter_map(|pick| pick.item.as_ref()?.parse().ok()).collect();
                let mut sounds = if add || toggle { session.picked().sounds } else { Vec::new() };
                for sound in clicked {
                    if toggle && sounds.contains(&sound) {
                        sounds.retain(|at| *at != sound);
                    } else if !sounds.contains(&sound) {
                        sounds.push(sound);
                    }
                }
                if let Some(frame) = picks.iter().find_map(|pick| pick.frame) {
                    session.seek_film(frame);
                }
                session.select_sounds(sounds);
                return;
            }
            let clicked: Vec<u64> = picks.iter().filter_map(|pick| pick.item.as_ref()?.parse().ok()).collect();
            let mut clips = if add || toggle { session.picked().clips } else { Vec::new() };
            for clip in clicked {
                if toggle && clips.contains(&clip) {
                    clips.retain(|at| *at != clip);
                } else if !clips.contains(&clip) {
                    clips.push(clip);
                }
            }
            if let Some(frame) = picks.iter().find_map(|pick| pick.frame) {
                session.seek_film(frame);
            }
            session.select_clips(clips);
        }
        TimelineGesture::SelectCells { rows, start, end } => {
            let layers: Vec<u64> = rows.iter().filter_map(|row| row.parse().ok()).collect();
            if let Some(layer) = layers.first() {
                session.select_layer(*layer);
            }
            session.select_cells(layers, start, end);
            session.seek_clip(start.min(end));
        }
        TimelineGesture::Move { picks, frames, push, row } => {
            let on_sound = picks.iter().any(|pick| matches!(TrackRef::from_row(&pick.row), Some(TrackRef::Sound(_))));
            if !in_clip && on_sound {
                let mut sounds: Vec<u64> = picks.iter().filter_map(|pick| pick.item.as_ref()?.parse().ok()).collect();
                if sounds.is_empty() {
                    sounds = session.picked().sounds;
                }
                let to = match row.as_deref().map(TrackRef::from_row) {
                    Some(Some(TrackRef::Sound(track))) => Some(track),
                    // Onto the Shots track, or no row: a sound stays a sound.
                    Some(_) => return,
                    None => None,
                };
                session.move_sounds(&sounds, to, frames, push);
                session.select_sounds(sounds);
                return;
            }
            if !in_clip && matches!(row.as_deref().map(TrackRef::from_row), Some(Some(TrackRef::Sound(_)))) {
                // A clip dragged onto a Sound track stays where it was.
                return;
            }
            if in_clip {
                let (layers, start, end) = match session.picked().cells {
                    Some(cells) => (cells.layers, cells.start, cells.end),
                    None => {
                        let layers: Vec<u64> = picks.iter().filter_map(row_layer).collect();
                        let frame = picks.iter().filter_map(|pick| pick.frame).min().unwrap_or(0);
                        (layers, frame, frame)
                    }
                };
                session.move_frames(&layers, start, end + 1, frames, push);
            } else {
                let mut clips: Vec<u64> = picks.iter().filter_map(|pick| pick.item.as_ref()?.parse().ok()).collect();
                if clips.is_empty() {
                    clips = session.picked().clips;
                }
                session.move_clips(&clips, frames, push);
                session.select_clips(clips);
            }
        }
        TimelineGesture::Resize { pick, edge, frames } => {
            let end = edge == protocol::Edge::End;
            if in_clip {
                let (Some(layer), Some(frame)) = (row_layer(&pick), pick.frame) else {
                    return;
                };
                let key_frame = session
                    .open_clip()
                    .and_then(|clip| session.film().clip(clip)?.layer(layer)?.key_at(frame).map(|key| key.frame));
                if let Some(key_frame) = key_frame {
                    session.stretch_span(layer, key_frame, frames);
                }
            } else if let Some(item) = pick.item.as_ref().and_then(|item| item.parse().ok()) {
                if let Some(TrackRef::Sound(_)) = TrackRef::from_row(&pick.row) {
                    session.trim_sound(item, end, frames);
                    session.select_sounds(vec![item]);
                } else {
                    session.resize_clip(item, end, frames);
                    session.select_clips(vec![item]);
                }
            }
        }
        TimelineGesture::Range { id, start, end } => {
            if id != "onion" {
                return;
            }
            let view = session.view();
            let Some(layer) = view
                .clip
                .and_then(|clip| session.film().clip(clip)?.layer(view.layer?).cloned())
            else {
                return;
            };
            let before = layer.keys.iter().filter(|key| key.frame >= start && key.frame < view.frame).count();
            let shown = layer.keys.iter().rposition(|key| key.frame <= view.frame);
            let after = layer
                .keys
                .iter()
                .enumerate()
                .filter(|(index, key)| shown.is_some_and(|shown| *index > shown) && key.frame <= end)
                .count();
            session.set_onion(|onion| {
                onion.before = before.min(9) as u8;
                onion.after = after.min(9) as u8;
            });
        }
    }
}

/// Dragging the playhead, live or on release. Never an undo step.
pub fn seek(session: &mut Session, frame: u32) {
    if session.open_clip().is_some() {
        session.seek_clip(frame);
    } else {
        session.seek_film(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes_read_as_the_ruler_does() {
        assert_eq!(timecode(0), "00:00:00:00");
        assert_eq!(timecode(156), "00:00:06:12");
        assert_eq!(timecode(24 * 3600), "01:00:00:00");
    }

    #[test]
    fn the_film_and_a_clip_declare_their_timelines() {
        let mut session = Session::new();
        let mut boards = Boards::default();
        let mut looker = Looker::new();
        let empty = declare(&session, &mut boards, &mut looker, "1".into());
        assert_eq!(empty.empty_hint, "Right-click to add a clip");
        assert!(command(&mut session, "new_clip", None));
        assert!(command(&mut session, "new_clip", None));
        let film = declare(&session, &mut boards, &mut looker, "1".into());
        let RowContent::Items { items } = &film.rows[0].content else {
            panic!("the Shots track holds clips");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].label, "Shot 2");
        assert!(items[0].thumbnail.is_some());
        let clip: u64 = items[0].id.parse().unwrap();
        assert!(command(&mut session, "edit_clip", Some(&json!(clip))));
        let inside = declare(&session, &mut boards, &mut looker, "1".into());
        assert_eq!(inside.back.as_ref().unwrap().path, vec!["Film".to_string(), "Shot 1".to_string()]);
        assert_eq!(inside.ruler.unit, RulerUnit::Frames);
        assert_eq!(inside.rows.len(), 1);
        // A blank first key, held to the clip's end.
        let RowContent::Cells { marks, .. } = &inside.rows[0].content else {
            panic!("a layer holds cells");
        };
        assert!(matches!(marks[0].kind, MarkKind::BlankKey));
        assert!(command(&mut session, "leave_clip", None));
        assert!(session.open_clip().is_none());
    }

    #[test]
    fn sound_tracks_declare_their_rows_and_take_their_gestures() {
        let project = std::env::temp_dir().join(format!("storyboard-timeline-sound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&project);
        std::fs::create_dir_all(&project).unwrap();
        let source = project.join("line.wav");
        std::fs::write(&source, crate::sounds::wav_bytes(crate::sounds::RATE, &crate::sounds::clicks(48, &[10]))).unwrap();
        let mut session = Session::new();
        session.set_project(&project);
        session.add_clip(None, 96);
        let sound = session.import_sound(&source, None, Some(0)).unwrap();
        session.load_sounds();
        let mut boards = Boards::default();
        let mut looker = Looker::new();
        let film = declare(&session, &mut boards, &mut looker, "1".into());
        assert_eq!(film.rows.len(), 2);
        assert!(film.add_row.is_some());
        let row = &film.rows[1];
        assert_eq!((row.name.as_str(), row.toggles.len()), ("Sound", 2));
        let RowContent::Items { items } = &row.content else {
            panic!("a Sound track holds sounds");
        };
        assert_eq!(items[0].label, "line.wav");
        assert!(!items[0].waveform.is_empty());
        // The Shots track's toggles are its lock and eye.
        assert_eq!(film.rows[0].toggles.iter().map(|toggle| toggle.tip.as_str()).collect::<Vec<_>>(), ["Lock", "Hide"]);
        // A move on a Sound track moves the sound, not a clip.
        let pick = TimelinePick { row: row.id.clone(), item: Some(sound.to_string()), frame: None };
        gesture(&mut session, TimelineGesture::Move { picks: vec![pick.clone()], frames: 24, row: None, push: true });
        assert_eq!(session.film().sound(sound).unwrap().1.at, 24);
        assert_eq!(session.film().clips[0].start, 0);
        // Dragged onto the Shots track, it stays a sound where it was.
        gesture(&mut session, TimelineGesture::Move { picks: vec![pick.clone()], frames: 5, row: Some("shots".into()), push: true });
        assert_eq!(session.film().sound(sound).unwrap().1.at, 24);
        gesture(&mut session, TimelineGesture::Resize { pick, edge: protocol::Edge::End, frames: -8 });
        assert_eq!(session.film().sound(sound).unwrap().1.length, 40);
        // Its speaker mutes the track.
        let track = session.film().sound_tracks[0].id;
        assert!(command(&mut session, "track_flag", Some(&json!({ "track": format!("sound-{track}"), "flag": "muted" }))));
        assert!(session.film().sound_tracks[0].muted);
        // A locked Shots track keeps its clips where they are.
        let clip = session.film().clips[0].id;
        assert!(command(&mut session, "track_flag", Some(&json!({ "track": "shots", "flag": "locked" }))));
        let shots = TimelinePick { row: "shots".into(), item: Some(clip.to_string()), frame: None };
        gesture(&mut session, TimelineGesture::Move { picks: vec![shots.clone()], frames: 30, row: None, push: true });
        gesture(&mut session, TimelineGesture::Resize { pick: shots, edge: protocol::Edge::End, frames: 30 });
        assert_eq!((session.film().clips[0].start, session.film().clips[0].length), (0, 96));
        let locked = declare(&session, &mut boards, &mut looker, "1".into());
        let RowContent::Items { items } = &locked.rows[0].content else {
            panic!("the Shots track holds clips");
        };
        assert!(!items[0].movable);
        // Its page in the panel.
        session.select_sounds(vec![sound]);
        let page = panel_sections(&session).unwrap();
        assert_eq!(page[0].title, "Sound");
        assert!(property(&mut session, &json!({ "film": "sound", "sound": sound, "field": "from" }), &json!("0:4"), false));
        assert_eq!(session.film().sound(sound).unwrap().1.from, 4);
    }

    /// The plan's measurement: what one whole declaration of the film's
    /// timeline costs to build, write, and read for 150 clips and two
    /// Sound tracks of 20 minutes, which decides whether playing needs a
    /// message that moves only the playhead. Run by hand with --ignored.
    #[test]
    #[ignore]
    fn measure_a_long_films_timeline() {
        let mut session = Session::new();
        for _ in 0..150 {
            session.add_clip(None, 192);
        }
        let frames = 20 * 60 * FPS;
        for name in ["a.wav", "b.wav"] {
            let loaded = std::sync::Arc::new(crate::sounds::Loaded {
                sound: std::sync::Arc::new(media::Sound { rate: crate::sounds::RATE, samples: Vec::new() }),
                fingerprint: name.to_string(),
                peaks: vec![0.3; frames as usize * crate::sounds::PEAKS_PER_FRAME],
                frames,
                stamp: None,
            });
            let imported = crate::sounds::Imported { file: name.to_string(), name: name.to_string(), loaded, path: std::path::PathBuf::from(name) };
            session.place_imported(imported, None, Some(0));
        }
        let mut boards = Boards::default();
        let mut looker = Looker::new();
        declare(&session, &mut boards, &mut looker, "1".into());
        let runs = 20;
        let started = std::time::Instant::now();
        let mut bytes = 0;
        for frame in 0..runs {
            session.seek_film(frame * 10);
            let timeline = declare(&session, &mut boards, &mut looker, "1".into());
            let text = serde_json::to_string(&protocol::AppMessage::Timeline(timeline)).unwrap();
            bytes = text.len();
            let _: protocol::AppMessage = serde_json::from_str(&text).unwrap();
        }
        let each = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        let timeline = declare(&session, &mut boards, &mut looker, "1".into());
        let started = std::time::Instant::now();
        for _ in 0..runs {
            let text = serde_json::to_string(&protocol::AppMessage::Timeline(timeline.clone())).unwrap();
            let _: protocol::AppMessage = serde_json::from_str(&text).unwrap();
        }
        let wire = started.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        println!("one declaration: {each:.2} ms, of which writing and reading {wire:.2} ms; {bytes} bytes");
    }

    #[test]
    fn timecodes_parse_from_any_number_of_parts() {
        assert_eq!(parse_timecode("00:00:02:10"), Some(58));
        assert_eq!(parse_timecode("2:10"), Some(58));
        assert_eq!(parse_timecode("7"), Some(7));
        assert_eq!(parse_timecode("x"), None);
    }

    #[test]
    fn gestures_move_and_retime_clips() {
        let mut session = Session::new();
        session.add_clip(None, 48);
        session.add_clip(None, 48);
        let first = session.film().clips[0].id;
        gesture(&mut session, TimelineGesture::Resize {
            pick: TimelinePick { row: "shots".into(), item: Some(first.to_string()), frame: None },
            edge: protocol::Edge::End,
            frames: 12,
        });
        assert_eq!(session.film().clips[1].start, 60);
        gesture(&mut session, TimelineGesture::Move {
            picks: vec![TimelinePick { row: "shots".into(), item: Some(first.to_string()), frame: None }],
            frames: 200,
            row: None,
            push: true,
        });
        assert_eq!(session.film().clip(first).unwrap().start, 200);
        session.undo().unwrap();
        assert_eq!(session.film().clip(first).unwrap().start, 0);
    }
}
