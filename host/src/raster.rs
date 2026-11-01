//! A raster tab: a picture of pixels a sub-application composites, drawn
//! through a camera the host keeps, and the pointer on it. The host draws the
//! picture, its selection's marching edge, the crop box, and the active
//! layer's box, previews a paint drag while it is drawn, and sends each
//! finished gesture in picture pixels. It knows no editor: what a gesture
//! does is the sub-application's, and the next `raster` view shows it. A
//! view with a filmstrip gives the strip the bottom of the area, and the
//! picture the rest. While the view has a text being typed in, the host
//! draws its box and caret, and the keyboard and the pointer in the box go
//! to the text.

use crate::process::RasterDisplay;
use crate::tablet::Tablet;
use crate::widgets;
use draw::document::Camera;
use eframe::egui::{
    self, Color32, CursorIcon, Event, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect,
    Sense, pos2, vec2,
};
use protocol::{RasterGesture, RasterPatch, RasterTool, RasterView, TextEditing, TextInput, TextKey};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The gray around the picture, as 0.10's approved views show it.
pub(crate) const GROUND: Color32 = Color32::from_rgb(234, 236, 240);
/// The squares drawn under transparency.
const CHECK_LIGHT: Color32 = Color32::from_rgb(255, 255, 255);
const CHECK_DARK: Color32 = Color32::from_rgb(224, 226, 230);
/// A checker square's side, in points.
const CHECK: f32 = 8.0;
/// The move tool's box and handles, in the mockup's blue.
const BOUNDS: Color32 = Color32::from_rgb(37, 99, 235);
/// What lies outside the crop box is dimmed this much, as crop.png shows.
const CROP_DIM: Color32 = Color32::from_black_alpha(110);

/// A raster tab zooms further than a drawing does, so single pixels can be
/// worked on and large photographs can be seen whole.
const MIN_ZOOM: f64 = 1.0 / 64.0;
const MAX_ZOOM: f64 = 64.0;
/// The space Fit leaves around the picture, in points.
const FIT_MARGIN: f64 = 40.0;
/// The toolbar's zoom buttons step as the drawing surface's do.
const BUTTON_STEP: f64 = 5.0 / 4.0;
/// A click of the magnifier doubles the zoom, or halves it with Alt.
const CLICK_STEP: f64 = 2.0;
const NOTCH_POINTS: f64 = 50.0;
/// How near the pointer must be to a crop box's edge to take it.
const GRAB: f32 = 8.0;
/// A sent gesture's preview ends with the next view, or after this long
/// when none comes, as when a locked layer refuses it.
const HELD_FOR: Duration = Duration::from_secs(3);
/// A change of pen pressure that keeps a sample however near the last.
const PRESSURE_STEP: f64 = 0.02;
/// The marching edge's dash, in points, and how often it moves.
const DASH: f32 = 4.0;
const MARCH: Duration = Duration::from_millis(120);
/// Where a tool copies from: a cross of this half-width, dark over white.
const CROSS: f32 = 7.0;
const CROSS_DARK: Color32 = Color32::from_black_alpha(200);
/// A text's selected spans, in the box's blue.
const TEXT_SELECTED: Color32 = Color32::from_rgba_unmultiplied_const(37, 99, 235, 70);
/// The caret shows for half of each second, from the last key or press.
const BLINK: f64 = 1.0;
/// A text box's side is taken this far outside it, never inside, so a
/// press at a line's very start or end places the caret there.
const SIDE_OUT: f32 = 8.0;
const SIDE_IN: f32 = 0.0;

/// What the pointer finished this frame. A paint drag is its own case: the
/// window names it as the protocol does. The filmstrip sends declared
/// actions.
pub(crate) enum Out {
    Gesture(RasterGesture),
    Painted(Vec<[f64; 3]>),
    /// The newest points of a paint drag still open, after `from` sent.
    PaintPart { from: usize, points: Vec<[f64; 3]> },
    /// A paint drag, or a move whose parts were sent, let go with Escape.
    PaintLetGo,
    /// The keyboard or the pointer, for the text being typed in.
    Text(TextInput),
    Action(protocol::Action),
}

/// What a drag shows while it is open, and after it is sent until the
/// sub-application answers.
#[derive(Clone, Debug, PartialEq)]
enum Live {
    /// A paint drag: its points, and how many were sent while it was drawn.
    /// The sub-application paints them as they come; the host draws no
    /// trail of its own, so what is seen while dragging is what is laid.
    Paint { points: Vec<[f64; 3]>, sent: usize },
    Rect { from: [f64; 2], to: [f64; 2] },
    Lasso { points: Vec<[f64; 2]> },
    Move { from: [f64; 2], to: [f64; 2] },
    Crop { rect: [f64; 4] },
    /// The text tool's drag, the box a new text wraps in.
    TextNew { from: [f64; 2], to: [f64; 2] },
    /// A press inside the box of the text typed in, and its drag.
    TextPlace,
    /// A side of the box of the text typed in, dragged: the box as it is
    /// now, and whether the left side was taken.
    TextSide { rect: [f64; 4], left: bool },
}

/// Which part of the crop box a drag took.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    /// A new box, from the press to the pointer.
    New,
    /// The whole box, moved.
    Inside,
    /// These edges of the box: left, top, right, bottom.
    Edges([bool; 4]),
}

struct Drag {
    live: Live,
    /// Where the press was, in picture pixels.
    press: [f64; 2],
    /// The crop box when the press took it.
    crop: Option<(Grab, [f64; 4])>,
    moved: bool,
    /// The last point sent while the drag is open: a move's part, or a
    /// text's selection.
    told: [f64; 2],
}

struct Held {
    live: Live,
    /// The tab's view count when it was sent.
    version: u64,
    at: Instant,
}

/// The pointer's state between frames on the raster tab shown.
pub(crate) struct Pointer {
    drag: Option<Drag>,
    pan: Option<PointerButton>,
    /// The open paint drag takes its samples from the tablet.
    pen: bool,
    held: Vec<Held>,
    /// Where the picture was last drawn: the tab's area above its
    /// filmstrip, when it has one.
    pub(crate) shown: Rect,
    film: crate::film::Strip,
    /// When the text typed in last had a key or a press, in input time,
    /// so the caret shows at once and then blinks.
    typed_at: f64,
}

impl Default for Pointer {
    fn default() -> Self {
        Self {
            drag: None,
            pan: None,
            pen: false,
            held: Vec::new(),
            shown: Rect::NOTHING,
            film: crate::film::Strip::default(),
            typed_at: 0.0,
        }
    }
}

