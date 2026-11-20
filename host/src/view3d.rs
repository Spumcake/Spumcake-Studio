//! The host's 3D view: a place's scene drawn in the host's own window, as the
//! scene contract's section 4 sets out, so orbiting needs no frame sent
//! through a file. A sub-application names the scene file, the look, and the
//! camera; the host keeps the free camera while the user turns it, and
//! frames a scene camera exactly as its pre-vis builder did, so a published
//! render lies on it edge for edge.
//!
//! The engine, Graphics Engine Work's `Live`, draws each frame on the
//! window's own GPU device, the same exact drawing whether the camera is
//! free or one of the builder's, and the host paints it into its pass. The
//! names and the outline of a picked object come from the engine's object
//! pass, read when the camera settles.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use eframe::egui_wgpu;
use spumcake_render::{Live, Look, Request, View, scene, wgpu};

/// The room's up, as every pre-vis builder has it.
const UP: [f64; 3] = [0.0, 0.0, 1.0];
/// Turning: degrees per point dragged.
const TURN: f64 = 0.3;
/// The free camera never looks straighter up or down than this, so its
/// right stays defined.
const MAX_PITCH: f64 = 85.0;
/// The nearest the free camera comes to what it turns around, in metres.
const NEAREST: f64 = 0.2;
/// A notch of the wheel moves the camera this share of the way to its target.
const DOLLY: f64 = 0.15;

/// A camera as a pre-vis builder sets one: where it stands, what it looks
/// at, and its horizontal field of view in degrees, in the builder's frame,
/// z up, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera {
    pub position: [f64; 3],
    pub target: [f64; 3],
    pub hfov: f64,
}

/// The camera's axes as the builder computes them: forward to the target,
/// right = normalize(forward × z), and up = right × forward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Axes {
    pub forward: [f64; 3],
    pub right: [f64; 3],
    pub up: [f64; 3],
}

impl Camera {
    pub fn axes(&self) -> Axes {
        let forward = normalize(sub(self.target, self.position));
        let right = normalize(cross(forward, UP));
        let up = cross(right, forward);
        Axes { forward, right, up }
    }

    /// Where a point falls in a frame `size` pixels wide and high, by the
    /// builder's own projection: the focal length is half the width over
    /// tan(hfov / 2), and y grows downward. None behind the camera.
    #[cfg(test)]
    pub fn project(&self, point: [f64; 3], size: [f64; 2]) -> Option<[f64; 2]> {
        let axes = self.axes();
        let v = sub(point, self.position);
        let (x, y, z) = (dot(v, axes.right), dot(v, axes.up), dot(v, axes.forward));
        if z <= 1e-9 {
            return None;
        }
        let focal = (size[0] / 2.0) / (self.hfov.to_radians() / 2.0).tan();
        Some([size[0] / 2.0 + focal * x / z, size[1] / 2.0 - focal * y / z])
    }

    /// Turns the camera around its target: dragging right turns it about the
    /// room's up, and dragging down tilts it, never past straight up or down.
    pub fn orbit(&mut self, drag: [f64; 2]) {
        let offset = sub(self.position, self.target);
        let distance = length(offset);
        let yaw = offset[1].atan2(offset[0]) - (drag[0] * TURN).to_radians();
        let pitch = (offset[2] / distance).asin().to_degrees() + drag[1] * TURN;
        let pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH).to_radians();
        self.position = add(
            self.target,
            [distance * pitch.cos() * yaw.cos(), distance * pitch.cos() * yaw.sin(), distance * pitch.sin()],
        );
    }

    /// Slides the camera and its target together across the view, so the
    /// point under the pointer stays under it at the target's depth.
    pub fn pan(&mut self, drag: [f64; 2], size: [f64; 2]) {
        let axes = self.axes();
        let distance = length(sub(self.target, self.position));
        let focal = (size[0] / 2.0) / (self.hfov.to_radians() / 2.0).tan();
        let metres = distance / focal;
        let shift = add(scale(axes.right, -drag[0] * metres), scale(axes.up, drag[1] * metres));
        self.position = add(self.position, shift);
        self.target = add(self.target, shift);
    }

    /// Moves the camera toward its target by notches of the wheel, or away
    /// with negative notches, never nearer than `NEAREST`.
    pub fn dolly(&mut self, notches: f64) {
        let offset = sub(self.position, self.target);
        let distance = (length(offset) * (1.0 - DOLLY).powf(notches)).max(NEAREST);
        self.position = add(self.target, scale(normalize(offset), distance));
    }

    /// The vertical field of view a frame of this shape shows, in degrees, as
    /// glTF stores it: 2·atan(tan(hfov/2)·height/width).
    #[cfg(test)]
    pub fn vfov(&self, size: [f64; 2]) -> f64 {
        2.0 * ((self.hfov.to_radians() / 2.0).tan() * size[1] / size[0]).atan().to_degrees()
    }
}

/// The largest rectangle of the image's shape that fits the area, centred,
/// in whole points, so a scene camera's frame keeps its builder's shape
/// whatever the window's size.
pub(crate) fn letterbox(area: Rect, image: [u32; 2]) -> Rect {
    let aspect = image[0] as f32 / image[1] as f32;
    let (width, height) = if area.width() / area.height() > aspect {
        (area.height() * aspect, area.height())
    } else {
        (area.width(), area.width() / aspect)
    };
    let size = vec2(width.floor(), height.floor());
    let min = (area.center() - size / 2.0).round();
    Rect::from_min_size(min, size)
}

/// How a published render is shown with the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Reference {
    /// The view and the render side by side, each in its own frame.
    Beside,
    /// The render laid over the view, at this opacity from 0 to 1.
    Over(f32),
}

/// Where the view's frame goes, and the render's when it is shown beside:
/// beside, the area is split in two halves with a gap, and each frame is
/// letterboxed in its half, the view on the left.
pub(crate) fn frames(area: Rect, image: [u32; 2], reference: Option<Reference>, gap: f32) -> (Rect, Option<Rect>) {
    match reference {
        Some(Reference::Beside) => {
            let half = (area.width() - gap) / 2.0;
            let left = Rect::from_min_size(area.min, vec2(half, area.height()));
            let right = Rect::from_min_size(pos2(area.min.x + half + gap, area.min.y), vec2(half, area.height()));
            (letterbox(left, image), Some(letterbox(right, image)))
        }
        _ => (letterbox(area, image), None),
    }
}

