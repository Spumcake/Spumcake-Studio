//! What the Image Editor declares for the host to draw: the toolbar and its
//! settings row, as the approved views show them, the panel's Layers,
//! Properties, and Adjustments pages, the filmstrip, the menu, the keys,
//! and the tab list.

use crate::paint::Combine;
use crate::picture::{Blend, Kind, Node};
use crate::session::{Image, Page, Session, ToolKind};
use crate::text::{Align, Face};
use protocol::{
    Action, Binding, Chord, ChoicePreview, Film, FilmItem, FlagIcons, HeadingAction, IconToggle, Keys, LayerRow, Layers,
    MenuEntry, PanelPage, PropValue, Properties, Row, RowKind, Section, TabEntry, Tabs, Thumbnail, ToolItem, Toolbar,
};
use serde_json::json;

/// The crop ratios: their words and width over height, none for Free.
pub const RATIOS: [(&str, Option<f64>); 5] = [
    ("Free", None),
    ("1:1", Some(1.0)),
    ("4:3", Some(4.0 / 3.0)),
    ("3:2", Some(1.5)),
    ("16:9", Some(16.0 / 9.0)),
];

/// The zoom menu's rows: a word and the zoom it sets, none for Fit.
const ZOOMS: [(&str, Option<f64>); 4] = [("Fit", None), ("50%", Some(0.5)), ("100%", Some(1.0)), ("200%", Some(2.0))];

/// The four locks, in the order the Lock row shows them.
const LOCK_ICONS: [(&str, &str); 4] = [
    ("view-grid", "Lock transparency"),
    ("edit-pencil", "Lock pixels"),
    ("drag", "Lock position"),
    ("lock", "Lock all"),
];

const SLIDER_W: f32 = 110.0;

/// Each tool's icon, name, and key.
fn tool_look(tool: ToolKind) -> (&'static str, &'static str, &'static str) {
    match tool {
        ToolKind::Move => ("cursor-pointer", "Move", "V"),
        ToolKind::Marquee => ("square-dashed", "Rectangle select", "M"),
        ToolKind::Lasso => ("chat-bubble-empty", "Lasso select", "L"),
        ToolKind::Crop => ("crop", "Crop", "C"),
        ToolKind::Eyedropper => ("color-picker", "Eyedropper", "I"),
        // Drawn by the host from the mockup's shapes; Iconoir has neither.
        ToolKind::Heal => ("bandage", "Healing brush", "J"),
        ToolKind::Stamp => ("stamp", "Clone stamp", "S"),
        // Drawn by the host in Iconoir's style; Iconoir has no brush.
        ToolKind::Brush => ("brush", "Brush", "B"),
        ToolKind::Eraser => ("erase", "Eraser", "E"),
        ToolKind::Text => ("text", "Text", "T"),
        ToolKind::Hand => ("drag-hand-gesture", "Hand", "H"),
        ToolKind::Zoom => ("search", "Zoom", "Z"),
    }
}

fn button(icon: &str, tip: &str, action: Action, enabled: bool, selected: bool) -> ToolItem {
    ToolItem::Button {
        icon: icon.into(),
        tip: tip.into(),
        enabled,
        selected,
        action: Some(action),
        menu: Vec::new(),
    }
}

fn row(label: &str, action: Action) -> MenuEntry {
    MenuEntry {
        label: label.into(),
        icon: None,
        enabled: true,
        divider: false,
        action: Some(action),
        submenu: Vec::new(),
        rename: false,
    }
}

/// The toolbar: open and export, undo and redo, the tools, then the zoom
/// menu, zoom out, zoom in, fit, and full screen, with the chosen tool's
/// settings in the row under it.
pub fn toolbar(session: &Session) -> Toolbar {
    let image = session.active_image();
    // While the strip has the keyboard, these are its history's.
    let can_undo = image.is_some_and(Image::can_undo);
    let can_redo = image.is_some_and(Image::can_redo);
    let mut export = button("floppy-disk", "Export as PNG", Action::new("export_png"), image.is_some(), false);
    if let ToolItem::Button { menu, .. } = &mut export {
        *menu = vec![row("Export PNG", Action::new("export_png")), row("Export JPEG", Action::new("export_jpeg"))];
    }
    let mut items = vec![
        button("folder", "Open (Ctrl+O)", Action::new("open"), true, false),
        export,
        ToolItem::Separator,
        button("undo", "Undo (Ctrl+Z)", Action::new("undo"), can_undo, false),
        button("redo", "Redo (Ctrl+Shift+Z)", Action::new("redo"), can_redo, false),
        ToolItem::Separator,
    ];
    for tool in ToolKind::ALL {
        let (icon, name, key) = tool_look(tool);
        items.push(button(
            icon,
            &format!("{name} ({key})"),
            Action::with_arg("tool", json!({ "tool": tool.key() })),
            true,
            session.tools.tool == tool,
        ));
    }
    let zoom = image.and_then(|image| image.frame().camera.as_ref()).map_or(1.0, |camera| camera.zoom);
    let trailing = vec![
        ToolItem::Separator,
        ToolItem::Menu {
            icon: String::new(),
            label: format!("{}%", (zoom * 100.0).round()),
            enabled: true,
            rows: ZOOMS
                .iter()
                .map(|(label, zoom)| match zoom {
                    Some(zoom) => row(label, Action::with_arg("canvas.zoom", json!({ "zoom": zoom }))),
                    None => row(label, Action::new("canvas.fit")),
                })
                .collect(),
        },
        ToolItem::Separator,
        button("zoom-out", "Zoom out (Ctrl+-)", Action::new("canvas.zoom_out"), true, false),
        button("zoom-in", "Zoom in (Ctrl++)", Action::new("canvas.zoom_in"), true, false),
        button("frame", "Fit (Ctrl+0)", Action::new("canvas.fit"), true, false),
        ToolItem::Maximize { icon: "maximize".into(), restore_icon: "reduce".into(), tip: "Full screen".into() },
    ];
    let (options, options_trailing) = settings_row(session);
    Toolbar { items, trailing, options, options_trailing }
}

