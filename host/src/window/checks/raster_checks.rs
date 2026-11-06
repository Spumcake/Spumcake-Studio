//! Window checks for raster tabs, against a stand-in sub-application that
//! no part of the host was written for. It declares a tab of pixels, writes a
//! known image as raw RGBA, and logs every line the host sends it, so each
//! check reads what the host drew and what it sent. They spend nothing.

use super::{StandIn, Window, scratch, start};
use crate::window::Started;
use eframe::egui::{self, Color32, Pos2, Rect, pos2};
use protocol::{
    Action, AppMessage, Binding, Chord, ChoicePreview, DocumentCamera, Film, FilmItem, FlagIcons, HeadingAction,
    IconToggle, Keys, LayerRow, Layers, MenuEntry, PanelPage, PropValue, Properties, RasterTool, RasterView, Row,
    RowKind, Section, TabEntry, Tabs, TextEditing, Thumbnail, ToolItem, Toolbar,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The stand-in: it prints its greeting, then logs each line it is sent.
/// A command `tool.<name>` is answered with the lines in `<name>.jsonl`, the
/// view and toolbar of that tool. A text's Copy is answered with the lines
/// in `copied.jsonl`.
const EASEL: &str = r#"#!/bin/sh
dir="__DIR__"
cat "$dir/greeting.jsonl"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$dir/log"
  case "$line" in
    *'"type":"ping"'*) echo '{"type":"pong"}' ;;
    *'"type":"shutdown"'*) exit 0 ;;
    *'"type":"quit"'*) echo '{"type":"quit_ready"}' ;;
    *'"type":"raster_text"'*'"key":"copy"'*) cat "$dir/copied.jsonl" ;;
    *'"id":"tool.'*)
      tool=$(printf '%s' "$line" | sed 's/.*"id":"tool\.\([a-z]*\)".*/\1/')
      cat "$dir/$tool.jsonl" ;;
  esac
done
"#;

/// The image's size, and the camera the view carries: the image is
/// drawn 50 points in from the contents' corner, 10 points to a pixel.
const W: u32 = 40;
const H: u32 = 20;
const ORIGIN: [f64; 2] = [-5.0, -5.0];
const ZOOM: f64 = 10.0;
const BRUSH: [u8; 3] = [200, 30, 40];
const CHIP: [u8; 3] = [20, 20, 22];

fn line(message: AppMessage) -> String {
    protocol::encode_app_message(&message)
}

fn view(dir: &Path, tool: RasterTool) -> RasterView {
    RasterView {
        tab: String::from("r1"),
        width: W,
        height: H,
        image: dir.join("image.rgba").display().to_string(),
        revision: 1,
        patch: None,
        tool,
        selection: vec![vec![[10.0, 0.0], [30.0, 0.0], [30.0, 20.0], [10.0, 20.0]]],
        crop: Some([0.0, 0.0, 20.0, 10.0]),
        bounds: Some([0.0, 0.0, 40.0, 20.0]),
        camera: Some(DocumentCamera { origin: ORIGIN, zoom: ZOOM }),
        film: None,
        text: None,
    }
}

fn brush() -> RasterTool {
    RasterTool::Brush { rgb: BRUSH, size: 1.0, hardness: 1.0, opacity: 1.0, pressure: true }
}

/// Where the stamp copies from until a stroke sets an offset, and the
/// offset an aligned stamp holds.
const SOURCE: [f64; 2] = [30.0, 15.0];
const OFFSET: [f64; 2] = [10.0, 5.0];

fn sampler(offset: Option<[f64; 2]>) -> RasterTool {
    RasterTool::Sampler { size: 4.0, pressure: false, source: Some(SOURCE), offset }
}

/// The text typed in: its box, its caret after its fifth pixel, and a
/// selected span.
fn typed() -> TextEditing {
    TextEditing { rect: [10.0, 5.0, 20.0, 6.0], caret: [15.0, 5.0, 0.2, 6.0], selection: vec![[11.0, 5.0, 3.0, 6.0]] }
}

/// The stand-in's shortcuts: undo and redo, as the views name them, and a
/// tool's single letter.
fn keys() -> Keys {
    let chord = |key: &str, command: bool, shift: Option<bool>| Chord { key: key.to_string(), command, shift, alt: false };
    let bind = |chord: Chord, id: &str| Binding { chord, then: Vec::new(), action: Action::new(id) };
    Keys {
        bindings: vec![
            bind(chord("Z", true, Some(false)), "undo"),
            bind(chord("Z", true, Some(true)), "redo"),
            bind(chord("Y", true, Some(false)), "redo"),
            bind(chord("B", false, Some(false)), "tool.brush"),
        ],
        copy: None,
        cut: None,
        paste: None,
    }
}

/// The settings row of each kind of tool, as the views show them.
fn toolbar(kind: &str) -> Toolbar {
    let (options, options_trailing) = match kind {
        "select" => (
            vec![
                ToolItem::Segmented {
                    options: ["New", "Add", "Subtract", "Intersect"].map(String::from).to_vec(),
                    selected: 0,
                    set: Action::new("set_mode"),
                },
                ToolItem::Separator,
                ToolItem::Number {
                    caption: String::from("Feather"),
                    value: 0.0,
                    min: 0.0,
                    max: 100.0,
                    suffix: String::from("px"),
                    set: Action::new("set_feather"),
                    icon: None,
                },
            ],
            vec![ToolItem::Push { label: String::from("Deselect"), dark: false, enabled: true, action: Action::new("deselect") }],
        ),
        "crop" => (
            vec![
                ToolItem::Choice {
                    caption: String::from("Ratio"),
                    options: ["Free", "1:1", "16:9"].map(String::from).to_vec(),
                    selected: 0,
                    set: Action::new("set_ratio"),
                },
                ToolItem::Label { text: String::from("3200 × 1800"), strong: true },
            ],
            vec![
                ToolItem::Push { label: String::from("Later"), dark: false, enabled: false, action: Action::new("later") },
                ToolItem::Push { label: String::from("Cancel"), dark: false, enabled: true, action: Action::new("cancel_crop") },
                ToolItem::Push { label: String::from("Apply"), dark: true, enabled: true, action: Action::new("apply_crop") },
            ],
        ),
        _ => (
            vec![
                ToolItem::Color { rgb: CHIP, tip: String::from("Colour"), set: Action::new("set_color") },
                ToolItem::Separator,
                ToolItem::Slider {
                    caption: String::from("Size"),
                    width: 190.0,
                    value: 24,
                    min: 1,
                    max: 200,
                    tint: None,
                    set: Action::new("set_size"),
                    finish: Some(Action::new("finish_size")),
                    suffix: String::from("px"),
                },
                ToolItem::Switch { caption: String::from("Pen pressure"), on: true, action: Action::new("toggle_pressure") },
            ],
            Vec::new(),
        ),
    };
    Toolbar { items: Vec::new(), trailing: Vec::new(), options, options_trailing }
}

/// A row of the Layers list. A layer has an image; a group has none.
fn layer_row(dir: &Path, id: u64, parent: Option<u64>, depth: u32, name: &str, group: bool) -> LayerRow {
    LayerRow {
        id,
        parent,
        depth,
        icon: String::from(if group { "folder" } else { "frame" }),
        name: String::from(name),
        group,
        selected: true,
        visible: true,
        locked: false,
        renamable: true,
        click: Action::new("select_layer"),
        rename: Action::new("rename_layer"),
        toggle_visible: Action::new("toggle_visible"),
        toggle_locked: Action::new("toggle_lock"),
        menu: Vec::new(),
        colors: Vec::new(),
        thumbnail: (!group).then(|| Thumbnail { path: dir.join("thumb.rgba").display().to_string(), width: W, height: H, revision: 1 }),
        mark: None,
    }
}

fn properties(rows: Vec<LayerRow>) -> Properties {
    let heading = |icon: &str, tip: &str, id: &str| HeadingAction {
        icon: icon.to_string(),
        tip: tip.to_string(),
        enabled: true,
        action: Action::new(id),
    };
    let lock = |icon: &str| IconToggle { icon: icon.to_string(), tip: icon.to_string(), on: false };
    Properties {
        open: true,
        toggle: Action::new("toggle_panel"),
        tip: String::from("Layers"),
        finish: Action::new("finish_edit"),
        back: None,
        layers: Some(Layers {
            rows,
            reorder: Action::new("reorder"),
            sort_color: Action::new("sort_color"),
            nest: true,
            flags: FlagIcons {
                eye_open: String::from("eye"),
                eye_closed: String::from("eye-closed"),
                lock_closed: String::from("lock"),
                lock_open: String::from("lock-slash"),
            },
            actions: vec![
                heading("plus", "Add a layer", "add_layer"),
                heading("folder", "Group", "group_layers"),
                heading("trash", "Delete", "delete_layer"),
            ],
        }),
        sections: vec![
            Section {
                title: String::new(),
                rows: vec![Row {
                    prop: json!("blend"),
                    label: String::from("Blend Mode"),
                    kind: RowKind::Choice {
                        options: vec![String::from("Normal"), String::from("Multiply")],
                        segmented: false,
                        preview: ChoicePreview::Label,
                    },
                    value: PropValue::Index(0),
                }],
                icon: None,
            },
            Section {
                title: String::new(),
                rows: vec![Row {
                    prop: json!("lock"),
                    label: String::from("Lock:"),
                    kind: RowKind::Icons { icons: vec![lock("square-dashed"), lock("edit-pencil"), lock("drag"), lock("lock")] },
                    value: PropValue::Idle,
                }],
                icon: None,
            },
        ],
        pages: vec![
            PanelPage { label: String::from("Layers"), active: true, enabled: true, action: None },
            PanelPage { label: String::from("Properties"), active: false, enabled: false, action: Some(Action::new("page_properties")) },
            PanelPage { label: String::from("Adjustments"), active: false, enabled: true, action: Some(Action::new("page_adjustments")) },
        ],
    }
}

/// The Properties page with these sections and no Layers list, as a chosen
/// adjustment shows it.
fn adjusting(sections: Vec<Section>) -> Properties {
    Properties {
        layers: None,
        sections,
        pages: vec![
            PanelPage { label: String::from("Layers"), active: false, enabled: true, action: Some(Action::new("page_layers")) },
            PanelPage { label: String::from("Properties"), active: true, enabled: true, action: None },
        ],
        ..properties(Vec::new())
    }
}

/// A row with no value of its own.
fn idle_row(label: &str, kind: RowKind) -> Row {
    Row { prop: json!(label), label: label.to_string(), kind, value: PropValue::Idle }
}

/// As properties-grade.png and adjustments.png show: a section with its
/// icon, a centred number, a button, a note, and an item.
fn grade_sections() -> Vec<Section> {
    vec![Section {
        title: String::from("Color Grade"),
        icon: Some(String::from("filter-list")),
        rows: vec![
            Row {
                prop: json!("contrast"),
                label: String::from("Contrast"),
                kind: RowKind::Number { min: -100.0, max: 100.0, percent: false, places: 0, centered: true },
                value: PropValue::Number(30.0),
            },
            idle_row("Reset", RowKind::Button { action: Action::new("reset_grade") }),
            idle_row(NOTE, RowKind::Note),
            idle_row("Curves", RowKind::Item { icon: String::from("filter-list"), detail: String::from("Tone and colour by curve"), action: Action::new("add_curves") }),
        ],
    }]
}

