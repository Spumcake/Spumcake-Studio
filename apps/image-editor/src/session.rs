//! The Image Editor's tabs, each a gallery of layered pictures, and
//! everything the host sends them: commands, gestures, panel edits, the
//! filmstrip's clicks, files, and the project.
//!
//! Inside a project, which the host names with `project`, every image is
//! written into the project's folder by itself, a second after it changes,
//! and before its tab closes, a `keep` is answered, or the host quits, as
//! Mission Control's notebooks are. Save and Save As are offered only
//! outside a project.

use crate::declare;
use crate::doc::{Doc, Sampler, Setting};
use crate::files;
use crate::gallery::{Frame, Gallery};
use crate::paint::{Brush, Combine, Mask};
use crate::picture::{Blend, Curves, Grade, Kind, Picture, Pixels};
use crate::text::{Align, Face, Style};
use protocol::{
    AppMessage, Ask, AskButton, ContentsKind, DocumentCamera, FileFilter, FileKind, HostMessage, PanelSide,
    PropValue, RasterGesture, RasterTool, RasterView, TextEditing, Thumbnail,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How many files of rows sent while a stroke is drawn are kept for each
/// picture, for a host that falls behind; one that falls further waits for
/// the stroke's end.
const KEPT_PATCHES: usize = 32;

/// How long a change waits before it is written, so a burst of edits is
/// written once.
const WRITE_AFTER: Duration = Duration::from_secs(1);
/// How long a failed write waits before it is tried again by itself.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// A new picture's name, as the owner set on 2026-10-30: it is an asset.
pub const UNTITLED: &str = "Untitled Asset";
/// A new image's size, as the owner set on 2026-10-30.
pub const NEW_SIZE: [u32; 2] = [1920, 1080];
/// The largest a layer's thumbnail is, in pixels.
const THUMB_SIZE: [u32; 2] = [56, 40];
/// The largest a filmstrip thumbnail is, in pixels: 44 tall, as the gallery
/// views draw them, and no wider than four times that.
const FILM_SIZE: [u32; 2] = [176, 44];
/// The space kept around a picture when it is fitted to the window.
const FIT_MARGIN: f64 = 40.0;

/// The tools of the toolbar, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Move,
    Marquee,
    Lasso,
    Crop,
    Eyedropper,
    Heal,
    Stamp,
    Brush,
    Eraser,
    Text,
    Hand,
    Zoom,
}

impl ToolKind {
    /// In the mockup's order: healing and the stamp after the eyedropper,
    /// and text after the eraser.
    pub const ALL: [ToolKind; 12] = [
        ToolKind::Move,
        ToolKind::Marquee,
        ToolKind::Lasso,
        ToolKind::Crop,
        ToolKind::Eyedropper,
        ToolKind::Heal,
        ToolKind::Stamp,
        ToolKind::Brush,
        ToolKind::Eraser,
        ToolKind::Text,
        ToolKind::Hand,
        ToolKind::Zoom,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ToolKind::Move => "move",
            ToolKind::Marquee => "marquee",
            ToolKind::Lasso => "lasso",
            ToolKind::Crop => "crop",
            ToolKind::Eyedropper => "eyedropper",
            ToolKind::Heal => "heal",
            ToolKind::Stamp => "stamp",
            ToolKind::Brush => "brush",
            ToolKind::Eraser => "eraser",
            ToolKind::Text => "text",
            ToolKind::Hand => "hand",
            ToolKind::Zoom => "zoom",
        }
    }

    pub fn named(key: &str) -> Option<ToolKind> {
        ToolKind::ALL.into_iter().find(|tool| tool.key() == key)
    }
}

/// A brush's or the eraser's settings, as the settings row shows them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushSettings {
    /// In picture pixels.
    pub size: u32,
    /// Percents.
    pub hardness: u32,
    pub opacity: u32,
    pub pressure: bool,
}

/// What the toolbar and its settings row hold, for every tab.
#[derive(Debug, Clone)]
pub struct Tools {
    pub tool: ToolKind,
    pub rgb: [u8; 3],
    pub brush: BrushSettings,
    pub eraser: BrushSettings,
    pub combine: Combine,
    pub feather: f64,
    /// An index into `declare::RATIOS`.
    pub ratio: usize,
    /// The clone stamp's and the healing brush's sizes and hardness, and
    /// the stamp's opacity; pressure is not theirs.
    pub stamp: BrushSettings,
    pub heal: BrushSettings,
    /// The stamp keeps its offset from one stroke to the next.
    pub aligned: bool,
    /// Where the stamp and healing copy from, set with Alt-click.
    pub source: Option<[f64; 2]>,
    /// The offset kept for the next stroke, aligned, and the offset of the
    /// stroke being drawn.
    pub kept: Option<[f64; 2]>,
    pub stroke: Option<[f64; 2]>,
    /// The style a new text takes, and the edited text's as it changes.
    pub text: Style,
}

impl Default for Tools {
    /// As brush.png shows: black, 24 px, 80 % hard, 100 % opaque, pressure on.
    fn default() -> Self {
        let brush = BrushSettings { size: 24, hardness: 80, opacity: 100, pressure: true };
        Self {
            tool: ToolKind::Brush,
            rgb: [0, 0, 0],
            brush,
            eraser: brush,
            combine: Combine::New,
            feather: 0.0,
            ratio: 0,
            // As clone.png and heal.png show.
            stamp: BrushSettings { size: 60, hardness: 50, opacity: 100, pressure: false },
            heal: BrushSettings { size: 40, hardness: 30, opacity: 100, pressure: false },
            aligned: true,
            source: None,
            kept: None,
            stroke: None,
            text: Style::default(),
        }
    }
}

/// The panel's pages, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Layers,
    Properties,
    Adjustments,
}

/// One tab: a gallery of pictures, kept in one file.
pub struct Image {
    pub id: String,
    pub label: String,
    pub path: Option<PathBuf>,
    pub gallery: Gallery,
    pub dirty: bool,
    /// The filmstrip, not the picture, has the keyboard: Undo and Redo act
    /// on the strip's history until the picture is used again.
    pub strip_focus: bool,
    /// Inside a project: when its change was first seen unwritten, and its
    /// last failed write, why and when.
    unwritten_since: Option<Instant>,
    write_failed: Option<(String, Instant)>,
    /// Being written by the writer's thread: what the image was, and where.
    saving: Option<(Signature, PathBuf)>,
    /// A camera to send with the next picture: the picture is new, opened,
    /// or newly shown.
    send_camera: bool,
    /// Its thumbnails were left unwritten while a slider was held.
    thumbs_owed: bool,
}

impl Image {
    /// The picture shown.
    pub fn frame(&self) -> &Frame {
        self.gallery.frame()
    }

    pub fn frame_mut(&mut self) -> &mut Frame {
        self.gallery.frame_mut()
    }

    /// The shown picture's document.
    pub fn doc(&self) -> &Doc {
        &self.gallery.frame().doc
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.gallery.frame_mut().doc
    }

    /// Whether Undo, as the keyboard has it now, has something to undo.
    pub fn can_undo(&self) -> bool {
        if self.strip_focus { self.gallery.can_undo() } else { self.doc().can_undo() }
    }

    pub fn can_redo(&self) -> bool {
        if self.strip_focus { self.gallery.can_redo() } else { self.doc().can_redo() }
    }
}

/// What a file dialog's answer is for.
#[derive(Debug, Clone)]
enum Purpose {
    Open,
    OpenLayer,
    /// The filmstrip's "Open PNG or JPEG…": a new picture in this tab.
    OpenPicture { tab: String },
    Export { jpeg: bool },
    Save { tab: String, then_quit: bool },
}

pub struct Session {
    pub images: Vec<Image>,
    pub active: usize,
    next_tab: u64,
    pub tools: Tools,
    pub panel_open: bool,
    /// The panel's page shown.
    pub page: Page,
    /// The curve the Properties page shows of a Curves layer: 0 for all
    /// three channels, then red, green, and blue.
    pub curve_channel: usize,
    /// The curve point last moved, for its input and output.
    pub curve_point: Option<[f64; 2]>,
    pub project: Option<PathBuf>,
    /// Where pictures and thumbnails for the host are written, and whether
    /// it is a folder of this session's own in memory, which stays when a
    /// project is named.
    cache: PathBuf,
    in_memory: bool,
    /// A picture file for the host could not be written in memory: the
    /// pictures move to disk, and are sent again.
    display_failed: bool,
    viewport: [f64; 2],
    file: Option<(String, Purpose)>,
    next_file: u64,
    /// Every picture file and thumbnail written gets the next revision. It
    /// starts from the time the program started, so a picture of an earlier
    /// run is never taken for a current one.
    next_revision: u64,
    /// Points of a stroke were painted and the picture is not sent yet.
    live_due: bool,
    /// A held slider changed the picture, which is sent, with everything
    /// else, once the lines waiting have all been read, or before anything
    /// else is answered.
    emit_due: bool,
    /// The thread that writes images due inside a project, started with the
    /// first.
    writer: Option<Writer>,
    /// Tab ids still to ask about while quitting, after the one asked.
    quit_rest: Vec<String>,
    quitting: bool,
    /// A close of one image is asking, rather than the window's quit.
    closing: bool,
}

impl Session {
    /// Launched: one new image, already declared.
    pub fn launch() -> Self {
        let mut session = Self::empty();
        session.push_image(new_gallery(), None, None);
        session
    }