/// The chosen tool's settings, as brush.png, selection.png, crop.png,
/// clone.png, heal.png, and text.png show them.
fn settings_row(session: &Session) -> (Vec<ToolItem>, Vec<ToolItem>) {
    let tools = &session.tools;
    let slider = |caption: &str, value: u32, min: i32, max: i32, suffix: &str, tool: &str, setting: &str| ToolItem::Slider {
        caption: caption.into(),
        // The host lays the caption, the track, and the value out within the
        // width, as Simple Sketch's Size control is: the track keeps its own
        // width beside them.
        width: SLIDER_W + caption.chars().count() as f32 * 7.5 + 70.0,
        value: value as i32,
        min,
        max,
        tint: None,
        set: Action::with_arg("set_brush", json!({ "tool": tool, "setting": setting })),
        finish: Some(Action::new("finish_brush")),
        suffix: suffix.into(),
    };
    let caption = |text: &str| vec![ToolItem::Label { text: text.into(), strong: false }];
    // Why the chosen tool cannot change the active layer, after its
    // settings; the gesture is still refused.
    let reason = |erase: bool, moving: bool| {
        let doc = session.active_image()?.doc();
        if moving { doc.move_refusal() } else { doc.paint_refusal(erase) }
            .map(|text| ToolItem::Label { text, strong: true })
    };
    let refused = |text: Option<String>| text.map(|text| ToolItem::Label { text, strong: true });
    match tools.tool {
        ToolKind::Brush | ToolKind::Eraser => {
            let (settings, key) = if tools.tool == ToolKind::Brush { (tools.brush, "brush") } else { (tools.eraser, "eraser") };
            let mut row = Vec::new();
            if tools.tool == ToolKind::Brush {
                row.push(ToolItem::Color { rgb: tools.rgb, tip: "Brush colour".into(), set: Action::new("set_color") });
                row.push(ToolItem::Separator);
            }
            row.push(slider("Size", settings.size, 1, 500, "px", key, "size"));
            row.push(slider("Hardness", settings.hardness, 0, 100, "%", key, "hardness"));
            row.push(slider("Opacity", settings.opacity, 1, 100, "%", key, "opacity"));
            row.push(ToolItem::Separator);
            row.push(ToolItem::Switch {
                caption: "Pen pressure".into(),
                on: settings.pressure,
                action: Action::with_arg("toggle_pressure", json!({ "tool": key })),
            });
            if let Some(reason) = reason(tools.tool == ToolKind::Eraser, false) {
                row.push(ToolItem::Separator);
                row.push(reason);
            }
            (row, Vec::new())
        }
        ToolKind::Marquee | ToolKind::Lasso => (
            vec![
                ToolItem::Segmented {
                    options: Combine::ALL.iter().map(|how| how.label().to_string()).collect(),
                    selected: Combine::ALL.iter().position(|how| *how == tools.combine).unwrap_or(0),
                    set: Action::new("select_mode"),
                },
                ToolItem::Separator,
                ToolItem::Number {
                    caption: "Feather".into(),
                    value: tools.feather,
                    min: 0.0,
                    max: 1000.0,
                    suffix: "px".into(),
                    set: Action::new("feather"),
                    icon: None,
                },
            ],
            vec![
                ToolItem::Push { label: "Deselect".into(), dark: false, enabled: true, action: Action::new("deselect") },
                ToolItem::Push { label: "Invert".into(), dark: false, enabled: true, action: Action::new("invert") },
            ],
        ),
        ToolKind::Crop => {
            let image = session.active_image();
            let size = image
                .map(|image| match image.frame().crop {
                    Some(rect) => (rect[2].round() as u32, rect[3].round() as u32),
                    None => (image.doc().picture.width, image.doc().picture.height),
                })
                .unwrap_or((0, 0));
            (
                vec![
                    ToolItem::Choice {
                        caption: "Ratio".into(),
                        options: RATIOS.iter().map(|(label, _)| label.to_string()).collect(),
                        selected: tools.ratio,
                        set: Action::new("crop_ratio"),
                    },
                    ToolItem::Label { text: format!("{} × {}", size.0, size.1), strong: true },
                ],
                vec![
                    ToolItem::Push { label: "Cancel".into(), dark: false, enabled: true, action: Action::new("crop_cancel") },
                    ToolItem::Push { label: "Apply".into(), dark: true, enabled: image.is_some(), action: Action::new("crop_apply") },
                ],
            )
        }
        ToolKind::Move => {
            let doc = session.active_image().map(Image::doc);
            if doc.is_some_and(|doc| doc.selection.is_some()) {
                let mut row = caption("Drag to move the selected pixels.");
                row.extend(refused(doc.and_then(|doc| doc.move_pixels_refusal())));
                return (row, Vec::new());
            }
            let mut row = caption("Drag to move the active layer.");
            row.extend(reason(false, true));
            (row, Vec::new())
        }
        ToolKind::Stamp | ToolKind::Heal => {
            let heal = tools.tool == ToolKind::Heal;
            let (settings, key) = if heal { (tools.heal, "heal") } else { (tools.stamp, "stamp") };
            let mut row = vec![
                slider("Size", settings.size, 1, 500, "px", key, "size"),
                slider("Hardness", settings.hardness, 0, 100, "%", key, "hardness"),
            ];
            if !heal {
                row.push(slider("Opacity", settings.opacity, 1, 100, "%", key, "opacity"));
                row.push(ToolItem::Separator);
                row.push(ToolItem::Switch { caption: "Aligned".into(), on: tools.aligned, action: Action::new("toggle_aligned") });
            } else {
                row.push(ToolItem::Separator);
            }
            row.push(ToolItem::Label { text: "Alt-click to set the source".into(), strong: false });
            if let Some(reason) = refused(session.active_image().and_then(|image| image.doc().sample_refusal())) {
                row.push(ToolItem::Separator);
                row.push(reason);
            }
            (row, Vec::new())
        }
        ToolKind::Text => {
            let style = tools.text;
            let style_action = |key: &str| Action::with_arg("text_style", json!({ "key": key }));
            let toggle = |icon: &str, tip: &str, key: &str, on: bool| button(icon, tip, style_action(key), true, on);
            let mut row = vec![
                ToolItem::Color { rgb: style.color, tip: "Text colour".into(), set: style_action("color") },
                ToolItem::Choice {
                    caption: String::new(),
                    options: vec!["Inter".into(), "Monospace".into()],
                    selected: usize::from(style.face == Face::Mono),
                    set: style_action("face"),
                },
                ToolItem::Number {
                    caption: String::new(),
                    value: f64::from(style.size).round(),
                    min: 1.0,
                    max: 2000.0,
                    suffix: "px".into(),
                    set: style_action("size"),
                    icon: None,
                },
                ToolItem::Separator,
                toggle("bold", "Bold", "bold", style.bold),
                toggle("italic", "Italic", "italic", style.italic),
                toggle("underline", "Underline", "underline", style.underline),
                ToolItem::Separator,
            ];
            for (index, (align, icon, tip)) in [
                (Align::Left, "align-left", "Align left"),
                (Align::Center, "align-center", "Centre"),
                (Align::Right, "align-right", "Align right"),
                (Align::Justify, "align-justify", "Justify"),
            ]
            .into_iter()
            .enumerate()
            {
                row.push(button(icon, tip, Action::with_arg("text_style", json!({ "key": "align", "align": index })), true, style.align == align));
            }
            row.push(ToolItem::Separator);
            row.push(ToolItem::Number {
                caption: String::new(),
                value: (f64::from(style.line_height) * 100.0).round() / 100.0,
                min: 0.5,
                max: 5.0,
                suffix: String::new(),
                set: style_action("line_height"),
                icon: Some("line-space".into()),
            });
            row.push(ToolItem::Number {
                caption: "Spacing".into(),
                value: f64::from(style.letter_spacing),
                min: -200.0,
                max: 1000.0,
                suffix: String::new(),
                set: style_action("letter_spacing"),
                // An icon in the caption's place, as line height has, so
                // the row fits a narrower window.
                icon: Some("align-horizontal-spacing".into()),
            });
            // All caps as an icon toggle, as bold, italic, and underline are.
            row.push(toggle("text-size", "All caps", "caps", style.all_caps));
            (row, Vec::new())
        }
        ToolKind::Eyedropper => (caption("Click the picture to take its colour for the brush."), Vec::new()),
        ToolKind::Hand => (caption("Drag to pan."), Vec::new()),
        ToolKind::Zoom => (caption("Click to zoom in, Alt-click to zoom out."), Vec::new()),
    }
}