const NOTE: &str = "White shows the layer beneath, black hides it. Paint it with the brush and the eraser.";

/// As properties-curves.png shows: a square, free curve over a histogram.
fn curve_sections() -> Vec<Section> {
    let histogram: Vec<f32> = (0..32).map(|bin| if bin % 4 == 0 { 0.0 } else { bin as f32 / 32.0 }).collect();
    vec![Section {
        title: String::from("Curves"),
        icon: None,
        rows: vec![Row {
            prop: json!("curve"),
            label: String::new(),
            kind: RowKind::Curve { histogram, square: true, free: true },
            value: PropValue::Curve(vec![(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)]),
        }],
    }]
}

/// The image: opaque red, but for one clear pixel in its corner, so it
/// is drawn over squares.
fn image_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for index in 0..W * H {
        bytes.extend_from_slice(if index == 0 { &[0, 0, 0, 0] } else { &[220, 40, 40, 255] });
    }
    bytes
}

/// Starts the stand-in in a window. Returns the window and its folder.
fn easel(name: &str) -> (Window, PathBuf, StandIn) {
    easel_with(name, |dir| vec![layer_row(dir, 1, None, 0, "Background", false)])
}

/// The stand-in with these rows in its Layers list.
fn easel_with(name: &str, rows: impl FnOnce(&Path) -> Vec<LayerRow>) -> (Window, PathBuf, StandIn) {
    easel_of(name, rows, false)
}

/// The film's images: their names and their thumbnails' sizes, a wide one,
/// a tall one, and a square one.
const IMAGES: [(&str, u32, u32); 3] = [("Harbor", 32, 18), ("Tower", 18, 32), ("Tile", 20, 20)];

/// A gallery of the three, the second shown, whose rows and tiles send
/// actions only this stand-in declares. Its view leaves the camera to the
/// host, so the host frames the image above the strip.
fn film(dir: &Path) -> Film {
    let row = |label: &str, id: &str, arg: Option<Value>| MenuEntry {
        label: label.to_string(),
        icon: None,
        enabled: true,
        divider: false,
        action: Some(Action { id: id.to_string(), arg }),
        submenu: Vec::new(),
        rename: false,
    };
    Film {
        items: IMAGES
            .iter()
            .enumerate()
            .map(|(index, (name, width, height))| FilmItem {
                name: name.to_string(),
                thumbnail: Thumbnail { path: dir.join(format!("film{index}.rgba")).display().to_string(), width: *width, height: *height, revision: 1 },
                menu: vec![
                    row("Duplicate", "copy_item", Some(json!({ "item": index }))),
                    row("Rename", "rename_item", Some(json!({ "item": index }))),
                    row("Delete", "remove_item", Some(json!({ "item": index }))),
                ],
            })
            .collect(),
        shown: 1,
        select: Action::new("show_item"),
        reorder: Action::with_arg("order_items", json!({ "strip": true })),
        add: vec![row("New image", "new_item", None), row("Open PNG or JPEG…", "open_item", None)],
        focus: Action::new("strip_focus"),
    }
}

/// The stand-in with a filmstrip of three images.
fn filmed(name: &str) -> (Window, PathBuf, StandIn) {
    easel_of(name, |dir| vec![layer_row(dir, 1, None, 0, "Background", false)], true)
}

fn easel_of(name: &str, rows: impl FnOnce(&Path) -> Vec<LayerRow>, filmed: bool) -> (Window, PathBuf, StandIn) {
    let dir = scratch(&format!("raster-{name}"));
    std::fs::write(dir.join("image.rgba"), image_bytes()).unwrap();
    std::fs::write(dir.join("thumb.rgba"), image_bytes()).unwrap();
    for (index, (_, width, height)) in IMAGES.iter().enumerate() {
        std::fs::write(dir.join(format!("film{index}.rgba")), vec![200; (width * height * 4) as usize]).unwrap();
    }
    let declared = |dir: &Path, tool: RasterTool| {
        let mut view = view(dir, tool);
        if filmed {
            view.film = Some(film(dir));
            view.camera = None;
        }
        view
    };
    let typing = |dir: &Path| {
        let mut view = declared(dir, RasterTool::Text);
        view.text = Some(typed());
        view
    };
    let greeting = [
        String::from("{\"type\":\"hello\",\"protocol\":1}\n"),
        String::from("{\"type\":\"tab\",\"id\":\"r1\",\"label\":\"Image\"}\n"),
        String::from("{\"type\":\"contents\",\"tab\":\"r1\",\"kind\":\"raster\"}\n"),
        String::from("{\"type\":\"panel\",\"tab\":\"r1\",\"side\":\"right\"}\n"),
        String::from("{\"type\":\"end\"}\n"),
        String::from("{\"type\":\"tools\",\"groups\":[{\"id\":\"easel\",\"name\":\"Easel\",\"description\":\"Shows an image.\",\"instructions\":\"Nothing to do.\",\"tools\":[{\"name\":\"look\",\"description\":\"Look.\",\"input\":{\"type\":\"object\",\"properties\":{}}}]}]}\n"),
        line(AppMessage::Tabs(Tabs {
            tabs: vec![TabEntry { id: String::from("r1"), label: String::from("Image"), dirty: false, close_waits: false, path: None, saves: false }],
            active: String::from("r1"),
            new_tab: None,
            new_tab_tip: String::new(),
        })),
        line(AppMessage::Properties(properties(rows(&dir)))),
        line(AppMessage::Raster(declared(&dir, brush()))),
        line(AppMessage::Toolbar(toolbar("brush"))),
        line(AppMessage::Keys(keys())),
    ];
    std::fs::write(dir.join("greeting.jsonl"), greeting.concat()).unwrap();
    for (name, tool, bar) in [
        ("brush", brush(), "brush"),
        ("eraser", RasterTool::Erase { size: 2.0, hardness: 1.0, opacity: 1.0, pressure: false }, "brush"),
        ("marquee", RasterTool::Marquee, "select"),
        ("lasso", RasterTool::Lasso, "select"),
        ("move", RasterTool::Move, "brush"),
        ("crop", RasterTool::Crop, "crop"),
        ("eyedropper", RasterTool::Eyedropper, "brush"),
        ("hand", RasterTool::Hand, "brush"),
        ("zoom", RasterTool::Zoom, "brush"),
        ("stamp", sampler(None), "brush"),
        ("aligned", sampler(Some(OFFSET)), "brush"),
        ("text", RasterTool::Text, "brush"),
    ] {
        let lines = [line(AppMessage::Raster(declared(&dir, tool))), line(AppMessage::Toolbar(toolbar(bar)))];
        std::fs::write(dir.join(format!("{name}.jsonl")), lines.concat()).unwrap();
    }
    std::fs::write(dir.join("typing.jsonl"), [line(AppMessage::Raster(typing(&dir))), line(AppMessage::Toolbar(toolbar("brush")))].concat()).unwrap();
    std::fs::write(dir.join("copied.jsonl"), line(AppMessage::Clipboard { text: String::from("Hi") })).unwrap();
    std::fs::write(dir.join("grade.jsonl"), line(AppMessage::Properties(adjusting(grade_sections())))).unwrap();
    std::fs::write(dir.join("curves.jsonl"), line(AppMessage::Properties(adjusting(curve_sections())))).unwrap();
    let program = dir.join("easel");
    std::fs::write(&program, EASEL.replace("__DIR__", &dir.display().to_string())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let stand_in = StandIn::start(Vec::new());
    // A script written a moment ago can be busy for another check's
    // fork, so starting it is tried again.
    let mut started: Started = start("easel", "Easel", &program);
    for _ in 0..20 {
        if started.running.answers() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        started = start("easel", "Easel", &program);
    }
    let mut window = Window::new(vec![started], &stand_in);
    window.until(|app| app.apps[0].running.declared.rasters.contains_key("r1") && app.apps[0].running.declared.properties.is_some());
    window.step();
    window.step();
    (window, dir, stand_in)
}

/// Every line the host sent the stand-in, in order.
fn sent(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn of_type(dir: &Path, kind: &str) -> Vec<Value> {
    sent(dir).into_iter().filter(|line| line["type"] == kind).collect()
}

/// Every finished gesture: the parts of a stroke or a move sent while it is
/// drawn are left out.
fn gestures(dir: &Path) -> Vec<Value> {
    all_gestures(dir).into_iter().filter(|gesture| gesture["kind"] != "stroke_part" && gesture["kind"] != "drag_part").collect()
}

fn all_gestures(dir: &Path) -> Vec<Value> {
    of_type(dir, "raster_gesture").into_iter().map(|line| line["gesture"].clone()).collect()
}

fn commands(dir: &Path, id: &str) -> Vec<Value> {
    of_type(dir, "command").into_iter().filter(|line| line["id"] == id).collect()
}

fn raster(window: &Window) -> &crate::process::RasterDisplay {
    &window.app.apps[0].running.declared.rasters["r1"]
}

/// Where an image point is on screen, through the host's camera.
fn screen(window: &Window, point: [f64; 2]) -> Pos2 {
    let rect = window.app.raster.shown;
    let camera = raster(window).camera.expect("the tab has a camera");
    pos2(
        ((point[0] - camera.origin[0]) * camera.zoom) as f32 + rect.min.x,
        ((point[1] - camera.origin[1]) * camera.zoom) as f32 + rect.min.y,
    )
}

/// Chooses a tool through the stand-in, as a toolbar button does.
fn use_tool(window: &mut Window, name: &str, is: fn(&RasterTool) -> bool) {
    window.app.run_action(&Action::new(format!("tool.{name}")));
    window.until(move |app| is(&app.apps[0].running.declared.rasters["r1"].view.tool));
    window.step();
}

fn press(window: &mut Window, at: Pos2, pressed: bool) {
    window.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
}

/// A drag through image points: pressed at the first, moved through the
/// rest, released at the last. `during` runs on the frame before release.
fn drag(window: &mut Window, points: &[[f64; 2]], during: impl FnOnce(&Window)) {
    let screens: Vec<Pos2> = points.iter().map(|point| screen(window, *point)).collect();
    window.events.push(egui::Event::PointerMoved(screens[0]));
    window.step();
    press(window, screens[0], true);
    window.step();
    for at in &screens[1..] {
        window.events.push(egui::Event::PointerMoved(*at));
        window.step();
    }
    during(window);
    press(window, *screens.last().unwrap(), false);
    window.step();
}

fn click(window: &mut Window, at: Pos2) {
    window.events.push(egui::Event::PointerMoved(at));
    window.step();
    press(window, at, true);
    window.step();
    press(window, at, false);
    window.step();
}

/// Frames enough for a menu to fade in.
fn settle(window: &mut Window) {
    for _ in 0..12 {
        window.step();
    }
}

fn text_at(window: &Window, text: &str) -> Rect {
    window
        .texts
        .iter()
        .filter(|(shown, _)| shown == text)
        .map(|(_, rect)| *rect)
        .next_back()
        .unwrap_or_else(|| panic!("{text:?} was not drawn"))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

fn point(value: &Value) -> [f64; 2] {
    [value[0].as_f64().unwrap(), value[1].as_f64().unwrap()]
}

/// Rects painted with a texture of this name, and where.
fn textured(window: &Window, name: &str) -> Vec<Rect> {
    let manager = window.ctx.tex_manager();
    let manager = manager.read();
    window
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) => rect.brush.as_ref().map(|brush| (rect.rect, brush.fill_texture_id)),
            egui::Shape::Mesh(mesh) => Some((mesh.calc_bounds(), mesh.texture_id)),
            _ => None,
        })
        .filter(|(_, id)| manager.meta(*id).is_some_and(|meta| meta.name == name))
        .map(|(rect, _)| rect)
        .collect()
}

