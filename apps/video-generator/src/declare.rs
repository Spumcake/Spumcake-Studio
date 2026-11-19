//! What the Video Generator declares for the host to draw: the toolbar, the
//! filmstrip with its rounds and a take's menu, the panel's Prompt page,
//! the menu, the keys, and the tab list, in the Image Generator's words
//! wherever a video needs nothing more.

use crate::gallery::Mode;
use crate::rounds::{self, With};
use crate::session::{Page, Session, Tab, Tool};
use protocol::{
    Action, Binding, CellSetting, Chord, Film, FilmGroup, FilmItem, FilmStack, FlagIcons, HeadingAction, Keys, Layers, MenuEntry, PanelCell, PanelPage,
    Properties, ReferenceRole, RowKind, TabEntry, Tabs, Thumbnail, ToolItem, Toolbar,
};
use protocol::{Transport, TransportClock, TransportRow};
use serde_json::json;

/// The Prompt page's words in an empty prompt, as one constant while the
/// owner's word on them is awaited.
pub const HINT: &str = "Describe the shot…";

/// The zoom menu's rows, as the Image Generator's: a word and the zoom it
/// sets, none for Fit. At 100%, two takes side by side are the before/after
/// crop.
const ZOOMS: [(&str, Option<f64>); 4] = [("Fit", None), ("50%", Some(0.5)), ("100%", Some(1.0)), ("200%", Some(2.0))];

fn button(icon: &str, tip: &str, action: Action, enabled: bool, selected: bool) -> ToolItem {
    ToolItem::Button { icon: icon.into(), tip: tip.into(), enabled, selected, action: Some(action), menu: Vec::new() }
}

fn row(label: &str, action: Action) -> MenuEntry {
    MenuEntry { tip: None, label: label.into(), icon: None, enabled: true, divider: false, action: Some(action), submenu: Vec::new(), rename: false }
}

/// Each tool's icon, name, and key, as the Image Generator's.
fn tool_look(tool: Tool) -> (&'static str, &'static str, &'static str) {
    match tool {
        Tool::Move => ("cursor-pointer", "Move", "V"),
        Tool::Hand => ("drag-hand-gesture", "Hand", "H"),
        Tool::Zoom => ("search", "Zoom", "Z"),
        // A speech bubble, as vg-notes-at-time shows; it has no key.
        Tool::Notes => ("chat-bubble", "Notes", ""),
    }
}

/// The toolbar: open, undo and redo, the tools, and the frame strips'
/// grid button, then the zoom menu and full screen, with the notes tool's
/// settings in the row under it.
pub fn toolbar(session: &Session) -> Toolbar {
    let tab = session.active_tab();
    let can_undo = tab.is_some_and(|tab| tab.gallery.can_undo());
    let can_redo = tab.is_some_and(|tab| tab.gallery.can_redo());
    // The grid shows the gallery's takes as frame strips, or a round's
    // take's round whole, as vg-frame-strips shows.
    let strips = tab.is_some_and(|tab| tab.round_view().as_ref().unwrap_or(&tab.gallery).view() == Mode::Strips);
    let grid = tab.is_some_and(|tab| !tab.gallery.takes.is_empty() || tab.rounds.shown.is_some());
    let mut items = vec![
        button("folder", "Open (Ctrl+O)", Action::new("open"), true, false),
        ToolItem::Separator,
        button("undo", "Undo (Ctrl+Z)", Action::new("undo"), can_undo, false),
        button("redo", "Redo (Ctrl+Shift+Z)", Action::new("redo"), can_redo, false),
        ToolItem::Separator,
    ];
    for tool in Tool::ALL {
        let (icon, name, key) = tool_look(tool);
        let tip = if key.is_empty() { name.to_string() } else { format!("{name} ({key})") };
        items.push(button(icon, &tip, Action::with_arg("tool", json!({ "tool": tool.key() })), true, session.tools.tool == tool));
    }
    items.extend([
        ToolItem::Separator,
        // Its tip is the guide's word for what it shows whole.
        button("view-grid", "Round", Action::new("view_strips"), grid, strips),
    ]);
    let zoom = tab.and_then(|tab| tab.zoom);
    let label = match zoom {
        Some(zoom) => format!("{}%", (zoom * 100.0).round()),
        None => String::from("Fit"),
    };
    // The pre-vis check, as the Image Generator's: a round's take shown
    // over its animatic, as vg-take-over-animatic shows; off for a take not
    // in a round, or a round with neither a Motion reference nor a Start
    // frame.
    let animatic = tab.is_some_and(|tab| session.animatic_of(tab).is_some());
    let over = tab.is_some_and(|tab| tab.over_animatic) && animatic;
    let trailing = vec![
        ToolItem::Separator,
        button("cube", "Pre-vis check", Action::new("previs_toggle"), animatic, over),
        ToolItem::Separator,
        ToolItem::Menu {
            icon: String::new(),
            label,
            enabled: true,
            rows: ZOOMS.iter().map(|(label, zoom)| row(label, Action::with_arg("view_zoom", json!({ "zoom": zoom })))).collect(),
        },
        ToolItem::Maximize { icon: "maximize".into(), restore_icon: "reduce".into(), tip: "Full screen".into() },
    ];
    Toolbar { items, trailing, options: settings_row(session), options_trailing: Vec::new() }
}