impl Pointer {
    /// Another tab is shown: whatever was being dragged ends.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Draws the tab in `rect` and handles its pointer, with its filmstrip
    /// along the bottom when the view has one. Finished gestures and the
    /// strip's actions go to `out`. Returns whether the camera moved.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        display: &mut RasterDisplay,
        rect: Rect,
        tablet: &Tablet,
        locked: bool,
        dismissing: bool,
        out: &mut Vec<Out>,
    ) -> bool {
        let (rect, strip) = crate::film::split(rect, display.view.film.as_ref());
        self.shown = rect;
        let viewport = [f64::from(rect.width()), f64::from(rect.height())];
        let mut moved = false;
        if display.camera.is_none_or(|camera| !usable(&camera)) {
            fit(display, viewport);
            moved = true;
        }
        let version = display.version;
        self.held
            .retain(|held| held.version == version && held.at.elapsed() < HELD_FOR);

        let response = ui.interact(rect, ui.id().with("raster"), Sense::click_and_drag());
        let hovered = response.hovered();
        let (pos, delta, space, alt) = ui.input(|input| {
            (
                input.pointer.latest_pos(),
                input.pointer.delta(),
                input.key_down(Key::Space),
                input.modifiers.alt,
            )
        });
        let pressed = |button| ui.input(|input| input.pointer.button_pressed(button));
        let down = |button| ui.input(|input| input.pointer.button_down(button));
        let released = |button| ui.input(|input| input.pointer.button_released(button));
        let over = hovered && pos.is_some_and(|pos| rect.contains(pos));
        // A press that closes a menu does nothing else.
        if dismissing && over && (pressed(PointerButton::Primary) || pressed(PointerButton::Middle)) {
            egui::Popup::close_all(ui.ctx());
        }
        let press_inside = over && !dismissing;
        let tool = display.view.tool.clone();
        // Space types into a text rather than pans.
        let typing = display.view.text.is_some();
        let space = space && !typing;
        if typing && !locked && !ui.ctx().text_edit_focused() {
            self.keyboard(ui, out);
        }

        if self.drag.is_some() && ui.input(|input| input.key_pressed(Key::Escape)) {
            let parted = match &self.drag {
                Some(Drag { live: Live::Paint { sent, .. }, .. }) => *sent > 0,
                Some(Drag { live: Live::Move { .. }, press, told, .. }) => told != press,
                _ => false,
            };
            if parted {
                out.push(Out::PaintLetGo);
            }
            self.drag = None;
            self.pen = false;
        }
        if self.pan.is_none() && self.drag.is_none() && press_inside {
            if pressed(PointerButton::Middle) {
                self.pan = Some(PointerButton::Middle);
            } else if pressed(PointerButton::Primary) {
                if space || tool == RasterTool::Hand {
                    self.pan = Some(PointerButton::Primary);
                } else if tool == RasterTool::Zoom {
                    if let Some(pos) = pos {
                        let factor = if alt { 1.0 / CLICK_STEP } else { CLICK_STEP };
                        zoom_toward(display, factor, pos - rect.min);
                        moved = true;
                    }
                } else if !locked && let Some(pos) = pos {
                    self.press(ui, display, rect, tablet, pos, out);
                    self.send_part(out);
                }
            }
        }

        if let Some(button) = self.pan {
            if delta != egui::Vec2::ZERO
                && let Some(camera) = display.camera.as_mut()
            {
                pan(camera, delta);
                moved = true;
            }
            if released(button) || !down(button) {
                self.pan = None;
            }
        } else if self.drag.is_some() {
            self.update(ui, display, rect, tablet, out);
            self.send_part(out);
            if released(PointerButton::Primary) || !down(PointerButton::Primary) {
                self.finish(display, out);
            }
        }
        // A double-click in the box of the text typed in takes its word.
        if let (Some(text), Some(camera), Some(pos)) = (&display.view.text, display.camera, pos)
            && press_inside
            && !locked
            && self.pan.is_none()
            && ui.input(|input| input.pointer.button_double_clicked(PointerButton::Primary))
            && box_on_screen(&camera, rect, text.rect).contains(pos)
        {
            out.push(Out::Text(TextInput::Word { at: picture_at(&camera, rect, pos) }));
        }

        moved |= wheel(ui, display, rect, over || self.drag.is_some() || self.pan.is_some());

        let cursor = self.cursor(ui, display, rect, &tool, space, alt, locked);
        response.on_hover_cursor(cursor);
        self.paint(ui, display, rect);
        if let (Some(film), Some(strip)) = (&display.view.film, strip) {
            let mut actions = Vec::new();
            self.film.show(ui, film, strip, locked, dismissing, &mut actions);
            out.extend(actions.into_iter().map(Out::Action));
        }
        moved
    }

    /// A primary press with the declared tool.
    fn press(
        &mut self,
        ui: &egui::Ui,
        display: &RasterDisplay,
        rect: Rect,
        tablet: &Tablet,
        pos: Pos2,
        out: &mut Vec<Out>,
    ) {
        let Some(camera) = display.camera else {
            return;
        };
        let at = picture_at(&camera, rect, pos);
        let (shift, alt) = ui.input(|input| (input.modifiers.shift, input.modifiers.alt));
        // While a text is typed in, a press in its box goes to the text, a
        // side drags the box, and a press elsewhere is the text tool's.
        if let Some(text) = &display.view.text {
            self.typed_at = ui.input(|input| input.time);
            let frame = box_on_screen(&camera, rect, text.rect);
            if let Some(left) = text_side(frame, pos) {
                self.drag = Some(Drag {
                    live: Live::TextSide { rect: text.rect, left },
                    press: at,
                    crop: None,
                    moved: false,
                    told: at,
                });
                return;
            }
            let live = if frame.contains(pos) {
                out.push(Out::Text(TextInput::Place { at, extend: shift }));
                Live::TextPlace
            } else {
                Live::TextNew { from: at, to: at }
            };
            self.drag = Some(Drag { live, press: at, crop: None, moved: false, told: at });
            return;
        }
        let live = match &display.view.tool {
            // Alt-click sets where the tool copies from.
            RasterTool::Sampler { .. } if alt => {
                out.push(Out::Gesture(RasterGesture::Source { at }));
                return;
            }
            RasterTool::Brush { .. } | RasterTool::Erase { .. } | RasterTool::Sampler { .. } => {
                Live::Paint { points: Vec::new(), sent: 0 }
            }
            RasterTool::Text => Live::TextNew { from: at, to: at },
            RasterTool::Marquee => Live::Rect { from: at, to: at },
            RasterTool::Lasso => Live::Lasso { points: vec![at] },
            RasterTool::Move => Live::Move { from: at, to: at },
            RasterTool::Crop => {
                let (grab, start) = match display.view.crop {
                    Some(crop) => (crop_grab(&camera, rect, crop, pos), crop),
                    None => (Grab::New, [at[0], at[1], 0.0, 0.0]),
                };
                let mut drag = Drag {
                    live: Live::Crop { rect: start },
                    press: at,
                    crop: Some((grab, start)),
                    moved: false,
                    told: at,
                };
                if grab == Grab::New {
                    drag.live = Live::Crop { rect: [at[0], at[1], 0.0, 0.0] };
                }
                self.drag = Some(drag);
                return;
            }
            RasterTool::Eyedropper => {
                out.push(Out::Gesture(RasterGesture::Pick { at }));
                return;
            }
            RasterTool::Hand | RasterTool::Zoom => return,
        };
        let paints = matches!(live, Live::Paint { .. });
        self.drag = Some(Drag { live, press: at, crop: None, moved: false, told: at });
        if paints {
            self.pen = tablet.input().down();
            self.sample(ui, &camera, rect, tablet, spacing(&display.view.tool));
        }
    }

    /// The open drag follows the pointer. A move sends its part, and a drag
    /// in a text's box its selection, each time the pointer moves.
    fn update(&mut self, ui: &egui::Ui, display: &RasterDisplay, rect: Rect, tablet: &Tablet, out: &mut Vec<Out>) {
        let Some(camera) = display.camera else {
            return;
        };
        let Some(pos) = ui.input(|input| input.pointer.latest_pos()) else {
            return;
        };
        let at = picture_at(&camera, rect, pos);
        let size = [f64::from(display.view.width), f64::from(display.view.height)];
        let paints = matches!(self.drag.as_ref().map(|drag| &drag.live), Some(Live::Paint { .. }));
        if paints {
            self.sample(ui, &camera, rect, tablet, spacing(&display.view.tool));
            return;
        }
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if at != drag.press {
            drag.moved = true;
        }
        match &mut drag.live {
            Live::Rect { to, .. } | Live::TextNew { to, .. } => *to = at,
            Live::Move { from, to } => {
                *to = at;
                if drag.told != at {
                    drag.told = at;
                    out.push(Out::Gesture(RasterGesture::DragPart { from: *from, to: at }));
                }
            }
            Live::TextPlace => {
                if drag.told != at {
                    drag.told = at;
                    self.typed_at = ui.input(|input| input.time);
                    out.push(Out::Text(TextInput::Select { at }));
                }
            }
            Live::TextSide { rect: box_, left } => {
                if let Some(text) = &display.view.text {
                    *box_ = dragged_side(text.rect, *left, at[0]);
                }
            }
            Live::Lasso { points } => {
                if points.last() != Some(&at) {
                    points.push(at);
                }
            }
            Live::Crop { rect: crop } => {
                if let Some((grab, start)) = drag.crop {
                    *crop = dragged_crop(grab, start, drag.press, at, size);
                }
            }
            Live::Paint { .. } => {}
        }
    }

    /// Adds this frame's pointer or pen samples to the open paint drag, in
    /// picture pixels, with the pen's pressure or 1 with a mouse. A sample
    /// nearer the last than `spacing`, at much the same pressure, is left
    /// out: the paint moves less than that from where it would be, and a
    /// large brush is not painted over itself for every pixel moved.
    fn sample(&mut self, ui: &egui::Ui, camera: &Camera, rect: Rect, tablet: &Tablet, spacing: f64) {
        let Some(Drag { live: Live::Paint { points, .. }, .. }) = self.drag.as_mut() else {
            return;
        };
        let mut samples: Vec<(Pos2, f64)> = Vec::new();
        if self.pen {
            for sample in tablet.input().samples() {
                samples.push((sample.pos, sample.pressure.unwrap_or(1.0).clamp(0.0, 1.0)));
            }
        } else {
            ui.input(|input| {
                for event in &input.events {
                    match event {
                        Event::PointerMoved(pos) => samples.push((*pos, 1.0)),
                        Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => {
                            samples.push((*pos, 1.0));
                        }
                        _ => {}
                    }
                }
            });
            if samples.is_empty()
                && points.is_empty()
                && let Some(pos) = ui.input(|input| input.pointer.latest_pos())
            {
                samples.push((pos, 1.0));
            }
        }
        for (pos, pressure) in samples {
            let at = picture_at(camera, rect, pos);
            let point = [at[0], at[1], pressure];
            let apart = |last: &[f64; 3]| {
                (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing || (point[2] - last[2]).abs() > PRESSURE_STEP
            };
            if points.last().is_none_or(apart) {
                points.push(point);
            }
        }
    }

    /// The paint drag's points not sent yet go to the sub-application, which
    /// paints them at once.
    fn send_part(&mut self, out: &mut Vec<Out>) {
        if let Some(Drag { live: Live::Paint { points, sent }, .. }) = self.drag.as_mut()
            && points.len() > *sent
        {
            out.push(Out::PaintPart { from: *sent, points: points[*sent..].to_vec() });
            *sent = points.len();
        }
    }

    /// The drag is released: its gesture is sent, and its preview held.
    fn finish(&mut self, display: &RasterDisplay, out: &mut Vec<Out>) {
        self.pen = false;
        let Some(drag) = self.drag.take() else {
            return;
        };
        let gesture = match &drag.live {
            // The paint is on the picture already: nothing is held.
            Live::Paint { points, .. } if !points.is_empty() => {
                out.push(Out::Painted(points.clone()));
                return;
            }
            Live::Rect { from, to } => Out::Gesture(RasterGesture::Rect { from: *from, to: *to }),
            Live::Lasso { points } if points.len() >= 3 => {
                Out::Gesture(RasterGesture::Lasso { points: points.clone() })
            }
            Live::Move { from, to } if drag.moved => Out::Gesture(RasterGesture::Drag { from: *from, to: *to }),
            // A click, or a drag that makes a box to wrap in.
            Live::TextNew { from, to } if drag.moved => Out::Gesture(RasterGesture::Rect { from: *from, to: *to }),
            Live::TextNew { from, .. } => {
                out.push(Out::Gesture(RasterGesture::Click { at: *from }));
                return;
            }
            Live::TextSide { rect, .. } if drag.moved => Out::Gesture(RasterGesture::TextBox { rect: *rect }),
            Live::Crop { rect } if drag.moved => {
                let rect = whole_pixels(*rect, [display.view.width, display.view.height]);
                if rect[2] < 1.0 || rect[3] < 1.0 {
                    return;
                }
                self.held.push(Held { live: Live::Crop { rect }, version: display.version, at: Instant::now() });
                out.push(Out::Gesture(RasterGesture::Crop { rect }));
                return;
            }
            _ => return,
        };
        self.held.push(Held { live: drag.live, version: display.version, at: Instant::now() });
        out.push(gesture);
    }

    #[allow(clippy::too_many_arguments)]
    fn cursor(
        &self,
        ui: &egui::Ui,
        display: &RasterDisplay,
        rect: Rect,
        tool: &RasterTool,
        space: bool,
        alt: bool,
        locked: bool,
    ) -> CursorIcon {
        if self.pan.is_some() {
            return CursorIcon::Grabbing;
        }
        if space || *tool == RasterTool::Hand {
            return CursorIcon::Grab;
        }
        if *tool == RasterTool::Zoom {
            return if alt { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn };
        }
        if locked {
            return CursorIcon::Default;
        }
        if let (Some(text), Some(camera), Some(pos)) = (&display.view.text, display.camera, ui.input(|input| input.pointer.hover_pos())) {
            let frame = box_on_screen(&camera, rect, text.rect);
            let side = matches!(&self.drag, Some(Drag { live: Live::TextSide { .. }, .. }));
            return if side || (self.drag.is_none() && text_side(frame, pos).is_some()) {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::Text
            };
        }
        match tool {
            RasterTool::Move => CursorIcon::Move,
            // Alt with a tool that copies from a source aims at it.
            RasterTool::Sampler { .. } if alt => CursorIcon::Crosshair,
            // The brush's own ring stands for the pointer, however small.
            RasterTool::Brush { .. } | RasterTool::Erase { .. } | RasterTool::Sampler { .. } => CursorIcon::None,
            RasterTool::Text => CursorIcon::Text,
            RasterTool::Crop => {
                let grab = match (&self.drag, display.camera, display.view.crop, ui.input(|input| input.pointer.hover_pos())) {
                    (Some(drag), _, _, _) => drag.crop.map(|(grab, _)| grab),
                    (None, Some(camera), Some(crop), Some(pos)) => Some(crop_grab(&camera, rect, crop, pos)),
                    _ => None,
                };
                match grab {
                    Some(Grab::Edges([left, top, right, bottom])) => match (left || right, top || bottom) {
                        (true, false) => CursorIcon::ResizeHorizontal,
                        (false, true) => CursorIcon::ResizeVertical,
                        _ if (left && top) || (right && bottom) => CursorIcon::ResizeNwSe,
                        _ => CursorIcon::ResizeNeSw,
                    },
                    Some(Grab::Inside) => CursorIcon::Move,
                    _ => CursorIcon::Crosshair,
                }
            }
            _ => CursorIcon::Crosshair,
        }
    }

    /// The picture, then what lies over it: paint previews, the selection,
    /// the crop box, the layer's box, and the brush's ring.
    fn paint(&self, ui: &egui::Ui, display: &RasterDisplay, rect: Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, GROUND);
        let Some(camera) = display.camera else {
            return;
        };
        let view = &display.view;
        let picture = picture_rect(&camera, rect, view);
        // The picture, then a border around it, as the owner set on
        // 2026-10-30, so a white picture stands apart from the grey.
        match raw_picture(ui.ctx(), &view.picture, view.width, view.height, view.revision, &display.patches, view.patch.as_ref().is_some_and(|patch| patch.file.is_some())) {
            Some((texture, opaque)) => {
                if !opaque {
                    checker(ui.ctx(), &painter, picture);
                }
                painter.image(texture, picture, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => checker(ui.ctx(), &painter, picture),
        }
        widgets::outlined(&painter, picture.expand(1.0), 0.0, Color32::TRANSPARENT, widgets::RAIL);

        let lives = self.held.iter().map(|held| &held.live).chain(self.drag.iter().map(|drag| &drag.live));
        let mut crop = view.crop;
        let mut offset = [0.0, 0.0];
        let mut marching = false;
        let mut text_box = view.text.as_ref().map(|text| text.rect);
        for live in lives {
            match live {
                Live::Paint { .. } | Live::TextPlace => {}
                Live::TextNew { from, to } => {
                    let frame = Rect::from_two_pos(screen_at(&camera, rect, *from), screen_at(&camera, rect, *to));
                    hairline_box(&painter, frame);
                }
                Live::TextSide { rect: box_, .. } => {
                    if text_box.is_some() {
                        text_box = Some(*box_);
                    }
                }
                Live::Rect { from, to } => {
                    let corners = [*from, [to[0], from[1]], *to, [from[0], to[1]]];
                    let points: Vec<Pos2> = corners.iter().map(|point| screen_at(&camera, rect, *point)).collect();
                    marching_edge(ui, &painter, &points, true);
                    marching = true;
                }
                Live::Lasso { points } => {
                    let points: Vec<Pos2> = points.iter().map(|point| screen_at(&camera, rect, *point)).collect();
                    marching_edge(ui, &painter, &points, false);
                    marching = true;
                }
                Live::Move { from, to } => offset = [to[0] - from[0], to[1] - from[1]],
                Live::Crop { rect: box_ } => crop = Some(*box_),
            }
        }
        for outline in &view.selection {
            let points: Vec<Pos2> = outline.iter().map(|point| screen_at(&camera, rect, *point)).collect();
            marching_edge(ui, &painter, &points, true);
            marching = true;
        }
        if marching {
            ui.ctx().request_repaint_after(MARCH);
        }
        if let Some(crop) = crop {
            paint_crop(&painter, &camera, rect, picture, crop);
        }
        if view.tool == RasterTool::Move
            && let Some(bounds) = view.bounds
        {
            let moved = [bounds[0] + offset[0], bounds[1] + offset[1], bounds[2], bounds[3]];
            paint_bounds(&painter, box_on_screen(&camera, rect, moved));
        }
        if let (Some(text), Some(box_)) = (&view.text, text_box) {
            self.paint_text(ui, &painter, &camera, rect, text, box_);
        }
        let hover = ui.input(|input| input.pointer.hover_pos()).filter(|pos| rect.contains(*pos) && self.pan.is_none());
        if let RasterTool::Brush { size, .. } | RasterTool::Erase { size, .. } | RasterTool::Sampler { size, .. } = view.tool
            && let Some(pos) = hover
        {
            // A small brush still shows a ring the eye can find.
            let radius = ((size * camera.zoom / 2.0) as f32).max(3.0);
            widgets::ring(&painter, pos, radius, 1.0, Color32::from_black_alpha(160));
            widgets::ring(&painter, pos, radius + 1.0, 1.0, Color32::from_white_alpha(200));
        }
        // Where the tool copies from: the pointer plus the offset while one
        // holds, or else the source.
        if let RasterTool::Sampler { source, offset, .. } = view.tool {
            let from = match (offset, hover) {
                (Some(offset), Some(pos)) => {
                    let at = picture_at(&camera, rect, pos);
                    Some([at[0] + offset[0], at[1] + offset[1]])
                }
                (Some(_), None) => None,
                (None, _) => source,
            };
            if let Some(from) = from {
                cross(&painter, screen_at(&camera, rect, from));
            }
        }
    }

    /// The text typed in, as text.png shows it: its selected spans, its box
    /// as a blue hairline with handles, and its caret, blinking. Only the
    /// handles that work are drawn: the corners and the middles of the left
    /// and right sides, which change the box's width.
    fn paint_text(&self, ui: &egui::Ui, painter: &egui::Painter, camera: &Camera, rect: Rect, text: &TextEditing, box_: [f64; 4]) {
        for span in &text.selection {
            painter.rect_filled(box_on_screen(camera, rect, *span), 0.0, TEXT_SELECTED);
        }
        let frame = box_on_screen(camera, rect, box_);
        hairline_box(painter, frame);
        for handle in [
            frame.left_top(),
            frame.left_center(),
            frame.left_bottom(),
            frame.right_top(),
            frame.right_center(),
            frame.right_bottom(),
        ] {
            widgets::outlined(painter, Rect::from_center_size(handle, vec2(7.0, 7.0)), 1.0, Color32::WHITE, BOUNDS);
        }
        let now = ui.input(|input| input.time);
        let since = (now - self.typed_at).max(0.0);
        let phase = since % BLINK;
        if phase < BLINK / 2.0 {
            let caret = box_on_screen(camera, rect, text.caret);
            let caret = Rect::from_center_size(caret.center(), vec2(caret.width().max(1.5), caret.height()));
            widgets::outlined(painter, caret.expand(0.5), 0.0, Color32::WHITE, Color32::from_black_alpha(90));
        }
        let next = if phase < BLINK / 2.0 { BLINK / 2.0 - phase } else { BLINK - phase };
        ui.ctx().request_repaint_after(Duration::from_secs_f64(next.max(0.01)));
    }

    /// While a text is typed in, the keyboard is the text's: typed and
    /// pasted words, its keys, and Copy and Cut. Undo and redo stay the
    /// window's shortcuts.
    fn keyboard(&mut self, ui: &egui::Ui, out: &mut Vec<Out>) {
        let (events, now) = ui.input(|input| (input.events.clone(), input.time));
        // The window's Copy and Cut come as events of their own, and their
        // key-down may come too: the event is the one taken.
        let clipboard = events.iter().any(|event| matches!(event, Event::Copy | Event::Cut));
        for event in events {
            let input = match event {
                Event::Text(text) | Event::Paste(text) if !text.is_empty() => TextInput::Insert { text },
                Event::Copy => TextInput::Key { key: TextKey::Copy, shift: false, word: false },
                Event::Cut => TextInput::Key { key: TextKey::Cut, shift: false, word: false },
                Event::Key { key, pressed: true, modifiers, .. } => match text_key(key, modifiers, clipboard) {
                    Some(key) => TextInput::Key { key, shift: modifiers.shift, word: word_held(modifiers) },
                    None => continue,
                },
                _ => continue,
            };
            self.typed_at = now;
            out.push(Out::Text(input));
        }
    }
}

/// The text key a pressed key is, if any. Ctrl+Z, Ctrl+Y, and other
/// shortcuts are none.
fn text_key(key: Key, modifiers: Modifiers, clipboard: bool) -> Option<TextKey> {
    Some(match key {
        Key::Backspace => TextKey::Backspace,
        Key::Delete => TextKey::Delete,
        Key::ArrowLeft => TextKey::Left,
        Key::ArrowRight => TextKey::Right,
        Key::ArrowUp => TextKey::Up,
        Key::ArrowDown => TextKey::Down,
        Key::Home => TextKey::Home,
        Key::End => TextKey::End,
        Key::Enter => TextKey::Enter,
        Key::Escape => TextKey::Escape,
        Key::A if modifiers.command => TextKey::SelectAll,
        Key::C if modifiers.command && !clipboard => TextKey::Copy,
        Key::X if modifiers.command && !clipboard => TextKey::Cut,
        _ => return None,
    })
}

/// Moving by words: Ctrl held, or Alt on a Mac.
fn word_held(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") { modifiers.alt } else { modifiers.ctrl }
}

/// Which side of a text's box a press at `pos` takes, as true for the left:
/// the side's edge, a little inside it and more outside, along its height.
fn text_side(frame: Rect, pos: Pos2) -> Option<bool> {
    if pos.y < frame.top() - SIDE_OUT || pos.y > frame.bottom() + SIDE_OUT {
        return None;
    }
    if pos.x >= frame.left() - SIDE_OUT && pos.x <= frame.left() + SIDE_IN {
        Some(true)
    } else if pos.x >= frame.right() - SIDE_IN && pos.x <= frame.right() + SIDE_OUT {
        Some(false)
    } else {
        None
    }
}

/// A text's box with one side dragged to `x`, at least a pixel wide.
fn dragged_side(box_: [f64; 4], left: bool, x: f64) -> [f64; 4] {
    let right = box_[0] + box_[2];
    if left {
        let x = x.min(right - 1.0);
        [x, box_[1], right - x, box_[3]]
    } else {
        [box_[0], box_[1], (x - box_[0]).max(1.0), box_[3]]
    }
}

/// A thin blue edge, as a text's box is drawn.
fn hairline_box(painter: &egui::Painter, frame: Rect) {
    let line = (1.0, BOUNDS);
    painter.line_segment([frame.left_top(), frame.right_top()], line);
    painter.line_segment([frame.right_top(), frame.right_bottom()], line);
    painter.line_segment([frame.right_bottom(), frame.left_bottom()], line);
    painter.line_segment([frame.left_bottom(), frame.left_top()], line);
}

/// A thin cross, dark over white so it shows on any picture.
fn cross(painter: &egui::Painter, at: Pos2) {
    for (width, color) in [(3.0, Color32::from_white_alpha(200)), (1.0, CROSS_DARK)] {
        painter.line_segment([at - vec2(CROSS, 0.0), at + vec2(CROSS, 0.0)], (width, color));
        painter.line_segment([at - vec2(0.0, CROSS), at + vec2(0.0, CROSS)], (width, color));
    }
}

/// How far apart, in picture pixels, a paint drag's samples are kept: a
/// twenty-fifth of the brush, from half a pixel to eight.
fn spacing(tool: &RasterTool) -> f64 {
    match tool {
        RasterTool::Brush { size, .. } | RasterTool::Erase { size, .. } | RasterTool::Sampler { size, .. } => {
            (size / 25.0).clamp(0.5, 8.0)
        }
        _ => 0.5,
    }
}

/// A camera that can draw: a finite origin and a zoom above nothing.
fn usable(camera: &Camera) -> bool {
    camera.origin[0].is_finite() && camera.origin[1].is_finite() && camera.zoom.is_finite() && camera.zoom > 0.0
}

/// The picture's place on screen.
fn picture_rect(camera: &Camera, rect: Rect, view: &RasterView) -> Rect {
    Rect::from_min_max(
        screen_at(camera, rect, [0.0, 0.0]),
        screen_at(camera, rect, [f64::from(view.width), f64::from(view.height)]),
    )
}

fn picture_at(camera: &Camera, rect: Rect, pos: Pos2) -> [f64; 2] {
    [
        camera.origin[0] + f64::from(pos.x - rect.min.x) / camera.zoom,
        camera.origin[1] + f64::from(pos.y - rect.min.y) / camera.zoom,
    ]
}

fn screen_at(camera: &Camera, rect: Rect, point: [f64; 2]) -> Pos2 {
    pos2(
        ((point[0] - camera.origin[0]) * camera.zoom) as f32 + rect.min.x,
        ((point[1] - camera.origin[1]) * camera.zoom) as f32 + rect.min.y,
    )
}

fn box_on_screen(camera: &Camera, rect: Rect, box_: [f64; 4]) -> Rect {
    Rect::from_two_pos(
        screen_at(camera, rect, [box_[0], box_[1]]),
        screen_at(camera, rect, [box_[0] + box_[2], box_[1] + box_[3]]),
    )
}

// The camera, which the host keeps while the tab is shown.

/// Frames the whole picture in the viewport, with room around it.
pub(crate) fn fit(display: &mut RasterDisplay, viewport: [f64; 2]) {
    let size = [f64::from(display.view.width.max(1)), f64::from(display.view.height.max(1))];
    frame(display, [0.0, 0.0, size[0], size[1]], viewport);
}

/// Frames the selection, or the whole picture when nothing is selected.
pub(crate) fn zoom_to_selection(display: &mut RasterDisplay, viewport: [f64; 2]) {
    let points = display.view.selection.iter().flatten();
    let mut bounds: Option<[f64; 4]> = None;
    for point in points {
        let [x0, y0, x1, y1] = bounds.unwrap_or([point[0], point[1], point[0], point[1]]);
        bounds = Some([x0.min(point[0]), y0.min(point[1]), x1.max(point[0]), y1.max(point[1])]);
    }
    match bounds {
        Some([x0, y0, x1, y1]) if x1 > x0 && y1 > y0 => frame(display, [x0, y0, x1 - x0, y1 - y0], viewport),
        _ => fit(display, viewport),
    }
}

fn frame(display: &mut RasterDisplay, box_: [f64; 4], viewport: [f64; 2]) {
    if viewport[0] <= 0.0 || viewport[1] <= 0.0 {
        return;
    }
    let room = [(viewport[0] - 2.0 * FIT_MARGIN).max(1.0), (viewport[1] - 2.0 * FIT_MARGIN).max(1.0)];
    let zoom = (room[0] / box_[2].max(1.0)).min(room[1] / box_[3].max(1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
    let center = [box_[0] + box_[2] / 2.0, box_[1] + box_[3] / 2.0];
    display.camera = Some(Camera {
        origin: [center[0] - viewport[0] / (2.0 * zoom), center[1] - viewport[1] / (2.0 * zoom)],
        zoom,
    });
}

/// The toolbar's zoom in or out, about the viewport's center.
pub(crate) fn zoom_step(display: &mut RasterDisplay, zoom_in: bool, viewport: [f64; 2]) {
    let factor = if zoom_in { BUTTON_STEP } else { 1.0 / BUTTON_STEP };
    zoom_toward(display, factor, vec2((viewport[0] / 2.0) as f32, (viewport[1] / 2.0) as f32));
}

/// A zoom chosen by number, such as 1 for 100%, about the viewport's center.
pub(crate) fn set_zoom(display: &mut RasterDisplay, zoom: f64, viewport: [f64; 2]) {
    let Some(camera) = display.camera else {
        return;
    };
    if !zoom.is_finite() || zoom <= 0.0 {
        return;
    }
    zoom_toward(display, zoom / camera.zoom, vec2((viewport[0] / 2.0) as f32, (viewport[1] / 2.0) as f32));
}

/// Zooms by `factor`, keeping the picture point under `pointer`, which is
/// measured from the viewport's corner, where it is.
fn zoom_toward(display: &mut RasterDisplay, factor: f64, pointer: egui::Vec2) {
    let Some(camera) = display.camera.as_mut() else {
        return;
    };
    if !factor.is_finite() || factor <= 0.0 {
        return;
    }
    let old = camera.zoom;
    let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let at = [f64::from(pointer.x), f64::from(pointer.y)];
    let point = [camera.origin[0] + at[0] / old, camera.origin[1] + at[1] / old];
    camera.zoom = new;
    camera.origin = [point[0] - at[0] / new, point[1] - at[1] / new];
}

fn pan(camera: &mut Camera, delta: egui::Vec2) {
    camera.origin[0] -= f64::from(delta.x) / camera.zoom;
    camera.origin[1] -= f64::from(delta.y) / camera.zoom;
}

/// The wheel pans, sideways with Shift, and zooms at the pointer with the
/// command key, as on the drawing surface.
fn wheel(ui: &egui::Ui, display: &mut RasterDisplay, rect: Rect, inside: bool) -> bool {
    if !inside {
        return false;
    }
    let hover = ui.input(|input| input.pointer.hover_pos());
    let events = ui.input(|input| input.events.clone());
    let mut moved = false;
    for event in events {
        let Event::MouseWheel { unit, delta, modifiers, .. } = event else {
            continue;
        };
        let scale = match unit {
            MouseWheelUnit::Point => 1.0,
            MouseWheelUnit::Line => NOTCH_POINTS,
            MouseWheelUnit::Page => f64::from(rect.height()),
        };
        let x = f64::from(delta.x) * scale;
        let y = f64::from(delta.y) * scale;
        if modifiers.command {
            if y != 0.0 {
                let pointer = hover.unwrap_or(rect.center()) - rect.min;
                zoom_toward(display, (5.0_f64 / 4.0).powf(y / NOTCH_POINTS), pointer);
                moved = true;
            }
        } else if let Some(camera) = display.camera.as_mut() {
            let step = if modifiers.shift { vec2((x + y) as f32, 0.0) } else { vec2(x as f32, y as f32) };
            pan(camera, step);
            moved = true;
        }
    }
    moved
}

// The crop box.

/// Which part of the shown crop box a press at `pos` takes.
fn crop_grab(camera: &Camera, rect: Rect, crop: [f64; 4], pos: Pos2) -> Grab {
    let frame = box_on_screen(camera, rect, crop);
    let along_y = pos.y >= frame.top() - GRAB && pos.y <= frame.bottom() + GRAB;
    let along_x = pos.x >= frame.left() - GRAB && pos.x <= frame.right() + GRAB;
    let edges = [
        along_y && (pos.x - frame.left()).abs() <= GRAB,
        along_x && (pos.y - frame.top()).abs() <= GRAB,
        along_y && (pos.x - frame.right()).abs() <= GRAB,
        along_x && (pos.y - frame.bottom()).abs() <= GRAB,
    ];
    if edges.iter().any(|edge| *edge) {
        Grab::Edges(edges)
    } else if frame.contains(pos) {
        Grab::Inside
    } else {
        Grab::New
    }
}

/// The crop box a drag from `press` to `at` makes of `start`, kept on the
/// picture.
fn dragged_crop(grab: Grab, start: [f64; 4], press: [f64; 2], at: [f64; 2], size: [f64; 2]) -> [f64; 4] {
    let clamp = |point: [f64; 2]| [point[0].clamp(0.0, size[0]), point[1].clamp(0.0, size[1])];
    let corners = |a: [f64; 2], b: [f64; 2]| {
        [a[0].min(b[0]), a[1].min(b[1]), (a[0] - b[0]).abs(), (a[1] - b[1]).abs()]
    };
    match grab {
        Grab::New => corners(clamp(press), clamp(at)),
        Grab::Inside => {
            let x = (start[0] + at[0] - press[0]).clamp(0.0, (size[0] - start[2]).max(0.0));
            let y = (start[1] + at[1] - press[1]).clamp(0.0, (size[1] - start[3]).max(0.0));
            [x, y, start[2], start[3]]
        }
        Grab::Edges([left, top, right, bottom]) => {
            let at = clamp(at);
            let mut a = [start[0], start[1]];
            let mut b = [start[0] + start[2], start[1] + start[3]];
            if left {
                a[0] = at[0];
            }
            if top {
                a[1] = at[1];
            }
            if right {
                b[0] = at[0];
            }
            if bottom {
                b[1] = at[1];
            }
            corners(a, b)
        }
    }
}

/// A crop box on whole pixels, inside the picture.
fn whole_pixels(box_: [f64; 4], size: [u32; 2]) -> [f64; 4] {
    let x0 = box_[0].round().clamp(0.0, f64::from(size[0]));
    let y0 = box_[1].round().clamp(0.0, f64::from(size[1]));
    let x1 = (box_[0] + box_[2]).round().clamp(0.0, f64::from(size[0]));
    let y1 = (box_[1] + box_[3]).round().clamp(0.0, f64::from(size[1]));
    [x0, y0, x1 - x0, y1 - y0]
}

/// The crop box as crop.png shows it: the picture outside it dimmed, a
/// white edge with thirds, and heavier corners.
fn paint_crop(painter: &egui::Painter, camera: &Camera, rect: Rect, picture: Rect, crop: [f64; 4]) {
    let frame = box_on_screen(camera, rect, crop);
    for dim in [
        Rect::from_min_max(picture.min, pos2(picture.right(), frame.top())),
        Rect::from_min_max(pos2(picture.left(), frame.bottom()), picture.max),
        Rect::from_min_max(pos2(picture.left(), frame.top()), pos2(frame.left(), frame.bottom())),
        Rect::from_min_max(pos2(frame.right(), frame.top()), pos2(picture.right(), frame.bottom())),
    ] {
        if dim.is_positive() {
            painter.rect_filled(dim, 0.0, CROP_DIM);
        }
    }
    let line = (1.0, Color32::WHITE);
    for third in [1.0 / 3.0, 2.0 / 3.0] {
        let x = frame.left() + frame.width() * third;
        let y = frame.top() + frame.height() * third;
        painter.line_segment([pos2(x, frame.top()), pos2(x, frame.bottom())], (1.0, Color32::from_white_alpha(190)));
        painter.line_segment([pos2(frame.left(), y), pos2(frame.right(), y)], (1.0, Color32::from_white_alpha(190)));
    }
    painter.line_segment([frame.left_top(), frame.right_top()], line);
    painter.line_segment([frame.right_top(), frame.right_bottom()], line);
    painter.line_segment([frame.right_bottom(), frame.left_bottom()], line);
    painter.line_segment([frame.left_bottom(), frame.left_top()], line);
    let arm = 14.0_f32.min(frame.width() / 2.0).min(frame.height() / 2.0);
    let heavy = (3.0, Color32::WHITE);
    for (corner, dx, dy) in [
        (frame.left_top(), 1.0, 1.0),
        (frame.right_top(), -1.0, 1.0),
        (frame.right_bottom(), -1.0, -1.0),
        (frame.left_bottom(), 1.0, -1.0),
    ] {
        painter.line_segment([corner, corner + vec2(dx * arm, 0.0)], heavy);
        painter.line_segment([corner, corner + vec2(0.0, dy * arm)], heavy);
    }
}

/// The active layer's box, as the mockup shows the move tool's: a thin blue
/// edge with white handles at the corners and the middle of each side.
fn paint_bounds(painter: &egui::Painter, frame: Rect) {
    let line = (1.0, BOUNDS);
    painter.line_segment([frame.left_top(), frame.right_top()], line);
    painter.line_segment([frame.right_top(), frame.right_bottom()], line);
    painter.line_segment([frame.right_bottom(), frame.left_bottom()], line);
    painter.line_segment([frame.left_bottom(), frame.left_top()], line);
    for handle in [
        frame.left_top(),
        frame.center_top(),
        frame.right_top(),
        frame.right_center(),
        frame.right_bottom(),
        frame.center_bottom(),
        frame.left_bottom(),
        frame.left_center(),
    ] {
        widgets::outlined(painter, Rect::from_center_size(handle, vec2(8.0, 8.0)), 1.0, Color32::WHITE, BOUNDS);
    }
}

/// An outline drawn as a selection's marching edge: white under black
/// dashes that move along it.
fn marching_edge(ui: &egui::Ui, painter: &egui::Painter, points: &[Pos2], closed: bool) {
    if points.len() < 2 {
        return;
    }
    let mut path = points.to_vec();
    if closed {
        path.push(points[0]);
    }
    painter.line(path.clone(), (1.0, Color32::WHITE));
    let phase = (ui.input(|input| input.time) / MARCH.as_secs_f64()) as f32 % (2.0 * DASH);
    // How far along the dash pattern the walk is: dark for the first half.
    let mut along = 2.0 * DASH - phase;
    for pair in path.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let length = from.distance(to);
        if length <= 0.0 {
            continue;
        }
        let step = (to - from) / length;
        let mut walked = 0.0;
        while walked < length {
            let in_dash = along % (2.0 * DASH);
            let (dark, left) = if in_dash < DASH { (true, DASH - in_dash) } else { (false, 2.0 * DASH - in_dash) };
            let next = (walked + left).min(length);
            if dark {
                painter.line_segment([from + step * walked, from + step * next], (1.0, Color32::BLACK));
            }
            along += next - walked;
            walked = next;
        }
    }
}

/// A paint drag as it is drawn, in the tool's colour, size, and opacity,
/// or as a light trail for the eraser.
// Pictures read from raw RGBA files.
//
// Files are read, and their pixels made into images, on threads of their
// own. The window only uploads finished images, and shows what it last
// uploaded until the next is ready.

/// A picture as its texture shows it.
#[derive(Clone)]
struct Loaded {
    revision: u64,
    size: [u32; 2],
    texture: egui::TextureHandle,
    opaque: bool,
}

/// A picture's place among those read: what its texture shows, and the
/// reading under way for it, at most one at a time.
#[derive(Clone, Default)]
struct Entry {
    shown: Option<Loaded>,
    reading: Option<Arc<Reading>>,
    /// The revision whose changes could not be read: the picture is read
    /// whole instead.
    failed: Option<u64>,
    /// A file that was not whole is tried again after this.
    retry: Option<Instant>,
    /// The last pass that drew it, so pictures no longer drawn are let go.
    used: u64,
}

/// A reading on a thread of its own, and what it brought once done.
#[derive(Default)]
struct Reading {
    done: Mutex<Option<Read>>,
    ready: Condvar,
}

/// What a reading brought.
enum Read {
    /// The whole picture, as `revision`, at `size`.
    Whole { revision: u64, size: [u32; 2], image: egui::ColorImage, opaque: bool },
    /// The parts that take a texture holding revision `from` to `to`, each
    /// with where it goes and whether it is opaque.
    Parts { from: u64, to: u64, parts: Vec<([usize; 2], egui::ColorImage, bool)> },
    /// Nothing could be read: a file was missing or not whole yet.
    Nothing { from: Option<u64> },
}

/// What a reading reads.
enum Job {
    Whole { revision: u64 },
    /// From revision `from`, each step with the revision it reaches.
    Parts { from: u64, steps: Vec<(u64, Step)> },
}

/// One read of the changes a texture catches up with.
#[derive(Debug, PartialEq)]
enum Step {
    /// Rows written to a file of their own, while a change is being made.
    Rows { file: String, rect: [u32; 4] },
    /// Rows of the picture file.
    Picture { rect: [u32; 4] },
}

/// Pictures not drawn for this many passes are let go, and read again if
/// they are shown again.
const FORGET_AFTER: u64 = 600;
/// How long a picture with nothing of its size shown yet is waited for, so
/// a picture opened shows at once rather than after a frame of squares.
const FIRST_WAIT: Duration = Duration::from_millis(250);
/// How long a file that was not whole is left before it is tried again.
const RETRY: Duration = Duration::from_millis(100);

/// A file of raw RGBA, `width × height × 4` bytes with straight alpha, as a
/// texture, read again when `revision` changes. `patches` are the changes
/// the views brought, oldest first, each with its picture and revision: the
/// texture takes those that follow what it holds, reading only their rows,
/// and changes read from the picture file one after another are read as
/// one, or as the whole picture when they cover most of it. While the
/// view's own change is still being made (`making`), its rows in a file of
/// their own, the picture file is older than the view: a texture that
/// cannot follow keeps what it shows rather than read it whole. A picture
/// file of the wrong length is not whole yet: the last texture is kept and
/// the file is tried again later. A texture never goes back to an older
/// revision. Returns the texture and whether every pixel is opaque.
pub(crate) fn raw_picture(
    ctx: &egui::Context,
    path: &str,
    width: u32,
    height: u32,
    revision: u64,
    patches: &[(String, u64, RasterPatch)],
    making: bool,
) -> Option<(egui::TextureId, bool)> {
    let id = egui::Id::new("raster-pictures");
    let pass = ctx.cumulative_pass_nr();
    let size = [width, height];
    let mut entry: Entry = ctx
        .data_mut(|data| data.get_temp_mut_or_default::<HashMap<String, Entry>>(id).get(path).cloned())
        .unwrap_or_default();
    take(ctx, path, &mut entry, size, revision);
    let waits = entry.retry.is_some_and(|retry| Instant::now() < retry);
    if entry.reading.is_none()
        && !waits
        && let Some(job) = plan(&entry, path, size, revision, patches, making, ctx.input(|input| input.max_texture_side))
    {
        entry.retry = None;
        entry.reading = Some(start(ctx, path, size, ctx.input(|input| input.max_texture_side), job));
    }
    let current = |entry: &Entry| entry.shown.as_ref().filter(|shown| shown.size == size).map(|shown| (shown.texture.id(), shown.opaque));
    if current(&entry).is_none()
        && let Some(reading) = &entry.reading
    {
        let done = reading.done.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        drop(reading.ready.wait_timeout_while(done, FIRST_WAIT, |done| done.is_none()));
        take(ctx, path, &mut entry, size, revision);
    }
    let answer = current(&entry);
    ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<HashMap<String, Entry>>(id);
        cache.insert(path.to_string(), Entry { used: pass, ..entry });
        cache.retain(|_, picture| pass.saturating_sub(picture.used) < FORGET_AFTER);
    });
    answer
}