fn texture_size(window: &Window, name: &str) -> Option<[usize; 2]> {
    let manager = window.ctx.tex_manager();
    let manager = manager.read();
    manager.allocated().find(|(_, meta)| meta.name == name).map(|(_, meta)| meta.size)
}

fn segments(window: &Window, color: Color32) -> Vec<[Pos2; 2]> {
    window
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::LineSegment { points, stroke } if stroke.color == color => Some(*points),
            _ => None,
        })
        .collect()
}

fn discs(window: &Window, color: Color32) -> usize {
    window
        .shapes
        .iter()
        .filter(|clipped| matches!(&clipped.shape, egui::Shape::Circle(dot) if dot.fill == color))
        .count()
}

/// Ends the window, whose stand-in exits when its input closes, and
/// removes the check's folder.
fn close(window: Window, dir: PathBuf) {
    drop(window);
    let _ = std::fs::remove_dir_all(dir);
}

/// Catches a raster tab that is not drawn, or drawn somewhere its camera
/// does not put it: the stand-in's raw image becomes a texture of its own
/// size, at the place the view's camera says, over squares for its clear
/// pixel, with its size under it; the selection's outline is a white edge
/// with black dashes along it; and the crop box dims what lies outside it
/// and is divided in thirds.
#[test]
fn the_image_its_selection_and_its_crop_box_are_drawn_where_the_camera_puts_them() {
    let (mut window, dir, _stand_in) = easel("drawn");
    let name = format!("raster:{}", dir.join("image.rgba").display());
    assert_eq!(texture_size(&window, &name), Some([W as usize, H as usize]), "the image was not read whole");
    let rect = window.app.raster.shown;
    let expected = Rect::from_min_size(rect.min + egui::vec2(50.0, 50.0), egui::vec2(400.0, 200.0));
    let drawn = textured(&window, &name);
    assert!(drawn.iter().any(|at| (at.min - expected.min).length() < 0.01 && (at.max - expected.max).length() < 0.01), "the image is not where the camera puts it: {drawn:?}, not {expected:?}");
    assert!(textured(&window, "raster-checker").iter().any(|at| *at == expected), "no squares under the clear pixel");
    // No size caption under the image, as the owner set on 2026-10-30.
    assert!(window.texts.iter().all(|(text, _)| text != "40 × 20"), "the image's size is drawn under it");

    // The selection: a white edge along its outline, and black dashes on it.
    let outline = Rect::from_two_pos(screen(&window, [10.0, 0.0]), screen(&window, [30.0, 20.0]));
    let white = window.shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::Path(path) => {
            path.stroke.color == egui::epaint::ColorMode::Solid(Color32::WHITE)
                && path.points.first() == Some(&outline.left_top())
                && path.points.len() == 5
        }
        _ => false,
    });
    assert!(white, "the selection's edge was not drawn");
    let dashes = segments(&window, Color32::BLACK);
    assert!(dashes.len() >= 20, "the selection has no marching dashes: {}", dashes.len());
    assert!(dashes.iter().flatten().all(|at| outline.expand(0.5).contains(*at) && !outline.shrink(0.5).contains(*at)), "a dash is off the selection's edge");

    // The crop box: the outside dimmed, and thirds across it.
    let frame = Rect::from_two_pos(screen(&window, [0.0, 0.0]), screen(&window, [20.0, 10.0]));
    let dimmed = window.shapes.iter().filter(|clipped| matches!(&clipped.shape, egui::Shape::Rect(shown) if shown.fill == Color32::from_black_alpha(110))).count();
    assert!(dimmed >= 2, "the outside of the crop box is not dimmed");
    let thirds = segments(&window, Color32::from_white_alpha(190));
    let x = frame.left() + frame.width() / 3.0;
    assert!(thirds.iter().any(|[a, b]| (a.x - x).abs() < 0.01 && (b.x - x).abs() < 0.01), "the crop box has no thirds");

    // The move tool's box: eight handles in blue on the layer's bounds.
    let handles = |window: &Window| -> Vec<Rect> {
        window
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(shown) if shown.stroke.color == Color32::from_rgb(37, 99, 235) && shown.rect.width() < 10.0 => Some(shown.rect),
                _ => None,
            })
            .collect()
    };
    assert!(handles(&window).is_empty(), "the layer's box shows without the move tool");
    use_tool(&mut window, "move", |tool| *tool == RasterTool::Move);
    let shown = handles(&window);
    assert_eq!(shown.len(), 8, "the move tool does not show the layer's handles");
    assert!(shown.iter().any(|handle| (handle.center() - expected.min).length() < 0.01));
    close(window, dir);
}

