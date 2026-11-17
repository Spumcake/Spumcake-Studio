//! The 3D Viewport: a place's pre-vis shown as it stands, from its scene
//! files. The host draws the scene; this program chooses the place, the
//! scene file, the camera, the look, and the published render shown with it,
//! and declares the camera list, the toolbar, and the menu, as the approved
//! view set (`research/scene-2026-11-15/viewport-views.md` in the team
//! repository) shows them.

mod places;

use places::{Place, Render};
use protocol::{
    Action, AppMessage, CameraChoice, CameraPose, ContentsKind, FlagIcons, HostMessage, LayerRow, Layers, MenuEntry,
    PanelPage, PanelSide, PropValue, Properties, ReferenceShow, Row, RowKind, TabEntry, Tabs, Thumbnail, Tool, ToolGroup,
    ToolItem, ToolResult, Toolbar, Transport, View3dCaption, View3dLook, View3dReference, View3dTool,
};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const TAB: &str = "3d-viewport";
/// The tab's label while no place is open.
const NAME: &str = "3D Viewport";
/// The camera list's rows: Free, the headings, the newest scene's cameras
/// from `CAMERA`, and the renders from `RENDER`.
const FREE: u64 = 1;
const CAMERAS_HEADING: u64 = 2;
const RENDERS_HEADING: u64 = 3;
const CAMERA: u64 = 100;
const RENDER: u64 = 1000;
/// A render's thumbnail in the list, in pixels.
const THUMB: (u32, u32) = (72, 40);
const LOOKS: [&str; 3] = ["Grey", "Colour-coded", "Simple"];

/// Which row of the camera list is chosen. Only one ever is.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    Free,
    Camera(String),
    Render(usize),
}

struct Viewport {
    project: Option<PathBuf>,
    places: Vec<Place>,
    place: usize,
    /// The scene file shown, of the place's.
    scene: usize,
    pick: Pick,
    look: View3dLook,
    names: bool,
    tool: View3dTool,
    over: bool,
    opacity: f64,
    /// Where the user left the free camera.
    free: Option<CameraPose>,
    time: f64,
    playing: bool,
    panel_open: bool,
    /// The decoded render's file, and the video frame it holds, if any.
    decoded: Option<(PathBuf, Option<u32>)>,
    problems: Vec<String>,
}

impl Viewport {
    fn new() -> Self {
        Self {
            project: None,
            places: Vec::new(),
            place: 0,
            scene: 0,
            pick: Pick::Free,
            look: View3dLook::Grey,
            names: true,
            tool: View3dTool::Orbit,
            over: true,
            opacity: 0.5,
            free: None,
            time: 0.0,
            playing: false,
            panel_open: true,
            decoded: None,
            problems: Vec::new(),
        }
    }

    fn label(&self) -> String {
        self.places.get(self.place).map_or_else(|| NAME.to_string(), Place::name)
    }

    fn renders(&self) -> Vec<Render> {
        self.places.get(self.place).map(Place::renders).unwrap_or_default()
    }

    fn shown(&self) -> Option<&places::SceneFile> {
        self.places.get(self.place)?.scenes.get(self.scene)
    }

    /// Reads the project's places, and opens the first.
    fn read_project(&mut self, folder: &str) {
        let project = PathBuf::from(folder);
        let (places, problems) = places::find(&project);
        self.places = places;
        self.problems = problems;
        self.project = Some(project);
        self.open_place(0);
    }

    fn open_place(&mut self, place: usize) {
        self.place = place.min(self.places.len().saturating_sub(1));
        self.scene = self.places.get(self.place).map_or(0, Place::newest);
        self.pick = Pick::Free;
        self.free = None;
        self.time = 0.0;
        self.playing = false;
        self.decoded = None;
        let _ = self.write_thumbnails();
    }