/// The menu. In a project every change is written by itself, so Save and
/// Save As are not offered.
pub fn menu(session: &Session) -> Vec<MenuEntry> {
    let mut rows = vec![
        row("New image", Action::new("new_image")),
        row("Open…", Action::new("open")),
        row("Open as layer…", Action::new("open_layer")),
    ];
    if session.project.is_none() {
        rows.push(row("Save", Action::new("save")));
        rows.push(row("Save As…", Action::new("save_as")));
    }
    let mut export = row("Export PNG…", Action::new("export_png"));
    export.divider = true;
    rows.push(export);
    rows.push(row("Export JPEG…", Action::new("export_jpeg")));
    let mut undo = row("Undo", Action::new("undo"));
    undo.divider = true;
    undo.enabled = session.active_image().is_some_and(Image::can_undo);
    rows.push(undo);
    let mut redo = row("Redo", Action::new("redo"));
    redo.enabled = session.active_image().is_some_and(Image::can_redo);
    rows.push(redo);
    let mut deselect = row("Deselect", Action::new("deselect"));
    deselect.divider = true;
    rows.push(deselect);
    rows.push(row("Invert selection", Action::new("invert")));
    rows
}

/// The keys. The tools take Photoshop's letters; F4 stays the host's.
pub fn keys() -> Keys {
    let chord = |key: &str, command: bool, shift: Option<bool>| Chord { key: key.into(), command, shift, alt: false };
    let bind = |chord: Chord, action: Action| Binding { chord, then: Vec::new(), action };
    let mut bindings = vec![
        bind(chord("Z", true, Some(false)), Action::new("undo")),
        bind(chord("Z", true, Some(true)), Action::new("redo")),
        bind(chord("Y", true, Some(false)), Action::new("redo")),
        bind(chord("N", true, Some(false)), Action::new("new_image")),
        bind(chord("O", true, Some(false)), Action::new("open")),
        bind(chord("D", true, Some(false)), Action::new("deselect")),
        bind(chord("I", true, Some(true)), Action::new("invert")),
        bind(chord("Enter", false, Some(false)), Action::new("crop_apply")),
        bind(chord("Escape", false, Some(false)), Action::new("crop_cancel")),
        bind(chord("BracketLeft", false, Some(false)), Action::new("size_down")),
        bind(chord("BracketRight", false, Some(false)), Action::new("size_up")),
        bind(chord("Plus", true, None), Action::new("canvas.zoom_in")),
        bind(chord("Equals", true, None), Action::new("canvas.zoom_in")),
        bind(chord("Minus", true, None), Action::new("canvas.zoom_out")),
        bind(chord("Digit0", true, Some(false)), Action::new("canvas.fit")),
    ];
    for tool in ToolKind::ALL {
        let (_, _, key) = tool_look(tool);
        bindings.push(bind(chord(key, false, Some(false)), Action::with_arg("tool", json!({ "tool": tool.key() }))));
    }
    Keys { bindings, copy: None, cut: None, paste: None }
}