/// Catches gestures that are not sent, sent more than once, or sent in
/// screen points: each tool's drag or click on the stand-in's image is
/// one `raster_gesture` in image pixels, through the camera. The brush
/// sends its points as they come, in order, and then the whole stroke; it
/// draws no trail of its own, so the stroke seen is the image's.
#[test]
fn each_tool_sends_one_gesture_in_image_pixels() {
    let (mut window, dir, _stand_in) = easel("gestures");

    // The brush: one paint drag, its points sent as they come.
    let mut trail = 0;
    drag(&mut window, &[[5.0, 5.0], [10.0, 6.0], [15.0, 8.0], [25.0, 12.0]], |window| {
        trail += discs(window, Color32::from_rgb(BRUSH[0], BRUSH[1], BRUSH[2]));
    });
    assert_eq!(trail, 0, "the host drew a trail of its own over the image");
    window.until(|_| !gestures(&dir).is_empty());
    let sent = gestures(&dir);
    assert_eq!(sent.len(), 1, "one drag sent {} gestures", sent.len());
    assert_eq!(sent[0]["kind"], "stroke");
    let points = sent[0]["points"].as_array().unwrap();
    assert!(points.len() >= 4, "the drag lost its points: {points:?}");
    let mut streamed: Vec<Value> = Vec::new();
    for part in all_gestures(&dir).iter().filter(|gesture| gesture["kind"] == "stroke_part") {
        assert_eq!(part["from"].as_u64(), Some(streamed.len() as u64), "a part of the stroke came out of order");
        streamed.extend(part["points"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(&streamed, points, "the parts sent while drawing are not the stroke laid");
    let first = &points[0];
    let last = points.last().unwrap();
    assert!(near(first[0].as_f64().unwrap(), 5.0) && near(first[1].as_f64().unwrap(), 5.0) && near(first[2].as_f64().unwrap(), 1.0), "{first}");
    assert!(near(last[0].as_f64().unwrap(), 25.0) && near(last[1].as_f64().unwrap(), 12.0), "{last}");

    // The marquee: a rectangle, corner to corner.
    use_tool(&mut window, "marquee", |tool| *tool == RasterTool::Marquee);
    drag(&mut window, &[[12.0, 3.0], [15.0, 6.0], [18.0, 9.0]], |_| {});
    window.until(|_| gestures(&dir).len() >= 2);
    let rect = &gestures(&dir)[1];
    assert_eq!(rect["kind"], "rect");
    assert_eq!((point(&rect["from"]), point(&rect["to"])), ([12.0, 3.0], [18.0, 9.0]));

    // The lasso: the outline drawn.
    use_tool(&mut window, "lasso", |tool| *tool == RasterTool::Lasso);
    drag(&mut window, &[[5.0, 5.0], [10.0, 5.0], [10.0, 10.0], [5.0, 10.0]], |_| {});
    window.until(|_| gestures(&dir).len() >= 3);
    let lasso = &gestures(&dir)[2];
    assert_eq!(lasso["kind"], "lasso");
    let outline: Vec<[f64; 2]> = lasso["points"].as_array().unwrap().iter().map(point).collect();
    assert_eq!(outline, vec![[5.0, 5.0], [10.0, 5.0], [10.0, 10.0], [5.0, 10.0]]);

    // Move: from the press to the release, with the layer's box following.
    use_tool(&mut window, "move", |tool| *tool == RasterTool::Move);
    let corner = screen(&window, [0.0, 0.0]);
    let mut followed = false;
    drag(&mut window, &[[20.0, 10.0], [22.0, 11.0], [24.0, 12.0]], |window| {
        followed = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::Rect(shown) if shown.stroke.color == Color32::from_rgb(37, 99, 235) && (shown.rect.center() - (corner + egui::vec2(40.0, 20.0))).length() < 0.01));
    });
    assert!(followed, "the layer's box did not follow the move");
    window.until(|_| gestures(&dir).len() >= 4);
    let moved = &gestures(&dir)[3];
    assert_eq!(moved["kind"], "drag");
    assert_eq!((point(&moved["from"]), point(&moved["to"])), ([20.0, 10.0], [24.0, 12.0]));

    // Crop: a new box dragged outside the shown one, then the shown box's
    // right edge dragged out, each on whole pixels.
    use_tool(&mut window, "crop", |tool| *tool == RasterTool::Crop);
    drag(&mut window, &[[32.0, 16.0], [20.0, 9.0], [10.2, 4.4]], |_| {});
    window.until(|_| gestures(&dir).len() >= 5);
    let crop = &gestures(&dir)[4];
    assert_eq!(crop["kind"], "crop");
    assert_eq!(crop["rect"], json!([10.0, 4.0, 22.0, 12.0]));
    drag(&mut window, &[[20.0, 5.0], [25.0, 5.0], [30.0, 5.0]], |_| {});
    window.until(|_| gestures(&dir).len() >= 6);
    assert_eq!(gestures(&dir)[5]["rect"], json!([0.0, 0.0, 30.0, 10.0]), "dragging the box's edge did not move it");

    // The eyedropper: a click picks there.
    use_tool(&mut window, "eyedropper", |tool| *tool == RasterTool::Eyedropper);
    let at = screen(&window, [7.0, 8.0]);
    click(&mut window, at);
    window.until(|_| gestures(&dir).len() >= 7);
    let pick = &gestures(&dir)[6];
    assert_eq!((pick["kind"].clone(), point(&pick["at"])), (json!("pick"), [7.0, 8.0]));

    // The hand and the magnifier move the camera and send no gesture.
    use_tool(&mut window, "hand", |tool| *tool == RasterTool::Hand);
    let before = raster(&window).camera.unwrap();
    drag(&mut window, &[[10.0, 10.0], [8.0, 9.0], [6.0, 8.0]], |_| {});
    let after = raster(&window).camera.unwrap();
    assert!(near(after.origin[0], before.origin[0] + 4.0) && near(after.origin[1], before.origin[1] + 2.0), "the hand did not pan: {after:?}");
    use_tool(&mut window, "zoom", |tool| *tool == RasterTool::Zoom);
    let at = screen(&window, [20.0, 10.0]);
    click(&mut window, at);
    assert!(near(raster(&window).camera.unwrap().zoom, 2.0 * ZOOM), "the magnifier did not zoom in");
    window.until(|_| of_type(&dir, "camera").iter().any(|camera| camera["zoom"] == json!(2.0 * ZOOM)));
    assert_eq!(gestures(&dir).len(), 7, "the hand or the magnifier sent a gesture");
    close(window, dir);
}

/// Catches toolbar zoom commands that act only on a drawing: Fit frames the
/// image, `canvas.zoom` sets a zoom by number, and each change is reported.
#[test]
fn the_toolbar_zooms_and_fits_a_raster_tab() {
    let (mut window, dir, _stand_in) = easel("zoom");
    window.app.run_action(&Action::with_arg("canvas.zoom", json!({"zoom": 1})));
    window.step();
    assert!(near(raster(&window).camera.unwrap().zoom, 1.0));
    window.until(|_| of_type(&dir, "camera").iter().any(|camera| camera["zoom"] == json!(1.0)));
    window.app.run_action(&Action::new("canvas.fit"));
    window.step();
    let rect = window.app.raster.shown;
    let fitted = raster(&window).camera.unwrap();
    let zoom = ((f64::from(rect.width()) - 80.0) / f64::from(W)).min((f64::from(rect.height()) - 80.0) / f64::from(H));
    assert!(near(fitted.zoom, zoom), "Fit chose {} for {zoom}", fitted.zoom);
    let middle = screen(&window, [f64::from(W) / 2.0, f64::from(H) / 2.0]);
    assert!((middle - rect.center()).length() < 0.01, "Fit did not center the image");
    window.app.run_action(&Action::new("canvas.zoom_in"));
    assert!(near(raster(&window).camera.unwrap().zoom, zoom * 1.25));
    let log = dir.clone();
    window.until(move |_| of_type(&log, "camera").iter().any(|camera| near(camera["zoom"].as_f64().unwrap(), zoom * 1.25)));
    close(window, dir);
}

/// Catches settings-row items and panel pieces that draw and do nothing:
/// each one the stand-in declares sends its action with its value, and one
/// that waits sends nothing.
#[test]
fn the_settings_row_and_the_panel_send_their_actions() {
    let (mut window, dir, _stand_in) = easel("controls");

    // The colour chip opens Studio's swatches; one is chosen.
    let chip = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(dot) if dot.fill == Color32::from_rgb(CHIP[0], CHIP[1], CHIP[2]) => Some(dot.center),
            _ => None,
        })
        .expect("the colour chip was not drawn");
    click(&mut window, chip);
    settle(&mut window);
    let red = draw::defaults::PALETTE[1].rgb;
    let swatch = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(dot) if dot.fill == Color32::from_rgb(red[0], red[1], red[2]) => Some(dot.center),
            _ => None,
        })
        .expect("the chip opened no swatches");
    click(&mut window, swatch);
    window.until(|_| !commands(&dir, "set_color").is_empty());
    assert_eq!(commands(&dir, "set_color")[0]["arg"]["value"], json!(red));

    // The slider, with its unit, and the switch.
    let value = text_at(&window, "24");
    let unit = text_at(&window, " px");
    assert!(unit.left() > value.right() && unit.left() - value.right() < 8.0, "the slider's value has no unit after it");
    let size = text_at(&window, "Size");
    click(&mut window, pos2(size.right() + 30.0, size.center().y));
    window.until(|_| !commands(&dir, "set_size").is_empty() && !commands(&dir, "finish_size").is_empty());
    let at = text_at(&window, "Pen pressure").center();
    click(&mut window, at);
    window.until(|_| !commands(&dir, "toggle_pressure").is_empty());

    // The selection tools' row: a segment, a typed number, and a button.
    use_tool(&mut window, "marquee", |tool| *tool == RasterTool::Marquee);
    let at = text_at(&window, "Add").center();
    click(&mut window, at);
    window.until(|_| !commands(&dir, "set_mode").is_empty());
    assert_eq!(commands(&dir, "set_mode")[0]["arg"]["value"], json!(1));
    let at = text_at(&window, "0 px").center();
    click(&mut window, at);
    window.step();
    window.events.push(egui::Event::Text(String::from("4")));
    window.step();
    window.events.push(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    window.step();
    window.until(|_| !commands(&dir, "set_feather").is_empty());
    assert_eq!(commands(&dir, "set_feather")[0]["arg"]["value"].as_f64(), Some(4.0));
    let at = text_at(&window, "Deselect").center();
    click(&mut window, at);
    window.until(|_| !commands(&dir, "deselect").is_empty());

    // The crop row: a dropdown, a strong label, and Cancel and Apply at the
    // right, beside a button that waits.
    use_tool(&mut window, "crop", |tool| *tool == RasterTool::Crop);
    assert!(window.texts.iter().any(|(text, _)| text == "3200 × 1800"));
    let apply = text_at(&window, "Apply");
    assert!(apply.right() < window.app.raster.shown.right(), "the row's right group is not above the contents");
    let at = text_at(&window, "Free").center();
    click(&mut window, at);
    settle(&mut window);
    let at = text_at(&window, "16:9").center();
    click(&mut window, at);
    window.until(|_| !commands(&dir, "set_ratio").is_empty());
    assert_eq!(commands(&dir, "set_ratio")[0]["arg"]["value"], json!(2));
    let at = text_at(&window, "Later").center();
    click(&mut window, at);
    click(&mut window, apply.center());
    window.until(|_| !commands(&dir, "apply_crop").is_empty());
    assert!(commands(&dir, "later").is_empty(), "a button that waits was sent");

    // The panel: page tabs, the lock icons, the heading's buttons, and the
    // layer's image.
    let at = text_at(&window, "Properties").center();
    click(&mut window, at);
    let at = text_at(&window, "Adjustments").center();
    click(&mut window, at);
    window.until(|_| !commands(&dir, "page_adjustments").is_empty());
    assert!(commands(&dir, "page_properties").is_empty(), "a page that waits was sent");
    let lock = text_at(&window, "Lock:");
    let s = crate::widgets::Scale::PANEL;
    let third = pos2(lock.right() + s.of(14.0) + 2.0 * s.of(40.0) + s.of(17.0), lock.center().y);
    click(&mut window, third);
    window.until(|_| !of_type(&dir, "property").is_empty());
    let property = &of_type(&dir, "property")[0];
    assert_eq!((property["prop"].clone(), property["value"].clone()), (json!("lock"), json!({"index": 2})));
    let heading = window
        .texts
        .iter()
        .filter(|(text, _)| text == "Layers")
        .map(|(_, rect)| *rect)
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap();
    let panel = Rect::from_min_max(pos2(window.app.raster.shown.right(), 0.0), pos2(1280.0, 800.0));
    let right = panel.right() - 16.0;
    click(&mut window, pos2(right - 17.0, heading.center().y));
    click(&mut window, pos2(right - 3.0 * 34.0 + 17.0, heading.center().y));
    window.until(|_| !commands(&dir, "delete_layer").is_empty() && !commands(&dir, "add_layer").is_empty());
    assert!(commands(&dir, "group_layers").is_empty());
    let thumb = format!("raster:{}", dir.join("thumb.rgba").display());
    let drawn = textured(&window, &thumb);
    assert!(drawn.iter().any(|at| panel.contains_rect(*at) && at.width() > 20.0), "the layer's image was not drawn: {drawn:?}");
    close(window, dir);
}

/// A drag in the window from one screen point to another, through a few
/// points between, so it is a drag and not a click.
fn drag_on_screen(window: &mut Window, from: Pos2, to: Pos2) {
    window.events.push(egui::Event::PointerMoved(from));
    window.step();
    press(window, from, true);
    window.step();
    for step in 1..=6 {
        window.events.push(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 6.0)));
        window.step();
    }
    window.step();
    press(window, to, false);
    window.step();
}

/// Catches a Layers list that cannot put a layer into a group or take one
/// out: a row dropped on the middle of a group's row is sent to the top of
/// that group, a group's layer dropped between two top-level rows is sent
/// to the top level, between them, and one dropped below the list to the
/// bottom of the top level.
#[test]
fn layers_are_dragged_into_and_out_of_groups() {
    let (mut window, dir, _stand_in) = easel_with("nest", |dir| {
        vec![
            layer_row(dir, 2, None, 0, "Top layer", false),
            layer_row(dir, 10, None, 0, "Folder", true),
            layer_row(dir, 11, Some(10), 1, "Inner layer", false),
            layer_row(dir, 1, None, 0, "Background", false),
        ]
    });
    let folder = text_at(&window, "Folder").center();
    let from = text_at(&window, "Background").center();
    drag_on_screen(&mut window, from, folder);
    window.until(|_| !commands(&dir, "reorder").is_empty());
    assert_eq!(commands(&dir, "reorder")[0]["arg"], json!({ "node": 1, "value": 0, "parent": 10 }), "the drop on the group's middle did not put the layer in it");

    // Just under the top of the group's row is between it and the row above.
    let from = text_at(&window, "Inner layer").center();
    drag_on_screen(&mut window, from, pos2(folder.x, folder.y - 52.0 / 2.0 + 4.0));
    window.until(|_| commands(&dir, "reorder").len() > 1);
    assert_eq!(commands(&dir, "reorder")[1]["arg"], json!({ "node": 11, "value": 1, "parent": null }), "the drop between top-level rows did not take the layer out of its group");

    // Below the last row is the bottom of the top level.
    let below = text_at(&window, "Background").center() + egui::vec2(0.0, 52.0 / 2.0 + 6.0);
    let from = text_at(&window, "Inner layer").center();
    drag_on_screen(&mut window, from, below);
    window.until(|_| commands(&dir, "reorder").len() > 2);
    assert_eq!(commands(&dir, "reorder")[2]["arg"], json!({ "node": 11, "value": 3, "parent": null }), "the drop below the list did not reach the bottom");
    close(window, dir);
}

// The filmstrip.

/// Where each of the film's thumbnails was drawn, in order.
fn thumbnails(window: &Window, dir: &Path) -> Vec<Rect> {
    (0..IMAGES.len())
        .map(|index| {
            let name = format!("raster:{}", dir.join(format!("film{index}.rgba")).display());
            textured(window, &name).into_iter().next_back().unwrap_or_else(|| panic!("thumbnail {index} was not drawn"))
        })
        .collect()
}

/// Rectangles drawn with an edge of this colour.
fn edged(window: &Window, color: Color32) -> Vec<Rect> {
    window
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shown) if shown.stroke.color == color && shown.stroke.width > 0.0 => Some(shown.rect),
            _ => None,
        })
        .collect()
}

/// The ids of the commands sent, in order.
fn command_ids(dir: &Path) -> Vec<String> {
    of_type(dir, "command").iter().filter_map(|line| line["id"].as_str().map(String::from)).collect()
}

fn press_with(window: &mut Window, at: Pos2, button: egui::PointerButton, pressed: bool) {
    window.events.push(egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() });
}

const BLUE: Color32 = Color32::from_rgb(37, 99, 235);