    /// Chooses a row of the camera list.
    fn choose(&mut self, row: u64) {
        let renders = self.renders();
        if row == FREE {
            self.pick = Pick::Free;
        } else if row >= RENDER {
            let Some(render) = renders.get((row - RENDER) as usize) else { return };
            self.scene = render.scene;
            self.pick = Pick::Render((row - RENDER) as usize);
            // A published render is drawn in the look it was made in, as its
            // camera's `reference_look` records it; grey when it does not say.
            let look = self
                .shown()
                .and_then(|s| s.scene.camera(&render.camera))
                .and_then(|c| c.reference_look.clone());
            self.look = match look.as_deref() {
                Some("simple") => View3dLook::Simple,
                Some("coded") => View3dLook::Coded,
                _ => View3dLook::Grey,
            };
        } else if row >= CAMERA {
            let newest = self.places.get(self.place).map_or(0, Place::newest);
            let Some(file) = self.places.get(self.place).and_then(|p| p.scenes.get(newest)) else { return };
            let Some(camera) = file.scene.cameras.get((row - CAMERA) as usize) else { return };
            self.pick = Pick::Camera(camera.name.clone());
            self.scene = newest;
        }
        if self.look == View3dLook::Simple && self.shown().is_none_or(|s| s.scene.looks.simple.is_none()) {
            self.look = View3dLook::Grey;
        }
        self.time = 0.0;
        self.playing = false;
    }

    /// The camera list, as vp-camera-list shows it.
    fn list(&self) -> Vec<LayerRow> {
        let mut rows = vec![row(FREE, None, "camera", "Free", self.pick == Pick::Free, None, None)];
        if let Some(place) = self.places.get(self.place) {
            rows.push(heading(CAMERAS_HEADING, "Cameras"));
            if let Some(newest) = place.scenes.get(place.newest()) {
                for (i, camera) in newest.scene.cameras.iter().enumerate() {
                    let picked = self.pick == Pick::Camera(camera.name.clone());
                    rows.push(row(CAMERA + i as u64, Some(CAMERAS_HEADING), "camera", &camera.name, picked, None, None));
                }
            }
            let renders = place.renders();
            if !renders.is_empty() {
                rows.push(heading(RENDERS_HEADING, "Pre-vis renders"));
            }
            for (i, render) in renders.iter().enumerate() {
                let thumbnail = self.project.as_ref().map(|p| Thumbnail {
                    path: thumbnail_file(p, render).display().to_string(),
                    width: THUMB.0,
                    height: THUMB.1,
                    revision: 1,
                });
                let picked = self.pick == Pick::Render(i);
                let id = RENDER + i as u64;
                rows.push(row(id, Some(RENDERS_HEADING), "media-image", &render.label, picked, thumbnail, Some(render.date.clone())));
            }
        }
        rows
    }

    /// What the host draws: `view3d`, with the published render decoded into
    /// the project's cache when one is chosen.
    fn view(&mut self) -> Option<AppMessage> {
        let file = self.shown()?;
        let path = file.path.clone();
        let scene = &file.scene;
        let objects = scene.objects.len();
        let builder = places::builder_name(&scene.builder);
        let camera = match &self.pick {
            Pick::Free => CameraChoice::Free { start: self.free },
            Pick::Camera(name) => CameraChoice::Named { name: name.clone() },
            Pick::Render(i) => CameraChoice::Named { name: self.renders().get(*i)?.camera.clone() },
        };
        let swing = scene.swing.as_ref().map(|swing| {
            let starts = swing.times.iter().zip(&swing.angles).find(|(_, angle)| **angle != 0.0).map(|(t, _)| *t);
            let fps = if swing.times.len() > 1 { 1.0 / (swing.times[1] - swing.times[0]) } else { 24.0 };
            (swing.length() + 1.0 / fps, fps, starts, swing.name.split(' ').next().unwrap_or("").to_string())
        });
        let status = match &camera {
            CameraChoice::Named { name } => format!("{name} · Scene from build-previs {builder} · {objects} objects"),
            CameraChoice::Free { .. } => format!("Scene from build-previs {builder} · {objects} objects"),
        };
        let transport = swing.map(|(length, fps, starts, what)| Transport {
            length,
            fps: fps.round(),
            time: self.time,
            playing: self.playing,
            clock: protocol::TransportClock::Host,
            cues: starts.map(|at| protocol::Cue { at, label: what, colour: None }).into_iter().collect(),
            rows: Vec::new(),
            play: Action::new("transport.play"),
            step: Action::new("transport.step"),
        });
        // While the swing plays, the host moves the time and tells this
        // program only where it stops, so a video's frame decoded here would
        // stand still over the moving door. The render steps aside while
        // playing, and comes back at the frame where the playing stops.
        let reference = match self.pick.clone() {
            Pick::Render(_) if self.playing => None,
            Pick::Render(i) => {
                let render = self.renders().get(i)?.clone();
                self.reference(&render)
            }
            _ => None,
        };
        Some(AppMessage::View3d {
            tab: TAB.to_string(),
            scene: path,
            look: self.look,
            camera,
            reference,
            time: self.time,
            names: self.names,
            tool: self.tool,
            transport,
            status: Some(status),
        })
    }