    /// Started with `--restore`: no image until the host sends `open`.
    pub fn restored() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64 * 1000)
            .unwrap_or(0);
        let memory = memory_folder();
        Self {
            images: Vec::new(),
            active: 0,
            next_tab: 1,
            tools: Tools::default(),
            panel_open: true,
            page: Page::Layers,
            curve_channel: 0,
            curve_point: None,
            project: None,
            in_memory: memory.is_some(),
            display_failed: false,
            cache: memory.unwrap_or_else(scratch_folder),
            viewport: [1200.0, 800.0],
            file: None,
            next_file: 1,
            next_revision: seed,
            live_due: false,
            emit_due: false,
            writer: None,
            quit_rest: Vec::new(),
            quitting: false,
            closing: false,
        }
    }

    pub fn greeting(&mut self) -> Vec<AppMessage> {
        let mut messages = vec![AppMessage::Hello { protocol: protocol::PROTOCOL_VERSION }];
        if let Some(image) = self.images.first() {
            messages.push(AppMessage::Tab { id: image.id.clone(), label: image.label.clone() });
            messages.push(AppMessage::Contents { tab: image.id.clone(), kind: ContentsKind::Raster });
            messages.push(AppMessage::Panel { tab: image.id.clone(), side: PanelSide::Right });
        }
        messages.push(AppMessage::Skills { skills: crate::assist::skills() });
        messages.push(AppMessage::Edits { media_types: vec![String::from("image/png"), String::from("image/jpeg")] });
        messages.push(AppMessage::End);
        if !self.images.is_empty() {
            messages.extend(self.emit());
        }
        messages
    }

    /// One message from the host, and every reply.
    pub fn handle(&mut self, message: HostMessage) -> Vec<AppMessage> {
        let mut out = Vec::new();
        // A held slider's values are applied as they come, and the picture
        // and panel they make are sent once for all the lines waiting. Any
        // other message first sends what they changed, so every answer
        // comes after the change before it, except a panel edit or the
        // slider let go, which send everything themselves.
        let sends_all = match &message {
            HostMessage::Property { .. } => true,
            HostMessage::Command { id, .. } => id == "finish_edit",
            _ => false,
        };
        if !sends_all && self.emit_due {
            out.extend(self.emit());
        }
        let looked = self.looks();
        // A stroke being drawn lasts while its points come. A message that
        // could change the picture takes it off first; the host's ping each
        // second, the camera, and the viewport leave it be, so a stroke held
        // longer than a second stays.
        let drawing = matches!(
            &message,
            HostMessage::RasterGesture {
                gesture: RasterGesture::StrokePart { .. }
                    | RasterGesture::Stroke { .. }
                    | RasterGesture::DragPart { .. }
                    | RasterGesture::Drag { .. },
                ..
            } | HostMessage::Camera { .. }
                | HostMessage::Viewport { .. }
                | HostMessage::Ping
                | HostMessage::Keep { .. }
                | HostMessage::Claim { .. }
        );
        if !drawing {
            for image in &mut self.images {
                for frame in &mut image.gallery.frames {
                    frame.doc.end_live();
                }
            }
        }
        // A text being typed in stays open through the keyboard, the text
        // tool's own gestures and settings, and the host's own messages;
        // anything else ends its editing first, which makes its step.
        let typing = match &message {
            HostMessage::RasterText { .. }
            | HostMessage::Camera { .. }
            | HostMessage::Viewport { .. }
            | HostMessage::Ping
            | HostMessage::Keep { .. }
            | HostMessage::Claim { .. }
            | HostMessage::Answer { .. } => true,
            HostMessage::RasterGesture { gesture: RasterGesture::Click { .. } | RasterGesture::Rect { .. } | RasterGesture::TextBox { .. }, .. } => {
                self.tools.tool == ToolKind::Text
            }
            HostMessage::Command { id, .. } => {
                matches!(id.as_str(), "text_style" | "undo" | "redo" | "toggle_panel" | "panel_page") || id.starts_with("canvas.")
            }
            _ => false,
        };
        if !typing && let Some(image) = self.images.get_mut(self.active) {
            let before = image.doc().edits;
            image.doc_mut().end_typing();
            if image.doc().edits != before {
                image.dirty = true;
            }
        }
        match message {
            HostMessage::Shutdown => return Vec::new(),
            HostMessage::Ping => {
                out.push(AppMessage::Pong);
                return out;
            }
            HostMessage::Quit => {
                if self.project.is_some() {
                    // Everything is written first, so the quit asks nothing.
                    for index in 0..self.images.len() {
                        let _ = self.write_into_project(index);
                    }
                }
                self.note_quit(&mut out);
            }
            HostMessage::Project { folder } => {
                let folder = PathBuf::from(folder);
                let folder = folder.canonicalize().unwrap_or(folder);
                // Pictures in memory stay where they are; otherwise they go
                // into the project's cache.
                if !self.in_memory {
                    if self.project.is_none() {
                        // The scratch pictures written before the project
                        // was known are not needed any more.
                        let _ = std::fs::remove_dir_all(&self.cache);
                    }
                    self.cache = folder.join(".inpainter").join("cache").join("image-editor");
                    // Rows a stroke left there in an earlier run are not
                    // needed.
                    for entry in std::fs::read_dir(&self.cache).into_iter().flatten().flatten() {
                        if entry.path().extension().is_some_and(|extension| extension == "patch") {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                    // Every picture is written again where the host now
                    // looks.
                    for image in &mut self.images {
                        for frame in &mut image.gallery.frames {
                            frame.written = None;
                            frame.thumbs.clear();
                            frame.film = None;
                        }
                    }
                }
                self.project = Some(folder);
                out.extend(self.emit());
            }
            // An image file from outside the project opens nothing.
            HostMessage::Open { path: Some(path) }
                if files::is_ipx(Path::new(&path)) && !self.in_project(Path::new(&path)) => {}
            HostMessage::Open { path } => {
                self.open_path(path.map(PathBuf::from), &mut out);
            }
            HostMessage::OpenAsNew { path, name, from } => self.open_as_new(Path::new(&path), &name, from, &mut out),
            HostMessage::Command { id, tab, arg, viewport, .. } => {
                if viewport[0] > 0.0 && viewport[1] > 0.0 {
                    self.viewport = viewport;
                }
                if let Some(tab) = &tab {
                    self.activate(tab);
                }
                if !keeps_focus(&id) {
                    self.picture_focus();
                }
                self.command(&id, tab.as_deref(), arg.unwrap_or(Value::Null), &mut out);
            }
            // The newest points of a stroke: painted at once, and the
            // picture sent when the lines waiting have all been read.
            HostMessage::RasterGesture { tab, gesture: RasterGesture::StrokePart { from, points } } => {
                self.activate(&tab);
                self.picture_focus();
                if let Some(brush) = self.brush()
                    && let Some(image) = self.images.get_mut(self.active)
                {
                    image.doc_mut().paint_live(from, &points, &brush);
                    self.live_due = true;
                } else if let Some(sampler) = self.sampler((from == 0).then(|| points.first().map(|point| [point[0], point[1]])).flatten())
                    && let Some(image) = self.images.get_mut(self.active)
                {
                    image.doc_mut().sample_live(from, &points, &sampler);
                    self.live_due = true;
                }
            }
            // The move tool's drag while it is open: with a selection, the
            // selected pixels move as it goes; without one, the layer moves
            // when it is let go.
            HostMessage::RasterGesture { tab, gesture: RasterGesture::DragPart { from, to } } => {
                self.activate(&tab);
                self.picture_focus();
                if self.tools.tool == ToolKind::Move
                    && let Some(image) = self.images.get_mut(self.active)
                    && image.doc().selection.is_some()
                {
                    let _ = image.doc_mut().drag_part(from, to);
                    self.live_due = true;
                }
            }
            HostMessage::RasterText { tab, input } => {
                self.activate(&tab);
                self.picture_focus();
                if let Some(Some(text)) = self.change(|doc| doc.text_input(&input)) {
                    out.push(AppMessage::Clipboard { text });
                }
                out.extend(self.emit());
            }
            HostMessage::RasterGesture { tab, gesture } => {
                self.activate(&tab);
                self.picture_focus();
                self.gesture(gesture);
                out.extend(self.emit());
            }
            HostMessage::Camera { tab, origin, zoom } => {
                if let Some(image) = self.images.iter_mut().find(|image| image.id == tab) {
                    let frame = image.frame_mut();
                    frame.camera = Some(DocumentCamera { origin, zoom });
                    frame.host_camera = true;
                    // The zoom menu shows the new percent.
                    out.push(AppMessage::Toolbar(declare::toolbar(self)));
                }
            }
            HostMessage::Viewport { viewport } => {
                if viewport[0] > 0.0 && viewport[1] > 0.0 && viewport != self.viewport {
                    self.viewport = viewport;
                    // A picture fitted before the window's size was known is
                    // fitted again, unless the host has moved its camera.
                    for image in &mut self.images {
                        if !image.frame().host_camera {
                            image.send_camera = true;
                        }
                    }
                    out.extend(self.emit());
                }
            }
            HostMessage::Answer { id, button } => {
                if id == "keep-image" {
                    self.answer(&button, &mut out);
                }
            }
            // A held slider's value is sent with the lines waiting, and
            // only when it changed something; the value let go is always
            // sent.
            HostMessage::Property { prop, value, preview } => {
                self.picture_focus();
                self.property(&prop, &value, preview);
                if !preview {
                    out.extend(self.emit());
                } else if self.looks() != looked || self.active_image().is_some_and(|image| image.doc().out_of_date()) {
                    self.emit_due = true;
                }
            }
            HostMessage::FileResult { id, path } => self.file_result(&id, path, &mut out),
            HostMessage::Invoke { call, tab, tool, input, group } => {
                out.extend(self.invoke(call, &tab, &tool, input, &group));
            }
            HostMessage::Keep { tab } => {
                let kept = match self.images.iter().position(|image| image.id == tab) {
                    Some(index) if self.project.is_some() => self.write_into_project(index),
                    Some(index) => Ok(self.images[index].path.clone()),
                    None => Ok(None),
                };
                out.extend(self.declared());
                out.push(match kept {
                    Ok(path) => AppMessage::Kept { tab, path: path.map(|path| path.display().to_string()), error: None },
                    Err(error) => AppMessage::Kept { tab, path: None, error: Some(error) },
                });
            }
            HostMessage::Claim { id, path } => {
                let path = Path::new(&path);
                out.push(AppMessage::Claimed { id, yours: files::is_ipx(path) && self.in_project(path) });
            }
            _ => {}
        }
        if self.finish_writes(false) {
            out.extend(self.declared());
        }
        out
    }

    /// The loop's wake with no message: writes what is due inside a project,
    /// and sends what that changed, such as a tab's new name.
    pub fn tick(&mut self) -> Vec<AppMessage> {
        let mut out = self.catch_up();
        // Thumbnails left while a slider was held, which ended without a
        // message, such as when its step was written.
        if self.active_image().is_some_and(|image| image.thumbs_owed && !image.doc().held()) {
            out.extend(self.emit());
        }
        if self.project.is_some() {
            self.write_due(Instant::now());
        }
        if self.finish_writes(false) {
            out.extend(self.declared());
        }
        out
    }

    /// While the program is idle, before anything else it does then: the
    /// picture shown keeps the picture beneath its active layer, made now,
    /// so a slider's first value is as quick as the rest; every other
    /// picture lets its go. Returns whether one was made.
    pub fn keep_ahead(&mut self) -> bool {
        let active = self.active;
        for (index, image) in self.images.iter_mut().enumerate() {
            let shown = image.gallery.shown;
            for (position, frame) in image.gallery.frames.iter_mut().enumerate() {
                if index != active || position != shown {
                    frame.doc.let_beneath_go();
                }
            }
        }
        self.images.get_mut(active).is_some_and(|image| image.doc_mut().keep_ahead())
    }

    /// Whether the picture shown has a picture beneath to make when idle.
    pub fn ahead_due(&self) -> bool {
        self.active_image().is_some_and(|image| image.doc().ahead_due())
    }

    /// Writes whole every picture file a stroke left behind, while nothing
    /// is being drawn, and tells the host the picture shown is whole again.
    fn catch_up(&mut self) -> Vec<AppMessage> {
        if self.images.iter().any(|image| image.gallery.frames.iter().any(|frame| frame.doc.drawing())) {
            return Vec::new();
        }
        let cache = self.cache.clone();
        let mut shown = false;
        for (index, image) in self.images.iter_mut().enumerate() {
            let shown_uid = image.frame().uid;
            for frame in &mut image.gallery.frames {
                if !frame.behind {
                    continue;
                }
                let picture = cache.join(format!("{}-{}.rgba", image.id, frame.uid));
                if files::write_atomic(&picture, frame.doc.composite()).is_ok() {
                    caught_up(frame);
                    shown |= index == self.active && frame.uid == shown_uid;
                }
            }
        }
        if !shown {
            return Vec::new();
        }
        self.raster_view(self.active, false).map(AppMessage::Raster).into_iter().collect()
    }

    /// Inside a project, writes every image not yet written, as when the
    /// host has gone. Outside one, the scratch pictures are removed.
    pub fn write_all(&mut self) {
        if self.project.is_some() {
            for index in 0..self.images.len() {
                let _ = self.write_into_project(index);
            }
        } else {
            let _ = std::fs::remove_dir_all(&self.cache);
        }
    }

    fn in_project(&self, path: &Path) -> bool {
        let Some(project) = &self.project else {
            return true;
        };
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        path.starts_with(project)
    }

    pub fn image_index(&self, tab: &str) -> Option<usize> {
        self.images.iter().position(|image| image.id == tab)
    }

    fn activate(&mut self, tab: &str) {
        if let Some(index) = self.image_index(tab) {
            self.active = index;
        }
    }

    pub fn active_image(&self) -> Option<&Image> {
        self.images.get(self.active)
    }

    /// The picture, not the filmstrip, has the keyboard again.
    fn picture_focus(&mut self) {
        if let Some(image) = self.images.get_mut(self.active) {
            image.strip_focus = false;
        }
    }

    fn push_image(&mut self, mut gallery: Gallery, path: Option<PathBuf>, label: Option<String>) -> usize {
        let id = self.next_tab.to_string();
        self.next_tab += 1;
        let label = label
            .or_else(|| path.as_deref().map(file_title))
            .unwrap_or_else(|| String::from(UNTITLED));
        let frame = gallery.frame_mut();
        frame.camera = Some(fit(frame.doc.picture.width, frame.doc.picture.height, self.viewport));
        self.images.push(Image {
            id,
            label,
            path,
            gallery,
            dirty: false,
            strip_focus: false,
            unwritten_since: None,
            write_failed: None,
            saving: None,
            send_camera: true,
            thumbs_owed: false,
        });
        self.active = self.images.len() - 1;
        if self.tools.tool == ToolKind::Crop {
            self.start_crop();
        }
        self.active
    }

    /// A tab made after the greeting declares its contents and panel, then
    /// everything else.
    fn declare_new(&mut self, out: &mut Vec<AppMessage>) {
        let tab = self.images[self.active].id.clone();
        out.push(AppMessage::Contents { tab: tab.clone(), kind: ContentsKind::Raster });
        out.push(AppMessage::Panel { tab, side: PanelSide::Right });
        out.extend(self.emit());
    }

    /// Runs `change` on the shown picture's document, and marks the image
    /// changed when it was.
    fn change<T>(&mut self, change: impl FnOnce(&mut Doc) -> T) -> Option<T> {
        let image = self.images.get_mut(self.active)?;
        let before = image.doc().edits;
        let result = change(image.doc_mut());
        if image.doc().edits != before {
            image.dirty = true;
        }
        Some(result)
    }

    /// Runs `change` on the active tab's gallery: the strip shows what it
    /// changed, and its new picture gets its camera.
    fn film<T>(&mut self, change: impl FnOnce(&mut Gallery) -> T) -> Option<T> {
        let index = self.active;
        let result = self.film_of(index, change);
        if self.tools.tool == ToolKind::Crop && self.images.get(index).is_some_and(|image| image.frame().crop.is_none()) {
            self.start_crop();
        }
        result
    }

    fn film_of<T>(&mut self, index: usize, change: impl FnOnce(&mut Gallery) -> T) -> Option<T> {
        let in_project = self.project.is_some();
        let image = self.images.get_mut(index)?;
        // A held slider on the picture shown becomes its step first, and a
        // text being typed in its own.
        image.doc_mut().end_typing();
        image.doc_mut().finish_edit();
        let (edits, shown) = (image.gallery.edits, image.frame().uid);
        let result = change(&mut image.gallery);
        // Which picture is shown is kept too, but only a project's file is
        // written for it by itself: outside one, it asks nothing on close.
        if image.gallery.edits != edits || (in_project && image.frame().uid != shown) {
            image.dirty = true;
        }
        if image.frame().uid != shown {
            image.send_camera = true;
        }
        Some(result)
    }

    fn command(&mut self, id: &str, tab: Option<&str>, arg: Value, out: &mut Vec<AppMessage>) {
        let number = |key: &str| arg.get(key).and_then(Value::as_f64);
        let node = arg.get("node").and_then(Value::as_u64);
        match id {
            "new_image" => {
                self.push_image(new_gallery(), None, None);
                self.declare_new(out);
                return;
            }
            "open" => {
                self.ask_file(FileKind::Open, None, Purpose::Open, image_filter(true), out);
                return;
            }
            "open_layer" => {
                self.ask_file(FileKind::Open, None, Purpose::OpenLayer, image_filter(false), out);
                return;
            }
            "export_png" | "export_jpeg" => {
                let jpeg = id == "export_jpeg";
                let Some(image) = self.active_image() else {
                    return;
                };
                // The picture shown is exported, under its own name.
                let name = format!("{}.{}", image.frame().name, if jpeg { "jpg" } else { "png" });
                let filter = if jpeg {
                    FileFilter { name: "JPEG".into(), extensions: vec!["jpg".into(), "jpeg".into()] }
                } else {
                    FileFilter { name: "PNG".into(), extensions: vec!["png".into()] }
                };
                self.ask_file(FileKind::Save, Some(name), Purpose::Export { jpeg }, filter, out);
                return;
            }
            "save" | "save_as" if self.project.is_some() => {
                let _ = self.write_into_project(self.active);
            }
            "save" | "save_as" => {
                let Some(image) = self.active_image() else {
                    return;
                };
                match (&image.path, id) {
                    (Some(path), "save") => {
                        let path = path.clone();
                        self.write_at(self.active, &path, false, out);
                    }
                    _ => {
                        let purpose = Purpose::Save { tab: image.id.clone(), then_quit: false };
                        let name = format!("{}.ipx", image.label);
                        self.ask_file(FileKind::Save, Some(name), purpose, ipx_filter(), out);
                    }
                }
                return;
            }
            // While the strip has the keyboard, Undo and Redo act on its
            // own history; otherwise on the picture shown, and only it.
            "undo" | "redo" if self.active_image().is_some_and(|image| image.strip_focus) => {
                self.film(if id == "undo" { Gallery::undo } else { Gallery::redo });
            }
            "undo" | "redo" => {
                let size = self.active_image().map(|image| (image.doc().picture.width, image.doc().picture.height));
                self.change(if id == "undo" { Doc::undo } else { Doc::redo });
                // A crop undone or redone: the box starts again from the
                // whole picture.
                let now = self.active_image().map(|image| (image.doc().picture.width, image.doc().picture.height));
                if self.tools.tool == ToolKind::Crop && size != now {
                    self.start_crop();
                }
            }
            // A press on the filmstrip: it has the keyboard.
            "film_focus" => {
                if let Some(image) = self.images.get_mut(self.active) {
                    image.strip_focus = true;
                }
            }
            // Choosing a picture shows it with the keyboard: Undo is its
            // own, as the person who clicks a thumbnail and presses Undo
            // expects. The +, a thumbnail's menu, and a drag leave the
            // keyboard with the strip.
            "film_select" => {
                if let Some(index) = number("value") {
                    self.film(|gallery| gallery.select(index.max(0.0) as usize));
                }
                self.picture_focus();
            }
            "film_reorder" => {
                if let (Some(from), Some(to)) = (number("from"), number("to")) {
                    self.film(|gallery| gallery.move_to(from.max(0.0) as usize, to.max(0.0) as usize));
                }
            }
            "film_new" => {
                self.film(|gallery| gallery.add(UNTITLED, new_doc(), None));
            }
            "film_open" => {
                let Some(image) = self.active_image() else {
                    return;
                };
                let purpose = Purpose::OpenPicture { tab: image.id.clone() };
                self.ask_file(FileKind::Open, None, purpose, image_filter(false), out);
                return;
            }
            "film_duplicate" => {
                if let Some(index) = number("picture") {
                    self.film(|gallery| gallery.duplicate(index.max(0.0) as usize));
                }
            }
            "film_delete" => {
                if let Some(index) = number("picture") {
                    self.film(|gallery| gallery.remove(index.max(0.0) as usize));
                }
            }
            // The thumbnail's Rename row: the name typed in place comes as
            // `value`. A host that cannot take a name sends none; the person
            // is told how a picture is renamed instead.
            "film_rename" => {
                let Some(index) = number("picture") else {
                    return;
                };
                match arg.get("value").and_then(Value::as_str) {
                    Some(name) => {
                        let name = name.to_string();
                        if let Some(Err(reason)) = self.film(|gallery| gallery.rename(index.max(0.0) as usize, &name)) {
                            out.push(notice("The picture could not be renamed", &reason));
                        }
                    }
                    None => {
                        out.push(notice("Rename a picture", "Ask the assistant to rename this picture, and give it the new name."));
                        return;
                    }
                }
            }
            "close_tab" => {
                self.close_tab(tab, out);
                return;
            }
            "select_tab" => {}
            "tool" => {
                if let Some(tool) = arg.get("tool").and_then(Value::as_str).and_then(ToolKind::named) {
                    self.choose_tool(tool);
                }
            }
            "panel_page" => {
                self.page = match number("value").or_else(|| number("page")).map(|page| page as i64) {
                    Some(1) => Page::Properties,
                    Some(2) => Page::Adjustments,
                    _ => Page::Layers,
                };
            }
            "toggle_aligned" => {
                self.tools.aligned = !self.tools.aligned;
                self.tools.kept = None;
            }
            "text_style" => self.text_style(&arg),
            "add_mask" => {
                let target = node.or_else(|| self.active_image().and_then(|image| image.doc().active));
                if let Some(target) = target
                    && let Some(Err(reason)) = self.change(|doc| doc.add_mask(target, None))
                {
                    out.push(notice("The mask could not be added", &reason));
                }
            }
            "mask_invert" => {
                if let Some(target) = node.or_else(|| self.active_image().and_then(|image| image.doc().active)) {
                    self.change(|doc| doc.invert_mask(target, None));
                }
            }
            "delete_mask" => {
                if let Some(node) = node {
                    self.change(|doc| doc.delete_mask(node, None));
                }
            }
            "add_adjustment" => {
                let kind = match arg.get("kind").and_then(Value::as_str) {
                    Some("curves") => Kind::Curves(Curves::default()),
                    _ => Kind::Grade(Grade::default()),
                };
                if let Some(Ok(_)) = self.change(|doc| doc.add_adjustment(kind, None, None)) {
                    self.page = Page::Properties;
                    self.curve_channel = 0;
                    self.curve_point = None;
                }
            }
            "reset_adjustment" => {
                if let Some(active) = self.active_image().and_then(|image| image.doc().active) {
                    self.change(|doc| doc.reset_adjustment(active));
                    self.curve_point = None;
                }
            }
            "set_brush" => {
                let tool = arg.get("tool").and_then(Value::as_str);
                let settings = match tool {
                    Some("eraser") => &mut self.tools.eraser,
                    Some("stamp") => &mut self.tools.stamp,
                    Some("heal") => &mut self.tools.heal,
                    _ => &mut self.tools.brush,
                };
                if let (Some(setting), Some(value)) = (arg.get("setting").and_then(Value::as_str), number("value")) {
                    let value = value.round().max(0.0) as u32;
                    match setting {
                        "size" => settings.size = value.clamp(1, 1000),
                        "hardness" => settings.hardness = value.min(100),
                        "opacity" => settings.opacity = value.clamp(1, 100),
                        _ => {}
                    }
                }
                // A held slider sends its value many times; the toolbar is
                // declared again when it is let go.
                if arg.get("live").and_then(Value::as_bool) == Some(true) {
                    out.extend(self.raster_only());
                    return;
                }
            }
            // A brush slider let go: the toolbar shows its value again.
            "finish_brush" => {}
            "toggle_pressure" => {
                let eraser = arg.get("tool").and_then(Value::as_str) == Some("eraser");
                let settings = if eraser { &mut self.tools.eraser } else { &mut self.tools.brush };
                settings.pressure = !settings.pressure;
            }
            "size_down" | "size_up" => {
                let settings = match self.tools.tool {
                    ToolKind::Eraser => &mut self.tools.eraser,
                    ToolKind::Stamp => &mut self.tools.stamp,
                    ToolKind::Heal => &mut self.tools.heal,
                    _ => &mut self.tools.brush,
                };
                let step = (settings.size / 10).max(1);
                settings.size = if id == "size_up" { (settings.size + step).min(1000) } else { settings.size.saturating_sub(step).max(1) };
            }
            "set_color" => {
                if let Some(rgb) = arg.get("value").and_then(rgb_of) {
                    self.tools.rgb = rgb;
                }
            }
            "select_mode" => {
                if let Some(index) = number("value") {
                    self.tools.combine = Combine::ALL[(index.max(0.0) as usize).min(3)];
                }
            }
            "feather" => {
                if let Some(value) = number("value") {
                    self.tools.feather = value.clamp(0.0, 1000.0);
                }
            }
            "deselect" => {
                self.change(Doc::deselect);
            }
            "invert" => {
                self.change(Doc::invert);
            }
            "crop_ratio" => {
                if let Some(index) = number("value") {
                    self.tools.ratio = (index.max(0.0) as usize).min(declare::RATIOS.len() - 1);
                    if let Some(rect) = self.active_image().and_then(|image| image.frame().crop) {
                        self.set_crop(rect);
                    }
                }
            }
            // Enter and Escape mean these only while cropping.
            "crop_apply" | "crop_cancel" if self.tools.tool != ToolKind::Crop => return,
            "crop_apply" => self.apply_crop(),
            "crop_cancel" => {
                // Cancel drops the box; the crop tool shows the whole
                // picture's again, ready to drag.
                self.start_crop();
            }
            // A mask's row carries `mask`: its click chooses the mask, its
            // eye shows or hides it, and its name is its own.
            "layer_click" => {
                if let Some(node) = node {
                    if arg.get("mask").and_then(Value::as_bool) == Some(true) {
                        self.change(|doc| doc.choose_mask(node));
                    } else {
                        self.change(|doc| doc.activate(node));
                    }
                    self.curve_point = None;
                }
            }
            "layer_rename" => {
                if let (Some(node), Some(name)) = (node, arg.get("value").and_then(Value::as_str)) {
                    let name = name.to_string();
                    if arg.get("mask").and_then(Value::as_bool) == Some(true) {
                        self.change(|doc| doc.rename_mask(node, &name));
                    } else {
                        self.change(|doc| doc.rename(node, &name, None));
                    }
                }
            }
            "layer_visible" => {
                if let Some(node) = node {
                    let mask = arg.get("mask").and_then(Value::as_bool) == Some(true);
                    self.change(|doc| {
                        let found = doc.picture.find(node);
                        if mask {
                            let visible = found.and_then(|found| found.mask.as_ref()).is_some_and(|mask| mask.visible);
                            doc.set_mask_visible(node, !visible)
                        } else {
                            let visible = found.is_some_and(|found| found.visible);
                            doc.set_visible(node, !visible, None)
                        }
                    });
                }
            }
            "layer_lock" => {
                if let Some(node) = node {
                    self.change(|doc| doc.toggle_lock(node, 3));
                }
            }
            "reorder" => {
                if let (Some(node), Some(to)) = (node, number("value")) {
                    let to = to.max(0.0) as usize;
                    // With `parent`, null for the top level, the layer goes
                    // into that group or out of its own; without it, it
                    // stays among its siblings.
                    match arg.get("parent") {
                        Some(parent) => {
                            if parent.is_null() || parent.is_u64() {
                                let parent = parent.as_u64();
                                self.change(|doc| doc.move_node(node, parent, to, None));
                            }
                        }
                        None => {
                            self.change(|doc| doc.reorder(node, to, None));
                        }
                    }
                }
            }
            "add_layer" => {
                self.change(|doc| doc.add_layer("", None, None));
            }
            "add_group" => {
                self.change(Doc::group_active);
            }
            "delete_layer" => {
                let target = node.or_else(|| self.active_image().and_then(|image| image.doc().active));
                if let Some(target) = target {
                    self.change(|doc| doc.delete(target, None));
                }
            }
            "ungroup" => {
                if let Some(node) = node {
                    self.change(|doc| doc.ungroup(node));
                }
            }
            "finish_edit" => {
                self.change(Doc::finish_edit);
            }
            "toggle_panel" => self.panel_open = !self.panel_open,
            _ => return,
        }
        out.extend(self.emit());
    }

    pub fn choose_tool(&mut self, tool: ToolKind) {
        if self.tools.tool == tool {
            return;
        }
        if let Some(image) = self.images.get_mut(self.active) {
            image.doc_mut().end_typing();
        }
        self.tools.tool = tool;
        if tool == ToolKind::Crop {
            self.start_crop();
        } else if let Some(image) = self.images.get_mut(self.active) {
            image.frame_mut().crop = None;
        }
    }

    /// The crop tool starts with the whole picture's box, ready to drag.
    fn start_crop(&mut self) {
        if let Some(image) = self.images.get_mut(self.active) {
            let frame = image.frame_mut();
            frame.crop = Some([0.0, 0.0, f64::from(frame.doc.picture.width), f64::from(frame.doc.picture.height)]);
        }
    }

    /// Sets the crop box, kept inside the picture and to the chosen ratio.
    fn set_crop(&mut self, rect: [f64; 4]) {
        let ratio = declare::RATIOS[self.tools.ratio].1;
        let Some(image) = self.images.get_mut(self.active) else {
            return;
        };
        let image = image.frame_mut();
        if rect.iter().any(|value| !value.is_finite()) {
            return;
        }
        let width = f64::from(image.doc.picture.width);
        let height = f64::from(image.doc.picture.height);
        let x = rect[0].clamp(0.0, width - 1.0).round();
        let y = rect[1].clamp(0.0, height - 1.0).round();
        let mut w = rect[2].abs().min(width - x).max(1.0);
        let mut h = rect[3].abs().min(height - y).max(1.0);
        if let Some(ratio) = ratio {
            // The dimension the drag changed leads: the height when only it
            // changed, as when the top or bottom edge is dragged, and the
            // width otherwise. When the other would not fit, it leads.
            let height_leads = image
                .crop
                .is_some_and(|now| (rect[2].abs() - now[2]).abs() < 0.5 && (rect[3].abs() - now[3]).abs() >= 0.5);
            if height_leads {
                w = h * ratio;
                if w > width - x {
                    w = width - x;
                    h = w / ratio;
                }
            } else {
                h = w / ratio;
                if h > height - y {
                    h = height - y;
                    w = h * ratio;
                }
            }
        }
        image.crop = Some([x, y, w.round().max(1.0), h.round().max(1.0)]);
    }

    fn apply_crop(&mut self) {
        let Some(rect) = self.active_image().and_then(|image| image.frame().crop) else {
            return;
        };
        self.change(|doc| doc.crop(rect));
        self.start_crop();
    }

    /// The brush or eraser chosen, as the toolbar sets it.
    fn brush(&self) -> Option<Brush> {
        let tools = &self.tools;
        let (settings, erase) = match tools.tool {
            ToolKind::Brush => (tools.brush, false),
            ToolKind::Eraser => (tools.eraser, true),
            _ => return None,
        };
        Some(Brush {
            rgb: tools.rgb,
            size: f64::from(settings.size),
            hardness: f64::from(settings.hardness) / 100.0,
            opacity: f64::from(settings.opacity) / 100.0,
            pressure: settings.pressure,
            erase,
        })
    }

    /// The clone stamp or the healing brush chosen, as the toolbar sets it,
    /// with the offset of the stroke being drawn. `first`, the first point
    /// of a stroke beginning, sets that offset: kept from the last stroke
    /// when aligned, and otherwise from the source to the first point.
    /// None without a source.
    fn sampler(&mut self, first: Option<[f64; 2]>) -> Option<Sampler> {
        let tools = &mut self.tools;
        let heal = match tools.tool {
            ToolKind::Heal => true,
            ToolKind::Stamp => false,
            _ => return None,
        };
        let source = tools.source?;
        if let Some(first) = first {
            let aligned = heal || tools.aligned;
            tools.stroke = Some(match tools.kept {
                Some(kept) if aligned => kept,
                _ => [source[0] - first[0], source[1] - first[1]],
            });
        }
        let offset = tools.stroke?;
        let settings = if heal { tools.heal } else { tools.stamp };
        Some(Sampler {
            brush: Brush {
                rgb: [0, 0, 0],
                size: f64::from(settings.size),
                hardness: f64::from(settings.hardness) / 100.0,
                opacity: f64::from(settings.opacity) / 100.0,
                pressure: false,
                erase: false,
            },
            offset: [offset[0].round() as i32, offset[1].round() as i32],
            heal,
        })
    }

    /// The T tool's settings row changed: the style a new text takes, and
    /// the edited text's.
    fn text_style(&mut self, arg: &Value) {
        let value = arg.get("value");
        let number = value.and_then(Value::as_f64);
        let style = &mut self.tools.text;
        match arg.get("key").and_then(Value::as_str) {
            Some("face") => style.face = if number == Some(1.0) { Face::Mono } else { Face::Inter },
            Some("size") => style.size = number.map_or(style.size, |size| size as f32),
            Some("bold") => style.bold = !style.bold,
            Some("italic") => style.italic = !style.italic,
            Some("underline") => style.underline = !style.underline,
            Some("caps") => style.all_caps = !style.all_caps,
            Some("align") => {
                let index = arg.get("align").and_then(Value::as_u64).unwrap_or(0) as usize;
                style.align = Align::ALL[index.min(3)];
            }
            Some("line_height") => style.line_height = number.map_or(style.line_height, |value| value as f32),
            Some("letter_spacing") => style.letter_spacing = number.map_or(style.letter_spacing, |value| value as f32),
            Some("color") => {
                if let Some(rgb) = value.and_then(rgb_of) {
                    style.color = rgb;
                }
            }
            _ => return,
        }
        *style = style.clamped();
        let style = *style;
        self.change(|doc| {
            if doc.typing().is_some() {
                doc.set_typing_style(style);
            }
        });
    }

    /// What waited for the lines waiting to be read: a held slider's
    /// picture and panel, or the picture with the stroke being drawn, once
    /// its points have all been painted, only the picture, as nothing else
    /// changed.
    pub fn take_live(&mut self) -> Vec<AppMessage> {
        if self.emit_due {
            return self.emit();
        }
        if !std::mem::take(&mut self.live_due) {
            return Vec::new();
        }
        let view = self.raster_view(self.active, true);
        if self.leave_memory_if_full() {
            return self.emit();
        }
        view.map(AppMessage::Raster).into_iter().collect()
    }

    /// When a picture for the host could not be written in memory, as when
    /// the memory folder is full, the pictures move to the project's cache
    /// or a scratch folder on disk, to be written there whole. Returns
    /// whether they moved.
    fn leave_memory_if_full(&mut self) -> bool {
        if !std::mem::take(&mut self.display_failed) || !self.in_memory {
            return false;
        }
        let _ = std::fs::remove_dir_all(&self.cache);
        self.in_memory = false;
        self.cache = match &self.project {
            Some(folder) => folder.join(".inpainter").join("cache").join("image-editor"),
            None => scratch_folder(),
        };
        for image in &mut self.images {
            for frame in &mut image.gallery.frames {
                frame.written = None;
                frame.behind = false;
                frame.last_patch = None;
                frame.patches.clear();
                frame.thumbs.clear();
                frame.film = None;
            }
        }
        true
    }

    fn gesture(&mut self, gesture: RasterGesture) {
        let tools = self.tools.clone();
        match gesture {
            RasterGesture::Stroke { points } => {
                if let Some(brush) = self.brush() {
                    // A stroke on a locked or hidden layer changes nothing.
                    self.change(|doc| doc.stroke(&points, &brush));
                    return;
                }
                let first = if self.tools.stroke.is_none() { points.first().map(|point| [point[0], point[1]]) } else { None };
                let Some(sampler) = self.sampler(first) else {
                    return;
                };
                self.change(|doc| doc.sample_stroke(&points, &sampler));
                // Aligned, the offset holds for the next stroke too; not
                // aligned, each stroke starts again from the source.
                let aligned = sampler.heal || self.tools.aligned;
                self.tools.kept = if aligned { self.tools.stroke } else { None };
                self.tools.stroke = None;
            }
            // Taken off already, as every message but a stroke's does.
            RasterGesture::StrokeCancel => self.tools.stroke = None,
            RasterGesture::StrokePart { .. } | RasterGesture::DragPart { .. } => {}
            RasterGesture::Source { at } => {
                if at[0].is_finite() && at[1].is_finite() {
                    self.tools.source = Some(at);
                    self.tools.kept = None;
                    self.tools.stroke = None;
                }
            }
            RasterGesture::Click { at } => {
                if self.tools.tool == ToolKind::Text
                    && let Some(Some(style)) = self.change(|doc| doc.text_click(at, tools.text))
                {
                    self.tools.text = style;
                }
            }
            RasterGesture::TextBox { rect } => {
                self.change(|doc| doc.text_box(rect));
            }
            // With the T tool, a drag makes the box a new text wraps in.
            RasterGesture::Rect { from, to } if tools.tool == ToolKind::Text => {
                let width = (to[0] - from[0]).abs();
                let corner = [from[0].min(to[0]), from[1].min(to[1])];
                self.change(|doc| {
                    doc.end_typing();
                    if width >= 8.0 {
                        doc.start_text(corner, Some(width as f32), tools.text);
                    } else {
                        doc.text_click(from, tools.text);
                    }
                });
            }
            RasterGesture::Rect { from, to } => {
                self.change(|doc| {
                    let shape = Mask::rect(doc.picture.width, doc.picture.height, from, to);
                    doc.select(shape, tools.combine, tools.feather);
                });
            }
            RasterGesture::Lasso { points } => {
                self.change(|doc| {
                    let shape = Mask::lasso(doc.picture.width, doc.picture.height, &points);
                    doc.select(shape, tools.combine, tools.feather);
                });
            }
            RasterGesture::Drag { from, to } => {
                // With a selection, the selected pixels are set down; a
                // layer they cannot move on is refused, as its settings row
                // says.
                if self.active_image().is_some_and(|image| image.doc().selection.is_some()) {
                    self.change(|doc| doc.drop_pixels(from, to));
                    return;
                }
                self.change(Doc::end_live);
                let dx = (to[0] - from[0]).round();
                let dy = (to[1] - from[1]).round();
                if dx.is_finite() && dy.is_finite() {
                    self.change(|doc| doc.move_active(dx as i32, dy as i32));
                }
            }
            RasterGesture::Pick { at } => {
                if let Some(rgba) = self.images.get_mut(self.active).and_then(|image| image.doc_mut().pick(at)) {
                    self.tools.rgb = [rgba[0], rgba[1], rgba[2]];
                }
            }
            RasterGesture::Crop { rect } => self.set_crop(rect),
        }
    }

    /// A Layers or Properties page edit: blend mode, opacity, fill, a lock,
    /// a curve and its channel, a grade, or a mask's feather.
    fn property(&mut self, prop: &Value, value: &Value, preview: bool) {
        let Some(name) = prop.as_str() else {
            return;
        };
        let value = serde_json::from_value::<PropValue>(value.clone()).ok();
        let Some(active) = self.active_image().and_then(|image| image.doc().active) else {
            return;
        };
        match (name, &value) {
            ("curve_channel", Some(PropValue::Index(index))) => {
                self.curve_channel = usize::from(*index).min(3);
                self.curve_point = None;
                return;
            }
            ("curve", Some(PropValue::Curve(points))) => {
                let points: Vec<[f64; 2]> = points.iter().map(|(x, y)| [*x, *y]).collect();
                let channel = self.curve_channel;
                // The point moved is the one that differs from the curve
                // as it was, for the input and output under it.
                let before = self.active_image().and_then(|image| match &image.doc().picture.find(active)?.kind {
                    Kind::Curves(curves) => Some(curves.channel(channel).clone()),
                    _ => None,
                });
                if let Some(moved) = points.iter().find(|point| !before.as_ref().is_some_and(|before| before.contains(point))) {
                    self.curve_point = Some(*moved);
                }
                self.change(|doc| doc.set_curve(active, channel, &points, preview, None));
                return;
            }
            // A text's settings, from its Properties.
            (prop, Some(value)) if prop.starts_with("text_") => {
                let Some(mut style) = self.active_image().and_then(|image| match &image.doc().picture.find(active)?.kind {
                    Kind::Text(text) => Some(text.style),
                    _ => None,
                }) else {
                    return;
                };
                match (prop, value) {
                    ("text_face", PropValue::Index(index)) => style.face = if *index == 1 { Face::Mono } else { Face::Inter },
                    ("text_size", PropValue::Number(number)) => style.size = *number as f32,
                    ("text_color", PropValue::Rgb(rgb)) => style.color = *rgb,
                    ("text_toggles", PropValue::Index(index)) => match index {
                        0 => style.bold = !style.bold,
                        1 => style.italic = !style.italic,
                        2 => style.underline = !style.underline,
                        _ => style.all_caps = !style.all_caps,
                    },
                    ("text_align", PropValue::Index(index)) => style.align = Align::ALL[usize::from(*index).min(3)],
                    ("text_line_height", PropValue::Number(number)) => style.line_height = *number as f32,
                    ("text_spacing", PropValue::Number(number)) => style.letter_spacing = *number as f32,
                    _ => return,
                }
                self.change(|doc| doc.set_text_style(active, style, preview));
                return;
            }
            ("feather", Some(PropValue::Number(number))) => {
                let number = *number;
                self.change(|doc| doc.set_feather(active, number, preview, None));
                return;
            }
            (grade, Some(PropValue::Number(number))) if grade.starts_with("grade_") => {
                let number = *number as f32;
                let Some(mut settings) = self.active_image().and_then(|image| match &image.doc().picture.find(active)?.kind {
                    Kind::Grade(grade) => Some(*grade),
                    _ => None,
                }) else {
                    return;
                };
                match grade {
                    "grade_exposure" => settings.exposure = number,
                    "grade_contrast" => settings.contrast = number,
                    "grade_saturation" => settings.saturation = number,
                    "grade_temperature" => settings.temperature = number,
                    _ => return,
                }
                self.change(|doc| doc.set_grade(active, settings, preview, None));
                return;
            }
            _ => {}
        }
        let setting = match (name, value) {
            ("blend", Some(PropValue::Index(index))) => Blend::ALL.get(usize::from(index)).copied().map(Setting::Blend),
            ("opacity", Some(PropValue::Number(number))) => Some(Setting::Opacity(number)),
            ("fill", Some(PropValue::Number(number))) => Some(Setting::Fill(number)),
            ("locks", Some(PropValue::Index(index))) => {
                self.change(|doc| match doc.active {
                    Some(active) => doc.toggle_lock(active, usize::from(index).min(3)),
                    None => Ok(()),
                });
                return;
            }
            _ => None,
        };
        if let Some(setting) = setting {
            self.change(|doc| doc.set_setting(setting, preview));
        }
    }

    fn ask_file(&mut self, kind: FileKind, name: Option<String>, purpose: Purpose, filter: FileFilter, out: &mut Vec<AppMessage>) {
        let id = format!("file-{}", self.next_file);
        self.next_file += 1;
        self.file = Some((id.clone(), purpose));
        out.push(AppMessage::File {
            id,
            kind,
            name,
            folder: self.project.as_ref().map(|folder| folder.display().to_string()),
            filter: Some(filter),
        });
    }

    fn file_result(&mut self, id: &str, path: Option<String>, out: &mut Vec<AppMessage>) {
        let Some((asked, purpose)) = self.file.take() else {
            return;
        };
        if asked != id {
            self.file = Some((asked, purpose));
            return;
        }
        let Some(path) = path.map(PathBuf::from) else {
            if matches!(purpose, Purpose::Save { then_quit: true, .. }) || self.quitting {
                self.cancel_quit(out);
            }
            return;
        };
        match purpose {
            Purpose::Open => {
                if files::is_ipx(&path) && !self.in_project(&path) {
                    out.push(notice("This image is not in the project", "Only images inside the project's folder open here."));
                    return;
                }
                self.open_path(Some(path), out);
            }
            Purpose::OpenLayer => match files::import(&path) {
                Ok(pixels) => {
                    let name = file_title(&path);
                    let result = self.change(|doc| doc.add_image(&name, pixels));
                    if let Some(Err(reason)) = result {
                        out.push(notice("The picture could not be added", &reason));
                    }
                    out.extend(self.emit());
                }
                Err(reason) => out.push(notice("This picture could not be opened", &reason)),
            },
            Purpose::OpenPicture { tab } => match files::import(&path) {
                Ok(pixels) => {
                    let Some(index) = self.image_index(&tab) else {
                        return;
                    };
                    self.active = index;
                    let title = file_title(&path);
                    let doc = Doc::new(Picture::from_pixels(&title, pixels));
                    self.film(|gallery| gallery.add(&title, doc, None));
                    out.extend(self.emit());
                }
                Err(reason) => out.push(notice("This picture could not be opened", &reason)),
            },
            Purpose::Export { jpeg } => {
                let Some(image) = self.images.get_mut(self.active) else {
                    return;
                };
                let doc = image.doc_mut();
                let (width, height) = (doc.picture.width, doc.picture.height);
                let pixels = doc.composite();
                let encoded = if jpeg {
                    files::encode_jpeg(width, height, pixels)
                } else {
                    files::encode_png(width, height, pixels)
                };
                if let Err(reason) = encoded.and_then(|bytes| files::write_atomic(&path, &bytes)) {
                    out.push(notice("The picture could not be exported", &reason));
                }
            }
            Purpose::Save { tab, then_quit } => {
                if let Some(index) = self.image_index(&tab) {
                    self.write_at(index, &path, then_quit, out);
                }
            }
        }
    }

    /// Opens a file in a new tab: an image file as itself, or a PNG or JPEG
    /// as a new image of one layer. No path makes a new image.
    fn open_path(&mut self, path: Option<PathBuf>, out: &mut Vec<AppMessage>) {
        let Some(path) = path else {
            self.push_image(new_gallery(), None, None);
            self.declare_new(out);
            return;
        };
        if let Some(index) = self.images.iter().position(|image| image.path.as_deref() == Some(path.as_path())) {
            self.active = index;
            out.extend(self.emit());
            return;
        }
        if files::is_ipx(&path) {
            match files::read_ipx(&path) {
                Ok(read) => {
                    let pictures = read
                        .pictures
                        .into_iter()
                        .map(|read| {
                            let mut doc = Doc::new(read.picture);
                            if read.active.is_some() {
                                doc.active = read.active;
                            }
                            (read.name, doc)
                        })
                        .collect();
                    let mut gallery = Gallery::new(pictures, read.shown);
                    gallery.source = read.source;
                    self.push_image(gallery, Some(path), None);
                    self.declare_new(out);
                }
                Err(reason) => out.push(notice("This image could not be opened", &reason)),
            }
            return;
        }
        match files::import(&path) {
            Ok(pixels) => {
                let title = file_title(&path);
                let gallery = Gallery::one(&title, Doc::new(Picture::from_pixels(&title, pixels)));
                let index = self.push_image(gallery, None, Some(title));
                // It is new work, so it is kept in the project as an image.
                self.images[index].dirty = true;
                self.declare_new(out);
            }
            Err(reason) => out.push(notice("This picture could not be opened", &reason)),
        }
    }

    /// Opens a picture as new work, as a cell's Edit asks: a new tab called
    /// `name`, with one picture of one layer, never the file itself. Inside
    /// a project it is written as a new file, and keeps where it came from.
    fn open_as_new(&mut self, path: &Path, name: &str, from: Option<protocol::OpenedFrom>, out: &mut Vec<AppMessage>) {
        let name = safe_name(name);
        let name = if name.is_empty() { file_title(path) } else { name };
        match files::import(path) {
            Ok(pixels) => {
                let mut gallery = Gallery::one(&name, Doc::new(Picture::from_pixels(&name, pixels)));
                gallery.source = from;
                let index = self.push_image(gallery, None, Some(name));
                self.images[index].dirty = true;
                self.declare_new(out);
            }
            Err(reason) => out.push(notice("This picture could not be opened", &reason)),
        }
    }

    /// Starts writing every image whose change is a second old, and tries a
    /// failed one again after a while. The writer's thread encodes and
    /// writes, so painting never waits for it.
    fn write_due(&mut self, now: Instant) {
        for index in 0..self.images.len() {
            let image = &mut self.images[index];
            if !image.dirty {
                image.unwritten_since = None;
                continue;
            }
            let since = *image.unwritten_since.get_or_insert(now);
            if now.duration_since(since) < WRITE_AFTER {
                continue;
            }
            if image.write_failed.as_ref().is_some_and(|(_, failed)| now.duration_since(*failed) < RETRY_AFTER) {
                continue;
            }
            self.start_write(index);
        }
    }

    /// Hands one image to the writer's thread: its pictures as they are now,
    /// and the file it goes to. Not while it is being written already, or
    /// while a stroke is drawn on it.
    fn start_write(&mut self, index: usize) {
        let Some(folder) = self.project.clone() else {
            return;
        };
        // Nothing is copied for a write while a stroke is drawn or a slider
        // held anywhere: taking the picture would finish the held step and
        // split one drag into two undo steps.
        if self.images.iter().any(|image| image.gallery.frames.iter().any(|frame| frame.doc.drawing() || frame.doc.held())) {
            return;
        }
        let taken: Vec<PathBuf> = self
            .images
            .iter()
            .flat_map(|image| image.path.iter().chain(image.saving.as_ref().map(|(_, path)| path)))
            .cloned()
            .collect();
        let Some(image) = self.images.get_mut(index) else {
            return;
        };
        if image.saving.is_some() {
            return;
        }
        let path = match &image.path {
            Some(path) => path.clone(),
            None => match free_names(&image.label)
                .map(|name| folder.join(format!("{name}.ipx")))
                .find(|path| !path.exists() && !taken.contains(path))
            {
                Some(path) => path,
                None => return,
            },
        };
        let (pictures, shown) = snapshot(&mut image.gallery);
        let job = Job { tab: image.id.clone(), source: image.gallery.source.clone(), signature: signature(image), path: path.clone(), pictures, shown };
        image.saving = Some((job.signature, path));
        let writer = self.writer.get_or_insert_with(Writer::start);
        if writer.jobs.send(job).is_ok() {
            writer.waiting += 1;
        } else if let Some(image) = self.images.get_mut(index) {
            image.saving = None;
        }
    }

    /// Takes the writer's finished writes, waiting for every one with
    /// `block`, as a write that must come after them does. Returns whether
    /// any finished.
    fn finish_writes(&mut self, block: bool) -> bool {
        let mut any = false;
        loop {
            let Some(writer) = self.writer.as_mut() else {
                return any;
            };
            if writer.waiting == 0 {
                return any;
            }
            let done = if block {
                writer.done.recv().map_err(|_| true)
            } else {
                writer.done.try_recv().map_err(|error| error == std::sync::mpsc::TryRecvError::Disconnected)
            };
            let done = match done {
                Ok(done) => done,
                Err(false) => return any,
                // The writer stopped: what it held is unwritten, and is tried
                // again with a new one after a while.
                Err(true) => {
                    self.writer = None;
                    for image in &mut self.images {
                        if image.saving.take().is_some() {
                            image.write_failed = Some(("The image could not be written.".into(), Instant::now()));
                        }
                    }
                    return true;
                }
            };
            writer.waiting -= 1;
            any = true;
            let Some(image) = self.images.iter_mut().find(|image| image.id == done.tab) else {
                continue;
            };
            image.saving = None;
            match done.result {
                Ok(encoded) => {
                    keep(&mut image.gallery, encoded);
                    image.label = file_title(&done.path);
                    image.path = Some(done.path);
                    image.write_failed = None;
                    // Changed while it was written: written again soon.
                    if signature(image) == done.signature {
                        image.dirty = false;
                        image.unwritten_since = None;
                    }
                }
                Err(reason) => image.write_failed = Some((reason, Instant::now())),
            }
        }
    }

    /// Writes one image into the project's folder: to its own file, or, for
    /// one never written, to a new file named after it that replaces
    /// nothing. Returns the file, or none for an untouched image.
    fn write_into_project(&mut self, index: usize) -> Result<Option<PathBuf>, String> {
        // The writer's earlier writes land first, so none lands after this.
        self.finish_writes(true);
        let Some(folder) = self.project.clone() else {
            return Err("No project folder.".into());
        };
        let Some(image) = self.images.get_mut(index) else {
            return Ok(None);
        };
        if !image.dirty {
            return Ok(image.path.clone());
        }
        let path = match &image.path {
            Some(path) => path.clone(),
            None => match free_names(&image.label).map(|name| folder.join(format!("{name}.ipx"))).find(|path| !path.exists()) {
                Some(path) => path,
                None => return Err("No free file name.".into()),
            },
        };
        match write_gallery(&path, &mut image.gallery) {
            Ok(()) => {
                image.label = file_title(&path);
                image.path = Some(path.clone());
                image.dirty = false;
                image.unwritten_since = None;
                image.write_failed = None;
                Ok(Some(path))
            }
            Err(reason) => {
                image.write_failed = Some((reason.clone(), Instant::now()));
                Err(reason)
            }
        }
    }

    /// Outside a project: Save, or Save As, to `path`.
    fn write_at(&mut self, index: usize, path: &Path, then_quit: bool, out: &mut Vec<AppMessage>) {
        self.finish_writes(true);
        let Some(image) = self.images.get_mut(index) else {
            return;
        };
        if let Err(reason) = write_gallery(path, &mut image.gallery) {
            out.push(notice("This image could not be saved", &reason));
            if then_quit || self.quitting {
                self.cancel_quit(out);
            }
            return;
        }
        image.path = Some(path.to_path_buf());
        image.label = file_title(path);
        image.dirty = false;
        out.extend(self.declared());
        if self.closing {
            self.closing = false;
            self.remove_image(index, out);
        } else if then_quit {
            self.advance_quit(out);
        }
    }

    fn close_tab(&mut self, tab: Option<&str>, out: &mut Vec<AppMessage>) {
        let Some(index) = tab.and_then(|tab| self.image_index(tab)).or((!self.images.is_empty()).then_some(self.active)) else {
            return;
        };
        if self.project.is_some() {
            // Written first, so closing asks nothing. An image that could
            // not be written still asks.
            let _ = self.write_into_project(index);
        }
        if self.images[index].dirty {
            self.active = index;
            self.closing = true;
            self.quitting = false;
            self.quit_rest.clear();
            out.push(AppMessage::Ask(keep_ask()));
            return;
        }
        self.remove_image(index, out);
    }

    /// Removes a tab and its pictures for the host, then sends what the
    /// host now shows. The last tab closed leaves a new image, declared as
    /// any new tab is.
    fn remove_image(&mut self, index: usize, out: &mut Vec<AppMessage>) {
        let image = self.images.remove(index);
        // Its picture, and every thumbnail of its pictures and their layers.
        let own = format!("{}-", image.id);
        for entry in std::fs::read_dir(&self.cache).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&own) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        if self.images.is_empty() {
            self.push_image(new_gallery(), None, None);
            self.declare_new(out);
            return;
        }
        if self.active >= self.images.len() {
            self.active = self.images.len() - 1;
        }
        out.extend(self.emit());
    }

    fn note_quit(&mut self, out: &mut Vec<AppMessage>) {
        if self.quitting {
            return;
        }
        let dirty: Vec<String> = self.images.iter().filter(|image| image.dirty).map(|image| image.id.clone()).collect();
        let Some(first) = dirty.first() else {
            out.push(AppMessage::QuitReady);
            return;
        };
        self.quitting = true;
        self.activate(&first.clone());
        self.quit_rest = dirty[1..].to_vec();
        out.push(AppMessage::Ask(keep_ask()));
    }

    fn answer(&mut self, button: &str, out: &mut Vec<AppMessage>) {
        match button {
            "save" => {
                let Some(image) = self.active_image() else {
                    return;
                };
                match image.path.clone() {
                    Some(path) => self.write_at(self.active, &path, self.quitting, out),
                    None => {
                        let purpose = Purpose::Save { tab: image.id.clone(), then_quit: self.quitting };
                        let name = format!("{}.ipx", image.label);
                        self.ask_file(FileKind::Save, Some(name), purpose, ipx_filter(), out);
                    }
                }
            }
            "discard" => {
                if self.closing {
                    self.closing = false;
                    self.remove_image(self.active, out);
                } else if self.quitting {
                    self.advance_quit(out);
                }
            }
            _ => self.cancel_quit(out),
        }
    }

    fn advance_quit(&mut self, out: &mut Vec<AppMessage>) {
        if !self.quitting {
            return;
        }
        if self.quit_rest.is_empty() {
            self.quitting = false;
            out.push(AppMessage::QuitReady);
            return;
        }
        let next = self.quit_rest.remove(0);
        self.activate(&next);
        out.push(AppMessage::Ask(keep_ask()));
    }

    fn cancel_quit(&mut self, out: &mut Vec<AppMessage>) {
        self.quitting = false;
        self.closing = false;
        self.quit_rest.clear();
        out.extend(self.declared());
    }

    // What the host draws.

    /// The active image's picture and every declaration.
    pub fn emit(&mut self) -> Vec<AppMessage> {
        self.live_due = false;
        self.emit_due = false;
        let mut out = self.raster_only();
        if self.leave_memory_if_full() {
            out = self.raster_only();
        }
        out.extend(self.declared());
        out
    }

    /// The active image's picture, written for the host when it changed.
    /// While a slider is held only the picture is written; its thumbnails
    /// and the filmstrip's wait until it is let go.
    fn raster_only(&mut self) -> Vec<AppMessage> {
        let index = self.active;
        let Some(image) = self.images.get_mut(index) else {
            return Vec::new();
        };
        let held = image.doc().held();
        image.thumbs_owed = held;
        if !held {
            self.write_film(index);
        }
        let view = self.raster_view(index, false);
        if !held {
            self.write_thumbnails(index);
        }
        view.map(AppMessage::Raster).into_iter().collect()
    }

    /// What tells whether a held slider's value changed anything shown:
    /// the tab and its picture's edits, which of strip or picture has the
    /// keyboard, and the curve shown and its point.
    fn looks(&self) -> (usize, Option<(u64, u64, bool)>, usize, Option<[f64; 2]>) {
        let image = self.active_image().map(|image| (image.frame().uid, image.doc().edits, image.strip_focus));
        (self.active, image, self.curve_channel, self.curve_point)
    }

    /// The view of the picture shown. While a stroke is drawn (`live`), only
    /// the rows it changed are written, to a file of their own; the picture
    /// file is written whole at the next view that is not.
    fn raster_view(&mut self, index: usize, live: bool) -> Option<RasterView> {
        let tool = self.raster_tool();
        let move_tool = self.tools.tool == ToolKind::Move;
        let crop_tool = self.tools.tool == ToolKind::Crop;
        let cache = self.cache.clone();
        let viewport = self.viewport;
        let mut next = self.next_revision;
        let image = self.images.get_mut(index)?;
        let film = declare::film(image);
        let send_camera = std::mem::take(&mut image.send_camera);
        let image_id = image.id.clone();
        let frame = image.frame_mut();
        // Each picture has its own file, written again only when it
        // changed, so the host keeps every picture it has shown and a
        // picture shown again needs no reading. A change says which part
        // of the picture it touched, so the host reads only that.
        let picture = cache.join(format!("{image_id}-{}.rgba", frame.uid));
        frame.doc.refresh();
        let changed = frame.doc.take_changed();
        let mut patch = None;
        let revision = match frame.written {
            Some((made, told)) if made == frame.doc.revision => {
                // Until the picture file catches up, when the program is
                // idle, the host is told the picture is still being made.
                if frame.behind {
                    patch = frame.last_patch.clone();
                }
                told
            }
            earlier => {
                next += 1;
                let rect = changed
                    .map(|rect| [rect.x.max(0) as u32, rect.y.max(0) as u32, rect.w.max(0) as u32, rect.h.max(0) as u32])
                    .filter(|rect| rect[2] > 0 && rect[3] > 0);
                let rows = match (earlier, rect) {
                    (Some((_, since)), Some(rect)) if live => {
                        let path = cache.join(format!("{image_id}-{}-{next}.patch", frame.uid));
                        let width = frame.doc.picture.width as usize;
                        let composite = frame.doc.composite();
                        let mut bytes = Vec::with_capacity(rect[2] as usize * rect[3] as usize * 4);
                        for y in rect[1] as usize..(rect[1] + rect[3]) as usize {
                            let start = (y * width + rect[0] as usize) * 4;
                            bytes.extend_from_slice(&composite[start..start + rect[2] as usize * 4]);
                        }
                        files::write_atomic(&path, &bytes).ok().map(|()| (since, rect, path))
                    }
                    _ => None,
                };
                match rows {
                    Some((since, rect, path)) => {
                        patch = Some(protocol::RasterPatch { since, rect, file: Some(path.display().to_string()) });
                        frame.last_patch = patch.clone();
                        frame.behind = true;
                        // The host reads each in turn; a few are kept for a
                        // host that falls behind.
                        frame.patches.push_back(path);
                        while frame.patches.len() > KEPT_PATCHES {
                            if let Some(old) = frame.patches.pop_front() {
                                let _ = std::fs::remove_file(old);
                            }
                        }
                    }
                    None => {
                        if files::write_atomic(&picture, frame.doc.composite()).is_err() {
                            self.display_failed = true;
                        }
                        caught_up(frame);
                        if let (Some((_, since)), Some(rect)) = (earlier, rect) {
                            patch = Some(protocol::RasterPatch { since, rect, file: None });
                        }
                    }
                }
                frame.written = Some((frame.doc.revision, next));
                next
            }
        };
        self.next_revision = next;
        let (width, height) = (frame.doc.picture.width, frame.doc.picture.height);
        // A new picture is fitted; one shown again goes back where the host
        // last had it.
        let camera = if send_camera {
            let camera = match (&frame.camera, frame.host_camera) {
                (Some(camera), true) => camera.clone(),
                _ => fit(width, height, viewport),
            };
            frame.camera = Some(camera.clone());
            Some(camera)
        } else {
            None
        };
        if crop_tool {
            // A tab or picture chosen, or a crop undone, since the box was
            // set: the box starts again from the whole picture.
            let (width, height) = (f64::from(width), f64::from(height));
            let fits = frame.crop.is_some_and(|rect| rect[0] + rect[2] <= width && rect[1] + rect[3] <= height);
            if !fits {
                frame.crop = Some([0.0, 0.0, width, height]);
            }
        }
        let bounds = if move_tool {
            frame.doc.active.and_then(|active| frame.doc.picture.node_rect(active))
        } else {
            None
        };
        let text = frame.doc.editing().map(|editing| TextEditing { rect: editing.rect, caret: editing.caret, selection: editing.selection });
        Some(RasterView {
            tab: image_id,
            width,
            height,
            picture: picture.display().to_string(),
            revision,
            patch,
            tool,
            selection: frame.doc.outlines(),
            crop: if crop_tool { frame.crop } else { None },
            bounds: bounds.map(|rect| [f64::from(rect.x), f64::from(rect.y), f64::from(rect.w), f64::from(rect.h)]),
            camera,
            film: Some(film),
            text,
        })
    }

    pub fn raster_tool(&self) -> RasterTool {
        let tools = &self.tools;
        match tools.tool {
            ToolKind::Move => RasterTool::Move,
            ToolKind::Marquee => RasterTool::Marquee,
            ToolKind::Lasso => RasterTool::Lasso,
            ToolKind::Crop => RasterTool::Crop,
            ToolKind::Eyedropper => RasterTool::Eyedropper,
            ToolKind::Brush => RasterTool::Brush {
                rgb: tools.rgb,
                size: f64::from(tools.brush.size),
                hardness: f64::from(tools.brush.hardness) / 100.0,
                opacity: f64::from(tools.brush.opacity) / 100.0,
                pressure: tools.brush.pressure,
            },
            ToolKind::Eraser => RasterTool::Erase {
                size: f64::from(tools.eraser.size),
                hardness: f64::from(tools.eraser.hardness) / 100.0,
                opacity: f64::from(tools.eraser.opacity) / 100.0,
                pressure: tools.eraser.pressure,
            },
            ToolKind::Heal | ToolKind::Stamp => {
                let settings = if tools.tool == ToolKind::Heal { tools.heal } else { tools.stamp };
                RasterTool::Sampler { size: f64::from(settings.size), pressure: false, source: tools.source, offset: tools.stroke.or(tools.kept) }
            }
            ToolKind::Text => RasterTool::Text,
            ToolKind::Hand => RasterTool::Hand,
            ToolKind::Zoom => RasterTool::Zoom,
        }
    }

    /// Each layer's small picture, in the picture's frame, written when it
    /// changed.
    fn write_thumbnails(&mut self, index: usize) {
        let cache = self.cache.clone();
        let mut next = self.next_revision;
        let Some(image) = self.images.get_mut(index) else {
            return;
        };
        let tab = image.id.clone();
        let frame = image.frame_mut();
        let (width, height) = thumb_size(frame.doc.picture.width, frame.doc.picture.height);
        // A Color Grade layer shows the picture.
        let graded = frame.doc.picture.every_node().iter().any(|node| matches!(node.kind, Kind::Grade(_)) && !node.is_group());
        let shown = if graded {
            let (w, h) = (frame.doc.picture.width, frame.doc.picture.height);
            Some(files::resample(w, h, frame.doc.composite(), width, height))
        } else {
            None
        };
        let picture = &frame.doc.picture;
        // Each layer's pixels, each mask's grey, and each Curves layer's
        // curve; a text shows its icon instead.
        let mut wanted: Vec<(u64, Vec<u8>)> = Vec::new();
        for node in picture.every_node() {
            if let Some(mask) = &node.mask
                && let Some(pixels) = picture.pixels.get(&mask.id)
            {
                wanted.push((mask.id, thumbnail(picture.width, picture.height, node.offset, pixels, width, height)));
            }
            if node.is_group() {
                continue;
            }
            match &node.kind {
                Kind::Pixels => {
                    if let Some(pixels) = picture.pixels.get(&node.id) {
                        wanted.push((node.id, thumbnail(picture.width, picture.height, node.offset, pixels, width, height)));
                    }
                }
                Kind::Curves(curves) => wanted.push((node.id, curve_thumbnail(curves, width, height))),
                Kind::Grade(_) => wanted.extend(shown.clone().map(|bytes| (node.id, bytes))),
                Kind::Text(_) => {}
            }
        }
        frame.thumbs.retain(|id, _| wanted.iter().any(|(kept, _)| kept == id));
        for (id, bytes) in wanted {
            if frame.thumbs.get(&id).is_some_and(|(kept, _)| *kept == bytes) {
                continue;
            }
            // Layer ids repeat between pictures, so the picture is named too.
            let path = cache.join(format!("{tab}-{}-{id}.rgba", frame.uid));
            if files::write_atomic(&path, &bytes).is_err() {
                continue;
            }
            next += 1;
            let thumb = Thumbnail { path: path.display().to_string(), width, height, revision: next };
            frame.thumbs.insert(id, (bytes, thumb));
        }
        self.next_revision = next;
    }

    /// Each picture's filmstrip thumbnail, its composite drawn small,
    /// written when that picture changed.
    fn write_film(&mut self, index: usize) {
        let cache = self.cache.clone();
        let mut next = self.next_revision;
        let Some(image) = self.images.get_mut(index) else {
            return;
        };
        for frame in &mut image.gallery.frames {
            frame.doc.refresh();
            let revision = frame.doc.revision;
            if frame.film.as_ref().is_some_and(|(made, _, _)| *made == revision) {
                continue;
            }
            let (width, height) = (frame.doc.picture.width, frame.doc.picture.height);
            let (to_w, to_h) = film_size(width, height);
            let bytes = files::resample(width, height, frame.doc.composite(), to_w, to_h);
            if let Some((made, kept, _)) = &mut frame.film
                && *kept == bytes
            {
                *made = revision;
                continue;
            }
            let path = cache.join(format!("{}-film-{}.rgba", image.id, frame.uid));
            // A thumbnail that could not be written is tried again with the
            // next change; the strip keeps its place meanwhile.
            let written = files::write_atomic(&path, &bytes).is_ok();
            next += 1;
            let thumb = Thumbnail { path: path.display().to_string(), width: to_w, height: to_h, revision: next };
            frame.film = Some(if written { (revision, bytes, thumb) } else { (u64::MAX, Vec::new(), thumb) });
        }
        self.next_revision = next;
    }

    /// The toolbar, menu, keys, panel, and tab list.
    pub fn declared(&self) -> Vec<AppMessage> {
        let mut messages = vec![
            AppMessage::Toolbar(declare::toolbar(self)),
            AppMessage::Menu { rows: declare::menu(self) },
            AppMessage::Keys(declare::keys()),
        ];
        if let Some(properties) = declare::properties(self) {
            messages.push(AppMessage::Properties(properties));
        }
        if let Some(tabs) = declare::tabs(self) {
            messages.push(AppMessage::Tabs(tabs));
        }
        messages
    }

    /// An assistant's call, carried out on its tab, then its result.
    fn invoke(&mut self, call: String, tab: &str, tool: &str, input: Value, group: &str) -> Vec<AppMessage> {
        let Some(index) = self.image_index(tab) else {
            return vec![crate::assist::result(call, Err(format!("There is no image in tab {tab}.")))];
        };
        let name = protocol::split_tool_key(tool).map_or(tool, |(_, _, name)| name);
        let in_project = self.project.is_some();
        let image = &mut self.images[index];
        let (edits, shown, doc_edits) = (image.gallery.edits, image.frame().uid, image.doc().edits);
        let (outcome, look) = crate::assist::run(&mut image.gallery, name, &input, group);
        let shown_changed = image.frame().uid != shown;
        let changed = image.gallery.edits != edits || (!shown_changed && image.doc().edits != doc_edits);
        if changed || (in_project && shown_changed) {
            image.dirty = true;
        }
        if shown_changed {
            image.send_camera = true;
        }
        let mut out = Vec::new();
        if changed || shown_changed {
            // The tab the call changed shows its picture, whichever tab is
            // selected; the declarations stay the selected tab's.
            let active = self.active;
            self.active = index;
            out.extend(self.raster_only());
            self.active = active;
            out.extend(self.declared());
        }
        out.push(crate::assist::result_with(call, outcome, look));
        out
    }
}