/// Uploads what a finished reading brought, unless the texture has moved
/// past it.
fn take(ctx: &egui::Context, path: &str, entry: &mut Entry, size: [u32; 2], revision: u64) {
    let Some(reading) = &entry.reading else {
        return;
    };
    let Some(read) = reading.done.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() else {
        return;
    };
    entry.reading = None;
    match read {
        Read::Whole { revision: read, size: read_size, image, opaque } => {
            if read_size != size {
                return;
            }
            match &mut entry.shown {
                // A texture newer than the reading keeps what it shows,
                // unless the view has gone back past it.
                Some(shown) if shown.size == size && read <= shown.revision && shown.revision <= revision => {}
                Some(shown) if shown.size == size => {
                    shown.texture.set(image, picture_options());
                    shown.revision = read;
                    shown.opaque = opaque;
                }
                _ => {
                    entry.shown = Some(Loaded {
                        revision: read,
                        size,
                        texture: ctx.load_texture(format!("raster:{path}"), image, picture_options()),
                        opaque,
                    });
                }
            }
        }
        Read::Parts { from, to, parts } => {
            if let Some(shown) = entry.shown.as_mut().filter(|shown| shown.size == size && shown.revision == from) {
                for (corner, image, opaque) in parts {
                    shown.texture.set_partial(corner, image, picture_options());
                    shown.opaque &= opaque;
                }
                shown.revision = to;
            }
        }
        Read::Nothing { from } => {
            match from {
                Some(from) => entry.failed = Some(from),
                None => entry.retry = Some(Instant::now() + RETRY),
            }
            ctx.request_repaint_after(RETRY);
        }
    }
}