    /// The chosen render, decoded into a raw file in the project's cache: a
    /// PNG once, a video's frame at the time shown. A new frame is written to
    /// a new file and the last one removed, since the host reads a file once.
    fn reference(&mut self, render: &Render) -> Option<View3dReference> {
        let project = self.project.clone()?;
        let source = project.join(&render.file);
        let stem = Path::new(&render.file).file_stem()?.to_string_lossy().to_string();
        let cache = places::cache(&project);
        std::fs::create_dir_all(&cache).ok()?;
        let (rgba, width, height, frame) = if render.video {
            let mut video = media::Video::open(&source).ok()?;
            let info = video.info();
            let last = info.frames.saturating_sub(1);
            // The frame on screen at this time, as the scene's swing holds
            // each key until the next: the last frame at or before it.
            let index = ((self.time * info.fps() + 1e-6).floor() as u32).min(last);
            // The frame already decoded is used again only when it is this
            // video's, at this frame.
            if let Some((file, Some(held))) = &self.decoded
                && *held == index
                && file.file_name().is_some_and(|n| *n == *format!("{stem}-f{index:05}.rgba"))
                && file.starts_with(&cache)
            {
                return Some(self.reference_of(file, &project, render, info.width, info.height));
            }
            let frame = video.frame(index, (info.width, info.height)).ok()?;
            (frame.rgba, frame.width, frame.height, Some(index))
        } else {
            let image = image::open(&source).ok()?.to_rgba8();
            let (w, h) = image.dimensions();
            (image.into_raw(), w, h, None)
        };
        let name = match frame {
            Some(index) => format!("{stem}-f{index:05}.rgba"),
            None => format!("{stem}.rgba"),
        };
        let file = cache.join(name);
        if !file.exists() {
            let scratch = file.with_extension("rgba.part");
            std::fs::write(&scratch, &rgba).ok()?;
            std::fs::rename(&scratch, &file).ok()?;
        }
        if let Some((old, Some(_))) = &self.decoded
            && *old != file
        {
            let _ = std::fs::remove_file(old);
        }
        self.decoded = Some((file.clone(), frame));
        Some(self.reference_of(&file, &project, render, width, height))
    }

    fn reference_of(&self, file: &Path, project: &Path, render: &Render, width: u32, height: u32) -> View3dReference {
        View3dReference {
            image: file.strip_prefix(project).unwrap_or(file).display().to_string(),
            width,
            height,
            show: if self.over { ReferenceShow::Over { opacity: self.opacity } } else { ReferenceShow::Beside },
            caption: Some(View3dCaption { view: "3D view".into(), reference: format!("Pre-vis render · {}", render.date) }),
        }
    }