/// Catches a filmstrip that is not drawn, is drawn over the image, or
/// does not show which image is shown: a white strip under a hairline
/// along the bottom of the working area, each thumbnail its own image 44
/// points tall at its proportions, in order, the shown one outlined in blue
/// and the others not, and a bordered + tile after them. The image's area
/// ends where the strip begins, the host's first camera and Fit frame the
/// image there, and commands report that area as their viewport.
#[test]
fn the_filmstrip_is_drawn_under_the_image_with_the_shown_one_outlined() {
    let (mut window, dir, _stand_in) = filmed("film-drawn");
    let area = window.app.raster.shown;
    let strip = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shown) if shown.fill == Color32::WHITE && (shown.rect.top() - area.bottom()).abs() < 0.01 && (shown.rect.width() - area.width()).abs() < 0.01 => Some(shown.rect),
            _ => None,
        })
        .expect("no white strip under the image's area");
    assert!((strip.height() - 68.0).abs() < 0.01, "the strip is {} points tall", strip.height());
    let hairline = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::LineSegment { points, stroke } if stroke.color == crate::widgets::BORDER && (points[0].y - strip.top()).abs() < 1.0 && (points[1].x - points[0].x).abs() >= strip.width() - 0.5));
    assert!(hairline, "the strip has no hairline above it");

    let thumbs = thumbnails(&window, &dir);
    for (index, thumb) in thumbs.iter().enumerate() {
        let (_, width, height) = IMAGES[index];
        assert!(strip.contains_rect(*thumb), "thumbnail {index} is off the strip: {thumb:?}");
        assert!((thumb.height() - 44.0).abs() < 0.5, "thumbnail {index} is {} tall", thumb.height());
        let proportions = (thumb.width() / thumb.height(), width as f32 / height as f32);
        assert!((proportions.0 - proportions.1).abs() < 0.05, "thumbnail {index} is not at its image's proportions: {proportions:?}");
        if index > 0 {
            assert!(thumb.left() > thumbs[index - 1].right(), "the thumbnails are out of order");
        }
    }
    let blue = edged(&window, BLUE);
    let outlined = |thumb: &Rect| blue.iter().any(|edge| edge.contains_rect(*thumb) && edge.width() - thumb.width() <= 6.5);
    assert!(outlined(&thumbs[1]), "the shown image is not outlined in blue");
    assert!(!outlined(&thumbs[0]) && !outlined(&thumbs[2]), "an image not shown is outlined in blue");
    let add = edged(&window, crate::widgets::BORDER)
        .into_iter()
        .find(|tile| strip.contains_rect(*tile) && tile.left() > thumbs[2].right() && (tile.height() - 44.0).abs() < 0.5)
        .expect("no bordered + tile after the thumbnails");
    assert!(add.left() - thumbs[2].right() < 20.0, "the + tile is not at the end of the thumbnails");

    // The first camera and Fit frame the image above the strip.
    let name = format!("raster:{}", dir.join("image.rgba").display());
    let fitted = |window: &Window| {
        let image = textured(window, &name).into_iter().next_back().expect("the image was not drawn");
        let rect = window.app.raster.shown;
        assert!(image.bottom() <= strip.top() - 39.0, "the image reaches the strip: {image:?}, strip at {}", strip.top());
        assert!((image.center() - rect.center()).length() < 0.5, "the image is not centered above the strip");
        let zoom = ((f64::from(rect.width()) - 80.0) / f64::from(W)).min((f64::from(rect.height()) - 80.0) / f64::from(H));
        assert!(near(raster(window).camera.unwrap().zoom, zoom), "the camera was not fitted to the area above the strip");
    };
    fitted(&window);
    window.app.run_action(&Action::with_arg("canvas.zoom", json!({"zoom": 1})));
    window.step();
    window.app.run_action(&Action::new("canvas.fit"));
    window.step();
    fitted(&window);
    window.app.run_action(&Action::new("deselect"));
    window.until(|_| !commands(&dir, "deselect").is_empty());
    let viewport = &commands(&dir, "deselect")[0]["viewport"];
    assert_eq!(viewport, &json!([area.width(), area.height()]), "a command's viewport is not the area above the strip");
    close(window, dir);
}

/// Catches a filmstrip that draws and does nothing, or sends the wrong
/// thing: a click on a thumbnail sends `focus` and then `select` with its
/// index, and paints nothing on the image; a right-click opens that
/// thumbnail's rows above it, on the menus' card, and a row sends its own
/// action; the + tile opens its rows above it; a thumbnail dragged along
/// the strip shows where it will land and sends `reorder` with `to` counted
/// after it is taken out; a drag that ends where it began sends nothing;
/// and resting on a thumbnail shows its name.
#[test]
fn the_filmstrip_sends_focus_select_its_menus_and_reorder() {
    let (mut window, dir, _stand_in) = filmed("film-actions");
    let thumbs = thumbnails(&window, &dir);

    // A click: focus, then select.
    click(&mut window, thumbs[2].center());
    window.until(|_| !commands(&dir, "show_item").is_empty());
    let ids = command_ids(&dir);
    let focus = ids.iter().position(|id| id == "strip_focus").expect("the press sent no focus");
    let select = ids.iter().position(|id| id == "show_item").unwrap();
    assert!(focus < select, "focus was not sent before select: {ids:?}");
    assert_eq!(commands(&dir, "show_item")[0]["arg"], json!({ "value": 2 }));
    assert_eq!(commands(&dir, "show_item")[0]["tab"], json!("r1"));
    assert!(gestures(&dir).is_empty(), "a click on the strip painted the image");

    // Resting on a thumbnail shows its name. A pointer moved just after a
    // click shows none, so the move waits a moment as a person's would.
    settle(&mut window);
    window.events.push(egui::Event::PointerMoved(thumbs[0].center()));
    for _ in 0..50 {
        window.step();
    }
    assert!(window.texts.iter().any(|(text, _)| text == "Harbor"), "resting on a thumbnail does not show its name");

    // A right-click opens the thumbnail's rows above it.
    let at = thumbs[0].center();
    window.events.push(egui::Event::PointerMoved(at));
    window.step();
    press_with(&mut window, at, egui::PointerButton::Secondary, true);
    window.step();
    press_with(&mut window, at, egui::PointerButton::Secondary, false);
    window.step();
    settle(&mut window);
    let rename = text_at(&window, "Rename");
    assert!(text_at(&window, "Duplicate").bottom() < thumbs[0].top() && text_at(&window, "Delete").bottom() < thumbs[0].top(), "the thumbnail's rows are not above it");
    assert!((text_at(&window, "Duplicate").left() - thumbs[0].left()).abs() < 40.0, "the thumbnail's rows are not over it");
    assert_eq!(command_ids(&dir).iter().filter(|id| *id == "strip_focus").count(), 2, "a right-click did not send focus");
    click(&mut window, rename.center());
    window.until(|_| !commands(&dir, "rename_item").is_empty());
    assert_eq!(commands(&dir, "rename_item")[0]["arg"], json!({ "item": 0 }));
    assert!(commands(&dir, "copy_item").is_empty() && commands(&dir, "remove_item").is_empty());
    assert_eq!(commands(&dir, "show_item").len(), 1, "the right-click or its row selected an image");

    // The + tile opens its rows above it.
    let add = edged(&window, crate::widgets::BORDER)
        .into_iter()
        .find(|tile| tile.left() > thumbs[2].right() && (tile.height() - 44.0).abs() < 0.5)
        .expect("no + tile");
    click(&mut window, add.center());
    settle(&mut window);
    let new = text_at(&window, "New image");
    let open = text_at(&window, "Open PNG or JPEG…");
    assert!(new.bottom() < add.top() && open.bottom() < add.top(), "the + tile's rows are not above it");
    click(&mut window, open.center());
    window.until(|_| !commands(&dir, "open_item").is_empty());
    assert!(commands(&dir, "new_item").is_empty());

    // A drag past the last thumbnail lands last; one before the first lands
    // first; each shows a blue line where it will land while it is held.
    let line = |window: &Window| segments(window, BLUE).into_iter().filter(|[a, b]| (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() > 40.0).map(|[a, _]| a.x).next();
    window.events.push(egui::Event::PointerMoved(thumbs[0].center()));
    window.step();
    press(&mut window, thumbs[0].center(), true);
    window.step();
    for step in 1..=8 {
        let to = thumbs[2].right() - 4.0;
        let x = thumbs[0].center().x + (to - thumbs[0].center().x) * step as f32 / 8.0;
        window.events.push(egui::Event::PointerMoved(pos2(x, thumbs[0].center().y)));
        window.step();
    }
    let shown_at = line(&window);
    press(&mut window, pos2(thumbs[2].right() - 4.0, thumbs[0].center().y), false);
    window.step();
    let landed = shown_at.expect("the drag showed no line where it would land");
    assert!(landed > thumbs[2].right() && landed < add.left(), "the line is not after the last thumbnail: {landed}");
    window.until(|_| !commands(&dir, "order_items").is_empty());
    assert_eq!(commands(&dir, "order_items")[0]["arg"], json!({ "strip": true, "from": 0, "to": 2 }));
    drag_on_screen(&mut window, thumbs[2].center(), pos2(thumbs[0].left() + 4.0, thumbs[0].center().y));
    window.until(|_| commands(&dir, "order_items").len() > 1);
    assert_eq!(commands(&dir, "order_items")[1]["arg"], json!({ "strip": true, "from": 2, "to": 0 }));
    drag_on_screen(&mut window, thumbs[1].center(), thumbs[1].center() + egui::vec2(8.0, 0.0));
    for _ in 0..10 {
        window.step();
    }
    assert_eq!(commands(&dir, "order_items").len(), 2, "a drag that ended where it began was sent");
    assert_eq!(commands(&dir, "show_item").len(), 1, "a drag selected an image");
    assert!(gestures(&dir).is_empty(), "the strip's pointer reached the image");
    close(window, dir);
}

// Retouching, text, and adjustments.

/// The dark of the cross that shows where a tool copies from.
const CROSS_DARK: Color32 = Color32::from_black_alpha(200);

fn hold(window: &mut Window, modifiers: egui::Modifiers) {
    window.events.push(egui::Event::ModifiersChanged(modifiers));
}

/// A key pressed and let go, with these modifiers held.
fn key(window: &mut Window, key: egui::Key, modifiers: egui::Modifiers) {
    hold(window, modifiers);
    window.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    window.step();
    window.events.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    hold(window, egui::Modifiers::NONE);
    window.step();
}

/// Every input the host sent the text typed in, in order.
fn text_inputs(dir: &Path) -> Vec<Value> {
    of_type(dir, "raster_text").into_iter().map(|line| line["input"].clone()).collect()
}

/// The stand-in's text tool, with a text typed in.
fn start_typing(window: &mut Window) {
    window.app.run_action(&Action::new("tool.typing"));
    window.until(|app| app.apps[0].running.declared.rasters["r1"].view.text.is_some());
    window.step();
}

/// The glyph an Iconoir icon is drawn with.
fn glyph(name: &str) -> String {
    let icon = iconflow::try_icon(iconflow::Pack::Iconoir, name, iconflow::Style::Regular, iconflow::Size::Regular).unwrap();
    char::from_u32(icon.codepoint).unwrap().to_string()
}

/// A drag through these screen points, held between each.
fn drag_path(window: &mut Window, points: &[Pos2]) {
    window.events.push(egui::Event::PointerMoved(points[0]));
    window.step();
    press(window, points[0], true);
    window.step();
    for at in &points[1..] {
        window.events.push(egui::Event::PointerMoved(*at));
        window.step();
    }
    press(window, *points.last().unwrap(), false);
    window.step();
}

/// Runs frames, as `until` does, until the window puts words on the system
/// clipboard, and returns them.
fn copied(window: &mut Window) -> Option<String> {
    use eframe::App as _;
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(20) {
        for started in &mut window.app.apps {
            started.running.poll();
        }
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        input.events.append(&mut window.events);
        let Window { ctx, app, frame, .. } = &mut *window;
        let output = ctx.run_ui(input, |ui| app.ui(ui, frame));
        for command in output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                return Some(text);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

/// Catches a stamp or healing brush that does not show where it copies
/// from, or that paints on Alt-click: the cross is drawn at the source
/// until an offset holds, and then at the pointer plus the offset; the
/// brush's ring is at the pointer; Alt-click sends `source` where it was
/// pressed and paints nothing; and a drag paints as the brush does.
#[test]
fn a_sampler_shows_its_source_and_alt_click_sets_it() {
    let (mut window, dir, _stand_in) = easel("sampler");
    use_tool(&mut window, "stamp", |tool| matches!(tool, RasterTool::Sampler { offset: None, .. }));
    let crossed = |window: &Window, at: Pos2| {
        let arms = segments(window, CROSS_DARK);
        arms.len() == 2 && arms.iter().all(|[a, b]| (*a + (*b - *a) / 2.0 - at).length() < 0.01)
    };
    let pointer = screen(&window, [5.0, 5.0]);
    window.events.push(egui::Event::PointerMoved(pointer));
    window.step();
    assert!(crossed(&window, screen(&window, SOURCE)), "the cross is not at the source: {:?}", segments(&window, CROSS_DARK));
    let ring = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::Circle(ring) if (ring.center - pointer).length() < 0.01 && (ring.radius - 20.0).abs() < 0.01));
    assert!(ring, "the brush's ring is not at the pointer");
    // Over the workspace's bar, which lies on the image's view, the
    // pointer is the bar's, so no ring is drawn.
    let bar = window.app.bar_rect.center();
    window.events.push(egui::Event::PointerMoved(bar));
    window.step();
    window.step();
    let ringed = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::Circle(ring) if (ring.center - bar).length() < 0.01));
    assert!(!ringed, "the brush's ring is drawn over the workspace's bar at {bar:?}");
    window.events.push(egui::Event::PointerMoved(pointer));
    window.step();

    let at = screen(&window, [8.0, 8.0]);
    hold(&mut window, egui::Modifiers::ALT);
    click(&mut window, at);
    hold(&mut window, egui::Modifiers::NONE);
    window.until(|_| !gestures(&dir).is_empty());
    let sent = all_gestures(&dir);
    assert_eq!(sent.len(), 1, "Alt-click sent more than its source: {sent:?}");
    assert_eq!((sent[0]["kind"].clone(), point(&sent[0]["at"])), (json!("source"), [8.0, 8.0]));

    use_tool(&mut window, "aligned", |tool| matches!(tool, RasterTool::Sampler { offset: Some(_), .. }));
    window.events.push(egui::Event::PointerMoved(pointer));
    window.step();
    assert!(crossed(&window, screen(&window, [5.0 + OFFSET[0], 5.0 + OFFSET[1]])), "the cross is not at the pointer plus the offset");
    drag(&mut window, &[[5.0, 5.0], [10.0, 6.0], [15.0, 8.0]], |_| {});
    window.until(|_| gestures(&dir).len() >= 2);
    assert_eq!(gestures(&dir)[1]["kind"], "stroke", "a drag did not paint");
    assert!(all_gestures(&dir).iter().any(|gesture| gesture["kind"] == "stroke_part"), "the drag was not painted as it was drawn");
    close(window, dir);
}