/// The chosen tool's settings, as the Image Generator's: the notes tool's
/// colour, then Pin or Draw, as vg-notes-at-time shows; the other tools
/// have none.
fn settings_row(session: &Session) -> Vec<ToolItem> {
    let tools = &session.tools;
    match tools.tool {
        Tool::Notes => vec![
            ToolItem::Color { rgb: tools.rgb, tip: "Colour".into(), set: Action::new("notes_colour") },
            ToolItem::Separator,
            ToolItem::Segmented { options: vec!["Pin".into(), "Draw".into()], selected: usize::from(tools.draw), set: Action::new("notes_mode") },
        ],
        Tool::Move | Tool::Hand | Tool::Zoom => Vec::new(),
    }
}

/// The menu: new and open, Save and Save as outside a project, and undo.
pub fn menu(session: &Session) -> Vec<MenuEntry> {
    let mut rows = vec![row("New video", Action::new("new_video")), row("Open video…", Action::new("open"))];
    if session.project.is_none() {
        rows.push(row("Save", Action::new("save")));
        rows.push(row("Save as…", Action::new("save_as")));
    }
    let tab = session.active_tab();
    let mut undo = row("Undo", Action::new("undo"));
    undo.divider = true;
    undo.enabled = tab.is_some_and(|tab| tab.gallery.can_undo());
    rows.push(undo);
    let mut redo = row("Redo", Action::new("redo"));
    redo.enabled = tab.is_some_and(|tab| tab.gallery.can_redo());
    rows.push(redo);
    rows
}

/// The keys: undo and redo, new and open, and the frame shown: Left and
/// Right one frame, with Shift one second, Home and End the first and last.
pub fn keys() -> Keys {
    let chord = |key: &str, command: bool, shift: Option<bool>| Chord { key: key.into(), command, shift, alt: false };
    let bind = |chord: Chord, action: Action| Binding { chord, then: Vec::new(), action };
    let frame = |arg: serde_json::Value| Action::with_arg("frame", arg);
    let bindings = vec![
        bind(chord("Z", true, Some(false)), Action::new("undo")),
        bind(chord("Z", true, Some(true)), Action::new("redo")),
        bind(chord("Y", true, Some(false)), Action::new("redo")),
        bind(chord("N", true, Some(false)), Action::new("new_video")),
        bind(chord("O", true, Some(false)), Action::new("open")),
        bind(chord("V", false, Some(false)), Action::with_arg("tool", json!({ "tool": "move" }))),
        bind(chord("H", false, Some(false)), Action::with_arg("tool", json!({ "tool": "hand" }))),
        bind(chord("Z", false, Some(false)), Action::with_arg("tool", json!({ "tool": "zoom" }))),
        bind(chord("Left", false, Some(false)), frame(json!({ "step": -1 }))),
        bind(chord("Right", false, Some(false)), frame(json!({ "step": 1 }))),
        bind(chord("Left", false, Some(true)), frame(json!({ "seconds": -1 }))),
        bind(chord("Right", false, Some(true)), frame(json!({ "seconds": 1 }))),
        bind(chord("Home", false, Some(false)), frame(json!({ "to": "first" }))),
        bind(chord("End", false, Some(false)), frame(json!({ "to": "last" }))),
    ];
    Keys { bindings, copy: None, cut: None, paste: None }
}