    /// Each render's thumbnail for the list, written once.
    fn write_thumbnails(&self) -> Option<()> {
        let project = self.project.as_ref()?;
        std::fs::create_dir_all(places::cache(project)).ok()?;
        for render in self.renders() {
            let file = thumbnail_file(project, &render);
            if file.exists() {
                continue;
            }
            let source = project.join(&render.file);
            let image = if render.video {
                let mut video = media::Video::open(&source).ok()?;
                let info = video.info();
                let frame = video.frame(info.frames / 2, (info.width, info.height)).ok()?;
                image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)?
            } else {
                image::open(&source).ok()?.to_rgba8()
            };
            let small = image::imageops::resize(&image, THUMB.0, THUMB.1, image::imageops::FilterType::Triangle);
            let scratch = file.with_extension("rgba.part");
            std::fs::write(&scratch, small.into_raw()).ok()?;
            std::fs::rename(&scratch, &file).ok()?;
        }
        Some(())
    }

    fn toolbar(&self) -> Toolbar {
        let simple = self.shown().is_some_and(|s| s.scene.looks.simple.is_some());
        let options: Vec<String> =
            LOOKS.iter().enumerate().filter(|(i, _)| *i < 2 || simple).map(|(_, l)| l.to_string()).collect();
        let look = match self.look {
            View3dLook::Grey => 0,
            View3dLook::Coded => 1,
            View3dLook::Simple => 2,
        };
        let render = matches!(self.pick, Pick::Render(_));
        let file = match &self.pick {
            Pick::Render(i) => self.renders().get(*i).map(|r| r.file.rsplit('/').next().unwrap_or(&r.file).to_string()),
            _ => None,
        };
        Toolbar {
            items: vec![
                button("rotate-camera-right", "Orbit", self.tool == View3dTool::Orbit, Action::with_arg("tool", json!("orbit"))),
                button("drag-hand-gesture", "Pan", self.tool == View3dTool::Pan, Action::with_arg("tool", json!("pan"))),
                ToolItem::Separator,
                ToolItem::Choice { caption: String::new(), options, selected: look, set: Action::new("look") },
                button("label", "Names", self.names, Action::new("names")),
            ],
            trailing: vec![
                ToolItem::Popover {
                    icon: "cube".into(),
                    tip: "Pre-vis render".into(),
                    rows: vec![
                        Row {
                            prop: json!("render"),
                            label: "Pre-vis render".into(),
                            kind: RowKind::Note,
                            value: PropValue::Text(file.unwrap_or_default()),
                        },
                        Row {
                            prop: json!("draw"),
                            label: "Draw".into(),
                            kind: RowKind::Choice {
                                options: vec!["Beside".into(), "Over".into()],
                                segmented: true,
                                preview: protocol::ChoicePreview::Label,
                            },
                            value: PropValue::Index(u8::from(self.over)),
                        },
                        Row {
                            prop: json!("opacity"),
                            label: "Opacity".into(),
                            kind: RowKind::Number { min: 0.0, max: 1.0, percent: true, places: 0, centered: false },
                            value: PropValue::Number(self.opacity),
                        },
                    ],
                    action: None,
                    selected: render,
                    enabled: render,
                },
                ToolItem::Separator,
                button("frame-select", "Fit", false, Action::new("fit")),
                ToolItem::Maximize { icon: "maximize".into(), restore_icon: "reduce".into(), tip: "Maximize".into() },
            ],
            options: Vec::new(),
            options_trailing: Vec::new(),
        }
    }

    /// The workspace menu: Open place, with each of the project's places and
    /// how many scene files it has.
    fn menu(&self) -> Vec<MenuEntry> {
        let places = self
            .places
            .iter()
            .enumerate()
            .map(|(i, place)| MenuEntry {
                tip: None,
                label: format!("{} · {} scene files", place.name(), place.scenes.len()),
                icon: None,
                enabled: true,
                divider: false,
                action: Some(Action::with_arg("open_place", json!(i))),
                submenu: Vec::new(),
                rename: false,
            })
            .collect::<Vec<_>>();
        vec![MenuEntry {
            tip: None,
            label: "Open place".into(),
            icon: None,
            enabled: !places.is_empty(),
            divider: false,
            action: None,
            submenu: places,
            rename: false,
        }]
    }

    fn properties(&self) -> Properties {
        let flag = String::new;
        Properties {
            open: self.panel_open,
            toggle: Action::new("panel"),
            tip: "Cameras".into(),
            finish: Action::new("finish"),
            back: None,
            layers: Some(Layers {
                rows: self.list(),
                reorder: Action::new("none"),
                sort_color: Action::new("none"),
                flags: FlagIcons { eye_open: flag(), eye_closed: flag(), lock_closed: flag(), lock_open: flag() },
                nest: false,
                plain: true,
                actions: Vec::new(),
            }),
            sections: Vec::new(),
            pages: vec![PanelPage { label: "Cameras".into(), active: true, enabled: true, action: None }],
            cell: None,
        }
    }

    /// Everything the tab declares, after any change.
    fn declare(&mut self) -> Vec<AppMessage> {
        let mut out = vec![
            tabs(&self.label()),
            AppMessage::Toolbar(self.toolbar()),
            AppMessage::Menu { rows: self.menu() },
            AppMessage::Properties(self.properties()),
        ];
        out.extend(self.view());
        out
    }

    /// A control was used.
    fn command(&mut self, id: &str, arg: Option<&Value>) {
        let value = arg.and_then(|a| a.get("value")).or(arg);
        match id {
            "pick" => {
                if let Some(row) = arg.and_then(|a| a.get("row")).and_then(Value::as_u64) {
                    self.choose(row);
                }
            }
            "look" => {
                self.look = match value.and_then(Value::as_u64) {
                    Some(1) => View3dLook::Coded,
                    Some(2) => View3dLook::Simple,
                    _ => View3dLook::Grey,
                }
            }
            "names" => self.names = !self.names,
            "tool" => {
                let pan = arg.and_then(|a| a.as_str().or_else(|| a.get("arg").and_then(Value::as_str))) == Some("pan");
                self.tool = if pan { View3dTool::Pan } else { View3dTool::Orbit };
            }
            "panel" => self.panel_open = !self.panel_open,
            "fit" => {
                self.free = None;
                self.pick = Pick::Free;
            }
            "open_place" => {
                if let Some(place) = arg.and_then(|a| a.as_u64().or_else(|| a.get("arg").and_then(Value::as_u64))) {
                    self.open_place(place as usize);
                }
            }
            "transport.play" => self.playing = value.and_then(Value::as_bool).unwrap_or(!self.playing),
            "transport.step" => {
                // The key times are stored as 32-bit floats, so the rate they
                // give is rounded to whole frames a second, as the transport's.
                let fps = self.shown().and_then(|s| s.scene.swing.as_ref()).map_or(24.0, |swing| {
                    if swing.times.len() > 1 { (1.0 / (swing.times[1] - swing.times[0])).round() } else { 24.0 }
                });
                let length = self.shown().and_then(|s| s.scene.swing.as_ref()).map_or(0.0, |s| s.length());
                let by = value.and_then(Value::as_f64).unwrap_or(1.0);
                self.playing = false;
                self.time = (((self.time * fps).round() + by) / fps).clamp(0.0, length);
            }
            _ => {}
        }
    }

    /// A panel row was edited: the render's Draw or Opacity.
    fn property(&mut self, prop: &Value, value: &Value) {
        let number = value.get("number").and_then(Value::as_f64).or_else(|| value.as_f64());
        let index = value.get("index").and_then(Value::as_u64).or_else(|| value.as_u64());
        match prop.as_str() {
            Some("draw") => self.over = index == Some(1),
            Some("opacity") => {
                if let Some(n) = number {
                    self.opacity = n.clamp(0.0, 1.0);
                }
            }
            _ => {}
        }
    }

    /// One of the assistant's tools, carried out as the hand actions are.
    fn invoke(&mut self, tool: &str, input: &Value) -> (bool, Value) {
        let name = tool.rsplit("__").next().unwrap_or(tool);
        match name {
            "list_views" => {
                let rows: Vec<Value> = self
                    .list()
                    .iter()
                    .filter(|r| !r.group)
                    .map(|r| json!({ "name": r.name, "made": r.value, "chosen": r.selected }))
                    .collect();
                (true, json!({ "place": self.label(), "views": rows }))
            }
            "look_through" => {
                let wanted = input.get("view").and_then(Value::as_str).unwrap_or("");
                let found = self.list().into_iter().find(|r| !r.group && (r.name == wanted || format!("{} {}", r.name, r.value.clone().unwrap_or_default()) == wanted));
                match found {
                    Some(row) => {
                        self.choose(row.id);
                        (true, json!({ "chosen": row.name, "made": row.value }))
                    }
                    None => (false, json!({ "error": format!("no view is named {wanted:?}; list_views gives every name") })),
                }
            }
            "set_look" => {
                let look = input.get("look").and_then(Value::as_str).unwrap_or("");
                let index = match look {
                    "grey" => 0,
                    "coded" => 1,
                    "simple" => 2,
                    _ => return (false, json!({ "error": "look is grey, coded, or simple" })),
                };
                self.command("look", Some(&json!({ "value": index })));
                (true, json!({ "look": look }))
            }
            _ => (false, json!({ "error": format!("no tool {name}") })),
        }
    }
}

