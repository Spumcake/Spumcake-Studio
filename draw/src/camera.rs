//! Camera arithmetic. It acts on a [`Camera`] and, for fit, a document.
//! It does not keep a session or a dirty flag.

use crate::document::{Camera, Point, Drawing, FIT_MARGIN, MAX_ZOOM, MIN_ZOOM};
use eframe::egui::{Pos2, Rect};

const ZOOM_IN_FACTOR: f64 = 5.0 / 4.0;
const ZOOM_OUT_FACTOR: f64 = 4.0 / 5.0;

pub fn pan_screen(camera: &mut Camera, delta: Point) {
    if !delta[0].is_finite() || !delta[1].is_finite() || camera.zoom == 0.0 {
        return;
    }
    camera.origin[0] -= delta[0] / camera.zoom;
    camera.origin[1] -= delta[1] / camera.zoom;
}

pub fn zoom_toward(camera: &mut Camera, factor: f64, pointer: Point) {
    if !factor.is_finite()
        || factor <= 0.0
        || !pointer[0].is_finite()
        || !pointer[1].is_finite()
        || camera.zoom <= 0.0
    {
        return;
    }
    let old = camera.zoom;
    let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let doc_x = camera.origin[0] + pointer[0] / old;
    let doc_y = camera.origin[1] + pointer[1] / old;
    camera.zoom = new;
    camera.origin = [doc_x - pointer[0] / new, doc_y - pointer[1] / new];
}

pub fn zoom_at_center(camera: &mut Camera, zoom_in: bool, viewport: Point) {
    let factor = if zoom_in {
        ZOOM_IN_FACTOR
    } else {
        ZOOM_OUT_FACTOR
    };
    zoom_toward(camera, factor, [viewport[0] / 2.0, viewport[1] / 2.0]);
}

pub fn fit(camera: &mut Camera, drawing: &Drawing, viewport: Point) {
    frame_bounds(camera, drawing.visible_bounds(), viewport);
}

/// Frames `bounds` the way Fit frames every visible object. No bounds
/// returns to the default view centered on the origin.
pub fn frame_bounds(camera: &mut Camera, bounds: Option<(Point, Point)>, viewport: Point) {
    if viewport[0] <= 0.0
        || viewport[1] <= 0.0
        || !viewport[0].is_finite()
        || !viewport[1].is_finite()
    {
        return;
    }
    let Some((min, max)) = bounds else {
        *camera = Camera::centered(viewport);
        return;
    };
    let width = (max[0] - min[0]) + 2.0 * FIT_MARGIN;
    let height = (max[1] - min[1]) + 2.0 * FIT_MARGIN;
    let mut zoom = if width > 0.0 && height > 0.0 {
        (viewport[0] / width).min(viewport[1] / height)
    } else {
        MAX_ZOOM
    };
    if !zoom.is_finite() {
        zoom = 1.0;
    }
    zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    camera.zoom = zoom;
    camera.origin = [
        center[0] - viewport[0] / (2.0 * zoom),
        center[1] - viewport[1] / (2.0 * zoom),
    ];
}

pub fn screen_to_document(rect: Rect, origin: [f64; 2], zoom: f64, pos: Pos2) -> [f64; 2] {
    [
        origin[0] + f64::from(pos.x - rect.min.x) / zoom,
        origin[1] + f64::from(pos.y - rect.min.y) / zoom,
    ]
}

pub fn document_to_screen(rect: Rect, origin: [f64; 2], zoom: f64, point: [f64; 2]) -> Pos2 {
    Pos2::new(
        ((point[0] - origin[0]) * zoom) as f32 + rect.min.x,
        ((point[1] - origin[1]) * zoom) as f32 + rect.min.y,
    )
}