/// What to read to bring the texture to `revision`, if anything.
fn plan(
    entry: &Entry,
    path: &str,
    size: [u32; 2],
    revision: u64,
    patches: &[(String, u64, RasterPatch)],
    making: bool,
    largest: usize,
) -> Option<Job> {
    // The picture file read while a change is still being made is older
    // than the view: it is kept as no revision, so the view without a file
    // reads it again.
    let Some(shown) = entry.shown.as_ref().filter(|shown| shown.size == size) else {
        return Some(Job::Whole { revision: if making { 0 } else { revision } });
    };
    if shown.revision == revision {
        return None;
    }
    // A texture shrunk to fit the graphics card is only read whole.
    if size[0].max(size[1]) as usize <= largest && entry.failed != Some(shown.revision) {
        let (steps, reached) = steps(path, size, shown.revision, patches);
        if !steps.is_empty() && (reached == revision || making) {
            return Some(Job::Parts { from: shown.revision, steps });
        }
    }
    (!making).then_some(Job::Whole { revision })
}

/// The reads that take a texture holding revision `from` along the changes
/// that follow it, and the revision they reach. Changes read from the
/// picture file one after another are read as one, over the rows they
/// cover, as the file holds the latest of them anyway. One that covers
/// most of the picture reads it whole, and nothing before it is read.
fn steps(path: &str, size: [u32; 2], from: u64, patches: &[(String, u64, RasterPatch)]) -> (Vec<(u64, Step)>, u64) {
    let [width, height] = size;
    let whole = [0, 0, width, height];
    let mut at = from;
    let mut steps: Vec<(u64, Step)> = Vec::new();
    for (picture, revision, patch) in patches {
        if picture != path || *revision <= at {
            continue;
        }
        if patch.since != at {
            break;
        }
        match &patch.file {
            Some(file) => steps.push((*revision, Step::Rows { file: file.clone(), rect: patch.rect })),
            None => {
                let [x, y, w, h] = patch.rect;
                let (x, y) = (x.min(width), y.min(height));
                let rect = [x, y, w.min(width - x), h.min(height - y)];
                match steps.last_mut() {
                    Some((reached, Step::Picture { rect: last })) => {
                        *last = union(*last, rect);
                        *reached = *revision;
                    }
                    _ => steps.push((*revision, Step::Picture { rect })),
                }
            }
        }
        at = *revision;
    }
    for (_, step) in &mut steps {
        if let Step::Picture { rect } = step
            && 2 * u64::from(rect[2]) * u64::from(rect[3]) >= u64::from(width) * u64::from(height)
        {
            *rect = whole;
        }
    }
    if let Some(last) = steps.iter().rposition(|(_, step)| *step == Step::Picture { rect: whole }) {
        steps.drain(..last);
    }
    (steps, at)
}