fn main() {
    // A workspace being reopened starts this with `--restore`: no tab
    // until the host sends `open`.
    let mut open = !std::env::args().any(|arg| arg == protocol::RESTORE_FLAG);
    let mut viewport = Viewport::new();

    let mut greeting = vec![AppMessage::Hello { protocol: protocol::PROTOCOL_VERSION }];
    if open {
        greeting.push(AppMessage::Tab { id: TAB.to_string(), label: NAME.to_string() });
        greeting.extend(declare_tab());
    }
    greeting.push(AppMessage::Tools { groups: tool_groups() });
    greeting.push(AppMessage::End);
    send(&greeting);

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let replies = match protocol::parse_host_message(&line) {
            Ok(HostMessage::Shutdown) => break,
            Ok(HostMessage::Ping) => vec![AppMessage::Pong],
            Ok(HostMessage::Quit) => vec![AppMessage::QuitReady],
            Ok(HostMessage::Keep { tab }) => vec![AppMessage::Kept { tab, path: None, error: None }],
            Ok(HostMessage::Claim { id, .. }) => vec![AppMessage::Claimed { id, yours: false }],
            Ok(HostMessage::Render { id, .. }) => vec![protocol::no_items(id)],
            Ok(HostMessage::Project { folder, .. }) => {
                viewport.read_project(&folder);
                if open { viewport.declare() } else { Vec::new() }
            }
            Ok(HostMessage::Open { .. }) if !open => {
                open = true;
                let mut replies = declare_tab();
                replies.extend(viewport.declare());
                replies
            }
            Ok(HostMessage::Command { id, arg, .. }) if open => {
                viewport.command(&id, arg.as_ref());
                viewport.declare()
            }
            Ok(HostMessage::Property { prop, value, .. }) if open => {
                viewport.property(&prop, &value);
                viewport.declare()
            }
            Ok(HostMessage::View3dCamera { position, target, fov, .. }) => {
                viewport.free = Some(CameraPose { position, target, fov });
                viewport.pick = Pick::Free;
                viewport.declare()
            }
            Ok(HostMessage::TransportTime { time, .. }) => {
                viewport.time = time;
                viewport.declare()
            }
            Ok(HostMessage::Invoke { call, tool, input, .. }) => {
                if !open {
                    vec![result(call, false, json!({ "error": "the tab is not open yet" }))]
                } else {
                    let (ok, output) = viewport.invoke(&tool, &input);
                    let mut replies = if ok { viewport.declare() } else { Vec::new() };
                    replies.push(result(call, ok, output));
                    replies
                }
            }
            _ => continue,
        };
        send(&replies);
    }
}