/// The panel: the Layers page, with the active layer's settings above the
/// list of layers; the Properties page, with the chosen layer's own
/// settings; and the Adjustments page, with the adjustments to add.
pub fn properties(session: &Session) -> Option<Properties> {
    let image = session.active_image()?;
    let doc = image.doc();
    let picture = &doc.picture;
    let active = doc.active.and_then(|id| picture.find(id));
    let mut sections = Vec::new();
    let mut layers = None;
    match session.page {
        Page::Layers => {
            if let Some(node) = active {
                sections.push(Section { title: String::new(), rows: layer_settings(node), icon: None });
            }
            let mut rows = Vec::new();
            for node in picture.nodes.iter().rev() {
                layer_rows(session, node, None, 0, &mut rows);
            }
            layers = Some(Layers {
                rows,
                reorder: Action::new("reorder"),
                sort_color: Action::new("sort_color"),
                nest: true,
                flags: FlagIcons {
                    eye_open: "eye".into(),
                    eye_closed: "eye-closed".into(),
                    lock_closed: "lock".into(),
                    lock_open: "lock-slash".into(),
                },
                actions: vec![
                    HeadingAction { icon: "plus".into(), tip: "New layer".into(), enabled: true, action: Action::new("add_layer") },
                    HeadingAction { icon: "folder".into(), tip: "New group".into(), enabled: true, action: Action::new("add_group") },
                    HeadingAction {
                        icon: "trash".into(),
                        tip: "Delete layer".into(),
                        enabled: doc.active.is_some() && picture.nodes.iter().map(|node| picture.ids_within(node.id).len()).sum::<usize>() > 1,
                        action: Action::new("delete_layer"),
                    },
                ],
            });
        }
        Page::Properties => sections.push(chosen_settings(session)),
        Page::Adjustments => sections.push(Section {
            title: String::new(),
            icon: None,
            rows: vec![
                Row {
                    prop: json!("add_curves"),
                    label: "Curves".into(),
                    kind: RowKind::Item {
                        icon: "ease-curve-control-points".into(),
                        detail: "Tone and colour by curve".into(),
                        action: Action::with_arg("add_adjustment", json!({ "kind": "curves" })),
                    },
                    value: PropValue::Idle,
                },
                Row {
                    prop: json!("add_grade"),
                    label: "Color Grade".into(),
                    kind: RowKind::Item {
                        icon: "color-filter".into(),
                        detail: "Exposure, contrast, and colour".into(),
                        action: Action::with_arg("add_adjustment", json!({ "kind": "grade" })),
                    },
                    value: PropValue::Idle,
                },
            ],
        }),
    }
    let page = |label: &str, own: Page, index: usize| PanelPage {
        label: label.into(),
        active: session.page == own,
        enabled: true,
        action: Some(Action::with_arg("panel_page", json!({ "page": index }))),
    };
    Some(Properties {
        open: session.panel_open,
        toggle: Action::new("toggle_panel"),
        tip: "Layers (F4)".into(),
        finish: Action::new("finish_edit"),
        back: None,
        layers,
        sections,
        pages: vec![page("Layers", Page::Layers, 0), page("Properties", Page::Properties, 1), page("Adjustments", Page::Adjustments, 2)],
    })
}

/// The Layers page's rows for the active layer: its blend mode, opacity,
/// fill, and locks.
fn layer_settings(node: &Node) -> Vec<Row> {
    let mut rows = vec![
        Row {
            prop: json!("blend"),
            label: "Blend Mode".into(),
            kind: RowKind::Choice {
                options: Blend::ALL.iter().map(|blend| blend.label().to_string()).collect(),
                segmented: false,
                preview: ChoicePreview::Label,
            },
            value: PropValue::Index(node.blend.index() as u8),
        },
        Row {
            prop: json!("opacity"),
            label: "Opacity".into(),
            kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
            value: PropValue::Number(f64::from(node.opacity)),
        },
    ];
    // A group has no pixels of its own for Fill to scale.
    if !node.is_group() {
        rows.push(Row {
            prop: json!("fill"),
            label: "Fill".into(),
            kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
            value: PropValue::Number(f64::from(node.fill)),
        });
    }
    rows.push(Row {
        prop: json!("locks"),
        label: "Lock:".into(),
        kind: RowKind::Icons {
            icons: LOCK_ICONS
                .iter()
                .enumerate()
                .map(|(index, (icon, tip))| IconToggle { icon: icon.to_string(), tip: tip.to_string(), on: node.locks.get(index) })
                .collect(),
        },
        value: PropValue::Idle,
    });
    rows
}

fn note(prop: &str, words: &str) -> Row {
    Row { prop: json!(prop), label: words.into(), kind: RowKind::Note, value: PropValue::Idle }
}

fn push_button(prop: &str, label: &str, action: Action) -> Row {
    Row { prop: json!(prop), label: label.into(), kind: RowKind::Button { action }, value: PropValue::Idle }
}