impl Drop for Session {
    /// The scratch pictures, and the folder in memory, go with the session;
    /// a project's cache stays.
    fn drop(&mut self) {
        if self.project.is_none() || self.in_memory {
            let _ = std::fs::remove_dir_all(&self.cache);
        }
    }
}

/// A folder of this session's own in memory for the host's pictures,
/// `$XDG_RUNTIME_DIR/inpainter/image-editor/<process>-<number>`, when the
/// system has one: the pictures, rows, and thumbnails are written many
/// times a second while a slider is held, read by the host, and never
/// kept. Folders left by programs no longer running are removed first.
fn memory_folder() -> Option<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if !runtime.is_absolute() || !runtime.is_dir() {
        return None;
    }
    let parent = runtime.join("inpainter").join("image-editor");
    let processes = Path::new("/proc/self").exists();
    for entry in std::fs::read_dir(&parent).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let process = name.to_str().and_then(|name| name.split('-').next()).and_then(|id| id.parse::<u32>().ok());
        if processes && let Some(process) = process && !Path::new("/proc").join(process.to_string()).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let folder = parent.join(format!("{}-{number}", std::process::id()));
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}

/// A scratch folder of this session's own for the host's pictures while
/// there is no project, so two sessions never write one file.
fn scratch_folder() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("inpainter-image-editor-{}-{number}", std::process::id()))
}