/// The smallest rect holding both, as `[x, y, width, height]`.
fn union(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    if a[2] == 0 || a[3] == 0 {
        return b;
    }
    if b[2] == 0 || b[3] == 0 {
        return a;
    }
    let (left, top) = (a[0].min(b[0]), a[1].min(b[1]));
    let right = (a[0] + a[2]).max(b[0] + b[2]);
    let bottom = (a[1] + a[3]).max(b[1] + b[3]);
    [left, top, right - left, bottom - top]
}

/// Starts a reading on a thread of its own, which asks for a frame when done.
fn start(ctx: &egui::Context, path: &str, size: [u32; 2], largest: usize, job: Job) -> Arc<Reading> {
    let reading = Arc::new(Reading::default());
    let (ctx, path, done) = (ctx.clone(), path.to_string(), reading.clone());
    let work = move || {
        let read = run(&path, size, largest, job);
        *done.done.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(read);
        done.ready.notify_all();
        ctx.request_repaint();
    };
    // Should no thread start, the reading is done here instead.
    let work = Arc::new(Mutex::new(Some(work)));
    let spare = work.clone();
    let started = std::thread::Builder::new().name(String::from("picture-reader")).spawn(move || {
        if let Some(work) = spare.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            work();
        }
    });
    if started.is_err()
        && let Some(work) = work.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
    {
        work();
    }
    reading
}