/// The Properties page: the chosen layer's own settings, as
/// properties-curves.png, properties-grade.png, and properties-mask.png
/// show them, and a note for the kinds with none.
fn chosen_settings(session: &Session) -> Section {
    let Some(image) = session.active_image() else {
        return Section { title: String::new(), rows: Vec::new(), icon: None };
    };
    let doc = image.doc();
    let Some(node) = doc.active.and_then(|id| doc.picture.find(id)) else {
        return Section { title: String::new(), rows: vec![note("none", "Choose a layer to see its properties.")], icon: None };
    };
    if doc.target_mask().is_some()
        && let Some(mask) = &node.mask
    {
        return Section {
            title: mask.name.clone(),
            icon: Some("square".into()),
            rows: vec![
                Row {
                    prop: json!("feather"),
                    label: "Feather".into(),
                    kind: RowKind::Number { min: 0.0, max: 250.0, percent: false, places: 0, centered: false },
                    value: PropValue::Number(f64::from(mask.feather)),
                },
                push_button("invert", "Invert", Action::with_arg("mask_invert", json!({ "node": node.id }))),
                note("about", "White shows the layer beneath, black hides it. Paint it with the brush and the eraser."),
            ],
        };
    }
    let centered = |prop: &str, label: &str, min: f64, max: f64, places: u8, value: f32| Row {
        prop: json!(prop),
        label: label.into(),
        kind: RowKind::Number { min, max, percent: false, places, centered: true },
        value: PropValue::Number(f64::from(value)),
    };
    match &node.kind {
        _ if node.is_group() => Section {
            title: node.name.clone(),
            icon: Some("folder".into()),
            rows: vec![note("about", "A group's settings are on the Layers page.")],
        },
        Kind::Curves(curves) => {
            let channel = session.curve_channel.min(3);
            let points = curves.channel(channel);
            let mut rows = vec![
                Row {
                    prop: json!("curve_channel"),
                    label: String::new(),
                    kind: RowKind::Choice {
                        options: vec!["RGB".into(), "Red".into(), "Green".into(), "Blue".into()],
                        segmented: true,
                        preview: ChoicePreview::Label,
                    },
                    value: PropValue::Index(channel as u8),
                },
                Row {
                    prop: json!("curve"),
                    label: String::new(),
                    kind: RowKind::Curve { histogram: doc.histogram(node.id, channel), square: true, free: true },
                    value: PropValue::Curve(points.iter().map(|point| (point[0], point[1])).collect()),
                },
            ];
            // The point last moved, or the last one inside the ends.
            let shown = session.curve_point.or_else(|| (points.len() > 2).then(|| points[points.len() - 2]));
            if let Some(point) = shown {
                let byte = |value: f64| (value * 255.0).round() as i32;
                rows.push(note("point", &format!("Input {}   Output {}", byte(point[0]), byte(point[1]))));
            }
            rows.push(push_button("reset", "Reset", Action::new("reset_adjustment")));
            Section { title: node.name.clone(), icon: Some("ease-curve-control-points".into()), rows }
        }
        Kind::Grade(grade) => Section {
            title: node.name.clone(),
            icon: Some("color-filter".into()),
            rows: vec![
                centered("grade_exposure", "Exposure", -3.0, 3.0, 1, grade.exposure),
                centered("grade_contrast", "Contrast", -100.0, 100.0, 0, grade.contrast),
                centered("grade_saturation", "Saturation", -100.0, 100.0, 0, grade.saturation),
                centered("grade_temperature", "Temperature", -100.0, 100.0, 0, grade.temperature),
                push_button("reset", "Reset", Action::new("reset_adjustment")),
            ],
        },
        // A text's own settings, as its settings row holds them, as the
        // owner set on 2026-10-31: Properties edits the chosen layer.
        Kind::Text(text) => {
            let style = text.style;
            let number = |prop: &str, label: &str, min: f64, max: f64, places: u8, value: f32| Row {
                prop: json!(prop),
                label: label.into(),
                kind: RowKind::Number { min, max, percent: false, places, centered: false },
                value: PropValue::Number(f64::from(value)),
            };
            let choice = |prop: &str, label: &str, options: &[&str], segmented: bool, index: usize| Row {
                prop: json!(prop),
                label: label.into(),
                kind: RowKind::Choice { options: options.iter().map(|option| option.to_string()).collect(), segmented, preview: ChoicePreview::Label },
                value: PropValue::Index(index as u8),
            };
            let toggles = [("bold", "Bold", style.bold), ("italic", "Italic", style.italic), ("underline", "Underline", style.underline), ("text-size", "All caps", style.all_caps)];
            Section {
                title: node.name.clone(),
                icon: Some("text".into()),
                rows: vec![
                    choice("text_face", "Face", &["Inter", "Monospace"], false, usize::from(style.face == crate::text::Face::Mono)),
                    number("text_size", "Size", 1.0, 2000.0, 0, style.size),
                    Row { prop: json!("text_color"), label: "Colour".into(), kind: RowKind::Rgb { quick: Vec::new() }, value: PropValue::Rgb(style.color) },
                    Row {
                        prop: json!("text_toggles"),
                        label: "Style".into(),
                        kind: RowKind::Icons { icons: toggles.iter().map(|(icon, tip, on)| IconToggle { icon: icon.to_string(), tip: tip.to_string(), on: *on }).collect() },
                        value: PropValue::Idle,
                    },
                    choice("text_align", "Align", &["Left", "Center", "Right", "Justify"], true, crate::text::Align::ALL.iter().position(|align| *align == style.align).unwrap_or(0)),
                    number("text_line_height", "Line height", 0.5, 5.0, 1, style.line_height),
                    number("text_spacing", "Spacing", -200.0, 1000.0, 0, style.letter_spacing),
                    note("about", "Click its words on the picture with the T tool to type."),
                ],
            }
        }
        Kind::Pixels => Section {
            title: node.name.clone(),
            icon: Some("media-image".into()),
            rows: vec![note("about", "A layer's settings are on the Layers page. Add a mask from its row's menu there.")],
        },
    }
}