/// What the tab shows: a 3D view, and the properties panel on the right.
fn declare_tab() -> Vec<AppMessage> {
    vec![
        AppMessage::Contents { tab: TAB.to_string(), kind: ContentsKind::View3d },
        AppMessage::Panel { tab: TAB.to_string(), side: PanelSide::Right },
    ]
}

/// The tab list: one tab, which is not a document saved to a file.
fn tabs(label: &str) -> AppMessage {
    AppMessage::Tabs(Tabs {
        tabs: vec![TabEntry {
            id: TAB.to_string(),
            label: label.to_string(),
            dirty: false,
            close_waits: false,
            path: None,
            saves: false,
        }],
        active: TAB.to_string(),
        new_tab: None,
        new_tab_tip: String::new(),
    })
}

fn button(icon: &str, tip: &str, selected: bool, action: Action) -> ToolItem {
    ToolItem::Button { icon: icon.into(), tip: tip.into(), enabled: true, selected, action: Some(action), menu: Vec::new() }
}

fn heading(id: u64, name: &str) -> LayerRow {
    let mut heading = row(id, None, "", name, false, None, None);
    heading.group = true;
    heading
}

fn row(
    id: u64,
    parent: Option<u64>,
    icon: &str,
    name: &str,
    selected: bool,
    thumbnail: Option<Thumbnail>,
    value: Option<String>,
) -> LayerRow {
    LayerRow {
        id,
        parent,
        depth: u32::from(parent.is_some()),
        icon: icon.into(),
        name: name.into(),
        group: false,
        selected,
        visible: true,
        locked: false,
        renamable: false,
        click: Action::with_arg("pick", json!({ "row": id })),
        rename: Action::new("none"),
        toggle_visible: Action::new("none"),
        toggle_locked: Action::new("none"),
        menu: Vec::new(),
        colors: Vec::new(),
        thumbnail,
        mark: None,
        detail: None,
        toggle_icon: None,
        value,
    }
}