/// Catches a text tool that sends nothing or the wrong thing: a click
/// sends `click` where it was, and a drag sends `rect` from the press to
/// the release, its box drawn in blue while it is dragged.
#[test]
fn the_text_tool_sends_a_click_or_a_box() {
    let (mut window, dir, _stand_in) = easel("text-tool");
    use_tool(&mut window, "text", |tool| *tool == RasterTool::Text);
    let at = screen(&window, [5.0, 15.0]);
    click(&mut window, at);
    window.until(|_| !gestures(&dir).is_empty());
    let sent = gestures(&dir);
    assert_eq!((sent[0]["kind"].clone(), point(&sent[0]["at"])), (json!("click"), [5.0, 15.0]));
    let mut boxed = false;
    drag(&mut window, &[[2.0, 2.0], [5.0, 3.0], [8.0, 4.0]], |window| {
        let frame = Rect::from_two_pos(screen(window, [2.0, 2.0]), screen(window, [8.0, 4.0]));
        boxed = segments(window, BLUE).iter().any(|[a, b]| *a == frame.left_top() && *b == frame.right_top());
    });
    assert!(boxed, "the box being dragged was not drawn");
    window.until(|_| gestures(&dir).len() >= 2);
    let rect = &gestures(&dir)[1];
    assert_eq!((rect["kind"].clone(), point(&rect["from"]), point(&rect["to"])), (json!("rect"), [2.0, 2.0], [8.0, 4.0]));
    assert!(text_inputs(&dir).is_empty(), "the text tool sent text with no text typed in");
    close(window, dir);
}

/// Catches typing that disappoints: while a text is typed in, its box is
/// drawn in blue with six handles, its selection and its blinking caret;
/// a press, a drag, and a double-click in the box send `place`, `select`,
/// and `word`; a press outside sends `click`; a side's drag sends the new
/// box; typed words, Space, keys, Select All, and Copy go to the text, and
/// Copy's answer reaches the system clipboard; a tool's letter types
/// rather than chooses the tool, Space types rather than pans, and Ctrl+Z
/// and Ctrl+Shift+Z still undo and redo.
#[test]
fn a_text_typed_in_takes_the_keyboard_and_the_pointer_in_its_box() {
    let (mut window, dir, _stand_in) = easel("typing");
    start_typing(&mut window);
    let edit = typed();
    let frame = Rect::from_two_pos(screen(&window, [10.0, 5.0]), screen(&window, [30.0, 11.0]));
    let edges = segments(&window, BLUE);
    assert!(edges.iter().any(|[a, b]| *a == frame.left_top() && *b == frame.right_top()), "the text's box was not drawn");
    let handles: Vec<Pos2> = edged(&window, BLUE).into_iter().filter(|handle| handle.width() < 10.0).map(|handle| handle.center()).collect();
    // Only working handles: the corners and the middles of the sides that
    // change the width.
    let corners = [frame.left_top(), frame.left_center(), frame.left_bottom(), frame.right_top(), frame.right_center(), frame.right_bottom()];
    assert_eq!(handles.len(), 6, "the box does not have its six handles: {handles:?}");
    assert!(corners.iter().all(|corner| handles.iter().any(|handle| (*handle - *corner).length() < 0.01)), "a handle is off the box");
    let span = Rect::from_two_pos(screen(&window, [11.0, 5.0]), screen(&window, [14.0, 11.0]));
    let selected = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::Rect(shown) if shown.rect == span && shown.fill.a() > 0 && shown.fill.a() < 255));
    assert!(selected, "the selection was not drawn");

    // A press in the box places the caret there, and the caret shows at
    // once, then blinks.
    let inside = screen(&window, [20.0, 8.0]);
    click(&mut window, inside);
    window.until(|_| !text_inputs(&dir).is_empty());
    assert_eq!(text_inputs(&dir)[0], json!({"kind": "place", "at": [20.0, 8.0], "extend": false}));
    let caret = screen(&window, [edit.caret[0] + edit.caret[2] / 2.0, edit.caret[1] + edit.caret[3] / 2.0]);
    let caret_shown = |window: &Window| edged(window, Color32::from_black_alpha(90)).iter().any(|shown| (shown.center() - caret).length() < 0.5);
    assert!(caret_shown(&window), "the caret was not drawn after a press");
    for _ in 0..40 {
        window.step();
    }
    assert!(!caret_shown(&window), "the caret does not blink");

    // A drag in the box selects; a double-click takes a word.
    drag(&mut window, &[[12.0, 7.0], [14.0, 7.0], [16.0, 7.0]], |_| {});
    window.until(|_| text_inputs(&dir).len() >= 4);
    assert_eq!(text_inputs(&dir)[1..4], [
        json!({"kind": "place", "at": [12.0, 7.0], "extend": false}),
        json!({"kind": "select", "at": [14.0, 7.0]}),
        json!({"kind": "select", "at": [16.0, 7.0]}),
    ]);
    click(&mut window, inside);
    click(&mut window, inside);
    window.until(|_| text_inputs(&dir).iter().any(|input| input["kind"] == "word"));
    assert!(text_inputs(&dir).contains(&json!({"kind": "word", "at": [20.0, 8.0]})));
    assert!(gestures(&dir).is_empty(), "a press in the box made a new box: {:?}", gestures(&dir));

    // A press outside the box is the text tool's click; a side's drag is
    // the box's new width.
    let outside = screen(&window, [35.0, 15.0]);
    click(&mut window, outside);
    window.until(|_| !gestures(&dir).is_empty());
    assert_eq!((gestures(&dir)[0]["kind"].clone(), point(&gestures(&dir)[0]["at"])), (json!("click"), [35.0, 15.0]));
    drag(&mut window, &[[30.0, 8.0], [32.0, 8.0], [34.0, 8.0]], |_| {});
    window.until(|_| gestures(&dir).len() >= 2);
    assert_eq!(gestures(&dir)[1], json!({"kind": "text_box", "rect": [10.0, 5.0, 24.0, 6.0]}));

    // The keyboard.
    let typed_before = text_inputs(&dir).len();
    window.events.push(egui::Event::Text(String::from("Hi")));
    window.step();
    window.events.push(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    window.events.push(egui::Event::Text(String::from(" ")));
    window.step();
    // With Space held, a press in the box is still the text's.
    let camera = raster(&window).camera.unwrap();
    press(&mut window, inside, true);
    window.step();
    press(&mut window, inside, false);
    window.events.push(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() });
    window.step();
    let after = raster(&window).camera.unwrap();
    assert!(after.origin == camera.origin && after.zoom == camera.zoom, "Space panned while typing");
    key(&mut window, egui::Key::Backspace, egui::Modifiers::NONE);
    key(&mut window, egui::Key::ArrowLeft, egui::Modifiers::SHIFT);
    key(&mut window, egui::Key::A, egui::Modifiers::COMMAND);
    window.events.push(egui::Event::Key { key: egui::Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
    window.events.push(egui::Event::Text(String::from("b")));
    window.step();
    window.events.push(egui::Event::Key { key: egui::Key::B, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() });
    window.step();
    window.until(|_| text_inputs(&dir).len() >= typed_before + 7);
    assert_eq!(text_inputs(&dir)[typed_before..typed_before + 7], [
        json!({"kind": "insert", "text": "Hi"}),
        json!({"kind": "insert", "text": " "}),
        json!({"kind": "place", "at": [20.0, 8.0], "extend": false}),
        json!({"kind": "key", "key": "backspace", "shift": false, "word": false}),
        json!({"kind": "key", "key": "left", "shift": true, "word": false}),
        json!({"kind": "key", "key": "select_all", "shift": false, "word": false}),
        json!({"kind": "insert", "text": "b"}),
    ]);
    assert!(commands(&dir, "tool.brush").is_empty(), "a tool's letter chose the tool while typing");

    // Undo and redo still run, and are not typed.
    let typed_now = text_inputs(&dir).len();
    key(&mut window, egui::Key::Z, egui::Modifiers::COMMAND);
    key(&mut window, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    window.until(|_| !commands(&dir, "undo").is_empty() && !commands(&dir, "redo").is_empty());
    assert_eq!(text_inputs(&dir).len(), typed_now, "undo or redo was typed");

    // Copy goes to the text, and its answer to the system clipboard.
    window.events.push(egui::Event::Copy);
    assert_eq!(copied(&mut window).as_deref(), Some("Hi"), "Copy's answer did not reach the clipboard");
    assert!(text_inputs(&dir).contains(&json!({"kind": "key", "key": "copy", "shift": false, "word": false})));
    close(window, dir);
}

/// Catches a move of selected pixels that the image cannot follow: the
/// move sends `drag_part` from the press to the pointer each time it moves,
/// in order, and then the finished `drag`.
#[test]
fn a_move_sends_its_parts_while_it_is_dragged() {
    let (mut window, dir, _stand_in) = easel("move-parts");
    use_tool(&mut window, "move", |tool| *tool == RasterTool::Move);
    drag(&mut window, &[[20.0, 10.0], [22.0, 11.0], [24.0, 12.0]], |_| {});
    window.until(|_| !gestures(&dir).is_empty());
    assert_eq!(all_gestures(&dir), vec![
        json!({"kind": "drag_part", "from": [20.0, 10.0], "to": [22.0, 11.0]}),
        json!({"kind": "drag_part", "from": [20.0, 10.0], "to": [24.0, 12.0]}),
        json!({"kind": "drag", "from": [20.0, 10.0], "to": [24.0, 12.0]}),
    ]);
    close(window, dir);
}

/// Catches panel rows that are not drawn or do nothing, as the Properties
/// and Adjustments views show them: a section's icon before its title; a
/// centred number's slider filled from its middle; a bordered button that
/// sends its action; a note wrapped to the panel; and an item's tile,
/// label, and muted detail, which sends its action as a command.
#[test]
fn the_panel_draws_items_buttons_notes_and_centred_numbers() {
    let (mut window, dir, _stand_in) = easel("panel-rows");
    window.app.run_action(&Action::new("tool.grade"));
    window.until(|app| app.apps[0].running.declared.properties.as_ref().is_some_and(|properties| properties.layers.is_none()));
    settle(&mut window);
    let s = crate::widgets::Scale::PANEL;

    let title = text_at(&window, "Color Grade");
    let icon = glyph("filter-list");
    let before = window.texts.iter().any(|(text, at)| *text == icon && at.right() < title.left() && (at.center().y - title.center().y).abs() < 3.0);
    assert!(before, "the section's icon is not before its title");

    let label = text_at(&window, "Contrast");
    let knob = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(dot) if dot.fill == crate::widgets::INK && (dot.center.y - label.center().y).abs() < 3.0 => Some(dot.center),
            _ => None,
        })
        .expect("the centred number has no knob");
    let fill = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shown) if shown.fill == crate::widgets::INK && (shown.rect.center().y - knob.y).abs() < 2.0 && (shown.rect.right() - knob.x).abs() < 0.5 => Some(shown.rect),
            _ => None,
        })
        .expect("the centred number's slider is not filled to its knob");
    let track = label.left() + s.of(92.0) + s.of(7.0);
    assert!(fill.left() - track > 20.0, "the centred slider fills from its left end, not its middle: {fill:?} on a track from {track}");

    let reset = text_at(&window, "Reset");
    assert!(edged(&window, crate::widgets::BORDER).iter().any(|button| button.contains_rect(reset) && button.height() < 40.0), "Reset has no border");
    click(&mut window, reset.center());
    window.until(|_| !commands(&dir, "reset_grade").is_empty());

    let note = text_at(&window, NOTE);
    let panel = Rect::from_min_max(pos2(window.app.raster.shown.right(), 0.0), pos2(1280.0, 800.0));
    assert!(note.height() > 30.0 && panel.contains_rect(note), "the note is not wrapped to the panel: {note:?}");

    let name = text_at(&window, "Curves");
    let detail = text_at(&window, "Tone and colour by curve");
    assert!(detail.top() > name.bottom() - 1.0 && (detail.left() - name.left()).abs() < 1.0, "the item's detail is not under its label");
    assert!(edged(&window, crate::widgets::BORDER).iter().any(|tile| tile.right() < name.left() && (tile.center().y - (name.top() + detail.bottom()) / 2.0).abs() < 4.0 && (tile.width() - tile.height()).abs() < 0.5), "the item has no icon tile");
    assert!(window.texts.iter().any(|(text, at)| *text == glyph("plus") && at.left() > detail.right() && panel.contains_rect(*at)), "the item has no plus");
    click(&mut window, name.center());
    window.until(|_| !commands(&dir, "add_curves").is_empty());
    close(window, dir);
}