/// The samples per pixel of the window's passes, as `window::run` asks.
pub(crate) const SAMPLES: u32 = 4;
/// The gap between the view and a render shown beside it, in points.
const GAP: f32 = 16.0;
/// A group of the builder's objects is named only where it shows at least
/// this many pixels, as the builder's labelled renders do.
const NAMED_FROM: usize = 40;
/// The picked object's outline, in the selection blue the host uses.
const PICKED: Color32 = Color32::from_rgb(37, 99, 235);
/// A notch of the wheel, in points.
const NOTCH: f32 = 50.0;
/// How long the pointer holds still before the object under it is named.
const HOLD_STILL: std::time::Duration = std::time::Duration::from_millis(150);
/// The room under frames shown beside for their captions, in points.
const CAPTION: f32 = 24.0;
/// A caption's grey, as the view set's captions are.
const CAPTION_GREY: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
/// The status line under the view, in points.
const STATUS: f32 = 28.0;

/// What a 3D tab shows, as its sub-application asked: the scene file, the
/// look, the camera, a published render to show with it, the time in the
/// scene's animation, whether names show, and what dragging does.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Shown {
    /// The scene file, already resolved inside the project.
    pub scene: PathBuf,
    pub look: Look,
    pub camera: Choice,
    pub reference: Option<Pane>,
    pub time: f64,
    pub names: bool,
    pub tool: Tool,
    /// The scene's animation, played under the view.
    pub transport: Option<protocol::Transport>,
    /// A line under the view, such as which builder the scene came from.
    pub status: Option<String>,
}

/// Which camera the view looks through.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Choice {
    /// The free camera, from where the user left it, or from the scene's
    /// first camera when none is given.
    Free(Option<Camera>),
    /// One of the builder's cameras, by name.
    Named(String),
}

/// A published render, decoded by the sub-application into a raw file of
/// `size` pixels, four bytes each, as raster tabs send their images.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pane {
    pub file: PathBuf,
    pub size: [u32; 2],
    pub show: Reference,
    /// Shown beside, the captions under the view's frame and the render's,
    /// such as "3D view" and "Pre-vis render · 10 Nov".
    pub captions: Option<[String; 2]>,
}

/// What a drag does in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Orbit,
    Pan,
}

/// What the view tells its sub-application.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Out {
    /// The user stopped turning the free camera, here.
    Camera(Camera),
    /// The user pointed at an object: its node name, and its label group.
    Picked { object: String, label: String },
    /// The user asked to see a scene file that could not be read.
    ShowFile(PathBuf),
    /// A transport control was used: play or pause, or a step, with the
    /// area's size as every command carries it.
    Command(protocol::Action, [f64; 2]),
    /// The user scrubbed to this time, or the host's clock stopped there.
    Time(f64),
}

impl Shown {
    /// What a tab's sub-application declared with `view3d`, with its paths
    /// resolved inside the project's folder. A scene path that leaves the
    /// folder is read as a missing file, so the view says it cannot read it.
    pub fn from_declared(declared: &crate::process::View3dShown, project: &Path) -> Shown {
        let inside = |relative: &str| {
            let path = Path::new(relative);
            let escapes = path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir));
            if escapes { project.join(".spumcake/no-such-file") } else { project.join(path) }
        };
        let look = match declared.look {
            protocol::View3dLook::Grey => Look::Grey,
            protocol::View3dLook::Coded => Look::Coded,
            protocol::View3dLook::Simple => Look::Simple,
        };
        let camera = match &declared.camera {
            protocol::CameraChoice::Free { start } => {
                Choice::Free(start.map(|p| Camera { position: p.position, target: p.target, hfov: p.fov }))
            }
            protocol::CameraChoice::Named { name } => Choice::Named(name.clone()),
        };
        let reference = declared.reference.as_ref().map(|r| Pane {
            file: inside(&r.image),
            size: [r.width, r.height],
            show: match r.show {
                protocol::ReferenceShow::Beside => Reference::Beside,
                protocol::ReferenceShow::Over { opacity } => Reference::Over(opacity as f32),
            },
            captions: r.caption.as_ref().map(|c| [c.view.clone(), c.reference.clone()]),
        });
        let tool = match declared.tool {
            protocol::View3dTool::Orbit => Tool::Orbit,
            protocol::View3dTool::Pan => Tool::Pan,
        };
        Shown {
            scene: inside(&declared.scene),
            look,
            camera,
            reference,
            time: declared.time,
            names: declared.names,
            tool,
            transport: declared.transport.clone(),
            status: declared.status.clone(),
        }
    }
}

impl Out {
    /// The message that tells the tab's sub-application, or none for what
    /// the window answers itself, such as showing a file.
    pub fn message(&self, tab: &str) -> Option<protocol::HostMessage> {
        match self {
            Out::Camera(c) => Some(protocol::HostMessage::View3dCamera {
                tab: tab.to_string(),
                position: c.position,
                target: c.target,
                fov: c.hfov,
            }),
            Out::Picked { object, label } => {
                Some(protocol::HostMessage::View3dPicked { tab: tab.to_string(), object: object.clone(), label: label.clone() })
            }
            Out::Command(action, viewport) => Some(protocol::HostMessage::Command {
                id: action.id.clone(),
                tab: Some(tab.to_string()),
                arg: action.arg.clone(),
                node: None,
                viewport: *viewport,
            }),
            Out::Time(time) => Some(protocol::HostMessage::TransportTime { tab: tab.to_string(), time: *time }),
            Out::ShowFile(_) => None,
        }
    }
}

/// The window's GPU, as the 3D view needs it: the device and queue egui
/// draws with, its target format, and egui's renderer, whose callback
/// resources keep each tab's engine.
#[derive(Clone, Copy)]
pub(crate) struct Gpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub renderer: &'a Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
}