/// Where the camera goes for a new or opened picture: the whole picture
/// with a margin, and no larger than its own pixels.
pub fn fit(width: u32, height: u32, viewport: [f64; 2]) -> DocumentCamera {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let zoom = ((viewport[0] - 2.0 * FIT_MARGIN) / w)
        .min((viewport[1] - 2.0 * FIT_MARGIN) / h)
        .clamp(0.01, 1.0);
    DocumentCamera {
        origin: [w / 2.0 - viewport[0] / (2.0 * zoom), h / 2.0 - viewport[1] / (2.0 * zoom)],
        zoom,
    }
}

/// A thumbnail's size: the picture's shape fitted into 56 × 40.
pub fn thumb_size(width: u32, height: u32) -> (u32, u32) {
    fit_into(width, height, THUMB_SIZE)
}

/// A filmstrip thumbnail's size: 44 tall at the picture's shape, unless it
/// is more than four times as wide as tall.
pub fn film_size(width: u32, height: u32) -> (u32, u32) {
    fit_into(width, height, FILM_SIZE)
}

fn fit_into(width: u32, height: u32, most: [u32; 2]) -> (u32, u32) {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = (f64::from(most[0]) / w).min(f64::from(most[1]) / h);
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// A new picture: white, 1920 × 1080.
pub fn new_doc() -> Doc {
    Doc::new(Picture::blank(NEW_SIZE[0], NEW_SIZE[1]))
}

/// A new tab's gallery: one new picture.
fn new_gallery() -> Gallery {
    Gallery::one(UNTITLED, new_doc())
}

/// The picture file holds the revision the host was told: the files of
/// rows sent while a stroke was drawn are not needed any more. The host
/// either holds that revision or reads the picture file whole.
fn caught_up(frame: &mut Frame) {
    frame.behind = false;
    frame.last_patch = None;
    for path in frame.patches.drain(..) {
        let _ = std::fs::remove_file(path);
    }
}

/// A name that is safe as a file's: path separators, control characters,
/// and line breaks become spaces, and runs of spaces one.
pub fn safe_name(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { ' ' } else { c }).collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.trim_start_matches('.').trim().to_string()
}