/// Catches a Curves plot that is drawn as the pressure curve's or keeps its
/// limits: the square plot is as tall as it is wide, with its histogram's
/// bars behind and a faint diagonal; its end point drags down below the
/// point before it, so the curve can fall, and its first end point drags
/// inward and to the top.
#[test]
fn a_free_curve_falls_as_well_as_rises() {
    let (mut window, dir, _stand_in) = easel("free-curve");
    window.app.run_action(&Action::new("tool.curves"));
    window.until(|app| app.apps[0].running.declared.properties.as_ref().is_some_and(|properties| properties.sections.first().is_some_and(|section| section.title == "Curves")));
    settle(&mut window);
    let panel = Rect::from_min_max(pos2(window.app.raster.shown.right(), 0.0), pos2(1280.0, 800.0));
    let plot = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shown) if shown.fill == Color32::WHITE && shown.stroke.color == crate::widgets::BORDER && panel.contains_rect(shown.rect) && shown.rect.width() > 150.0 => Some(shown.rect),
            _ => None,
        })
        .expect("no plot");
    assert!((plot.width() - plot.height()).abs() < 0.5, "the plot is not square: {plot:?}");
    let bars = window.shapes.iter().filter(|clipped| matches!(&clipped.shape, egui::Shape::Rect(shown) if shown.fill == Color32::from_rgb(226, 228, 232) && plot.contains_rect(shown.rect))).count();
    assert_eq!(bars, 24, "the histogram's bars are not drawn");
    let diagonal = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::LineSegment { points, .. } if points[0] == plot.left_bottom() && points[1] == plot.right_top()));
    assert!(diagonal, "the plot has no diagonal");

    let curve = |dir: &Path| -> Vec<(f64, f64)> {
        let property = of_type(dir, "property").into_iter().filter(|line| line["prop"] == "curve").next_back().expect("the curve sent nothing");
        property["value"]["curve"].as_array().unwrap().iter().map(|point| (point[0].as_f64().unwrap(), point[1].as_f64().unwrap())).collect()
    };
    let end = plot.right_top();
    drag_path(&mut window, &[end, end + egui::vec2(0.0, 4.0), end + egui::vec2(0.0, 8.0), end + egui::vec2(0.0, 100.0), plot.right_bottom() + egui::vec2(0.0, 10.0)]);
    window.until(|_| !commands(&dir, "finish_edit").is_empty());
    let fell = curve(&dir);
    assert_eq!((fell.len(), fell[1], fell[2]), (3, (0.5, 0.5), (1.0, 0.0)), "the end point did not fall below the point before it: {fell:?}");

    let start = plot.left_bottom();
    let inward = pos2(plot.left() + plot.width() * 0.2, plot.top() - 10.0);
    drag_path(&mut window, &[start, start + egui::vec2(0.0, -4.0), start + egui::vec2(0.0, -8.0), start + (inward - start) / 2.0, inward]);
    window.until(|_| commands(&dir, "finish_edit").len() >= 2);
    let moved = curve(&dir);
    assert!((moved[0].0 - 0.2).abs() < 0.01 && moved[0].1 == 1.0, "the first end point did not move inward and up: {moved:?}");

    // A drag that starts on the plot away from every point adds one there.
    let at = pos2(plot.left() + plot.width() * 0.3, plot.bottom() - plot.height() * 0.3);
    drag_path(&mut window, &[at, at + egui::vec2(0.0, -4.0), at + egui::vec2(0.0, -8.0), at + egui::vec2(0.0, -30.0)]);
    window.until(|_| commands(&dir, "finish_edit").len() >= 3);
    let added = of_type(&dir, "property").into_iter().filter(|line| line["prop"] == "curve").any(|line| {
        line["value"]["curve"].as_array().unwrap().iter().any(|point| (point[0].as_f64().unwrap() - 0.3).abs() < 0.02)
    });
    assert!(added, "a drag on the plot added no point where it started");
    close(window, dir);
}

/// Catches layer rows that hide what kind they are: a mask's row draws its
/// mark muted at the row's right, and a text's row, with no image, draws
/// its icon in the image's frame.
#[test]
fn a_layer_row_draws_its_mark_and_its_icon_in_the_frame() {
    let (window, dir, _stand_in) = easel_with("marks", |dir| {
        let mut text = layer_row(dir, 3, None, 0, "Golden hour", false);
        text.icon = String::from("text");
        text.thumbnail = None;
        text.selected = false;
        let mut mask = layer_row(dir, 2, None, 0, "Subject Mask", false);
        mask.mark = Some(String::from("link"));
        mask.selected = false;
        vec![text, mask, layer_row(dir, 1, None, 0, "Background", false)]
    });
    let panel = Rect::from_min_max(pos2(window.app.raster.shown.right(), 0.0), pos2(1280.0, 800.0));
    let name = text_at(&window, "Subject Mask");
    let mark = text_at(&window, &glyph("link"));
    assert!((mark.center().y - name.center().y).abs() < 2.0 && mark.left() > name.right() && panel.right() - mark.right() < 30.0, "the mask's mark is not at its row's right: {mark:?}");
    let text = text_at(&window, "Golden hour");
    let icon = text_at(&window, &glyph("text"));
    let frame = edged(&window, crate::widgets::BORDER).into_iter().find(|frame| frame.contains_rect(icon) && frame.right() < text.left()).expect("the text's icon is not in an image's frame");
    assert!((frame.center().y - text.center().y).abs() < 2.0, "the frame is not on the text's row");
    close(window, dir);
}

/// What the Image Generator's settings row said in the owner's window, where
/// the brush cannot paint, at full length.
const REFUSAL: &str = "Color Grade is an adjustment, with no pixels to paint. Add a mask to paint where it applies, or choose a pixel layer to paint on.";

/// Catches a settings row that widens the window's layout, pushing the
/// contents and the panel past the window's right edge, as the owner saw
/// with the conversation open: with a label longer than the row, the
/// contents keep their width, the label stops with an ellipsis inside the
/// window, and the panel keeps its own width with its pages and fields
/// inside the window. A row of more controls than fit does the same.
#[test]
fn a_long_settings_row_label_leaves_the_panel_inside_the_window() {
    let (mut window, dir, _stand_in) = easel("long-label");
    let contents = window.app.raster.shown;
    assert!((1280.0 - contents.right() - crate::panel::WIDTH).abs() < 0.5, "the panel is not at its own width before the label: {contents:?}");
    let mut bar = toolbar("brush");
    bar.options.push(ToolItem::Separator);
    bar.options.push(ToolItem::Label { text: String::from(REFUSAL), strong: false });
    std::fs::write(dir.join("refusal.jsonl"), line(AppMessage::Toolbar(bar))).unwrap();
    window.app.run_action(&Action::new("tool.refusal"));
    window.until(|app| app.apps[0].running.declared.toolbar.as_ref().is_some_and(|bar| bar.options.len() == 6));
    settle(&mut window);

    assert_eq!(window.app.raster.shown, contents, "the label widened the contents");
    let label = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == REFUSAL => Some(text),
            _ => None,
        })
        .expect("the label was not drawn");
    let last = label.galley.rows.last().and_then(|row| row.glyphs.last()).map(|glyph| glyph.chr);
    assert_eq!((label.galley.rows.len(), last), (1, Some('…')), "the label does not stop with an ellipsis on its one line");
    assert!(label.visual_bounding_rect().right() <= 1280.0, "the label runs past the window: {:?}", label.visual_bounding_rect());
    let inside = |window: &Window| {
        for words in ["Layers", "Adjustments", "Normal", "Lock:"] {
            let at = text_at(window, words);
            assert!(at.left() >= contents.right() && at.right() <= 1280.0, "{words:?} is not inside the panel and the window: {at:?}");
        }
    };
    inside(&window);

    let mut crowded = toolbar("brush");
    for index in 0..6 {
        crowded.options.push(ToolItem::Slider {
            caption: format!("Slider {index}"),
            width: 190.0,
            value: 24,
            min: 1,
            max: 200,
            tint: None,
            set: Action::new(format!("set_{index}")),
            finish: None,
            suffix: String::new(),
        });
    }
    std::fs::write(dir.join("crowded.jsonl"), line(AppMessage::Toolbar(crowded))).unwrap();
    window.app.run_action(&Action::new("tool.crowded"));
    window.until(|app| app.apps[0].running.declared.toolbar.as_ref().is_some_and(|bar| bar.options.len() == 10));
    settle(&mut window);
    assert_eq!(window.app.raster.shown, contents, "a crowded row widened the contents");
    inside(&window);
    close(window, dir);
}