/// A node's row, then its children's, top first, as the list shows them. A
/// node's mask has its own row directly above it.
fn layer_rows(session: &Session, node: &Node, parent: Option<u64>, depth: u32, rows: &mut Vec<LayerRow>) {
    let Some(image) = session.active_image() else {
        return;
    };
    let doc = image.doc();
    let id = node.id;
    let target = json!({ "node": id });
    let rename = MenuEntry {
        label: "Rename".into(),
        icon: None,
        enabled: true,
        divider: false,
        action: None,
        submenu: Vec::new(),
        rename: true,
    };
    if let Some(mask) = &node.mask {
        let own = json!({ "node": id, "mask": true });
        let mut delete = row("Delete mask", Action::with_arg("delete_mask", target.clone()));
        delete.divider = true;
        rows.push(LayerRow {
            id: mask.id,
            parent,
            depth,
            icon: "media-image".into(),
            name: mask.name.clone(),
            group: false,
            selected: doc.target_mask() == Some(mask.id),
            visible: mask.visible,
            locked: node.locks.all,
            renamable: true,
            click: Action::with_arg("layer_click", own.clone()),
            rename: Action::with_arg("layer_rename", own.clone()),
            toggle_visible: Action::with_arg("layer_visible", own),
            toggle_locked: Action::with_arg("layer_lock", target.clone()),
            menu: vec![rename.clone(), row("Invert", Action::with_arg("mask_invert", target.clone())), delete],
            colors: Vec::new(),
            thumbnail: image.frame().thumbs.get(&mask.id).map(|(_, thumb)| thumb.clone()),
            mark: Some("link".into()),
        });
    }
    let mut menu = vec![rename];
    if node.is_group() {
        menu.push(row("Ungroup", Action::with_arg("ungroup", target.clone())));
    }
    if node.mask.is_none() {
        menu.push(row("Add mask", Action::with_arg("add_mask", target.clone())));
    }
    menu.push(row("Delete", Action::with_arg("delete_layer", target.clone())));
    let (icon, mark) = match &node.kind {
        _ if node.is_group() => ("folder", None),
        Kind::Text(_) => ("text", None),
        Kind::Curves(_) | Kind::Grade(_) => ("filter-list", Some("filter-list".to_string())),
        Kind::Pixels => ("media-image", None),
    };
    rows.push(LayerRow {
        id,
        parent,
        depth,
        icon: icon.into(),
        name: node.name.clone(),
        group: node.is_group(),
        selected: doc.active == Some(id) && doc.target_mask().is_none(),
        visible: node.visible,
        locked: node.locks.all,
        renamable: true,
        click: Action::with_arg("layer_click", target.clone()),
        rename: Action::with_arg("layer_rename", target.clone()),
        toggle_visible: Action::with_arg("layer_visible", target.clone()),
        toggle_locked: Action::with_arg("layer_lock", target),
        menu,
        colors: Vec::new(),
        // A text shows its icon in the picture's frame.
        thumbnail: if node.text().is_some() { None } else { image.frame().thumbs.get(&id).map(|(_, thumb)| thumb.clone()) },
        mark,
    });
    if let Some(children) = &node.children {
        for child in children.iter().rev() {
            layer_rows(session, child, Some(id), depth + 1, rows);
        }
    }
}

/// The filmstrip, as gallery.png, picture-menu.png, and add-menu.png show
/// it: every picture's thumbnail with its menu, the shown one, and the +
/// tile's rows.
pub fn film(image: &Image) -> Film {
    let count = image.gallery.frames.len();
    let items = image
        .gallery
        .frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let target = json!({ "picture": index });
            // The host renames in place, as a layer's Rename row does, and
            // sends the name typed as the row's action with `value`.
            let mut rename = row("Rename", Action::with_arg("film_rename", target.clone()));
            rename.rename = true;
            let mut delete = row("Delete", Action::with_arg("film_delete", target.clone()));
            delete.divider = true;
            // The gallery keeps at least one picture.
            delete.enabled = count > 1;
            FilmItem {
                name: frame.name.clone(),
                thumbnail: frame.film.as_ref().map(|(_, _, thumb)| thumb.clone()).unwrap_or(Thumbnail {
                    path: String::new(),
                    width: 1,
                    height: 1,
                    revision: 0,
                }),
                menu: vec![row("Duplicate", Action::with_arg("film_duplicate", target)), rename, delete],
            }
        })
        .collect();
    Film {
        items,
        shown: image.gallery.shown,
        select: Action::new("film_select"),
        reorder: Action::new("film_reorder"),
        add: vec![row("New picture", Action::new("film_new")), row("Open PNG or JPEG…", Action::new("film_open"))],
        focus: Action::new("film_focus"),
    }
}