impl<'a> Gpu<'a> {
    pub fn of(state: &'a egui_wgpu::RenderState) -> Gpu<'a> {
        Gpu { device: &state.device, queue: &state.queue, format: state.target_format, renderer: &state.renderer }
    }
}

/// Every 3D tab the window has shown, by tab.
#[derive(Default)]
pub(crate) struct Views {
    tabs: HashMap<String, Tab>,
}

#[derive(Default)]
struct Tab {
    /// The scene file read, when it was last changed, and what it held.
    loaded: Option<(PathBuf, Option<SystemTime>, Result<Arc<scene::Scene>, String>)>,
    /// The free camera while the user turns it.
    free: Option<Camera>,
    moving: bool,
    /// The frame whose names and outline were read last.
    settled: Option<Key>,
    /// Each named group's words and where they go, in the frame's pixels.
    names: Vec<(String, [f32; 2])>,
    picked: Option<usize>,
    outline: Option<egui::TextureHandle>,
    reference: Option<(PathBuf, egui::TextureHandle)>,
    /// The transport row's clock.
    transport: crate::transport::Clock,
    /// Where the pointer rests over the view, and since when.
    hover: Option<(egui::Pos2, Instant)>,
    /// Where the object under the pointer was last read.
    pointed: Option<egui::Pos2>,
}

/// One frame asked of the engine. A frame is drawn again only when this
/// changes.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    camera: KeyCamera,
    look: Look,
    size: [u32; 2],
    time: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum KeyCamera {
    Named(String),
    Free(Camera),
}

/// The engine's state for every 3D tab, kept in egui_wgpu's callback
/// resources so the paint callback can reach it.
#[derive(Default)]
struct Lives {
    views: HashMap<String, (Live, Option<Key>)>,
}

/// One frame of one tab, handed to egui_wgpu to prepare and paint.
struct Paint {
    tab: String,
    scene: Arc<scene::Scene>,
    key: Key,
    format: wgpu::TextureFormat,
}

impl egui_wgpu::CallbackTrait for Paint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<Lives>().is_none() {
            resources.insert(Lives::default());
        }
        let lives = resources.get_mut::<Lives>().unwrap();
        let (live, drawn) = lives
            .views
            .entry(self.tab.clone())
            .or_insert_with(|| (Live::new(device, queue, self.format, SAMPLES), None));
        if drawn.as_ref() != Some(&self.key) {
            let camera = match &self.key.camera {
                KeyCamera::Named(name) => View::Named(name),
                KeyCamera::Free(c) => View::Free { position: c.position, target: c.target, hfov: c.hfov },
            };
            let request =
                Request { camera, look: self.key.look, width: self.key.size[0], height: self.key.size[1], time: self.key.time };
            match live.frame(device, queue, encoder, &self.scene, &request) {
                Ok(()) => *drawn = Some(self.key.clone()),
                Err(error) => eprintln!("the 3D view could not draw its frame: {error}"),
            }
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        if let Some((live, _)) = resources.get::<Lives>().and_then(|lives| lives.views.get(&self.tab)) {
            live.paint(pass);
        }
    }
}