/// Writes a tab's gallery to its file, each held slider finished first.
fn write_gallery(path: &Path, gallery: &mut Gallery) -> Result<(), String> {
    let (pictures, shown) = snapshot(gallery);
    let encoded = files::write_ipx(path, pictures, shown, gallery.source.clone())?;
    keep(gallery, encoded);
    Ok(())
}

/// A gallery's pictures as they are now, each held slider finished first,
/// and which is shown.
fn snapshot(gallery: &mut Gallery) -> (Vec<files::Snapshot>, usize) {
    let pictures = gallery
        .frames
        .iter_mut()
        .map(|frame| {
            frame.doc.finish_edit();
            files::Snapshot::take(frame.uid, &frame.name, &frame.doc.picture, frame.doc.active, frame.doc.generation, &frame.pngs)
        })
        .collect();
    (pictures, gallery.shown)
}

/// Keeps what a write encoded, in each picture still in the gallery.
fn keep(gallery: &mut Gallery, encoded: Vec<files::Encoded>) {
    for encoded in encoded {
        if let Some(frame) = gallery.frames.iter_mut().find(|frame| frame.uid == encoded.uid) {
            frame.pngs.keep(encoded);
        }
    }
}

/// What an image is, as far as its file goes: its gallery's changes, its
/// pictures' changes, and the picture shown. A write made of an image with
/// the same signature holds everything in it.
type Signature = (u64, u64, u64);

fn signature(image: &Image) -> Signature {
    let edits = image.gallery.frames.iter().map(|frame| frame.doc.edits).sum();
    (image.gallery.edits, edits, image.frame().uid)
}

/// The thread that writes images inside a project, one at a time and in
/// order, so encoding and writing never hold up painting.
struct Writer {
    jobs: std::sync::mpsc::Sender<Job>,
    done: std::sync::mpsc::Receiver<Done>,
    /// Writes handed over and not yet taken back.
    waiting: usize,
}

struct Job {
    tab: String,
    source: Option<protocol::OpenedFrom>,
    signature: Signature,
    path: PathBuf,
    pictures: Vec<files::Snapshot>,
    shown: usize,
}

struct Done {
    tab: String,
    signature: Signature,
    path: PathBuf,
    result: Result<Vec<files::Encoded>, String>,
}

impl Writer {
    fn start() -> Self {
        let (jobs, incoming) = std::sync::mpsc::channel::<Job>();
        let (finished, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for job in incoming {
                let result = files::write_ipx(&job.path, job.pictures, job.shown, job.source);
                if finished.send(Done { tab: job.tab, signature: job.signature, path: job.path, result }).is_err() {
                    break;
                }
            }
        });
        Self { jobs, done, waiting: 0 }
    }
}

/// The commands that leave the keyboard with the filmstrip once it has it:
/// its own, Undo and Redo, and those about the tab rather than its picture.
/// Every other command, as every gesture and panel edit, is the picture's.
fn keeps_focus(id: &str) -> bool {
    id.starts_with("film_")
        || id.starts_with("canvas.")
        || matches!(
            id,
            "undo" | "redo" | "select_tab" | "close_tab" | "new_image" | "open" | "save" | "save_as" | "export_png"
                | "export_jpeg" | "toggle_panel"
        )
}

/// A Curves layer's picture: its curve for all three channels, dark on
/// clear, across the frame.
fn curve_thumbnail(curves: &Curves, to_w: u32, to_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; to_w as usize * to_h as usize * 4];
    let points: Vec<crate::picture::CurvePoint> = curves.rgb.iter().map(|point| crate::picture::CurvePoint { x: point[0], y: point[1] }).collect();
    let row = |x: u32| {
        let value = crate::picture::curve_y(&points, f64::from(x) / f64::from(to_w.max(2) - 1));
        ((1.0 - value) * f64::from(to_h - 1)).round() as i32
    };
    for x in 0..to_w {
        let (here, next) = (row(x), row((x + 1).min(to_w - 1)));
        for y in here.min(next)..=here.max(next) {
            let at = ((y.clamp(0, to_h as i32 - 1) as u32 * to_w + x) * 4) as usize;
            out[at..at + 4].copy_from_slice(&[30, 30, 34, 255]);
        }
    }
    out
}

/// One layer drawn small in the picture's frame, clear where it does not
/// reach.
fn thumbnail(width: u32, height: u32, offset: [i32; 2], pixels: &Pixels, to_w: u32, to_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; to_w as usize * to_h as usize * 4];
    let samples = 3;
    for ty in 0..to_h {
        for tx in 0..to_w {
            let mut sum = [0u32; 4];
            for sy in 0..samples {
                for sx in 0..samples {
                    let px = ((f64::from(tx) + (f64::from(sx) + 0.5) / f64::from(samples)) * f64::from(width) / f64::from(to_w)) as i32;
                    let py = ((f64::from(ty) + (f64::from(sy) + 0.5) / f64::from(samples)) * f64::from(height) / f64::from(to_h)) as i32;
                    let lx = px - offset[0];
                    let ly = py - offset[1];
                    if lx < 0 || ly < 0 || lx >= pixels.width as i32 || ly >= pixels.height as i32 {
                        continue;
                    }
                    let at = (ly as usize * pixels.width as usize + lx as usize) * 4;
                    let alpha = u32::from(pixels.data[at + 3]);
                    for channel in 0..3 {
                        sum[channel] += u32::from(pixels.data[at + channel]) * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            if sum[3] > 0 {
                let at = (ty as usize * to_w as usize + tx as usize) * 4;
                for channel in 0..3 {
                    out[at + channel] = (sum[channel] / sum[3]) as u8;
                }
                out[at + 3] = (sum[3] / (samples * samples)) as u8;
            }
        }
    }
    out
}

fn rgb_of(value: &Value) -> Option<[u8; 3]> {
    let list = value.as_array()?;
    if list.len() != 3 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (slot, item) in rgb.iter_mut().zip(list) {
        *slot = u8::try_from(item.as_u64()?).ok()?;
    }
    Some(rgb)
}

fn image_filter(with_ipx: bool) -> FileFilter {
    let mut extensions = vec!["png".to_string(), "jpg".to_string(), "jpeg".to_string()];
    if with_ipx {
        extensions.push("ipx".to_string());
    }
    FileFilter { name: "Images".into(), extensions }
}

fn ipx_filter() -> FileFilter {
    FileFilter { name: "Image".into(), extensions: vec!["ipx".into()] }
}

fn notice(title: &str, line: &str) -> AppMessage {
    AppMessage::Ask(Ask {
        id: "image-notice".into(),
        title: title.into(),
        lines: vec![line.to_string()],
        buttons: vec![AskButton { id: "ok".into(), label: "OK".into() }],
        dismiss: Some("ok".into()),
    })
}

fn keep_ask() -> Ask {
    Ask {
        id: "keep-image".into(),
        title: "Keep this image?".into(),
        lines: vec!["The image has changes that are not saved.".into()],
        buttons: vec![
            AskButton { id: "save".into(), label: "Save".into() },
            AskButton { id: "discard".into(), label: "Discard".into() },
            AskButton { id: "cancel".into(), label: "Cancel".into() },
        ],
        dismiss: Some("cancel".into()),
    }
}

pub fn file_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| String::from(UNTITLED))
}

/// `name`, then the names after it: "Untitled Asset" gives "Untitled Asset",
/// "Untitled Asset 2", and on; "Beach 3" gives "Beach 3", "Beach 4".
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