pub fn tabs(session: &Session) -> Option<Tabs> {
    let active = session.active_image()?;
    Some(Tabs {
        tabs: session
            .images
            .iter()
            .map(|image| TabEntry {
                id: image.id.clone(),
                label: image.label.clone(),
                dirty: image.dirty,
                close_waits: false,
                path: image.path.as_ref().map(|path| path.display().to_string()),
                saves: true,
            })
            .collect(),
        active: active.id.clone(),
        new_tab: Some(Action::new("new_image")),
        new_tab_tip: "New image".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(items: &[ToolItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                ToolItem::Separator => "|".to_string(),
                ToolItem::Color { .. } => "colour".to_string(),
                ToolItem::Slider { caption, suffix, .. } => format!("{caption} {suffix}"),
                ToolItem::Switch { caption, .. } => caption.clone(),
                ToolItem::Segmented { options, .. } => options.join("/"),
                ToolItem::Number { caption, suffix, icon: Some(icon), .. } => format!("({icon}){caption} {suffix}"),
                ToolItem::Number { caption, suffix, .. } => format!("{caption} {suffix}"),
                ToolItem::Choice { caption, options, .. } => format!("{caption}: {}", options.join("/")),
                ToolItem::Label { text, .. } => format!("label {text}"),
                ToolItem::Push { label, dark, .. } => format!("{label}{}", if *dark { " (dark)" } else { "" }),
                ToolItem::Button { icon, selected: true, .. } => format!("{icon}*"),
                ToolItem::Button { icon, .. } => icon.clone(),
                ToolItem::Menu { label, .. } => format!("menu {label}"),
                ToolItem::Maximize { .. } => "maximize".to_string(),
                _ => "other".to_string(),
            })
            .collect()
    }

    /// Catches a settings row that drifts from the approved views: brush.png,
    /// selection.png, crop.png, clone.png, heal.png, and text.png, and the
    /// toolbar they share.
    #[test]
    fn the_settings_rows_follow_the_views() {
        let mut session = Session::launch();
        let bar = toolbar(&session);
        assert_eq!(
            kinds(&bar.items),
            [
                "folder", "floppy-disk", "|", "undo", "redo", "|", "cursor-pointer", "square-dashed",
                "chat-bubble-empty", "crop", "color-picker", "bandage", "stamp", "brush*", "erase", "text",
                "drag-hand-gesture", "search",
            ]
        );
        // A new picture is fitted: 1920 wide in 1200 less the margins is 58%.
        assert_eq!(kinds(&bar.trailing), ["|", "menu 58%", "|", "zoom-out", "zoom-in", "frame", "maximize"]);
        let ToolItem::Menu { rows, .. } = &bar.trailing[1] else { panic!() };
        let actions: Vec<_> = rows.iter().map(|row| row.action.clone().unwrap()).collect();
        assert_eq!(actions[0], Action::new("canvas.fit"));
        assert_eq!(actions[2], Action::with_arg("canvas.zoom", json!({ "zoom": 1.0 })));
        assert_eq!(
            kinds(&bar.options),
            ["colour", "|", "Size px", "Hardness %", "Opacity %", "|", "Pen pressure"]
        );
        let ToolItem::Slider { value, .. } = &bar.options[2] else { panic!() };
        assert_eq!(*value, 24);

        session.choose_tool(ToolKind::Eraser);
        assert_eq!(kinds(&toolbar(&session).options), ["Size px", "Hardness %", "Opacity %", "|", "Pen pressure"]);

        for tool in [ToolKind::Marquee, ToolKind::Lasso] {
            session.choose_tool(tool);
            let bar = toolbar(&session);
            assert_eq!(kinds(&bar.options), ["New/Add/Subtract/Intersect", "|", "Feather px"]);
            assert_eq!(kinds(&bar.options_trailing), ["Deselect", "Invert"]);
        }

        session.choose_tool(ToolKind::Crop);
        let bar = toolbar(&session);
        assert_eq!(kinds(&bar.options), ["Ratio: Free/1:1/4:3/3:2/16:9", "label 1920 × 1080"]);
        assert_eq!(kinds(&bar.options_trailing), ["Cancel", "Apply (dark)"]);

        for tool in [ToolKind::Move, ToolKind::Eyedropper] {
            session.choose_tool(tool);
            let bar = toolbar(&session);
            assert_eq!(bar.options.len(), 1);
            assert!(matches!(bar.options[0], ToolItem::Label { .. }));
        }

        session.choose_tool(ToolKind::Stamp);
        assert_eq!(
            kinds(&toolbar(&session).options),
            ["Size px", "Hardness %", "Opacity %", "|", "Aligned", "label Alt-click to set the source"]
        );
        session.choose_tool(ToolKind::Heal);
        assert_eq!(kinds(&toolbar(&session).options), ["Size px", "Hardness %", "|", "label Alt-click to set the source"]);
        session.choose_tool(ToolKind::Text);
        session.tools.text.bold = true;
        assert_eq!(
            kinds(&toolbar(&session).options),
            [
                "colour", ": Inter/Monospace", " px", "|", "bold*", "italic", "underline", "|", "align-left*", "align-center",
                "align-right", "align-justify", "|", "(line-space) ", "(align-horizontal-spacing)Spacing ", "text-size",
            ]
        );
    }

    /// Catches a tool that cannot act on the chosen layer and does not say
    /// why: the brush on a text, the stamp on a mask, and moving pixels on
    /// an adjustment.
    #[test]
    fn refusals_show_in_the_settings_row() {
        let mut session = Session::launch();
        let reason = |session: &Session| {
            toolbar(session).options.iter().find_map(|item| match item {
                ToolItem::Label { text, strong: true } => Some(text.clone()),
                _ => None,
            })
        };
        let doc = session.images[0].doc_mut();
        let text = doc.add_text("Hi", [0, 0], None, crate::text::Style::default(), None).unwrap();
        session.choose_tool(ToolKind::Brush);
        assert!(reason(&session).is_some_and(|text| text.contains("T tool")), "{:?}", reason(&session));
        let doc = session.images[0].doc_mut();
        doc.add_mask(text, None).unwrap();
        assert_eq!(reason(&session), None, "the brush refused the text's mask");
        session.choose_tool(ToolKind::Stamp);
        assert!(reason(&session).is_some_and(|text| text.contains("mask")));
        let doc = session.images[0].doc_mut();
        doc.add_adjustment(Kind::Grade(Default::default()), None, None).unwrap();
        doc.select(crate::paint::Mask::rect(1920, 1080, [0.0, 0.0], [10.0, 10.0]), Combine::New, 0.0);
        session.choose_tool(ToolKind::Move);
        assert!(reason(&session).is_some_and(|text| text.contains("adjustment")));
    }

    /// Catches a Layers page that drifts from the mockup: three pages, all
    /// enabled, Blend Mode, Opacity, Fill, and four locks above the list,
    /// the heading's add, group, and delete, and thumbnails.
    #[test]
    fn the_layers_page_follows_the_mockup() {
        let mut session = Session::launch();
        let _ = session.emit();
        let panel = properties(&session).unwrap();
        let pages: Vec<(String, bool)> = panel.pages.iter().map(|page| (page.label.clone(), page.enabled)).collect();
        assert_eq!(
            pages,
            [("Layers".to_string(), true), ("Properties".to_string(), true), ("Adjustments".to_string(), true)]
        );
        assert!(panel.pages[0].active);
        let labels: Vec<&str> = panel.sections[0].rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["Blend Mode", "Opacity", "Fill", "Lock:"]);
        let RowKind::Icons { icons } = &panel.sections[0].rows[3].kind else { panic!("the locks are not icons") };
        assert_eq!(icons.len(), 4);
        let layers = panel.layers.unwrap();
        let actions: Vec<&str> = layers.actions.iter().map(|action| action.icon.as_str()).collect();
        assert_eq!(actions, ["plus", "folder", "trash"]);
        assert_eq!(layers.rows.len(), 1);
        assert_eq!(layers.rows[0].name, "Layer 1");
        let thumbnail = layers.rows[0].thumbnail.as_ref().expect("no thumbnail");
        assert_eq!((thumbnail.width, thumbnail.height), (56, 32));
        let bytes = std::fs::read(&thumbnail.path).unwrap();
        assert_eq!(bytes.len(), 56 * 32 * 4);
        assert!(bytes.chunks(4).all(|pixel| pixel == [0, 0, 0, 0]));
    }

    /// Catches rows that drift from layers.png and the Properties and
    /// Adjustments views: a text's row has the text icon and no picture, a
    /// mask's row sits directly above its layer with the link mark, an
    /// adjustment's has the filter-list mark, a row's menu offers Add mask;
    /// Properties shows a curve with its channels, histogram, note, and
    /// Reset, a grade's four centred numbers, and a mask's Feather, Invert,
    /// and note; Adjustments lists Curves and Color Grade to add.
    #[test]
    fn the_layer_rows_and_pages_follow_the_views() {
        // A white picture, so the curve's histogram has something to show.
        let mut session = Session::launch();
        *session.images[0].doc_mut() = crate::doc::Doc::new(crate::picture::Picture::blank(1920, 1080));
        let doc = session.images[0].doc_mut();
        let background = doc.active.unwrap();
        doc.add_text("Golden hour", [10, 10], None, crate::text::Style::default(), None).unwrap();
        doc.activate(background);
        let grade = doc.add_adjustment(Kind::Grade(Default::default()), None, None).unwrap();
        let mask = doc.add_mask(grade, None).unwrap();
        let curves = doc.add_adjustment(Kind::Curves(Default::default()), Some(background), None).unwrap();
        let _ = session.emit();
        let panel = properties(&session).unwrap();
        let rows = panel.layers.unwrap().rows;
        let names: Vec<(&str, &str, Option<&str>)> = rows.iter().map(|row| (row.name.as_str(), row.icon.as_str(), row.mark.as_deref())).collect();
        assert_eq!(
            names,
            [
                ("Golden hour", "text", None),
                ("Color Grade Mask", "media-image", Some("link")),
                ("Color Grade", "filter-list", Some("filter-list")),
                ("Curves", "filter-list", Some("filter-list")),
                ("Background", "media-image", None),
            ]
        );
        assert!(rows[0].thumbnail.is_none(), "a text row has a picture");
        assert!(rows[1].thumbnail.is_some() && rows[3].thumbnail.is_some(), "a mask or curve has no picture");
        assert_eq!(rows[1].id, mask);
        assert!(rows[4].menu.iter().any(|entry| entry.label == "Add mask"));
        assert!(!rows[2].menu.iter().any(|entry| entry.label == "Add mask"), "a masked layer offers another");

        session.page = Page::Properties;
        session.images[0].doc_mut().activate(curves);
        let section = &properties(&session).unwrap().sections[0];
        assert_eq!(section.title, "Curves");
        assert!(matches!(&section.rows[0].kind, RowKind::Choice { segmented: true, options, .. } if options.len() == 4));
        let RowKind::Curve { histogram, square: true, free: true } = &section.rows[1].kind else { panic!("no square, free curve") };
        assert_eq!(histogram.len(), 128);
        assert!(histogram.iter().any(|value| *value > 0.9), "the histogram is empty");
        assert!(matches!(section.rows.last().unwrap().kind, RowKind::Button { .. }));

        session.images[0].doc_mut().activate(grade);
        let section = &properties(&session).unwrap().sections[0];
        let centred = section.rows.iter().filter(|row| matches!(row.kind, RowKind::Number { centered: true, .. })).count();
        assert_eq!(centred, 4);
        session.images[0].doc_mut().choose_mask(grade);
        let section = &properties(&session).unwrap().sections[0];
        let labels: Vec<&str> = section.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels[..2], ["Feather", "Invert"]);
        assert!(matches!(section.rows[2].kind, RowKind::Note));
        assert!(properties(&session).unwrap().layers.is_none());

        session.page = Page::Adjustments;
        let section = &properties(&session).unwrap().sections[0];
        let items: Vec<&str> = section.rows.iter().filter(|row| matches!(row.kind, RowKind::Item { .. })).map(|row| row.label.as_str()).collect();
        assert_eq!(items, ["Curves", "Color Grade"]);
    }

    /// Catches a key that the host keeps for itself, or two keys on one chord.
    #[test]
    fn keys_are_unique_and_leave_f4() {
        let keys = keys();
        let mut seen = std::collections::HashSet::new();
        for binding in &keys.bindings {
            assert_ne!(binding.chord.key, "F4");
            let chord = &binding.chord;
            assert!(seen.insert((chord.key.clone(), chord.command, chord.shift, chord.alt)), "{chord:?} twice");
        }
    }
}