/// The filmstrip, as the Image Generator's: each round's takes first, under
/// its label, as vg-filmstrip-rounds shows them, then every take of the
/// gallery, each its first frame with its length and its menu, the shown
/// one, and the + tile's row. A click with `extend` set shows a take of
/// the gallery beside the one shown.
pub fn film(session: &Session, tab: &Tab) -> Film {
    let rounds = &tab.rounds;
    let mut items: Vec<FilmItem> = rounds
        .tiles
        .iter()
        .enumerate()
        .map(|(index, tile)| {
            let read = tile.path().and_then(|path| rounds.read(path));
            // A take not made yet, or not read yet, is a place of the
            // takes' shape.
            let size = read.and_then(|read| read.info).map(|info| (info.width, info.height)).or_else(|| rounds.shape());
            let placeholder = || {
                let (width, height) = size.map_or((78, 44), |(width, height)| crate::session::film_size(width, height));
                Thumbnail { path: String::new(), width, height, revision: 0 }
            };
            // A round's tile is its asset, as the Image Generator's: outlined
            // while out of date, a stack while it has iterations,
            // Unverified when nothing records what it was made from. An
            // opened stack's tiles are numbered.
            let asset = tile.asset.as_ref();
            let round = tile.iteration.is_none();
            let (name, caption) = match (tile.iteration, asset) {
                (Some(at), Some(asset)) => (format!("Take {} · iteration {at} of {}", tile.take + 1, 1 + asset.iterations.len()), at.to_string()),
                (None, Some(asset)) if asset.unverified => (format!("Take {}", tile.take + 1), String::from("Unverified")),
                _ => (format!("Take {}", tile.take + 1), String::new()),
            };
            let outlined = round && asset.is_some_and(|asset| asset.outlined);
            FilmItem {
                name,
                thumbnail: read.and_then(|read| session.films.get(&read.file).cloned()).unwrap_or_else(placeholder),
                menu: tile.path().map(|path| take_menu(session, tab, tile, path, outlined)).unwrap_or_default(),
                // The take's length, as vg-filmstrip-rounds' badge shows it.
                badge: read.and_then(|read| read.info).map(|info| crate::preview::length(&info)).unwrap_or_default(),
                mark: tile.file.as_ref().filter(|file| file.canon).map(|_| String::from("badge-check")),
                // The take shown, and one beside it.
                chosen: rounds.shown == Some(index) || tile.path().is_some_and(|path| rounds.with == With::Tile(path.to_string())),
                pending: tile.pending.clone(),
                outlined,
                stack: asset.filter(|asset| round && !asset.iterations.is_empty()).map(|asset| FilmStack {
                    count: 1 + asset.iterations.len(),
                    open: Action::with_arg("take_stack", json!({ "take": asset.path })),
                }),
                caption,
            }
        })
        .collect();
    // A finishing round's run is labelled as its take is, "Take 1 · 60 fps
    // · 4K, from Take 7".
    let mut first = 0;
    let mut groups: Vec<FilmGroup> = rounds
        .groups()
        .into_iter()
        .map(|(label, count)| {
            let label = rounds.tiles.get(first).filter(|tile| tile.iteration.is_none()).and_then(|tile| tab.finishing_label(tile)).unwrap_or(label);
            first += count;
            FilmGroup { label, count, menu: Vec::new() }
        })
        .collect();
    if !groups.is_empty() && !tab.gallery.takes.is_empty() {
        // The gallery's own takes are a run of their own, with no label.
        groups.push(FilmGroup { label: String::new(), count: tab.gallery.takes.len(), menu: Vec::new() });
    }
    let own = tab
        .gallery
        .takes
        .iter()
        .enumerate()
        .map(|(index, take)| {
            let target = json!({ "take": index });
            // The host renames in place and sends the name typed as `value`.
            let mut rename = row("Rename", Action::with_arg("film_rename", target.clone()));
            rename.rename = true;
            let mut delete = row("Delete", Action::with_arg("film_delete", target.clone()));
            delete.divider = true;
            FilmItem {
                name: take.name.clone(),
                thumbnail: session.films.get(&take.file).cloned().unwrap_or(Thumbnail { path: String::new(), width: 1, height: 1, revision: 0 }),
                menu: vec![row("Duplicate", Action::with_arg("film_duplicate", target)), rename, delete],
                // The take's length, as vg-gallery-anchor's badge shows it.
                badge: take.seconds().map(|seconds| format!("{seconds:.1} s")).unwrap_or_default(),
                mark: None,
                // Beside a round's take shown.
                chosen: rounds.shown.is_some() && matches!(rounds.with, With::Own { uid, .. } if uid == take.uid),
                pending: None,
                outlined: false,
                stack: None,
                caption: String::new(),
            }
        });
    let tiles = items.len();
    items.extend(own);
    Film {
        items,
        shown: rounds.shown.unwrap_or(tiles + tab.gallery.shown),
        select: Action::new("film_select"),
        reorder: Action::new("film_reorder"),
        add: vec![row("Open video…", Action::new("film_open")), row(crate::session::OPEN_SOUND, Action::new("open_sound"))],
        focus: Action::new("film_focus"),
        groups,
    }
}