fn thumbnail_file(project: &Path, render: &Render) -> PathBuf {
    let stem = Path::new(&render.file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    places::cache(project).join(format!("{stem}-thumbnail.rgba"))
}

/// What the assistant can do here: list the views, look through one, and
/// choose the look, each as the user would by hand.
fn tool_groups() -> Vec<ToolGroup> {
    vec![ToolGroup {
        id: "view".to_string(),
        name: "3D Viewport".to_string(),
        description: "Look at a place's pre-vis in 3D: through its builder's cameras, or with a camera's published pre-vis render beside or over it."
            .to_string(),
        instructions: "Call list_views first: it names every view the camera list offers, Free, each camera, and each published pre-vis render with the day it was made. Call look_through with one of those names exactly, adding the day after a space when two renders share a name, such as \"far-end-wall 11 Nov\". Nothing here changes the pre-vis."
            .to_string(),
        tools: vec![
            Tool {
                name: "list_views".to_string(),
                description: "Every view of the open place, and which is chosen.".to_string(),
                input: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
                implements: Vec::new(),
            },
            Tool {
                name: "look_through".to_string(),
                description: "Choose a view of the camera list by its name, as list_views gives it.".to_string(),
                input: json!({
                    "type": "object",
                    "properties": { "view": { "type": "string", "description": "The view's name, such as \"far-end-wall 10 Nov\" or \"Free\"." } },
                    "required": ["view"],
                    "additionalProperties": false,
                }),
                implements: Vec::new(),
            },
            Tool {
                name: "set_look".to_string(),
                description: "Draw the scene grey, colour-coded by kind, or simple (white faces and grey edges).".to_string(),
                input: json!({
                    "type": "object",
                    "properties": { "look": { "type": "string", "enum": ["grey", "coded", "simple"] } },
                    "required": ["look"],
                    "additionalProperties": false,
                }),
                implements: Vec::new(),
            },
        ],
    }]
}

fn result(call: String, ok: bool, output: Value) -> AppMessage {
    AppMessage::ToolResult(ToolResult { call, ok, output, images: Vec::new() })
}

fn send(messages: &[AppMessage]) {
    let mut stdout = std::io::stdout().lock();
    for message in messages {
        let _ = stdout.write_all(protocol::encode_app_message(message).as_bytes());
    }
    let _ = stdout.flush();
}