/// One line of the protocol, and the lines to write back.
pub fn respond(session: &mut Session, line: &str) -> Vec<String> {
    let Ok(message) = protocol::parse_host_message(line) else {
        return Vec::new();
    };
    session.handle(message).iter().map(protocol::encode_app_message).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::ToolItem;
    use serde_json::json;

    fn command(session: &mut Session, id: &str, arg: Value) -> Vec<AppMessage> {
        session.handle(HostMessage::Command {
            id: id.into(),
            tab: Some(session.images[session.active].id.clone()),
            arg: Some(arg),
            node: None,
            viewport: [1200.0, 800.0],
        })
    }

    fn gesture(session: &mut Session, gesture: RasterGesture) -> Vec<AppMessage> {
        let tab = session.images[session.active].id.clone();
        session.handle(HostMessage::RasterGesture { tab, gesture })
    }

    fn last_raster(messages: &[AppMessage]) -> RasterView {
        messages
            .iter()
            .rev()
            .find_map(|message| match message {
                AppMessage::Raster(view) => Some(view.clone()),
                _ => None,
            })
            .expect("no raster message")
    }

    fn at(session: &mut Session, x: usize, y: usize) -> [u8; 4] {
        let image = &mut session.images[session.active];
        let width = image.doc().picture.width as usize;
        let composite = image.doc_mut().composite();
        let at = (y * width + x) * 4;
        [composite[at], composite[at + 1], composite[at + 2], composite[at + 3]]
    }

    fn stroke(session: &mut Session, from: [f64; 2], to: [f64; 2]) -> Vec<AppMessage> {
        gesture(session, RasterGesture::Stroke { points: vec![[from[0], from[1], 1.0], [to[0], to[1], 1.0]] })
    }

    /// Catches the tools reached through the protocol doing nothing: a
    /// lasso limits the brush, the eraser removes, the eyedropper takes the
    /// colour for the brush, a drag moves the layer, and the crop box
    /// waits for Apply, which crops.
    #[test]
    fn the_tools_work_through_the_protocol() {
        let mut session = Session::launch();
        let _ = session.greeting();
        command(&mut session, "set_color", json!({ "value": [200, 30, 40] }));
        command(&mut session, "tool", json!({ "tool": "lasso" }));
        let view = last_raster(&gesture(
            &mut session,
            RasterGesture::Lasso { points: vec![[100.0, 100.0], [300.0, 100.0], [300.0, 300.0], [100.0, 300.0]] },
        ));
        assert!(matches!(view.tool, RasterTool::Lasso));
        assert_eq!(view.selection.len(), 1, "the selection has no outline");
        command(&mut session, "tool", json!({ "tool": "brush" }));
        stroke(&mut session, [50.0, 200.0], [350.0, 200.0]);
        assert_eq!(at(&mut session, 200, 200), [200, 30, 40, 255]);
        assert_eq!(at(&mut session, 60, 200), [255, 255, 255, 255], "painted outside the lasso");
        command(&mut session, "deselect", json!({}));
        assert!(session.images[0].doc().selection.is_none());

        command(&mut session, "tool", json!({ "tool": "eraser" }));
        stroke(&mut session, [500.0, 500.0], [520.0, 500.0]);
        assert_eq!(at(&mut session, 510, 500)[3], 0, "the eraser left the pixel");

        command(&mut session, "tool", json!({ "tool": "eyedropper" }));
        command(&mut session, "set_color", json!({ "value": [1, 2, 3] }));
        gesture(&mut session, RasterGesture::Pick { at: [200.5, 200.5] });
        assert_eq!(session.tools.rgb, [200, 30, 40]);

        command(&mut session, "tool", json!({ "tool": "move" }));
        let view = last_raster(&gesture(&mut session, RasterGesture::Drag { from: [10.0, 10.0], to: [30.0, 5.0] }));
        assert_eq!(view.bounds, Some([20.0, -5.0, 1920.0, 1080.0]));

        command(&mut session, "tool", json!({ "tool": "crop" }));
        command(&mut session, "crop_ratio", json!({ "value": 4 }));
        let view = last_raster(&gesture(&mut session, RasterGesture::Crop { rect: [100.0, 50.0, 800.0, 100.0] }));
        // 16:9 from an 800 wide box makes it 450 tall.
        assert_eq!(view.crop, Some([100.0, 50.0, 800.0, 450.0]));
        assert_eq!((view.width, view.height), (1920, 1080), "the box cropped before Apply");
        let view = last_raster(&command(&mut session, "crop_apply", json!({})));
        assert_eq!((view.width, view.height), (800, 450));
        assert_eq!(std::fs::metadata(&view.picture).unwrap().len(), 800 * 450 * 4);
        let view = last_raster(&command(&mut session, "undo", json!({})));
        assert_eq!((view.width, view.height), (1920, 1080));
        assert_eq!(view.crop, Some([0.0, 0.0, 1920.0, 1080.0]));
    }

    /// Catches the Layers page's controls not reaching the layers: blend,
    /// a held opacity slider as one step, the locks row, and the list's
    /// add, group, rename, hide, reorder, and delete.
    #[test]
    fn the_layers_page_works_through_the_protocol() {
        let mut session = Session::launch();
        let _ = session.greeting();
        let background = session.images[0].doc().active.unwrap();
        let property = |session: &mut Session, prop: &str, value: PropValue, preview: bool| {
            session.handle(HostMessage::Property { prop: json!(prop), value: serde_json::to_value(value).unwrap(), preview })
        };
        property(&mut session, "blend", PropValue::Index(1), false);
        assert_eq!(session.images[0].doc().picture.nodes[0].blend, Blend::Multiply);
        for value in [0.8, 0.6, 0.4] {
            property(&mut session, "opacity", PropValue::Number(value), true);
        }
        command(&mut session, "finish_edit", json!({}));
        assert_eq!(session.images[0].doc().picture.nodes[0].opacity, 0.4);
        command(&mut session, "undo", json!({}));
        assert_eq!(session.images[0].doc().picture.nodes[0].opacity, 1.0, "the held slider was more than one step");
        assert_eq!(session.images[0].doc().picture.nodes[0].blend, Blend::Multiply);
        property(&mut session, "locks", PropValue::Index(1), false);
        assert!(session.images[0].doc().picture.nodes[0].locks.pixels);
        command(&mut session, "tool", json!({ "tool": "brush" }));
        stroke(&mut session, [10.0, 10.0], [40.0, 10.0]);
        assert_eq!(at(&mut session, 20, 10), [255, 255, 255, 255], "a pixel-locked layer took paint");

        command(&mut session, "add_layer", json!({}));
        let added = session.images[0].doc().active.unwrap();
        command(&mut session, "layer_rename", json!({ "node": added, "value": "Ink" }));
        command(&mut session, "layer_visible", json!({ "node": added }));
        command(&mut session, "reorder", json!({ "node": added, "value": 1 }));
        let messages = command(&mut session, "add_group", json!({}));
        let rows = messages
            .iter()
            .rev()
            .find_map(|message| match message {
                AppMessage::Properties(properties) => properties.layers.clone(),
                _ => None,
            })
            .unwrap()
            .rows;
        let names: Vec<(String, u32, bool)> = rows.iter().map(|row| (row.name.clone(), row.depth, row.visible)).collect();
        assert_eq!(
            names,
            [
                ("Background".to_string(), 0, true),
                ("Group 1".to_string(), 0, true),
                ("Ink".to_string(), 1, false),
            ]
        );
        assert!(rows[0].thumbnail.is_some() && rows[2].thumbnail.is_some());
        command(&mut session, "delete_layer", json!({}));
        assert!(session.images[0].doc().picture.find(added).is_none());
        assert!(session.images[0].doc().picture.find(background).is_some());
        command(&mut session, "undo", json!({}));
        assert!(session.images[0].doc().picture.find(added).is_some());
    }

    /// Catches the assistant's call answered without its picture or before
    /// the change it made is shown.
    #[test]
    fn an_invoke_shows_its_change_then_answers() {
        let mut session = Session::launch();
        let _ = session.greeting();
        let messages = session.handle(HostMessage::Invoke {
            call: "c1".into(),
            tab: "1".into(),
            tool: protocol::tool_key("image-editor", "image", "add_layer"),
            input: json!({ "name": "Notes" }),
            group: "r1".into(),
        });
        let AppMessage::ToolResult(result) = messages.last().unwrap() else { panic!("no result last") };
        assert!(result.ok, "{:?}", result.output);
        assert!(messages.iter().any(|message| matches!(message, AppMessage::Properties(_))));
        assert!(session.images[0].dirty);
        let messages = session.handle(HostMessage::Invoke {
            call: "c2".into(),
            tab: "1".into(),
            tool: protocol::tool_key("image-editor", "image", "look"),
            input: json!({}),
            group: "r1".into(),
        });
        let AppMessage::ToolResult(result) = messages.last().unwrap() else { panic!("no result last") };
        assert_eq!(result.images.len(), 1);
    }

    /// Catches a `keep` that never writes, Save offered in a project, a
    /// file outside the project claimed, and closing the last tab leaving
    /// the host with no declared tab.
    #[test]
    fn a_project_image_keeps_and_closes() {
        let folder = std::env::temp_dir().join(format!("image-editor-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let mut session = Session::launch();
        let _ = session.greeting();
        session.handle(HostMessage::Project { folder: folder.display().to_string() });
        let labels: Vec<String> = declare::menu(&session).into_iter().map(|row| row.label).collect();
        assert!(!labels.iter().any(|label| label.starts_with("Save")), "{labels:?}");
        command(&mut session, "add_layer", json!({}));
        let replies = session.handle(HostMessage::Keep { tab: "1".into() });
        let expected = folder.join("Untitled Asset.ipx").display().to_string();
        assert!(replies.iter().any(|reply| matches!(reply, AppMessage::Kept { path: Some(path), error: None, .. } if *path == expected)));
        assert_eq!(files::read_ipx(Path::new(&expected)).unwrap().pictures[0].picture.nodes.len(), 2);

        let outside = std::env::temp_dir().join(format!("image-editor-outside-{}.ipx", std::process::id()));
        std::fs::copy(&expected, &outside).unwrap();
        let replies = session.handle(HostMessage::Claim { id: "c".into(), path: outside.display().to_string() });
        assert!(replies.iter().any(|reply| matches!(reply, AppMessage::Claimed { yours: false, .. })));
        let _ = std::fs::remove_file(outside);

        let replies = command(&mut session, "close_tab", json!({}));
        assert!(replies.iter().any(|reply| matches!(reply, AppMessage::Contents { kind: ContentsKind::Raster, .. })));
        assert!(replies.iter().any(|reply| matches!(reply, AppMessage::Panel { .. })));
        assert_eq!(session.images.len(), 1);
        assert_ne!(session.images[0].id, "1");
        let replies = session.handle(HostMessage::Quit);
        assert!(replies.iter().any(|reply| matches!(reply, AppMessage::QuitReady)));
        let _ = std::fs::remove_dir_all(folder);
    }
    /// The settings row in the last toolbar sent.
    fn options(messages: &[AppMessage]) -> Vec<ToolItem> {
        messages
            .iter()
            .rev()
            .find_map(|message| match message {
                AppMessage::Toolbar(toolbar) => Some(toolbar.options.clone()),
                _ => None,
            })
            .expect("no toolbar was sent")
    }

    fn strong_labels(items: &[ToolItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|item| match item {
                ToolItem::Label { text, strong: true } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Catches a refused stroke or move that says nothing: the settings row
    /// says why after the tool's settings, and the stroke is still refused.
    #[test]
    fn the_settings_row_says_why_a_tool_cannot_change_the_layer() {
        let mut session = Session::launch();
        let _ = session.greeting();
        let messages = command(&mut session, "tool", json!({ "tool": "brush" }));
        assert!(strong_labels(&options(&messages)).is_empty(), "a reason shown for a layer that takes paint");
        let background = session.images[0].doc().active.unwrap();
        let messages = command(&mut session, "layer_lock", json!({ "node": background }));
        let row = options(&messages);
        assert_eq!(strong_labels(&row), ["Background is locked."]);
        assert!(matches!(row.last(), Some(ToolItem::Label { strong: true, .. })), "the reason is not after the settings");
        stroke(&mut session, [10.0, 10.0], [40.0, 10.0]);
        assert_eq!(at(&mut session, 20, 10), [255, 255, 255, 255], "a locked layer took paint");
        let messages = command(&mut session, "tool", json!({ "tool": "move" }));
        assert_eq!(strong_labels(&options(&messages)), ["Background is locked."]);
        command(&mut session, "add_group", json!({}));
        let group = session.images[0].doc().picture.nodes[0].id;
        let messages = command(&mut session, "layer_click", json!({ "node": group }));
        assert_eq!(strong_labels(&options(&messages)), ["Background is locked."], "moving a group with a locked layer gave no reason");
        let messages = command(&mut session, "tool", json!({ "tool": "eraser" }));
        assert_eq!(strong_labels(&options(&messages)), ["Choose a layer to paint on; a group holds layers."]);
    }

    /// Catches the Layers list's drop into or out of a group not reaching
    /// the picture, or a drop without `parent` leaving its siblings.
    #[test]
    fn a_reorder_with_a_parent_moves_a_layer_into_and_out_of_a_group() {
        let mut session = Session::launch();
        let _ = session.greeting();
        let background = session.images[0].doc().active.unwrap();
        command(&mut session, "add_layer", json!({}));
        let top = session.images[0].doc().active.unwrap();
        command(&mut session, "add_group", json!({}));
        let group = session.images[0].doc().picture.place(top).unwrap().parent.unwrap();
        command(&mut session, "reorder", json!({ "node": background, "value": 0, "parent": group }));
        assert_eq!(session.images[0].doc().picture.place(background).unwrap().parent, Some(group));
        command(&mut session, "reorder", json!({ "node": background, "value": 1, "parent": null }));
        let place = session.images[0].doc().picture.place(background).unwrap();
        assert_eq!((place.parent, place.index), (None, 0), "the layer did not come out under the group");
        command(&mut session, "reorder", json!({ "node": background, "value": 0 }));
        let place = session.images[0].doc().picture.place(background).unwrap();
        assert_eq!((place.parent, place.index), (None, 1), "a reorder without a parent left the siblings");
    }

    /// Catches a ratio that only the width can lead: with a ratio chosen,
    /// dragging the bottom edge changes the height, and the width follows.
    #[test]
    fn a_crop_ratio_follows_whichever_edge_is_dragged() {
        let mut session = Session::launch();
        let _ = session.greeting();
        command(&mut session, "tool", json!({ "tool": "crop" }));
        command(&mut session, "crop_ratio", json!({ "value": 1 }));
        let view = last_raster(&gesture(&mut session, RasterGesture::Crop { rect: [100.0, 50.0, 400.0, 400.0] }));
        assert_eq!(view.crop, Some([100.0, 50.0, 400.0, 400.0]));
        // The bottom edge dragged down: only the height changed.
        let view = last_raster(&gesture(&mut session, RasterGesture::Crop { rect: [100.0, 50.0, 400.0, 600.0] }));
        assert_eq!(view.crop, Some([100.0, 50.0, 600.0, 600.0]), "dragging the bottom edge did nothing");
        // The top edge dragged up: the height leads again.
        let view = last_raster(&gesture(&mut session, RasterGesture::Crop { rect: [100.0, 0.0, 600.0, 650.0] }));
        assert_eq!(view.crop, Some([100.0, 0.0, 650.0, 650.0]));
        // The right edge: the width leads, as before.
        let view = last_raster(&gesture(&mut session, RasterGesture::Crop { rect: [100.0, 0.0, 500.0, 650.0] }));
        assert_eq!(view.crop, Some([100.0, 0.0, 500.0, 500.0]));
    }

    /// The action a declared control carries, sent as the host sends it.
    fn run(session: &mut Session, action: &protocol::Action) -> Vec<AppMessage> {
        command(session, &action.id, action.arg.clone().unwrap_or(Value::Null))
    }

    fn film_of(messages: &[AppMessage]) -> protocol::Film {
        last_raster(messages).film.expect("the raster view has no filmstrip")
    }

    fn layer_count(messages: &[AppMessage]) -> usize {
        messages
            .iter()
            .rev()
            .find_map(|message| match message {
                AppMessage::Properties(properties) => properties.layers.as_ref().map(|layers| layers.rows.len()),
                _ => None,
            })
            .expect("no Layers page was sent")
    }

    fn undo_enabled(messages: &[AppMessage]) -> bool {
        messages
            .iter()
            .rev()
            .find_map(|message| match message {
                AppMessage::Menu { rows } => rows.iter().find(|row| row.label == "Undo").map(|row| row.enabled),
                _ => None,
            })
            .expect("no menu was sent")
    }

    /// A click on a thumbnail, as the host sends it: `focus`, then `select`.
    fn click(session: &mut Session, film: &protocol::Film, index: usize) -> Vec<AppMessage> {
        run(session, &film.focus);
        run(session, &film.select.with_value(json!(index)))
    }

    /// Catches a filmstrip that drifts from the views or the pictures: every
    /// picture as an item with its name, the shown one, a thumbnail file of
    /// width × height × 4 bytes made from that picture's own composite, the
    /// menu's three rows with Delete off for the last picture, and the +
    /// tile's two rows.
    #[test]
    fn the_raster_view_carries_the_filmstrip() {
        let mut session = Session::launch();
        let film = film_of(&session.greeting());
        assert_eq!(film.items.len(), 1);
        assert_eq!(film.items[0].name, "Untitled Asset");
        let menu: Vec<(&str, bool)> = film.items[0].menu.iter().map(|row| (row.label.as_str(), row.enabled)).collect();
        assert_eq!(menu, [("Duplicate", true), ("Rename", true), ("Delete", false)], "the last picture can be deleted");
        assert!(film.items[0].menu[1].rename, "Rename does not rename in place");
        let add: Vec<&str> = film.add.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(add, ["New picture", "Open PNG or JPEG…"]);

        // Paint the first picture red, then add two more.
        command(&mut session, "set_color", json!({ "value": [220, 0, 0] }));
        command(&mut session, "tool", json!({ "tool": "brush" }));
        command(&mut session, "set_brush", json!({ "tool": "brush", "setting": "size", "value": 1000 }));
        for y in (0..1080).step_by(200) {
            stroke(&mut session, [0.0, f64::from(y)], [1920.0, f64::from(y)]);
        }
        run(&mut session, &film.add[0].action.clone().unwrap());
        let film = film_of(&run(&mut session, &film.add[0].action.clone().unwrap()));
        let names: Vec<&str> = film.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["Untitled Asset", "Untitled Asset 2", "Untitled Asset 3"]);
        assert_eq!(film.shown, 2, "the added picture is not shown");
        assert!(film.items.iter().all(|item| item.menu[2].enabled), "Delete is off with three pictures");
        let mut paths = std::collections::HashSet::new();
        for item in &film.items {
            let thumb = &item.thumbnail;
            assert_eq!((thumb.width, thumb.height), (78, 44));
            let bytes = std::fs::read(&thumb.path).expect("a thumbnail that is not there");
            assert_eq!(bytes.len(), 78 * 44 * 4);
            assert!(paths.insert(thumb.path.clone()), "two pictures share a thumbnail");
        }
        let red = std::fs::read(&film.items[0].thumbnail.path).unwrap();
        assert_eq!(&red[..4], [220, 0, 0, 255], "the first thumbnail is not its picture");
        let white = std::fs::read(&film.items[1].thumbnail.path).unwrap();
        assert!(white.chunks(4).all(|pixel| pixel == [255, 255, 255, 255]));
    }

    /// Catches a stroke that writes the whole picture for every part, and
    /// a picture file or row files left behind: while drawn, each view's
    /// rows are a file of their own, holding the composite's rows, and the
    /// picture file is untouched, even when the stroke ends; once idle, the
    /// picture file is written whole under the revision the host already
    /// shows, and the row files go.
    #[test]
    fn a_stroke_drawn_sends_only_its_rows_and_the_picture_catches_up() {
        let mut session = Session::launch();
        let first = last_raster(&session.greeting());
        command(&mut session, "tool", json!({ "tool": "brush" }));
        let tab = session.images[0].id.clone();
        let before = std::fs::read(&first.picture).unwrap();
        let points = [[100.0, 100.0, 1.0], [300.0, 140.0, 1.0], [500.0, 100.0, 1.0]];
        let mut views: Vec<RasterView> = Vec::new();
        for (from, point) in points.iter().enumerate() {
            session.handle(HostMessage::RasterGesture { tab: tab.clone(), gesture: RasterGesture::StrokePart { from, points: vec![*point] } });
            let view = last_raster(&session.take_live());
            let patch = view.patch.clone().expect("a part came without its rows");
            assert_eq!(patch.since, views.last().map_or(first.revision, |view| view.revision), "a part does not follow the one before");
            let file = patch.file.as_ref().expect("a part's rows are not a file of their own");
            let [x, y, w, h] = patch.rect.map(|value| value as usize);
            let rows = std::fs::read(file).unwrap();
            let composite = session.images[0].doc_mut().composite();
            let mut want = Vec::new();
            for row in y..y + h {
                want.extend_from_slice(&composite[(row * 1920 + x) * 4..(row * 1920 + x + w) * 4]);
            }
            assert!(rows == want, "part {from}'s rows are not the picture's");
            views.push(view);
        }
        assert_eq!(std::fs::read(&first.picture).unwrap(), before, "the whole picture was written while the stroke was drawn");
        let last = views.last().unwrap().clone();
        let end = last_raster(&gesture(&mut session, RasterGesture::Stroke { points: points.to_vec() }));
        assert_eq!(end.revision, last.revision, "the stroke's end was sent as a new picture");
        assert_eq!(std::fs::read(&first.picture).unwrap(), before, "the whole picture was written as the stroke ended");
        let caught = last_raster(&session.tick());
        assert_eq!((caught.revision, caught.patch.is_none()), (last.revision, true), "the idle program did not say the picture is whole");
        let image = &mut session.images[0];
        assert_eq!(std::fs::read(&caught.picture).unwrap(), image.doc_mut().composite(), "the picture file did not catch up");
        let cache = Path::new(&caught.picture).parent().unwrap();
        let rows = std::fs::read_dir(cache).unwrap().flatten().filter(|entry| entry.path().extension().is_some_and(|extension| extension == "patch")).count();
        assert_eq!(rows, 0, "files of rows were left behind");
    }

    /// Catches a write in the background that starts while the person
    /// works, loses a change, or lands after a later one: a due write waits
    /// for the program to be idle; a picture changed while its write is made
    /// stays unwritten; and the next write holds both changes.
    #[test]
    fn a_change_made_while_written_is_written_too() {
        let folder = scratch_folder();
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let mut session = Session::launch();
        let _ = session.greeting();
        session.handle(HostMessage::Project { folder: folder.display().to_string() });
        command(&mut session, "tool", json!({ "tool": "brush" }));
        stroke(&mut session, [100.0, 100.0], [300.0, 100.0]);
        // Due, a write waits for the program to be idle, never a message.
        session.images[0].unwritten_since = Some(Instant::now() - WRITE_AFTER * 2);
        command(&mut session, "add_layer", json!({}));
        command(&mut session, "undo", json!({}));
        assert!(session.images[0].saving.is_none(), "a write was started while the person works");
        session.tick();
        assert!(session.images[0].saving.is_some(), "the write did not go to the writer");
        stroke(&mut session, [100.0, 300.0], [300.0, 300.0]);
        session.finish_writes(true);
        assert!(session.images[0].dirty, "a change made while writing was taken as written");
        let path = session.images[0].path.clone().expect("the first write named no file");
        let read = files::read_ipx(&path).unwrap();
        let pixel = |picture: &Picture, y: usize| picture.pixels.values().next().unwrap().data[(y * 1920 + 200) * 4];
        assert_eq!(pixel(&read.pictures[0].picture, 100), 0);
        session.write_into_project(0).unwrap();
        assert!(!session.images[0].dirty);
        let read = files::read_ipx(&path).unwrap();
        assert_eq!((pixel(&read.pictures[0].picture, 100), pixel(&read.pictures[0].picture, 300)), (0, 0), "the second write lost a stroke");
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Catches a Properties page that only shows a text: its size and bold,
    /// changed there, change the chosen text layer, each one undo step, and
    /// the page shows the new values.
    #[test]
    fn a_texts_properties_edit_its_style() {
        let mut session = Session::launch();
        let _ = session.greeting();
        let text = session.images[0].doc_mut().add_text("Hi", [100, 100], None, crate::text::Style::default(), None).unwrap();
        session.images[0].doc_mut().activate(text);
        command(&mut session, "panel_page", json!({ "page": 1 }));
        let style = |session: &Session| match &session.images[0].doc().picture.find(text).unwrap().kind {
            Kind::Text(text) => text.style,
            _ => panic!("not a text"),
        };
        let set = |session: &mut Session, prop: &str, value: PropValue| {
            session.handle(HostMessage::Property { prop: json!(prop), value: serde_json::to_value(value).unwrap(), preview: false })
        };
        set(&mut session, "text_size", PropValue::Number(40.0));
        let messages = set(&mut session, "text_toggles", PropValue::Index(0));
        assert_eq!((style(&session).size, style(&session).bold), (40.0, true));
        let shown = messages.iter().find_map(|message| match message {
            AppMessage::Properties(properties) => Some(properties.clone()),
            _ => None,
        });
        let properties = shown.expect("the panel was not sent again");
        let rows: Vec<&protocol::Row> = properties.sections.iter().flat_map(|section| section.rows.iter()).collect();
        assert!(rows.iter().any(|row| row.prop == json!("text_size") && row.value == PropValue::Number(40.0)), "the page does not show the new size");
        command(&mut session, "undo", json!({}));
        assert!(!style(&session).bold, "bold was not its own step");
        assert_eq!(style(&session).size, 40.0);
        command(&mut session, "undo", json!({}));
        assert_eq!(style(&session).size, crate::text::Style::default().size, "the size was not its own step");
    }

    /// Catches a stroke taken off the picture while it is still drawn: the
    /// host's ping each second, a camera move, and a new viewport between
    /// its parts leave every part on the picture.
    #[test]
    fn a_stroke_held_past_a_ping_stays_on_the_picture() {
        let mut session = Session::launch();
        let _ = session.greeting();
        command(&mut session, "tool", json!({ "tool": "brush" }));
        let tab = session.images[0].id.clone();
        let part = |session: &mut Session, from: usize, points: Vec<[f64; 3]>| {
            session.handle(HostMessage::RasterGesture { tab: tab.clone(), gesture: RasterGesture::StrokePart { from, points } });
        };
        part(&mut session, 0, vec![[100.0, 100.0, 1.0], [200.0, 100.0, 1.0]]);
        session.handle(HostMessage::Ping);
        session.handle(HostMessage::Camera { tab: tab.clone(), origin: [0.0, 0.0], zoom: 0.5 });
        session.handle(HostMessage::Viewport { viewport: [900.0, 600.0] });
        part(&mut session, 2, vec![[300.0, 100.0, 1.0], [400.0, 100.0, 1.0]]);
        assert_eq!(at(&mut session, 150, 100)[..3], [0, 0, 0], "the stroke's first part was taken off");
        assert_eq!(at(&mut session, 350, 100)[..3], [0, 0, 0], "the stroke's later part was not painted");
    }

    /// Catches a thumbnail click that changes nothing, or changes the
    /// picture without its layers: the shown picture's file, size, and
    /// Layers page follow the click.
    #[test]
    fn a_click_on_a_thumbnail_shows_that_picture_and_its_layers() {
        let mut session = Session::launch();
        let film = film_of(&session.greeting());
        command(&mut session, "add_layer", json!({}));
        command(&mut session, "add_layer", json!({}));
        let messages = run(&mut session, &film.add[0].action.clone().unwrap());
        assert_eq!(layer_count(&messages), 1);
        let second = last_raster(&messages);
        let messages = click(&mut session, &film_of(&messages), 0);
        let first = last_raster(&messages);
        assert_eq!(first.film.as_ref().unwrap().shown, 0);
        assert_eq!(layer_count(&messages), 3, "the Layers page is still the other picture's");
        assert_ne!(first.picture, second.picture, "two pictures share one file, so the host reads each again");
        assert!(first.camera.is_some(), "the picture shown came without its camera");
        // A picture shown again is the file and revision the host already
        // holds, so switching reads nothing.
        let messages = click(&mut session, &film_of(&messages), 1);
        assert_eq!(layer_count(&messages), 1);
        let again = last_raster(&messages);
        assert_eq!((again.picture.as_str(), again.revision), (second.picture.as_str(), second.revision), "a picture shown again was sent again");
        assert_eq!(session.images[0].gallery.shown, 1);
    }

    /// Catches one picture's Undo reaching another: each picture keeps its
    /// own history, and Undo with the picture's keyboard acts only on the
    /// picture shown.
    #[test]
    fn each_picture_undoes_on_its_own() {
        let mut session = Session::launch();
        let film = film_of(&session.greeting());
        command(&mut session, "tool", json!({ "tool": "brush" }));
        stroke(&mut session, [10.0, 10.0], [100.0, 10.0]);
        let messages = run(&mut session, &film.add[0].action.clone().unwrap());
        assert!(!undo_enabled(&messages), "a new picture offers another's undo");
        stroke(&mut session, [10.0, 10.0], [100.0, 10.0]);
        stroke(&mut session, [10.0, 50.0], [100.0, 50.0]);
        // A click on a thumbnail, as the host sends it (a press, then the
        // choice), gives Undo to the picture it shows, never the strip.
        let film = film_of(&messages);
        click(&mut session, &film, 0);
        let messages = command(&mut session, "undo", json!({}));
        assert_eq!(session.images[0].gallery.frames.len(), 2, "Undo after a thumbnail click undid the strip");
        assert_eq!(at(&mut session, 50, 10), [255, 255, 255, 255], "the first picture's stroke was not undone");
        assert!(!undo_enabled(&messages));
        click(&mut session, &film, 1);
        command(&mut session, "undo", json!({}));
        assert_eq!(session.images[0].gallery.frames.len(), 2, "Undo after a thumbnail click undid the strip");
        assert_eq!(at(&mut session, 50, 50), [255, 255, 255, 255], "the second picture's last stroke was not undone");
        click(&mut session, &film, 0);
        let second = &mut session.images[0].gallery.frames[1].doc;
        let painted = |doc: &mut Doc, y: usize| doc.composite()[(y * 1920 + 50) * 4];
        assert_eq!(painted(second, 10), 0, "an undo on the first picture reached the second");
        assert_eq!(painted(second, 50), 255);
        assert_eq!(session.images[0].gallery.frames.len(), 2, "an undo on the picture undid the strip");
    }

    /// Catches the strip's Undo missing a change, acting with the picture's
    /// keyboard, or keeping the keyboard after the picture is used: after a
    /// press on the strip, Undo reverses add, duplicate, delete, rename, and
    /// reorder, and the picture shown follows; a gesture gives Undo back to
    /// the picture.
    #[test]
    fn the_strips_undo_reverses_its_changes() {
        let mut session = Session::launch();
        let film = film_of(&session.greeting());
        let names = |session: &Session| -> Vec<String> {
            session.images[0].gallery.frames.iter().map(|frame| frame.name.clone()).collect()
        };
        run(&mut session, &film.focus);
        let messages = run(&mut session, &film.add[0].action.clone().unwrap());
        let film = film_of(&messages);
        run(&mut session, &film.focus);
        let film = film_of(&run(&mut session, &film.items[1].menu[0].action.clone().unwrap()));
        let rename = film.items[0].menu[1].action.clone().unwrap().with_value(json!("Beach"));
        let film = film_of(&run(&mut session, &rename));
        // A drag of the first thumbnail to the end, as the host sends it.
        let film = film_of(&command(&mut session, &film.reorder.id, json!({ "from": 0, "to": 2 })));
        assert_eq!(names(&session), ["Untitled Asset 2", "Untitled Asset 2 copy", "Beach"]);
        let messages = run(&mut session, &film.items[1].menu[2].action.clone().unwrap());
        assert_eq!(names(&session), ["Untitled Asset 2", "Beach"]);
        assert!(undo_enabled(&messages));
        assert!(session.images[0].dirty);

        // The strip's Undo, step by step, with the shown picture following.
        let mut seen = Vec::new();
        for _ in 0..5 {
            command(&mut session, "undo", json!({}));
            let gallery = &session.images[0].gallery;
            seen.push((names(&session).join(", "), gallery.frame().name.clone()));
        }
        let want = [
            ("Untitled Asset 2, Untitled Asset 2 copy, Beach", "Untitled Asset 2 copy"),
            ("Beach, Untitled Asset 2, Untitled Asset 2 copy", "Beach"),
            ("Untitled Asset, Untitled Asset 2, Untitled Asset 2 copy", "Untitled Asset"),
            ("Untitled Asset, Untitled Asset 2", "Untitled Asset 2"),
            ("Untitled Asset", "Untitled Asset"),
        ];
        let want: Vec<(String, String)> = want.iter().map(|(all, shown)| (all.to_string(), shown.to_string())).collect();
        assert_eq!(seen, want);
        let messages = command(&mut session, "undo", json!({}));
        assert!(!undo_enabled(&messages), "the strip's Undo offers the picture's history");
        command(&mut session, "redo", json!({}));
        assert_eq!(session.images[0].gallery.frames.len(), 2);

        // A stroke gives the keyboard back: Undo takes the stroke, not the
        // strip's step. The brush is already chosen, so only the stroke
        // reaches the picture.
        assert_eq!(session.tools.tool, ToolKind::Brush);
        stroke(&mut session, [10.0, 10.0], [100.0, 10.0]);
        command(&mut session, "undo", json!({}));
        assert_eq!(session.images[0].gallery.frames.len(), 2, "Undo after a stroke undid the strip");
        assert_eq!(at(&mut session, 50, 10), [255, 255, 255, 255]);
        // So do a Layers page command and a panel edit.
        command(&mut session, "film_focus", json!({}));
        command(&mut session, "add_layer", json!({}));
        command(&mut session, "undo", json!({}));
        assert_eq!(session.images[0].gallery.frames.len(), 2, "Undo after a layer command undid the strip");
        assert_eq!(session.images[0].doc().picture.nodes.len(), 1);
        command(&mut session, "film_focus", json!({}));
        session.handle(HostMessage::Property { prop: json!("opacity"), value: serde_json::to_value(PropValue::Number(0.5)).unwrap(), preview: false });
        command(&mut session, "undo", json!({}));
        assert_eq!(session.images[0].gallery.frames.len(), 2, "Undo after a panel edit undid the strip");
        assert_eq!(session.images[0].doc().picture.nodes[0].opacity, 1.0);
    }

    /// Catches the last picture deleted through its menu or a stale click.
    #[test]
    fn the_last_picture_is_not_deleted() {
        let mut session = Session::launch();
        let _ = session.greeting();
        command(&mut session, "film_focus", json!({}));
        let messages = command(&mut session, "film_delete", json!({ "picture": 0 }));
        assert_eq!(session.images[0].gallery.frames.len(), 1);
        assert!(!undo_enabled(&messages), "a refused delete became a step");
        assert!(!film_of(&messages).items[0].menu[2].enabled);
    }

    /// Catches a gallery that is not kept whole in its one file: three
    /// pictures, each with its own layers and name, and the one shown, are
    /// written into the project and open again as they were.
    #[test]
    fn a_gallery_is_kept_in_its_file() {
        let folder = std::env::temp_dir().join(format!("image-editor-gallery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let mut session = Session::launch();
        let _ = session.greeting();
        session.handle(HostMessage::Project { folder: folder.display().to_string() });
        command(&mut session, "film_new", json!({}));
        command(&mut session, "add_layer", json!({}));
        command(&mut session, "film_duplicate", json!({ "picture": 1 }));
        command(&mut session, "add_layer", json!({}));
        command(&mut session, "film_rename", json!({ "picture": 0, "value": "Sky" }));
        command(&mut session, "film_select", json!({ "value": 1 }));
        let replies = session.handle(HostMessage::Keep { tab: "1".into() });
        let path = folder.join("Untitled Asset.ipx");
        assert!(replies.iter().any(|reply| matches!(reply, AppMessage::Kept { path: Some(_), error: None, .. })));

        let mut again = Session::restored();
        again.handle(HostMessage::Project { folder: folder.display().to_string() });
        let messages = again.handle(HostMessage::Open { path: Some(path.display().to_string()) });
        let film = film_of(&messages);
        let names: Vec<&str> = film.items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["Sky", "Untitled Asset 2", "Untitled Asset 2 copy"]);
        assert_eq!(film.shown, 1);
        assert_eq!(layer_count(&messages), 2);
        let layers: Vec<usize> = again.images[0].gallery.frames.iter().map(|frame| frame.doc.picture.nodes.len()).collect();
        assert_eq!(layers, [1, 2, 3]);
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Catches an assistant gallery call that the tab does not show, or that
    /// is not one step of the strip's history.
    #[test]
    fn the_assistants_gallery_calls_show_and_undo_one_by_one() {
        let mut session = Session::launch();
        let _ = session.greeting();
        let invoke = |session: &mut Session, tool: &str, input: Value| {
            session.handle(HostMessage::Invoke {
                call: "c".into(),
                tab: "1".into(),
                tool: protocol::tool_key("image-editor", "image", tool),
                input,
                group: "r1".into(),
            })
        };
        let messages = invoke(&mut session, "add_picture", json!({ "name": "Moon" }));
        let AppMessage::ToolResult(result) = messages.last().unwrap() else { panic!("no result last") };
        assert!(result.ok, "{:?}", result.output);
        assert_eq!(film_of(&messages).items[1].name, "Moon", "the strip does not show the added picture");
        invoke(&mut session, "duplicate_picture", json!({ "picture": 1 }));
        invoke(&mut session, "rename_picture", json!({ "picture": 0, "name": "Sun" }));
        invoke(&mut session, "move_picture", json!({ "picture": 0, "to": 2 }));
        invoke(&mut session, "remove_picture", json!({ "picture": 1 }));
        let messages = invoke(&mut session, "show_picture", json!({ "picture": 0 }));
        assert_eq!(film_of(&messages).shown, 0);
        let messages = invoke(&mut session, "list_pictures", json!({}));
        let AppMessage::ToolResult(result) = messages.last().unwrap() else { panic!("no result last") };
        assert_eq!(result.output["pictures"].as_array().unwrap().len(), 2);
        command(&mut session, "film_focus", json!({}));
        let mut lengths = Vec::new();
        while session.images[0].gallery.can_undo() {
            command(&mut session, "undo", json!({}));
            lengths.push(session.images[0].gallery.frames.len());
        }
        assert_eq!(lengths, [3, 3, 3, 2, 1], "the five calls are not five steps");
    }

    fn sampler_offset(view: &RasterView) -> Option<[f64; 2]> {
        match view.tool {
            RasterTool::Sampler { offset, .. } => offset,
            _ => panic!("the tool is not a sampler"),
        }
    }

    /// Catches Aligned ignored: with it on, the offset from the first stroke
    /// holds for the next, which copies from beside where it paints; with
    /// it off, each stroke starts again from the source. The declared
    /// offset follows it, and the crosshair's source stays.
    #[test]
    fn the_stamp_keeps_its_offset_when_aligned_and_starts_again_when_not() {
        let mut session = Session::launch();
        let _ = session.greeting();
        {
            let doc = session.images[0].doc_mut();
            let id = doc.active.unwrap();
            let pixels = doc.picture.pixels.get_mut(&id).unwrap();
            for y in 0..1080usize {
                for x in 0..400usize {
                    let colour = if x < 200 { [255, 0, 0, 255] } else if x >= 250 && x < 350 { [0, 0, 255, 255] } else { continue };
                    pixels.data[(y * 1920 + x) * 4..(y * 1920 + x) * 4 + 4].copy_from_slice(&colour);
                }
            }
            doc.add_layer("Retouch", None, None).unwrap();
        }
        command(&mut session, "tool", json!({ "tool": "stamp" }));
        let view = last_raster(&gesture(&mut session, RasterGesture::Source { at: [100.0, 100.0] }));
        assert!(matches!(view.tool, RasterTool::Sampler { source: Some([100.0, 100.0]), offset: None, .. }));
        let view = last_raster(&stroke(&mut session, [400.0, 100.0], [402.0, 100.0]));
        assert_eq!(at(&mut session, 401, 100), [255, 0, 0, 255], "the first stroke did not copy the source");
        assert_eq!(sampler_offset(&view), Some([-300.0, 0.0]), "aligned, the offset was not kept");
        stroke(&mut session, [600.0, 300.0], [602.0, 300.0]);
        assert_eq!(at(&mut session, 601, 300), [0, 0, 255, 255], "aligned, the second stroke did not keep the offset");

        command(&mut session, "toggle_aligned", json!({}));
        gesture(&mut session, RasterGesture::Source { at: [100.0, 100.0] });
        let view = last_raster(&stroke(&mut session, [400.0, 500.0], [402.0, 500.0]));
        assert_eq!(sampler_offset(&view), None, "not aligned, the offset stayed");
        stroke(&mut session, [700.0, 700.0], [702.0, 700.0]);
        assert_eq!(at(&mut session, 701, 700), [255, 0, 0, 255], "not aligned, the stroke did not start again from the source");
    }

    /// The audit's picture: an opaque patterned layer and translucent ones
    /// above it, then `grades` Color Grades and `curves` Curves, with the
    /// Properties page shown.
    fn patterned(width: u32, height: u32, layers: usize, grades: usize, curves: usize) -> Session {
        let mut picture = Picture::blank(width, height);
        for layer in 0..layers {
            let id = if layer == 0 {
                picture.nodes[0].id
            } else {
                let id = picture.take_id();
                picture.nodes.push(crate::picture::Node::layer(id, &format!("Layer {layer}")));
                id
            };
            let mut pixels = Pixels::clear(width, height);
            for y in 0..height {
                for x in 0..width {
                    let at = ((y * width + x) * 4) as usize;
                    let r = ((x * 173 / width + y * 51 / height + layer as u32 * 13) % 256) as u8;
                    let g = ((y * 219 / height + x * 17 / width) % 256) as u8;
                    let b = ((x + y) % 251) as u8;
                    pixels.data[at..at + 4].copy_from_slice(&[r, g, b, if layer == 0 { 255 } else { 96 }]);
                }
            }
            picture.pixels.insert(id, pixels);
        }
        let mut doc = Doc::new(picture);
        for index in 0..grades {
            doc.add_adjustment(Kind::Grade(Grade { exposure: 0.1 * index as f32, saturation: 5.0, ..Grade::default() }), None, None).unwrap();
        }
        for _ in 0..curves {
            doc.add_adjustment(Kind::Curves(Curves::default()), None, None).unwrap();
        }
        let mut session = Session::launch();
        *session.images[0].doc_mut() = doc;
        session.page = Page::Properties;
        session.emit();
        session
    }

    fn preview(session: &mut Session, prop: &str, value: PropValue) -> Vec<AppMessage> {
        session.handle(HostMessage::Property { prop: json!(prop), value: serde_json::to_value(value).unwrap(), preview: true })
    }

    fn rasters(messages: &[AppMessage]) -> usize {
        messages.iter().filter(|message| matches!(message, AppMessage::Raster(_))).count()
    }

    /// The exposure the last Properties page sent shows.
    fn shown_exposure(messages: &[AppMessage]) -> Option<f64> {
        messages.iter().rev().find_map(|message| match message {
            AppMessage::Properties(properties) => properties.sections.iter().flat_map(|section| section.rows.iter()).find(|row| row.prop == json!("grade_exposure")).and_then(|row| match row.value {
                PropValue::Number(number) => Some(number),
                _ => None,
            }),
            _ => None,
        })
    }

    fn exposure(session: &Session) -> f32 {
        let doc = session.images[0].doc();
        match &doc.picture.find(doc.active.unwrap()).unwrap().kind {
            Kind::Grade(grade) => grade.exposure,
            _ => panic!("not a Color Grade"),
        }
    }

    /// The Color Grade layer's thumbnail bytes, and the filmstrip's.
    fn thumbnails(session: &Session) -> (Vec<u8>, Vec<u8>) {
        let frame = session.images[0].frame();
        let grade = frame.doc.active.unwrap();
        (frame.thumbs[&grade].0.clone(), frame.film.as_ref().unwrap().1.clone())
    }

    /// Catches a held slider that sends the picture and panel again for
    /// every value: the values waiting are applied in order and sent once,
    /// showing the last; the thumbnails wait until the slider is let go;
    /// and the gesture is one step.
    #[test]
    fn a_held_slider_is_sent_once_for_the_values_waiting() {
        let mut session = patterned(64, 48, 2, 1, 0);
        let before = thumbnails(&session);
        let mut waiting = Vec::new();
        for step in 1..=8 {
            waiting.extend(preview(&mut session, "grade_exposure", PropValue::Number(f64::from(step) * 0.25)));
        }
        assert_eq!(rasters(&waiting), 0, "a value was sent by itself");
        assert_eq!(exposure(&session), 2.0, "the values were not applied in order");
        let sent = session.take_live();
        assert_eq!(rasters(&sent), 1);
        assert_eq!(shown_exposure(&sent), Some(2.0));
        assert!(session.take_live().is_empty(), "the values were sent twice");
        assert_eq!(thumbnails(&session), before, "a thumbnail was written while the slider was held");

        let finished = command(&mut session, "finish_edit", json!({}));
        assert_eq!(shown_exposure(&finished), Some(2.0));
        let after = thumbnails(&session);
        assert_ne!(after.0, before.0, "the layer's thumbnail was not written when the slider was let go");
        assert_ne!(after.1, before.1, "the filmstrip's thumbnail was not written when the slider was let go");
        command(&mut session, "undo", json!({}));
        assert_eq!(exposure(&session), 0.0, "the held slider was more than one step");
        command(&mut session, "undo", json!({}));
        assert!(session.images[0].doc().picture.find(session.images[0].doc().active.unwrap()).is_none_or(|node| !matches!(node.kind, Kind::Grade(_))), "undo took back less than the gesture");
    }

    /// Catches a value the layer already has that is counted, composited,
    /// or sent again: twenty of them change nothing and send nothing, a
    /// value let go that changes nothing is no step, and the Layers page's
    /// settings and a curve do the same.
    #[test]
    fn the_same_value_again_changes_nothing() {
        let mut session = patterned(64, 48, 2, 1, 0);
        preview(&mut session, "grade_exposure", PropValue::Number(0.5));
        assert_eq!(rasters(&session.take_live()), 1);
        command(&mut session, "finish_edit", json!({}));
        let edits = session.images[0].doc().edits;
        let mut sent = Vec::new();
        for _ in 0..20 {
            sent.extend(preview(&mut session, "grade_exposure", PropValue::Number(0.5)));
            assert!(!session.images[0].doc().out_of_date(), "the same value made the picture out of date");
        }
        sent.extend(session.take_live());
        assert_eq!(rasters(&sent), 0, "the same value was sent again");
        assert_eq!(session.images[0].doc().edits, edits, "the same value was counted as an edit");

        let doc = session.images[0].doc_mut();
        let grade = doc.active.unwrap();
        let Kind::Grade(settings) = doc.picture.find(grade).unwrap().kind.clone() else { panic!("not a Color Grade") };
        let edits = doc.edits;
        doc.set_grade(grade, settings, false, None).unwrap();
        doc.set_setting(Setting::Opacity(1.0), false).unwrap();
        doc.set_setting(Setting::Blend(Blend::Normal), true).unwrap();
        doc.finish_edit();
        assert_eq!(doc.edits, edits, "a value the layer has was counted as an edit");
        doc.undo();
        assert_eq!(exposure(&session), 0.0, "a value the layer has made a step");

        let doc = session.images[0].doc_mut();
        let curves = doc.add_adjustment(Kind::Curves(Curves::default()), None, None).unwrap();
        doc.refresh();
        let edits = doc.edits;
        doc.set_curve(curves, 0, &[[0.0, 0.0], [1.0, 1.0]], true, None).unwrap();
        assert_eq!(doc.edits, edits, "the curve it has was counted as an edit");
        assert!(!doc.out_of_date(), "the curve it has made the picture out of date");
        assert!(!doc.held(), "the curve it has began a held slider");
    }

    /// Catches the value let go lost behind the held ones, or answers sent
    /// before the picture they follow: a ping after held values is answered
    /// after their picture, and a value let go is always sent, even when it
    /// changes nothing.
    #[test]
    fn the_value_let_go_always_lands() {
        let mut session = patterned(64, 48, 2, 1, 0);
        preview(&mut session, "grade_exposure", PropValue::Number(0.75));
        preview(&mut session, "grade_exposure", PropValue::Number(1.0));
        let answered = session.handle(HostMessage::Ping);
        let pong = answered.iter().position(|message| *message == AppMessage::Pong).expect("no pong");
        let raster = answered.iter().position(|message| matches!(message, AppMessage::Raster(_))).expect("the held values were not sent before the ping's answer");
        assert!(raster < pong);
        assert_eq!(shown_exposure(&answered), Some(1.0));

        preview(&mut session, "grade_exposure", PropValue::Number(1.5));
        let set = |session: &mut Session, value: f64| {
            session.handle(HostMessage::Property { prop: json!("grade_exposure"), value: serde_json::to_value(PropValue::Number(value)).unwrap(), preview: false })
        };
        let landed = set(&mut session, 1.25);
        assert_eq!(rasters(&landed), 1, "the value let go was not sent");
        assert_eq!(shown_exposure(&landed), Some(1.25));
        assert_eq!(exposure(&session), 1.25);
        assert!(!session.images[0].doc().held(), "letting go left the slider held");
        let again = set(&mut session, 1.25);
        assert_eq!(shown_exposure(&again), Some(1.25), "the value let go that changes nothing was not sent");
        assert!(session.take_live().is_empty());
        command(&mut session, "undo", json!({}));
        assert_eq!(exposure(&session), 0.0, "the held values and the value let go were not one step");
    }

    /// Catches a picture that silently stops changing when its memory
    /// folder is full: a picture that cannot be written there is written to
    /// a folder on disk instead, and sent from there.
    #[test]
    fn a_full_memory_folder_moves_the_pictures_to_disk() {
        let mut session = Session::launch();
        let _ = session.greeting();
        // A file where the folder should be: nothing can be written in it.
        let blocked = scratch_folder();
        let _ = std::fs::remove_dir_all(&blocked);
        std::fs::write(&blocked, b"full").unwrap();
        session.cache = blocked.clone();
        session.in_memory = true;
        session.images[0].frame_mut().written = None;
        let view = last_raster(&command(&mut session, "add_layer", json!({})));
        assert!(!session.in_memory, "the pictures stayed in the full folder");
        assert!(!view.picture.starts_with(blocked.display().to_string().as_str()));
        assert!(Path::new(&view.picture).is_file(), "the picture was not written on disk");
        let _ = std::fs::remove_file(blocked);
    }

    /// Catches autosave splitting a held slider into two undo steps: inside
    /// a project, a write falling due while the slider is held waits, and
    /// one Undo after letting go takes back the whole drag.
    #[test]
    fn autosave_waits_for_a_held_slider() {
        let folder = scratch_folder();
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let mut session = patterned(64, 48, 2, 1, 0);
        session.handle(HostMessage::Project { folder: folder.display().to_string() });
        preview(&mut session, "grade_exposure", PropValue::Number(0.75));
        session.take_live();
        session.images[0].unwritten_since = Some(Instant::now() - WRITE_AFTER * 2);
        session.tick();
        assert!(session.images[0].saving.is_none(), "a write started while the slider was held");
        preview(&mut session, "grade_exposure", PropValue::Number(1.5));
        session.handle(HostMessage::Property { prop: json!("grade_exposure"), value: serde_json::to_value(PropValue::Number(1.5)).unwrap(), preview: false });
        assert_eq!(exposure(&session), 1.5);
        command(&mut session, "undo", json!({}));
        assert_eq!(exposure(&session), 0.0, "the held drag became more than one step");
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Catches the histogram made again while a curve is dragged: what lies
    /// beneath the curve does not change, so it is made once; painting
    /// beneath it makes it again.
    #[test]
    fn dragging_a_curve_keeps_its_histogram() {
        let mut session = patterned(64, 48, 2, 0, 1);
        let first = session.images[0].doc().histograms_made.get();
        assert!(first >= 1, "the Curves page showed no histogram");
        for step in 0..10 {
            let points = vec![(0.0, 0.0), (0.5, 0.5 + f64::from(step) * 0.03), (1.0, 1.0)];
            preview(&mut session, "curve", PropValue::Curve(points));
            assert_eq!(rasters(&session.take_live()), 1);
        }
        command(&mut session, "finish_edit", json!({}));
        assert_eq!(session.images[0].doc().histograms_made.get(), first, "dragging the curve made its histogram again");

        let background = session.images[0].doc().picture.nodes[0].id;
        command(&mut session, "layer_click", json!({ "node": background }));
        command(&mut session, "tool", json!({ "tool": "brush" }));
        stroke(&mut session, [2.0, 2.0], [60.0, 40.0]);
        let curves = session.images[0].doc().picture.nodes.last().unwrap().id;
        command(&mut session, "layer_click", json!({ "node": curves }));
        assert!(session.images[0].doc().histograms_made.get() > first, "painting beneath the curve kept the old histogram");
    }

    /// Catches a save that copies every layer's pixels after only an
    /// adjustment changed: once written, a Color Grade's new value copies
    /// none, and a stroke copies its layer again.
    #[test]
    fn a_save_after_a_settings_change_copies_no_pixels() {
        let mut session = patterned(64, 48, 2, 1, 0);
        let path = std::env::temp_dir().join(format!("image-editor-copies-{}.ipx", std::process::id()));
        let write = |session: &mut Session| {
            let gallery = &mut session.images[0].gallery;
            let (pictures, shown) = snapshot(gallery);
            let copied = pictures[0].copied();
            keep(gallery, files::write_ipx(&path, pictures, shown, None).unwrap());
            copied
        };
        assert_eq!(write(&mut session), 2, "the first write did not copy both layers");
        preview(&mut session, "grade_exposure", PropValue::Number(0.5));
        command(&mut session, "finish_edit", json!({}));
        assert_eq!(write(&mut session), 0, "a change to the grade copied pixels");
        let background = session.images[0].doc().picture.nodes[0].id;
        command(&mut session, "layer_click", json!({ "node": background }));
        command(&mut session, "tool", json!({ "tool": "brush" }));
        stroke(&mut session, [2.0, 2.0], [60.0, 40.0]);
        assert_eq!(write(&mut session), 2, "a stroke did not copy the layers again");
        let _ = std::fs::remove_file(path);
    }

    /// Times a held slider's value from its line to the picture and panel
    /// sent, as the host sees it, at the owner's 1370 × 1148 with four
    /// layers: under 16 ms for a Color Grade and 25 ms for Curves.
    #[test]
    #[ignore]
    fn a_held_slider_keeps_up() {
        for (grades, curves) in [(1, 0), (3, 0), (0, 1), (1, 1)] {
            let mut session = patterned(1370, 1148, 4, grades, curves);
            let mut times = Vec::new();
            for step in 0..9 {
                let started = Instant::now();
                let value = if curves > 0 { json!({ "curve": [[0.0, 0.0], [0.5, 0.55 + f64::from(step) * 0.02], [1.0, 1.0]] }) } else { json!({ "number": 0.3 + f64::from(step) * 0.1 }) };
                let prop = if curves > 0 { "curve" } else { "grade_exposure" };
                let mut out = session.handle(HostMessage::Property { prop: json!(prop), value, preview: true });
                out.extend(session.take_live());
                let lines: usize = out.iter().map(|message| protocol::encode_app_message(message).len()).sum();
                assert!(lines > 0, "nothing was sent");
                times.push(started.elapsed());
            }
            times.sort();
            let median = times[times.len() / 2];
            eprintln!("{grades} grades, {curves} curves: fastest {:?}, median {median:?}, slowest {:?}", times[0], times[times.len() - 1]);
            if !cfg!(debug_assertions) {
                let most = if curves > 0 { 25 } else { 16 };
                assert!(median < Duration::from_millis(most), "{median:?}");
            }
        }
    }

    /// Catches typing that does not reach the text, a Copy with no answer,
    /// and Escape or another tool that leaves editing open: through
    /// `raster_text`, words are typed, a word selected with the keyboard is
    /// answered with `clipboard`, pasted back as typing, and the text ends
    /// when Escape is pressed, one step kept in the layers.
    #[test]
    fn typing_goes_through_raster_text() {
        let mut session = Session::launch();
        let _ = session.greeting();
        command(&mut session, "tool", json!({ "tool": "text" }));
        let view = last_raster(&gesture(&mut session, RasterGesture::Click { at: [100.0, 100.0] }));
        assert!(matches!(view.tool, RasterTool::Text));
        assert!(view.text.is_some(), "a click made no text to type in");
        let tab = session.images[0].id.clone();
        let text = |session: &mut Session, input: protocol::TextInput| session.handle(HostMessage::RasterText { tab: tab.clone(), input });
        let key = |key: protocol::TextKey, shift: bool, word: bool| protocol::TextInput::Key { key, shift, word };
        text(&mut session, protocol::TextInput::Insert { text: "Golden hour".into() });
        text(&mut session, key(protocol::TextKey::Left, true, true));
        let copied = text(&mut session, key(protocol::TextKey::Copy, false, false));
        assert!(copied.contains(&AppMessage::Clipboard { text: "hour".into() }), "Copy was not answered");
        text(&mut session, key(protocol::TextKey::End, false, false));
        text(&mut session, protocol::TextInput::Insert { text: " ".into() });
        let view = last_raster(&text(&mut session, protocol::TextInput::Insert { text: "hour".into() }));
        let editing = view.text.expect("editing ended while typing");
        assert!(editing.caret[0] > editing.rect[0] + 100.0);
        let view = last_raster(&text(&mut session, key(protocol::TextKey::Escape, false, false)));
        assert!(view.text.is_none(), "Escape did not end editing");
        let doc = session.images[0].doc();
        let words: Vec<String> = doc.picture.every_node().iter().filter_map(|node| node.text().map(|text| text.words.clone())).collect();
        assert_eq!(words, ["Golden hour hour"]);
        assert!(doc.can_undo());
    }
}