/// The transport row under the take shown, alone or first of two side by
/// side, as vg-gallery-anchor shows it: its length and frame rate, the time
/// the Player's clock gives while playing, or else the frame shown's, with
/// the clock the workspace's, so the host draws the time given and never
/// moves it; its sound's waveform as a row, "Sound", when it has one; and
/// each pin of its notes as a cue at its time, as vg-notes-at-time shows.
/// Frame strips have none.
pub fn transport(session: &Session, tab: &Tab) -> Option<Transport> {
    let shown = tab.round_view();
    let gallery = shown.as_ref().unwrap_or(&tab.gallery);
    if gallery.view() == Mode::Strips {
        return None;
    }
    let take = gallery.take()?;
    let info = take.info?;
    let fps = info.fps();
    let time = session.play_time().unwrap_or(f64::from(tab.frame_shown()) / fps.max(f64::EPSILON));
    // The take's own "Sound", then each heard sound layer's row.
    let mut rows: Vec<TransportRow> = session.wave(session.wave_width()).map(|image| TransportRow { label: "Sound".into(), image }).into_iter().collect();
    rows.extend(session.layer_rows_under(session.wave_width(), take.seconds().unwrap_or(0.0)));
    Some(Transport {
        length: take.seconds().unwrap_or(0.0),
        fps,
        time,
        playing: session.is_playing(),
        clock: TransportClock::Workspace,
        cues: session.note_cues(),
        rows,
        play: Action::new("transport_play"),
        step: Action::new("transport_step"),
    })
}

/// A take's menu, in vg-take-chosen's order: Next round from this, Ask the
/// assistant to use this take, Retime… and Upscale…, each a finishing
/// request, on only in a project, where the runner sends them; Edit;
/// Deliver…, on only in a project for a take whose round names its
/// artifact while no delivery is under way; for a finished take, Compare
/// with the source; and Open report.
fn take_menu(session: &Session, tab: &Tab, tile: &rounds::Tile, path: &str, outlined: bool) -> Vec<MenuEntry> {
    let target = json!({ "take": path });
    let mut retime = row(crate::finishing::RETIME, Action::with_arg("take_retime", target.clone()));
    retime.divider = true;
    retime.enabled = session.project.is_some();
    let mut upscale = row(crate::finishing::UPSCALE, Action::with_arg("take_upscale", target.clone()));
    upscale.enabled = session.project.is_some();
    let mut edit = row("Edit", Action::with_arg("take_edit", target.clone()));
    edit.divider = true;
    let mut deliver = row(crate::session::DELIVER, Action::with_arg("take_deliver", target.clone()));
    deliver.enabled = session.can_deliver(tab, path);
    // An opened stack's iteration starts a round or opens to edit.
    if tile.iteration.is_some() {
        return vec![row("Next round from this", Action::with_arg("take_next_round", target.clone())), edit];
    }
    let mut rows = vec![
        row("Next round from this", Action::with_arg("take_next_round", target.clone())),
        // Beside the takes in use, replacing none, so it asks nothing first.
        row("Use this take", Action::with_arg("take_use", target.clone())),
    ];
    // While out of date, as ig-out-of-date's menu shows.
    if outlined {
        let mut ignore = row("Ignore", Action::with_arg("take_ignore", target.clone()));
        ignore.divider = true;
        rows.push(ignore);
        rows.push(row("Ask the assistant to fix the dependency chain", Action::with_arg("take_fix", target.clone())));
    }
    rows.extend([retime, upscale, edit, deliver]);
    if tab.finishing_label(tile).is_some() {
        rows.push(row(crate::finishing::COMPARE, Action::with_arg("take_compare", target.clone())));
    }
    rows.push(row("Open report", Action::with_arg("take_report", target)));
    rows
}

