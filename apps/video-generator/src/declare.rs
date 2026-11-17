//! What the Video Generator declares for the host to draw: the toolbar, the
//! filmstrip, the menu, the keys, and the tab list, in the Image
//! Generator's words wherever a video needs nothing more.

use crate::gallery::Mode;
use crate::session::{Session, Tab};
use protocol::{Action, Binding, Chord, Film, FilmItem, Keys, MenuEntry, TabEntry, Tabs, Thumbnail, ToolItem, Toolbar};
use serde_json::json;

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

/// The toolbar: open, undo and redo, and the frame strips' grid button,
/// then the zoom menu and full screen.
pub fn toolbar(session: &Session) -> Toolbar {
    let tab = session.active_tab();
    let can_undo = tab.is_some_and(|tab| tab.gallery.can_undo());
    let can_redo = tab.is_some_and(|tab| tab.gallery.can_redo());
    let strips = tab.is_some_and(|tab| tab.gallery.view() == Mode::Strips);
    let items = vec![
        button("folder", "Open (Ctrl+O)", Action::new("open"), true, false),
        ToolItem::Separator,
        button("undo", "Undo (Ctrl+Z)", Action::new("undo"), can_undo, false),
        button("redo", "Redo (Ctrl+Shift+Z)", Action::new("redo"), can_redo, false),
        ToolItem::Separator,
        // Its tip is the guide's word for what it shows whole.
        button("view-grid", "Round", Action::new("view_strips"), tab.is_some_and(|tab| !tab.gallery.takes.is_empty()), strips),
    ];
    let zoom = tab.and_then(|tab| tab.zoom);
    let label = match zoom {
        Some(zoom) => format!("{}%", (zoom * 100.0).round()),
        None => String::from("Fit"),
    };
    let trailing = vec![
        ToolItem::Separator,
        ToolItem::Menu {
            icon: String::new(),
            label,
            enabled: true,
            rows: ZOOMS.iter().map(|(label, zoom)| row(label, Action::with_arg("view_zoom", json!({ "zoom": zoom })))).collect(),
        },
        ToolItem::Maximize { icon: "maximize".into(), restore_icon: "reduce".into(), tip: "Full screen".into() },
    ];
    Toolbar { items, trailing, options: Vec::new(), options_trailing: Vec::new() }
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
        bind(chord("Left", false, Some(false)), frame(json!({ "step": -1 }))),
        bind(chord("Right", false, Some(false)), frame(json!({ "step": 1 }))),
        bind(chord("Left", false, Some(true)), frame(json!({ "seconds": -1 }))),
        bind(chord("Right", false, Some(true)), frame(json!({ "seconds": 1 }))),
        bind(chord("Home", false, Some(false)), frame(json!({ "to": "first" }))),
        bind(chord("End", false, Some(false)), frame(json!({ "to": "last" }))),
    ];
    Keys { bindings, copy: None, cut: None, paste: None }
}

/// The filmstrip, as the Image Generator's: every take's thumbnail, its
/// first frame, with its menu, the shown one, and the + tile's row. A click
/// with `extend` set shows that take beside the one shown.
pub fn film(session: &Session, tab: &Tab) -> Film {
    let items = tab
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
                chosen: false,
                pending: None,
            }
        })
        .collect();
    Film {
        items,
        shown: tab.gallery.shown,
        select: Action::new("film_select"),
        reorder: Action::new("film_reorder"),
        add: vec![row("Open video…", Action::new("film_open"))],
        focus: Action::new("film_focus"),
        groups: Vec::new(),
    }
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