impl Views {
    /// Draws a 3D tab in `area`, and answers the user's turns and points.
    /// With no GPU, as in frame tests, it frames the view and draws nothing
    /// in it.
    pub fn show(&mut self, ui: &mut egui::Ui, gpu: Option<Gpu>, tab: &str, shown: &Shown, area: Rect) -> Vec<Out> {
        let mut out = Vec::new();
        let state = self.tabs.entry(tab.to_string()).or_default();
        let whole = area;
        let status_h = if shown.status.is_some() { STATUS } else { 0.0 };
        let transport_h = shown.transport.as_ref().map_or(0.0, crate::transport::height);
        let area = Rect::from_min_max(whole.min, pos2(whole.max.x, whole.max.y - status_h - transport_h));
        if let Some(status) = &shown.status {
            let at = pos2(whole.min.x + 12.0, whole.max.y - status_h / 2.0);
            ui.painter_at(whole).text(at, Align2::LEFT_CENTER, status, FontId::proportional(13.0), CAPTION_GREY);
        }
        let time = match &shown.transport {
            Some(transport) => {
                let band = Rect::from_min_max(pos2(whole.min.x, area.max.y), pos2(whole.max.x, whole.max.y - status_h));
                let viewport = [f64::from(whole.width()), f64::from(whole.height())];
                let mut events = Vec::new();
                let time = crate::transport::row(ui, &mut state.transport, transport, band, &mut events);
                out.extend(events.into_iter().map(|event| match event {
                    crate::transport::Event::Time(time) => Out::Time(time),
                    crate::transport::Event::Command(action) => Out::Command(action, viewport),
                }));
                time
            }
            None => shown.time,
        };
        let scene = match load(state, &shown.scene) {
            Ok(scene) => scene,
            Err(why) => {
                failed(ui, area, &shown.scene, &why, &mut out);
                return out;
            }
        };

        // Which camera, and the frame it is drawn in. A builder's camera
        // keeps its image's shape and is drawn at its image's own size, so
        // its published render lies on it pixel for pixel.
        let named = match &shown.camera {
            Choice::Named(name) if state.free.is_none() => scene.camera(name),
            _ => None,
        };
        let ppp = ui.ctx().pixels_per_point();
        let captions = shown.reference.as_ref().filter(|pane| pane.show == Reference::Beside).and_then(|pane| pane.captions.clone());
        let framed = if captions.is_some() { Rect::from_min_max(area.min, pos2(area.max.x, area.max.y - CAPTION)) } else { area };
        let (view, beside) = match named {
            Some(camera) => frames(framed, camera.image_size, shown.reference.as_ref().map(|pane| pane.show), GAP),
            None => (area, None),
        };
        let free = state.free.or(match &shown.camera {
            Choice::Free(Some(camera)) => Some(*camera),
            _ => None,
        });
        let start = || match &shown.camera {
            Choice::Named(name) => scene.camera(name),
            Choice::Free(_) => scene.cameras.first(),
        }
        .map(|c| Camera { position: c.position, target: c.target, hfov: c.hfov });
        let (camera, size) = match named {
            Some(c) => (KeyCamera::Named(c.name.clone()), c.image_size),
            None => {
                let size = [(view.width() * ppp).round().max(1.0) as u32, (view.height() * ppp).round().max(1.0) as u32];
                match free.or_else(start) {
                    Some(camera) => (KeyCamera::Free(camera), size),
                    None => return out,
                }
            }
        };
        let key = Key { camera, look: shown.look, size, time };

        // Turning, sliding, and the wheel. A drag on a builder's camera
        // starts the free camera from where that camera stands.
        let response = ui.allocate_rect(view, Sense::click_and_drag());
        let delta = response.drag_delta();
        if response.dragged() && delta != egui::Vec2::ZERO {
            let mut turning = state.free.or(free).or_else(start).unwrap_or(Camera { position: [0.0; 3], target: [1.0, 0.0, 0.0], hfov: 60.0 });
            let pan = shown.tool == Tool::Pan || response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary);
            let drag = [f64::from(delta.x), f64::from(delta.y)];
            if pan {
                turning.pan([drag[0] * f64::from(ppp), drag[1] * f64::from(ppp)], [f64::from(size[0]), f64::from(size[1])]);
            } else {
                turning.orbit(drag);
            }
            state.free = Some(turning);
            state.moving = true;
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let mut turning = state.free.or(free).or_else(start).unwrap_or(Camera { position: [0.0; 3], target: [1.0, 0.0, 0.0], hfov: 60.0 });
                turning.dolly(f64::from(scroll / NOTCH));
                state.free = Some(turning);
                state.moving = true;
            }
        }
        if state.moving && !response.dragged() && !ui.input(|input| input.smooth_scroll_delta.y != 0.0) {
            state.moving = false;
            if let Some(camera) = state.free {
                out.push(Out::Camera(camera));
            }
        }
        // Once the sub-application answers with the free camera, or with a
        // camera of its own choosing, the view follows it again.
        if !state.moving && state.free.is_some() && shown.camera != Choice::Free(None) {
            if let Choice::Free(Some(camera)) = &shown.camera {
                if Some(*camera) == state.free {
                    state.free = None;
                }
            }
        }

        // The frame itself, painted by the engine into the window's pass.
        let painter = ui.painter_at(area);
        painter.rect_filled(view, 0.0, Color32::WHITE);
        if let Some(gpu) = gpu {
            painter.add(egui_wgpu::Callback::new_paint_callback(
                view,
                Paint { tab: tab.to_string(), scene: scene.clone(), key: key.clone(), format: gpu.format },
            ));
        }
        if named.is_some() {
            painter.rect_stroke(view, 0.0, Stroke::new(1.0, Color32::from_rgb(0x6E, 0x6E, 0x6E)), egui::StrokeKind::Outside);
        }

        // The published render, over the view or beside it.
        if let (Some(pane), Some(_)) = (&shown.reference, named) {
            if let Some(texture) = reference(ui.ctx(), state, pane) {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                match (pane.show, beside) {
                    (Reference::Over(opacity), _) => {
                        painter.image(texture, view, uv, Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8));
                    }
                    (Reference::Beside, Some(right)) => {
                        painter.image(texture, right, uv, Color32::WHITE);
                        painter.rect_stroke(right, 0.0, Stroke::new(1.0, Color32::from_rgb(0x6E, 0x6E, 0x6E)), egui::StrokeKind::Outside);
                        if let Some([left_words, right_words]) = &captions {
                            for (frame, words) in [(view, left_words), (right, right_words)] {
                                let at = pos2(frame.min.x, frame.max.y + 6.0);
                                painter.text(at, Align2::LEFT_TOP, words, FontId::proportional(12.0), CAPTION_GREY);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Names and the picked object's outline, read from the frame the
        // engine drew once the camera holds still.
        if let Some(gpu) = gpu {
            let drawn = drawn_key(gpu, tab);
            if !state.moving && drawn.as_ref() == Some(&key) && state.settled.as_ref() != Some(&key) {
                settle(ui.ctx(), gpu, tab, &scene, state, size);
                state.settled = Some(key.clone());
            } else if drawn.as_ref() != Some(&key) {
                ui.ctx().request_repaint();
            }
            // Pointing names the object under the pointer once the pointer
            // holds still, as vp-picked shows; each pick reads one pixel
            // back from the GPU, so it waits rather than reading every frame.
            match response.hover_pos() {
                Some(at) if !state.moving && drawn.as_ref() == Some(&key) => {
                    let still = state.hover.is_some_and(|(p, since)| p == at && since.elapsed() >= HOLD_STILL);
                    if state.hover.is_none_or(|(p, _)| p != at) {
                        state.hover = Some((at, Instant::now()));
                        ui.ctx().request_repaint_after(HOLD_STILL);
                    } else if still && state.pointed != Some(at) {
                        state.pointed = Some(at);
                        let x = ((at.x - view.min.x) / view.width() * size[0] as f32) as u32;
                        let y = ((at.y - view.min.y) / view.height() * size[1] as f32) as u32;
                        let picked = object_at(gpu, tab, [x, y]);
                        if picked != state.picked {
                            state.picked = picked;
                            state.settled = None;
                            if let Some(object) = picked.and_then(|i| scene.objects.get(i)) {
                                out.push(Out::Picked { object: object.name.clone(), label: object.label.clone() });
                            }
                            ui.ctx().request_repaint();
                        }
                    }
                }
                Some(_) => {}
                None => {
                    state.hover = None;
                    state.pointed = None;
                    if state.picked.take().is_some() {
                        state.settled = None;
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
        if state.settled.as_ref() == Some(&key) {
            let to_screen = |p: [f32; 2]| pos2(view.min.x + p[0] / size[0] as f32 * view.width(), view.min.y + p[1] / size[1] as f32 * view.height());
            if let Some(outline) = &state.outline {
                painter.image(outline.id(), view, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if shown.names {
                for (words, at) in &state.names {
                    name_box(&painter, to_screen(*at), words);
                }
            }
            if let (Some(object), Some(pointer)) = (state.picked.and_then(|i| scene.objects.get(i)), response.hover_pos()) {
                // A group of one part, such as "far end wall, beside the door",
                // is named once.
                let words = if object.label_text == object.name {
                    object.name.clone()
                } else {
                    format!("{} · {}", object.label_text, object.name)
                };
                name_tip(&painter, pointer, &words);
            }
        }
        out
    }
}

/// Reads the scene file when it is new to the tab or has changed since.
fn load(state: &mut Tab, path: &Path) -> Result<Arc<scene::Scene>, String> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let fresh = matches!(&state.loaded, Some((p, m, _)) if p == path && *m == modified);
    if !fresh {
        let read = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| scene::read(&bytes).map_err(|e| e.to_string()));
        state.loaded = Some((path.to_path_buf(), modified, read.map(Arc::new)));
        state.settled = None;
        state.picked = None;
    }
    state.loaded.as_ref().unwrap().2.clone()
}

/// A scene file that could not be read: its name, why, and Show file.
fn failed(ui: &mut egui::Ui, area: Rect, path: &Path, why: &str, out: &mut Vec<Out>) {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
    child.add_space((area.height() / 2.0 - 60.0).max(0.0));
    child.label(egui::RichText::new(format!("Couldn't read {name}")).strong());
    child.label(egui::RichText::new(why).weak());
    if child.button("Show file").clicked() {
        out.push(Out::ShowFile(path.to_path_buf()));
    }
}

/// The published render's texture, read from its raw file once per file.
fn reference(ctx: &egui::Context, state: &mut Tab, pane: &Pane) -> Option<egui::TextureId> {
    if state.reference.as_ref().map(|(p, _)| p) != Some(&pane.file) {
        let bytes = std::fs::read(&pane.file).ok()?;
        let [w, h] = pane.size.map(|v| v as usize);
        if bytes.len() != w * h * 4 {
            return None;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &bytes);
        let texture = ctx.load_texture(format!("view3d:{}", pane.file.display()), image, egui::TextureOptions::LINEAR);
        state.reference = Some((pane.file.clone(), texture));
    }
    state.reference.as_ref().map(|(_, t)| t.id())
}

/// The frame the engine last drew for a tab.
fn drawn_key(gpu: Gpu, tab: &str) -> Option<Key> {
    let renderer = gpu.renderer.read();
    renderer.callback_resources.get::<Lives>()?.views.get(tab)?.1.clone()
}

fn object_at(gpu: Gpu, tab: &str, at: [u32; 2]) -> Option<usize> {
    let renderer = gpu.renderer.read();
    let (live, _) = renderer.callback_resources.get::<Lives>()?.views.get(tab)?;
    live.object_at(gpu.device, gpu.queue, at)
}

/// Reads the drawn frame's object pass, and from it each group's name and
/// place, at the median of its visible pixels as the builder places its
/// labels, and the picked object's outline.
fn settle(ctx: &egui::Context, gpu: Gpu, tab: &str, scene: &scene::Scene, state: &mut Tab, size: [u32; 2]) {
    let objects = {
        let renderer = gpu.renderer.read();
        match renderer.callback_resources.get::<Lives>().and_then(|lives| lives.views.get(tab)) {
            Some((live, _)) => live.objects(gpu.device, gpu.queue),
            None => return,
        }
    };
    let (width, height) = (size[0] as usize, size[1] as usize);
    if objects.len() != width * height {
        return;
    }
    state.names = names(scene, &objects, width);
    state.outline = state.picked.map(|picked| {
        let mut pixels = vec![Color32::TRANSPARENT; width * height];
        for y in 0..height {
            for x in 0..width {
                let here = objects[y * width + x] == picked as u32;
                let edge = here
                    && (x == 0
                        || y == 0
                        || x + 1 == width
                        || y + 1 == height
                        || objects[y * width + x - 1] != picked as u32
                        || objects[y * width + x + 1] != picked as u32
                        || objects[(y - 1) * width + x] != picked as u32
                        || objects[(y + 1) * width + x] != picked as u32);
                if edge {
                    pixels[y * width + x] = PICKED;
                }
            }
        }
        let image = egui::ColorImage { size: [width, height], source_size: vec2(width as f32, height as f32), pixels };
        ctx.load_texture(format!("view3d-outline:{tab}"), image, egui::TextureOptions::NEAREST)
    });
}

/// Each label group's words, at the median of its visible pixels, for the
/// groups that show at least `NAMED_FROM` pixels and that the scene file
/// marks as named, placed as the builder places its labels: group by group in the
/// order of their first objects, and a name that would sit on one already
/// placed moved 20 pixels down.
fn names(scene: &scene::Scene, objects: &[u32], width: usize) -> Vec<(String, [f32; 2])> {
    let mut groups: HashMap<&str, (usize, String, Vec<f32>, Vec<f32>)> = HashMap::new();
    for (i, o) in objects.iter().enumerate() {
        let Some(object) = scene.objects.get(*o as usize) else { continue };
        // The groups the builder's labelled renders leave unnamed, such as
        // the floor and the walls, as the scene file records them.
        if !object.named {
            continue;
        }
        let first = scene.objects.iter().position(|other| other.label == object.label).unwrap_or(*o as usize);
        let group = groups.entry(&object.label).or_insert_with(|| (first, object.label_text.clone(), Vec::new(), Vec::new()));
        group.2.push((i % width) as f32);
        group.3.push((i / width) as f32);
    }
    let median = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        let n = v.len();
        if n % 2 == 1 { v[n / 2] } else { ((v[n / 2 - 1] + v[n / 2]) / 2.0).floor() }
    };
    let mut ordered: Vec<(usize, String, Vec<f32>, Vec<f32>)> =
        groups.into_values().filter(|(_, _, xs, _)| xs.len() >= NAMED_FROM).collect();
    ordered.sort_by_key(|(first, ..)| *first);
    let mut named: Vec<(String, [f32; 2])> = Vec::new();
    for (_, words, mut xs, mut ys) in ordered {
        let (x, mut y) = (median(&mut xs), median(&mut ys));
        if named.iter().any(|(_, [px, py])| (x - px).abs() < 70.0 && (y - py).abs() < 18.0) {
            y += 20.0;
        }
        named.push((words, [x, y]));
    }
    named
}

/// A name in a small white box, as the builder's labelled renders draw them.
fn name_box(painter: &egui::Painter, at: egui::Pos2, words: &str) {
    let galley = painter.layout_no_wrap(words.to_string(), FontId::proportional(12.0), Color32::from_rgb(20, 20, 20));
    let rect = Rect::from_center_size(at, galley.size() + vec2(6.0, 4.0));
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    painter.galley(rect.min + vec2(3.0, 2.0), galley, Color32::from_rgb(20, 20, 20));
}

/// The picked object's label and part beside the pointer, dark with white
/// text.
fn name_tip(painter: &egui::Painter, pointer: egui::Pos2, words: &str) {
    let galley = painter.layout_no_wrap(words.to_string(), FontId::proportional(12.0), Color32::WHITE);
    let rect = Rect::from_min_size(pointer + vec2(14.0, -10.0), galley.size() + vec2(12.0, 8.0));
    painter.rect_filled(rect, 4.0, Color32::from_rgb(30, 41, 59));
    painter.text(rect.center(), Align2::CENTER_CENTER, words, FontId::proportional(12.0), Color32::WHITE);
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let l = length(a);
    if l == 0.0 { a } else { scale(a, 1.0 / l) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The diner's far-end-wall camera, as its builder sets it.
    fn far_end() -> Camera {
        Camera { position: [7.0, 2.6, 1.6], target: [0.0, 2.6, 1.3], hfov: 62.0 }
    }

    const IMAGE: [f64; 2] = [1664.0, 928.0];

    #[test]
    fn projects_as_the_builder_does() {
        // The target lands in the frame's middle.
        let camera = far_end();
        let middle = camera.project(camera.target, IMAGE).unwrap();
        assert!((middle[0] - 832.0).abs() < 1e-9 && (middle[1] - 464.0).abs() < 1e-9);
        // A point on the far end wall at the window wall's side, y = 0, lies
        // on the left of the frame, as the floor plan says the front door does.
        let left = camera.project([0.0, 0.0, 1.0], IMAGE).unwrap();
        assert!(left[0] < 832.0);
        // Behind the camera is not in the frame.
        assert!(camera.project([12.0, 2.6, 1.6], IMAGE).is_none());
    }

    #[test]
    fn the_vertical_field_matches_the_scene_files() {
        // As the contract converts it: 62° across at 1664 × 928.
        assert!((far_end().vfov(IMAGE) - 37.0515).abs() < 1e-3);
    }

    #[test]
    fn orbiting_keeps_the_distance_and_never_flips() {
        let mut camera = far_end();
        let before = length(sub(camera.position, camera.target));
        camera.orbit([120.0, 40.0]);
        assert!((length(sub(camera.position, camera.target)) - before).abs() < 1e-9);
        camera.orbit([0.0, 10_000.0]);
        let pitch = ((camera.position[2] - camera.target[2]) / before).asin().to_degrees();
        assert!((pitch - MAX_PITCH).abs() < 1e-6);
        assert!(camera.axes().right.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn panning_keeps_the_point_under_the_pointer() {
        let mut camera = far_end();
        let before = camera.project(camera.target, IMAGE).unwrap();
        let target = camera.target;
        camera.pan([50.0, -20.0], IMAGE);
        let after = camera.project(target, IMAGE).unwrap();
        assert!((after[0] - (before[0] + 50.0)).abs() < 1e-6);
        assert!((after[1] - (before[1] - 20.0)).abs() < 1e-6);
    }

    #[test]
    fn dollying_stops_short_of_the_target() {
        let mut camera = far_end();
        camera.dolly(1_000.0);
        assert!((length(sub(camera.position, camera.target)) - NEAREST).abs() < 1e-9);
    }

    /// A scene of two objects, a wall and a stool, for the names.
    fn two_objects() -> scene::Scene {
        let object = |name: &str, kind: &str, label: &str, words: &str, id: u32| scene::Object {
            name: name.into(),
            kind: kind.into(),
            label: label.into(),
            label_text: words.into(),
            named: kind != "wall",
            builder_id: id,
            positions: Vec::new(),
            normals: Vec::new(),
            rest: scene::Transform::IDENTITY,
            under_hinge: None,
        };
        scene::Scene {
            builder: "build-previs.py".into(),
            builder_sha256: String::new(),
            image_size: [10, 10],
            door_note: None,
            looks: scene::Looks {
                grey: Default::default(),
                coded: Default::default(),
                shading: scene::Shading { ambient: 0.72, diffuse: 0.28, light: [0.0, 0.0, 1.0] },
                edges: scene::Edges { grey: 55, coded: 55, simple: None },
                background: Some(255),
                simple: None,
            },
            objects: vec![object("back wall", "wall", "back wall", "back wall", 0), object("stool 3 seat", "stool", "stool", "stools", 1)],
            cameras: Vec::new(),
            swing: None,
        }
    }

    #[test]
    fn names_sit_at_the_median_of_each_groups_pixels() {
        // A 10 × 10 frame: the wall everywhere but a 7 × 7 block of stool,
        // 49 pixels, from x 2 to 8 and y 1 to 7.
        let scene = two_objects();
        let mut objects = vec![0u32; 100];
        for y in 1..8 {
            for x in 2..9 {
                objects[y * 10 + x] = 1;
            }
        }
        assert_eq!(names(&scene, &objects, 10), vec![("stools".to_string(), [5.0, 4.0])]);
        // Fewer pixels than the builder names, and the stool goes unnamed.
        let few: Vec<u32> = objects.iter().enumerate().map(|(i, o)| if *o == 1 && i % 10 > 6 { 0 } else { *o }).collect();
        assert!(names(&scene, &few, 10).is_empty());
    }

    #[test]
    fn a_scene_path_never_leaves_the_project() {
        let declared = |scene: &str| crate::process::View3dShown {
            scene: scene.into(),
            look: protocol::View3dLook::Grey,
            camera: protocol::CameraChoice::Named { name: "far-end-wall".into() },
            reference: None,
            time: 0.0,
            names: false,
            tool: protocol::View3dTool::Orbit,
            transport: None,
            status: None,
        };
        let project = Path::new("/projects/last-order");
        let inside = Shown::from_declared(&declared("prompts/resources/diner-scene.glb"), project);
        assert_eq!(inside.scene, project.join("prompts/resources/diner-scene.glb"));
        assert_eq!(inside.camera, Choice::Named("far-end-wall".into()));
        for outside in ["../other/scene.glb", "/home/someone/scene.glb", "prompts/../../scene.glb"] {
            let shown = Shown::from_declared(&declared(outside), project);
            assert!(shown.scene.starts_with(project), "{outside}");
            assert!(!shown.scene.to_string_lossy().contains(".."), "{outside}");
        }
    }

    /// The diner's scene file from the git-history builder, whose cameras
    /// made the canon's six published pre-vis renders, where Last order is
    /// on disk: set `LAST_ORDER` to its folder.
    fn diner_4195103() -> Option<PathBuf> {
        let Some(folder) = std::env::var_os("LAST_ORDER") else {
            eprintln!("LAST_ORDER is not set, so the 3D view is not drawn");
            return None;
        };
        let resources = PathBuf::from(folder).join("prompts/resources");
        let mut names: Vec<PathBuf> = std::fs::read_dir(&resources)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with("-diner-scene-4195103.glb"))
            .collect();
        names.sort();
        names.pop()
    }

    /// One frame of the window: the 3D view, tessellated and rendered by
    /// egui's own renderer into a target with the window's samples, its
    /// engine prepared and painted through the paint callback as in the
    /// window, and read back as RGB.
    fn window_frame(gpu: Gpu, views: &mut Views, shown: &Shown, size: [u32; 2]) -> Vec<u8> {
        let [w, h] = size;
        let ctx = egui::Context::default();
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(w as f32, h as f32));
        let input = egui::RawInput { screen_rect: Some(area), ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            views.show(ui, Some(gpu), "1", shown, area);
        });
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: output.pixels_per_point };
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                gpu.renderer.write().update_texture(gpu.device, gpu.queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            gpu.renderer.write().free_texture(id);
        }
        output.textures_delta.clear();
        let target = |samples, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: gpu.format,
                usage,
                view_formats: &[],
            })
        };
        let resolved = target(1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let multisampled = target(SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolved_view = resolved.create_view(&Default::default());
        let multisampled_view = multisampled.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let prepared = gpu.renderer.write().update_buffers(gpu.device, gpu.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &multisampled_view,
                    depth_slice: None,
                    resolve_target: Some(&resolved_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::RED), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            gpu.renderer.read().render(&mut pass, &jobs, &screen);
        }
        let row = (w * 4).next_multiple_of(256);
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &resolved, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit(prepared.into_iter().chain([encoder.finish()]));
        read.map_async(wgpu::MapMode::Read, .., |_| ());
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = read.get_mapped_range(..).unwrap();
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let k = (y * row + x * 4) as usize;
                rgb.extend_from_slice(&mapped[k..k + 3]);
            }
        }
        rgb
    }

    /// M7.1's "every edge in place", through the host: each of the
    /// builder's cameras, chosen in a 3D tab whose area is its image's
    /// size, shows exactly the engine's frame of that camera, which the
    /// engine's own check holds to the published render. Drawn on the
    /// software device, as the engine's check is, so it runs alike on any
    /// machine.
    #[test]
    fn a_builders_camera_shows_the_engines_frame_exactly() {
        let Some(path) = diner_4195103() else { return };
        let scene = scene::read(&std::fs::read(&path).unwrap()).unwrap();
        let (device, queue, _) = spumcake_render::device_on(true).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions { msaa_samples: SAMPLES, ..Default::default() },
        )));
        let gpu = Gpu { device: &device, queue: &queue, format, renderer: &renderer };
        let engine = spumcake_render::Renderer::software().unwrap();
        let mut views = Views::default();
        assert_eq!(scene.cameras.len(), 6);
        for camera in &scene.cameras {
            let shown = Shown {
                scene: path.clone(),
                look: Look::Grey,
                camera: Choice::Named(camera.name.clone()),
                reference: None,
                time: 0.0,
                names: false,
                tool: Tool::Orbit,
                transport: None,
                status: None,
            };
            let drawn = window_frame(gpu, &mut views, &shown, camera.image_size);
            let request = Request {
                camera: View::Named(&camera.name),
                look: Look::Grey,
                width: camera.image_size[0],
                height: camera.image_size[1],
                time: 0.0,
            };
            let frame = engine.render(&scene, &request).unwrap();
            let differ = drawn.iter().zip(&frame.colour).filter(|(a, b)| a != b).count();
            assert_eq!((drawn.len(), differ), (frame.colour.len(), 0), "{}: {differ} channels differ", camera.name);
        }
    }

    /// Slice 4: the door swing, as the 3D Viewport shows the simple swing,
    /// the 19-56 builder's far-end-wall camera in the Simple look, shows
    /// exactly the engine's frame at each of the swing's 192 frames. The
    /// engine's own check holds those frames to the published video within
    /// 0.05 grey levels a frame, so the two together are M7.1's "the door
    /// swing replays as its published video shows it".
    #[test]
    fn the_swing_shows_the_engines_frame_at_every_frame() {
        let Some(folder) = std::env::var_os("LAST_ORDER") else {
            eprintln!("LAST_ORDER is not set, so the swing is not drawn");
            return;
        };
        let resources = PathBuf::from(folder).join("prompts/resources");
        let mut names: Vec<PathBuf> = std::fs::read_dir(&resources)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with("-diner-scene-2026-11-14-19-56.glb"))
            .collect();
        names.sort();
        let path = names.pop().expect("no 19-56 diner scene file");
        let scene = scene::read(&std::fs::read(&path).unwrap()).unwrap();
        let swing = scene.swing.as_ref().expect("the 19-56 scene has no swing");
        let camera = scene
            .cameras
            .iter()
            .find(|c| c.reference_file.as_deref().is_some_and(|r| r.ends_with("-simple.mp4")))
            .expect("no camera names the simple swing");
        let (device, queue, _) = spumcake_render::device_on(true).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions { msaa_samples: SAMPLES, ..Default::default() },
        )));
        let gpu = Gpu { device: &device, queue: &queue, format, renderer: &renderer };
        let engine = spumcake_render::Renderer::software().unwrap();
        let mut views = Views::default();
        assert_eq!(swing.times.len(), 192);
        for frame in 0..swing.times.len() {
            let time = frame as f64 / 24.0;
            let shown = Shown {
                scene: path.clone(),
                look: Look::Simple,
                camera: Choice::Named(camera.name.clone()),
                reference: None,
                time,
                names: false,
                tool: Tool::Orbit,
                transport: None,
                status: None,
            };
            let drawn = window_frame(gpu, &mut views, &shown, camera.image_size);
            let request = Request {
                camera: View::Named(&camera.name),
                look: Look::Simple,
                width: camera.image_size[0],
                height: camera.image_size[1],
                time,
            };
            let expected = engine.render(&scene, &request).unwrap();
            let differ = drawn.iter().zip(&expected.colour).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "frame {frame}: {differ} channels differ");
        }
    }

    /// Finding 2 of the intent review: in the window a builder's camera is
    /// drawn smaller than its image, and its published render is laid over
    /// it scaled the same way. Here the "render" is the engine's own frame,
    /// written as the raw file the 3D Viewport sends, laid over the view at
    /// full opacity in a tab about two thirds the image's size: wherever the
    /// bare view has an edge, the covered view has one within a pixel, so a
    /// render that matches the engine at its own size still lines up when
    /// both are scaled.
    #[test]
    fn a_render_laid_over_lines_up_when_the_frame_is_scaled() {
        let Some(path) = diner_4195103() else { return };
        let scene = scene::read(&std::fs::read(&path).unwrap()).unwrap();
        let camera = scene.camera("far-end-wall").unwrap();
        let [w, h] = camera.image_size;
        let engine = spumcake_render::Renderer::software().unwrap();
        let request = Request { camera: View::Named(&camera.name), look: Look::Grey, width: w, height: h, time: 0.0 };
        let frame = engine.render(&scene, &request).unwrap();
        let rgba: Vec<u8> = frame.colour.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let file = std::env::temp_dir().join("far-end-wall-engine.rgba");
        std::fs::write(&file, &rgba).unwrap();

        let (device, queue, _) = spumcake_render::device_on(true).unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let renderer = Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions { msaa_samples: SAMPLES, ..Default::default() },
        )));
        let gpu = Gpu { device: &device, queue: &queue, format, renderer: &renderer };
        let size = [1100, 614];
        let shown = |opacity: f32| Shown {
            scene: path.clone(),
            look: Look::Grey,
            camera: Choice::Named(camera.name.clone()),
            reference: Some(Pane { file: file.clone(), size: [w, h], show: Reference::Over(opacity), captions: None }),
            time: 0.0,
            names: false,
            tool: Tool::Orbit,
            transport: None,
            status: None,
        };
        let bare = window_frame(gpu, &mut Views::default(), &shown(0.0), size);
        let covered = window_frame(gpu, &mut Views::default(), &shown(1.0), size);
        let (sw, sh) = (size[0] as usize, size[1] as usize);
        let grey = |rgb: &[u8], x: usize, y: usize| i32::from(rgb[(y * sw + x) * 3]);
        let edges = |rgb: &[u8]| -> Vec<bool> {
            let mut out = vec![false; sw * sh];
            for y in 1..sh - 1 {
                for x in 1..sw - 1 {
                    let g = (grey(rgb, x + 1, y) - grey(rgb, x - 1, y)).abs() + (grey(rgb, x, y + 1) - grey(rgb, x, y - 1)).abs();
                    out[y * sw + x] = g > 60;
                }
            }
            out
        };
        let (a, b) = (edges(&bare), edges(&covered));
        let near = |e: &[bool], x: usize, y: usize| {
            (y.saturating_sub(1)..=(y + 1).min(sh - 1)).any(|yy| (x.saturating_sub(1)..=(x + 1).min(sw - 1)).any(|xx| e[yy * sw + xx]))
        };
        let mut total = 0;
        let mut astray = 0;
        for y in 0..sh {
            for x in 0..sw {
                if a[y * sw + x] {
                    total += 1;
                    if !near(&b, x, y) {
                        astray += 1;
                    }
                }
            }
        }
        assert!(total > 1000, "the bare view shows {total} edge pixels");
        assert!(astray * 1000 <= total, "{astray} of {total} edge pixels have no edge within a pixel once covered");
    }

    /// Second intent review, finding 1: when the host's clock reaches the
    /// end it reports the time before it says playing stopped, and until the
    /// workspace's next declaration says so, it holds the end rather than
    /// start the clock again from the workspace's older time.
    #[test]
    fn a_scene_camera_keeps_its_builders_shape() {
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 400.0));
        let frame = letterbox(wide, [1664, 928]);
        assert_eq!(frame.height(), 400.0);
        assert!((frame.width() / frame.height() - 1664.0 / 928.0).abs() < 0.01);
        // Centred, to within the half point that whole points leave.
        assert!((frame.center().x - 500.0).abs() <= 0.5);

        let (view, render) = frames(wide, [1664, 928], Some(Reference::Beside), 16.0);
        let render = render.unwrap();
        assert!(view.max.x <= render.min.x - 16.0 + 1.0);
        assert_eq!(view.size(), render.size());
        let (over, none) = frames(wide, [1664, 928], Some(Reference::Over(0.5)), 16.0);
        assert_eq!((over, none), (frame, None));
    }
}