/// The panel, as the Image Generator's: Layers, as vg-layers shows it, the
/// take shown as its linked layer and the tab's sound layers; Properties,
/// the chosen row's Gain, and a sound layer's Starts at; and Prompt, with
/// the tab's cell. Adjustments waits.
pub fn properties(session: &Session) -> Option<Properties> {
    let tab = session.active_tab()?;
    let shown = session.page;
    let page = |label: &str, index: usize, which: Option<Page>| PanelPage {
        label: label.into(),
        active: which == Some(shown),
        enabled: which.is_some(),
        action: Some(Action::with_arg("panel_page", json!({ "page": index }))),
    };
    let layers = (shown == Page::Layers).then(|| Layers {
        rows: session.layer_rows(),
        reorder: Action::new("sound_reorder"),
        sort_color: Action::new("sound_lock"),
        flags: FlagIcons { eye_open: "eye".into(), eye_closed: "eye-closed".into(), lock_closed: "lock".into(), lock_open: "lock-slash".into() },
        nest: false,
        plain: false,
        actions: vec![
            HeadingAction { icon: "plus".into(), tip: crate::session::OPEN_SOUND.into(), enabled: true, action: Action::new("open_sound") },
            HeadingAction {
                icon: "trash".into(),
                tip: "Delete layer".into(),
                enabled: tab.sound.is_some_and(|own| own != 0),
                action: Action::new("sound_delete"),
            },
        ],
    });
    let sections = if shown == Page::Properties { session.mix_section().into_iter().collect() } else { Vec::new() };
    Some(Properties {
        open: session.panel_open,
        toggle: Action::new("toggle_panel"),
        tip: "Prompt (F4)".into(),
        finish: Action::new("finish_edit"),
        back: None,
        layers,
        sections,
        pages: vec![
            page("Layers", 0, Some(Page::Layers)),
            page("Properties", 1, Some(Page::Properties)),
            page("Adjustments", 2, None),
            page("Prompt", 3, Some(Page::Prompt)),
        ],
        cell: (shown == Page::Prompt).then(|| tab.gallery.current_cell().map(|cell| prompt_cell(&cell.id, session.has_animatic()))).flatten(),
    })
}

/// The Prompt page's cell, as vg-request-inputs shows it: the host draws
/// the card and runs it; this names the cell, the roles a reference can
/// have, Start frame and End frame each a row of its own, the references
/// it takes, a reference's menu, and the settings Size, Length in seconds,
/// and Seed.
pub fn prompt_cell(cell: &str, animatic: bool) -> PanelCell {
    PanelCell {
        cell: cell.to_string(),
        hint: HINT.into(),
        roles: rounds::ROLES
            .iter()
            .map(|label| ReferenceRole { label: label.to_string(), field: None, slot: rounds::SLOTS.contains(label) })
            .collect(),
        reference_types: rounds::REFERENCE_TYPES.iter().map(|kind| kind.to_string()).collect(),
        reference_menu: reference_menu(animatic),
        settings: vec![
            CellSetting { key: "size".into(), label: "Size".into(), kind: RowKind::Text },
            CellSetting { key: "length".into(), label: "Length".into(), kind: RowKind::Field { min: 1.0, max: 20.0, places: 0 } },
            CellSetting { key: "seed".into(), label: "Seed".into(), kind: RowKind::Field { min: 0.0, max: 2_147_483_647.0, places: 0 } },
        ],
        menu: Vec::new(),
    }
}

/// The rows a reference's menu adds between the host's Choose… and Clear,
/// as vg-frame-from shows them: the animatic's frame at the playhead, put
/// in the Start frame, off while the draft has no Motion reference, with a
/// tip saying why; and Open in the Image Generator, for an image.
fn reference_menu(animatic: bool) -> Vec<MenuEntry> {
    let mut take = row("The animatic's frame at the playhead", Action::new("frame_from_animatic"));
    take.enabled = animatic;
    if !animatic {
        take.tip = Some(crate::session::NO_ANIMATIC.into());
    }
    vec![take, row("Open in the Image Generator", Action::new("frame_open_image"))]
}

pub fn tabs(session: &Session) -> Option<Tabs> {
    let active = session.active_tab()?;
    Some(Tabs {
        tabs: session
            .tabs
            .iter()
            .map(|tab| TabEntry {
                id: tab.id.clone(),
                label: tab.label.clone(),
                dirty: tab.dirty,
                close_waits: false,
                path: tab.path.as_ref().map(|path| path.display().to_string()),
                saves: true,
            })
            .collect(),
        active: active.id.clone(),
        new_tab: Some(Action::new("new_video")),
        new_tab_tip: "New video".into(),
    })
}