/// Reads what a job asks for. A step that cannot be read ends the parts
/// there, at the last revision read.
fn run(path: &str, size: [u32; 2], largest: usize, job: Job) -> Read {
    let [width, height] = size;
    match job {
        Job::Whole { revision } => match read_raw(path, width, height, largest) {
            Some((image, opaque)) => Read::Whole { revision, size, image, opaque },
            None => Read::Nothing { from: None },
        },
        Job::Parts { from, steps } => {
            let mut to = from;
            let mut parts = Vec::new();
            for (reached, step) in steps {
                let rows = match step {
                    Step::Rows { file, rect } => read_rows(&file, width, height, rect),
                    Step::Picture { rect } => read_patch(path, width, height, rect),
                };
                let Some(rows) = rows else {
                    break;
                };
                parts.push(rows);
                to = reached;
            }
            if parts.is_empty() {
                Read::Nothing { from: Some(from) }
            } else {
                Read::Parts { from, to, parts }
            }
        }
    }
}

/// Reads a file holding only the rows of `rect`, `width × height × 4` bytes
/// for the rect, with where they go in the texture.
fn read_rows(file: &str, width: u32, height: u32, rect: [u32; 4]) -> Option<([usize; 2], egui::ColorImage, bool)> {
    let [x, y, w, h] = rect.map(|value| value as usize);
    if w == 0 || h == 0 || x + w > width as usize || y + h > height as usize {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    if bytes.len() != w * h * 4 {
        return None;
    }
    let opaque = bytes.chunks_exact(4).all(|pixel| pixel[3] == 255);
    Some(([x, y], egui::ColorImage::from_rgba_unmultiplied([w, h], &bytes), opaque))
}

/// Pixels stay square when zoomed in, and blend smoothly when zoomed out.
fn picture_options() -> egui::TextureOptions {
    egui::TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    }
}