/// Presses at the first point, then holds the pointer still at each point
/// for some frames, and lets go at the last: a person holding a control
/// while the application's replies lag. The stand-in never answers a
/// change, so the value shown stays the one it declared.
fn hold_along(window: &mut Window, points: &[Pos2]) {
    window.events.push(egui::Event::PointerMoved(points[0]));
    window.step();
    press(window, points[0], true);
    for at in points {
        window.events.push(egui::Event::PointerMoved(*at));
        for _ in 0..6 {
            window.step();
        }
    }
    press(window, *points.last().unwrap(), false);
    window.step();
}

/// Each value sent that repeats the one sent just before it.
fn repeats(values: &[Value]) -> Vec<Value> {
    values.windows(2).filter(|pair| pair[0] == pair[1]).map(|pair| pair[1].clone()).collect()
}

/// Catches a held slider that sends again what it sent, because it
/// compares with the application's echo, which lags: the panel's slider
/// and the settings row's, each held still at three places, send one value
/// for each place at most, and never the same value twice in a row.
#[test]
fn a_held_slider_sends_no_repeated_values_while_replies_lag() {
    let (mut window, dir, _stand_in) = easel("held-slider");
    let size = text_at(&window, "Size");
    let start = pos2(size.right() + 50.0, size.center().y);
    hold_along(&mut window, &[start, start + egui::vec2(20.0, 0.0), start + egui::vec2(40.0, 0.0)]);
    window.until(|_| !commands(&dir, "finish_size").is_empty());
    let sizes: Vec<Value> = commands(&dir, "set_size").into_iter().map(|line| line["arg"]["value"].clone()).collect();
    assert!((2..=3).contains(&sizes.len()) && repeats(&sizes).is_empty(), "the settings row's slider sent {sizes:?}");

    window.app.run_action(&Action::new("tool.grade"));
    window.until(|app| app.apps[0].running.declared.properties.as_ref().is_some_and(|properties| properties.layers.is_none()));
    settle(&mut window);
    let label = text_at(&window, "Contrast");
    let knob = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(dot) if dot.fill == crate::widgets::INK && (dot.center.y - label.center().y).abs() < 3.0 => Some(dot.center),
            _ => None,
        })
        .expect("the slider has no knob");
    hold_along(&mut window, &[knob, knob + egui::vec2(15.0, 0.0), knob + egui::vec2(30.0, 0.0)]);
    window.until(|_| !commands(&dir, "finish_edit").is_empty());
    let contrast: Vec<Value> = of_type(&dir, "property").into_iter().filter(|line| line["prop"] == "contrast").map(|line| line["value"].clone()).collect();
    assert!((2..=3).contains(&contrast.len()) && repeats(&contrast).is_empty(), "the panel's slider sent {contrast:?}");
    // Let go while the stand-in has not answered: the knob stays where it
    // was let go rather than flicking back to the old value.
    for _ in 0..5 {
        window.step();
    }
    let after = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(dot) if dot.fill == crate::widgets::INK && (dot.center.y - label.center().y).abs() < 3.0 => Some(dot.center),
            _ => None,
        })
        .expect("the slider has no knob");
    assert!((after.x - (knob.x + 30.0)).abs() < 4.0, "the knob flicked back after it was let go: {after:?}, let go at {}", knob.x + 30.0);
    // The stand-in never takes the value, so after a moment, or at once when
    // the pointer is pressed elsewhere, the knob shows the application's.
    let contrast_knob = |window: &Window| {
        window
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Circle(dot) if dot.fill == crate::widgets::INK && (dot.center.y - label.center().y).abs() < 3.0 => Some(dot.center),
                _ => None,
            })
            .expect("the slider has no knob")
    };
    for _ in 0..40 {
        window.step();
    }
    let waited = contrast_knob(&window);
    assert!((waited.x - knob.x).abs() < 4.0, "the knob never showed the application's value: {waited:?}, the application's at {}", knob.x);
    hold_along(&mut window, &[knob, knob + egui::vec2(30.0, 0.0)]);
    window.events.push(egui::Event::PointerMoved(label.center()));
    window.step();
    press(&mut window, label.center(), true);
    window.step();
    press(&mut window, label.center(), false);
    window.step();
    let pressed = contrast_knob(&window);
    assert!((pressed.x - knob.x).abs() < 4.0, "a press elsewhere left the knob where it was let go: {pressed:?}, the application's at {}", knob.x);
    close(window, dir);
}

/// Catches a curve drag that moves the application's lagging echo rather
/// than the curve the drag has made: a point added by a drag keeps moving
/// with the pointer before the application sends it back, a held point
/// sends each change once, and the plot shows the point where the pointer
/// holds it.
#[test]
fn a_held_curve_point_moves_on_the_curve_the_drag_made() {
    let (mut window, dir, _stand_in) = easel("held-curve");
    window.app.run_action(&Action::new("tool.curves"));
    window.until(|app| app.apps[0].running.declared.properties.as_ref().is_some_and(|properties| properties.sections.first().is_some_and(|section| section.title == "Curves")));
    settle(&mut window);
    let panel = Rect::from_min_max(pos2(window.app.raster.shown.right(), 0.0), pos2(1280.0, 800.0));
    let plot = window
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shown) if shown.fill == Color32::WHITE && shown.stroke.color == crate::widgets::BORDER && panel.contains_rect(shown.rect) && shown.rect.width() > 150.0 => Some(shown.rect),
            _ => None,
        })
        .expect("no plot");
    let curves = |dir: &Path| -> Vec<Value> { of_type(dir, "property").into_iter().filter(|line| line["prop"] == "curve").map(|line| line["value"].clone()).collect() };

    // A point added away from the others, carried up while the echo lags.
    let at = pos2(plot.left() + plot.width() * 0.3, plot.bottom() - plot.height() * 0.3);
    let up = plot.height() * 0.2;
    window.events.push(egui::Event::PointerMoved(at));
    window.step();
    press(&mut window, at, true);
    for to in [at, at + egui::vec2(0.0, -8.0), at + egui::vec2(0.0, -up)] {
        window.events.push(egui::Event::PointerMoved(to));
        for _ in 0..6 {
            window.step();
        }
    }
    let held = plot.bottom() - plot.height() * 0.5;
    let shown = window.shapes.iter().any(|clipped| matches!(&clipped.shape, egui::Shape::Circle(dot) if dot.fill == crate::widgets::INK && (dot.center.x - at.x).abs() < 1.0 && (dot.center.y - held).abs() < 1.0));
    assert!(shown, "the plot does not show the held point where the pointer holds it");
    press(&mut window, at + egui::vec2(0.0, -up), false);
    window.step();
    window.until(|_| !commands(&dir, "finish_edit").is_empty());
    let sent = curves(&dir);
    assert!(repeats(&sent).is_empty(), "the curve sent a value twice in a row: {sent:?}");
    let last: Vec<(f64, f64)> = sent.last().expect("the curve sent nothing")["curve"].as_array().unwrap().iter().map(|point| (point[0].as_f64().unwrap(), point[1].as_f64().unwrap())).collect();
    assert!(last.len() == 4 && (last[1].0 - 0.3).abs() < 0.02 && (last[1].1 - 0.5).abs() < 0.02, "the added point did not follow the pointer: {last:?}");
    assert!(sent.len() <= 3, "the curve sent more than one value for each place it was held: {sent:?}");

    // The middle point, held still and moved.
    let middle = pos2(plot.center().x, plot.center().y);
    hold_along(&mut window, &[middle, middle + egui::vec2(0.0, -10.0), middle + egui::vec2(0.0, -20.0)]);
    window.until(|_| commands(&dir, "finish_edit").len() >= 2);
    let moved = curves(&dir)[sent.len()..].to_vec();
    assert!((1..=3).contains(&moved.len()) && repeats(&moved).is_empty(), "the held point sent {moved:?}");
    close(window, dir);
}

/// Catches changes queued between two frames replayed one by one, each a
/// read of the whole image file and an upload of all of it, on the
/// window's own thread: sixteen views, each changing the whole image,
/// reach the window before a frame draws them; that frame only starts a
/// reading, and the texture then takes the latest image in one upload.
#[test]
fn queued_whole_changes_reach_the_image_in_one_upload() {
    let (mut window, dir, _stand_in) = easel("queued");
    let blue = [40u8, 60, 220, 255];
    std::fs::write(dir.join("image.rgba"), blue.repeat((W * H) as usize)).unwrap();
    let views: Vec<String> = (2..=17u64)
        .map(|revision| {
            let mut changed = view(&dir, brush());
            changed.revision = revision;
            changed.patch = Some(protocol::RasterPatch { since: revision - 1, rect: [0, 0, W, H], file: None });
            line(AppMessage::Raster(changed))
        })
        .collect();
    std::fs::write(dir.join("queued.jsonl"), views.concat()).unwrap();
    window.uploads.clear();
    window.app.run_action(&Action::new("tool.queued"));
    // Every view arrives before a frame is drawn.
    let start = std::time::Instant::now();
    while raster(&window).view.revision != 17 {
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "the views never arrived");
        window.app.apps[0].running.poll();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let name = format!("raster:{}", dir.join("image.rgba").display());
    let texture = |window: &Window| window.ctx.tex_manager().read().allocated().find(|(_, meta)| meta.name == name).map(|(id, _)| *id).expect("the image has no texture");
    let image = texture(&window);
    let uploaded = |window: &Window| window.uploads.iter().filter(|upload| upload.0 == image).cloned().collect::<Vec<_>>();
    // The frame that sees them only starts the reading, and draws what it
    // showed before.
    window.step();
    assert!(uploaded(&window).is_empty(), "the frame read the image itself");
    for _ in 0..50 {
        window.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let uploads = uploaded(&window);
    assert_eq!(uploads.len(), 1, "the queued changes were uploaded {} times", uploads.len());
    let (_, _, size, first) = uploads[0];
    assert_eq!((size, first), ([W as usize, H as usize], Color32::from_rgb(40, 60, 220)), "the upload is not the latest image, whole");
    close(window, dir);
}