/// Reads the rows of `rect` from a whole raw RGBA file, with where they go
/// in the texture and whether they are opaque. Rows as wide as most of the
/// picture are read in one piece. None when the file is not whole.
fn read_patch(path: &str, width: u32, height: u32, rect: [u32; 4]) -> Option<([usize; 2], egui::ColorImage, bool)> {
    use std::io::{Read, Seek, SeekFrom};
    let (width, height) = (width as usize, height as usize);
    let x = (rect[0] as usize).min(width);
    let y = (rect[1] as usize).min(height);
    let w = (rect[2] as usize).min(width - x);
    let h = (rect[3] as usize).min(height - y);
    if w == 0 || h == 0 {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() != (width * height * 4) as u64 {
        return None;
    }
    let mut bytes = vec![0u8; w * h * 4];
    if 2 * w >= width {
        let first = (y * width + x) * 4;
        let mut block = vec![0u8; ((h - 1) * width + w) * 4];
        file.seek(SeekFrom::Start(first as u64)).ok()?;
        file.read_exact(&mut block).ok()?;
        for (row, line) in bytes.chunks_exact_mut(w * 4).enumerate() {
            line.copy_from_slice(&block[row * width * 4..(row * width + w) * 4]);
        }
    } else {
        for (row, line) in bytes.chunks_exact_mut(w * 4).enumerate() {
            file.seek(SeekFrom::Start((((y + row) * width + x) * 4) as u64)).ok()?;
            file.read_exact(line).ok()?;
        }
    }
    let opaque = bytes.chunks_exact(4).all(|pixel| pixel[3] == 255);
    Some(([x, y], egui::ColorImage::from_rgba_unmultiplied([w, h], &bytes), opaque))
}

/// Reads a whole raw RGBA file, shrunk by whole steps when it is larger than
/// `largest`, the graphics card's largest texture.
fn read_raw(path: &str, width: u32, height: u32, largest: usize) -> Option<(egui::ColorImage, bool)> {
    let (width, height) = (width as usize, height as usize);
    let expected = width.checked_mul(height)?.checked_mul(4)?;
    if expected == 0 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() != expected {
        return None;
    }
    let opaque = bytes.chunks_exact(4).all(|pixel| pixel[3] == 255);
    let step = width.max(height).div_ceil(largest.max(1));
    if step <= 1 {
        return Some((egui::ColorImage::from_rgba_unmultiplied([width, height], &bytes), opaque));
    }
    let (small_w, small_h) = (width.div_ceil(step), height.div_ceil(step));
    let mut small = Vec::with_capacity(small_w * small_h * 4);
    for y in 0..small_h {
        for x in 0..small_w {
            let at = ((y * step).min(height - 1) * width + (x * step).min(width - 1)) * 4;
            small.extend_from_slice(&bytes[at..at + 4]);
        }
    }
    Some((egui::ColorImage::from_rgba_unmultiplied([small_w, small_h], &small), opaque))
}

/// Squares under a picture's transparency, fixed to the picture's corner.
pub(crate) fn checker(ctx: &egui::Context, painter: &egui::Painter, area: Rect) {
    let id = egui::Id::new("raster-checker");
    let texture = match ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        Some(texture) => texture,
        None => {
            let image = egui::ColorImage::new([2, 2], vec![CHECK_LIGHT, CHECK_DARK, CHECK_DARK, CHECK_LIGHT]);
            let options = egui::TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Nearest,
                wrap_mode: egui::TextureWrapMode::Repeat,
                mipmap_mode: None,
            };
            let texture = ctx.load_texture("raster-checker", image, options);
            ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        }
    };
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(area.width() / (2.0 * CHECK), area.height() / (2.0 * CHECK)));
    painter.image(texture.id(), area, uv, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the picture until no reading is under way for it, as frames
    /// do while the reader works.
    fn caught_up(ctx: &egui::Context, path: &str, revision: u64, patches: &[(String, u64, RasterPatch)], making: bool) -> bool {
        for _ in 0..400 {
            let shown = raw_picture(ctx, path, 8, 6, revision, patches, making).is_some();
            let reading = ctx.data_mut(|data| {
                data.get_temp_mut_or_default::<HashMap<String, Entry>>(egui::Id::new("raster-pictures")).get(path).is_some_and(|entry| entry.reading.is_some())
            });
            if !reading {
                return shown;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the picture was never read");
    }

    /// Catches a change lost when two views come between frames, a picture
    /// read whole for every change, and an older picture file shown as a
    /// newer revision: two changes queued at once both reach the texture,
    /// in order, as partial updates; one that does not follow what the
    /// texture holds, while its change is still being made, is waited for.
    #[test]
    fn queued_changes_reach_the_texture_in_order() {
        let dir = std::env::temp_dir().join(format!("inpainter-raster-patches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let picture = dir.join("1-1.rgba").display().to_string();
        std::fs::write(&picture, [255u8; 8 * 6 * 4]).unwrap();
        let ctx = egui::Context::default();
        let deltas = |ctx: &egui::Context| -> Vec<egui::epaint::ImageDelta> {
            let mut delta = ctx.tex_manager().write().take_delta();
            let set = delta.set.iter().flat_map(|(_, deltas)| deltas.iter().cloned()).collect();
            delta.clear();
            set
        };
        assert!(raw_picture(&ctx, &picture, 8, 6, 1, &[], false).is_some(), "a picture opened was not waited for");
        assert!(deltas(&ctx).iter().any(|delta| delta.pos.is_none()), "the picture was not read");

        let mut patches = Vec::new();
        for (revision, corner, colour) in [(2u64, [0u32, 0u32], [255u8, 0, 0, 255]), (3, [4, 4], [0, 0, 255, 255])] {
            let file = dir.join(format!("1-1-{revision}.patch"));
            std::fs::write(&file, colour.repeat(4)).unwrap();
            let patch = RasterPatch { since: revision - 1, rect: [corner[0], corner[1], 2, 2], file: Some(file.display().to_string()) };
            patches.push((picture.clone(), revision, patch));
        }
        assert!(caught_up(&ctx, &picture, 3, &patches, true));
        let set = deltas(&ctx);
        let corners: Vec<Option<[usize; 2]>> = set.iter().map(|delta| delta.pos).collect();
        assert_eq!(corners, vec![Some([0, 0]), Some([4, 4])], "the changes did not reach the texture in order, as parts");
        let egui::ImageData::Color(first) = &set[0].image;
        assert_eq!(first.pixels[0], Color32::from_rgb(255, 0, 0));

        // The change made: the same revision, nothing more to read.
        assert!(caught_up(&ctx, &picture, 3, &patches, false));
        assert!(deltas(&ctx).is_empty(), "a picture already shown was read again");

        // A change that does not follow what the texture holds, while it is
        // still being made, is waited for, not read from the older file.
        let file = dir.join("1-1-9.patch");
        std::fs::write(&file, [0u8; 16]).unwrap();
        let stray = vec![(picture.clone(), 9, RasterPatch { since: 8, rect: [0, 0, 2, 2], file: Some(file.display().to_string()) })];
        assert!(caught_up(&ctx, &picture, 9, &stray, true), "the picture shown was dropped");
        assert!(deltas(&ctx).is_empty(), "an older picture file was read for a change still being made");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Catches changes read from the picture file one by one: a run of them
    /// is one read over the rows they cover, one that covers most of the
    /// picture reads it whole with nothing before it, and rows written
    /// while a change is being made keep their place in the order.
    #[test]
    fn changes_from_the_picture_file_are_read_as_one() {
        let picture = |since: u64, rect: [u32; 4]| (String::from("p"), since + 1, RasterPatch { since, rect, file: None });
        let rows = |since: u64, rect: [u32; 4]| (String::from("p"), since + 1, RasterPatch { since, rect, file: Some(format!("r{since}")) });
        let full: Vec<_> = (1..=16).map(|since| picture(since, [0, 0, 100, 50])).collect();
        assert_eq!(steps("p", [100, 50], 1, &full), (vec![(17, Step::Picture { rect: [0, 0, 100, 50] })], 17));

        let small = [picture(1, [0, 0, 10, 10]), picture(2, [20, 5, 10, 10]), rows(3, [0, 0, 4, 4]), picture(4, [90, 40, 20, 20])];
        let expected = vec![
            (3, Step::Picture { rect: [0, 0, 30, 15] }),
            (4, Step::Rows { file: String::from("r3"), rect: [0, 0, 4, 4] }),
            (5, Step::Picture { rect: [90, 40, 10, 10] }),
        ];
        assert_eq!(steps("p", [100, 50], 1, &small), (expected, 5));

        let most = [rows(1, [0, 0, 4, 4]), picture(2, [0, 0, 100, 30]), rows(3, [0, 0, 4, 4])];
        let expected = vec![(3, Step::Picture { rect: [0, 0, 100, 50] }), (4, Step::Rows { file: String::from("r3"), rect: [0, 0, 4, 4] })];
        assert_eq!(steps("p", [100, 50], 1, &most), (expected, 4));
    }

    /// A crop drag keeps the box on the picture, whichever way it goes, and
    /// an edge crossed over the other swaps them instead of turning negative.
    #[test]
    fn crop_drags_stay_on_the_picture() {
        let size = [100.0, 50.0];
        assert_eq!(dragged_crop(Grab::New, [0.0; 4], [80.0, 40.0], [-20.0, 10.0], size), [0.0, 10.0, 80.0, 30.0]);
        assert_eq!(dragged_crop(Grab::Inside, [10.0, 10.0, 20.0, 20.0], [15.0, 15.0], [200.0, 15.0], size), [80.0, 10.0, 20.0, 20.0]);
        let left = Grab::Edges([true, false, false, false]);
        assert_eq!(dragged_crop(left, [10.0, 10.0, 20.0, 20.0], [10.0, 20.0], [40.0, 20.0], size), [30.0, 10.0, 10.0, 20.0]);
        assert_eq!(whole_pixels([0.4, 0.6, 10.2, 200.0], [100, 50]), [0.0, 1.0, 11.0, 49.0]);
    }
}
