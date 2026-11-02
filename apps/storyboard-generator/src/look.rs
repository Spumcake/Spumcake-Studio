//! An image of the storyboard with no window, for the assistant to look at.
//!
//! Ink, shapes, and the grid are drawn from the document's own paint lists
//! with `tiny-skia`. Text is laid out and painted by the same `draw::text`
//! calls the host's canvas makes, through an egui context with no window
//! and the host's Inter faces; its triangles and font atlas are then filled
//! in here, pixel by pixel. What it leaves out is said in `NOTES`.

use draw::document::{Point, SceneMark};
use egui::epaint::{ClippedPrimitive, ColorImage, Primitive, TextureId};
use egui::{FontData, FontDefinitions, FontFamily, Pos2, Rect};
use std::sync::Arc;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// The host's drawing surface and grid colors, and the grid's spacing.
const BACKGROUND: [u8; 3] = [251, 251, 252];
const GRID: [u8; 3] = [234, 235, 238];
const GRID_SPACING: f64 = 24.0;

/// What the image does not show as the screen does.
pub const NOTES: &str = "Drawn as the canvas shows it, with the grid every 24 units, but without the selection, handles, or the text being typed. Ink is anti-aliased, where the screen's is not. Text uses the host's Inter faces and layout; Monospace text uses egui's own monospace face, as the screen does.";

const INTER: &[u8] = include_bytes!("../fonts/Inter-Variable.ttf");
const INTER_ITALIC: &[u8] = include_bytes!("../fonts/Inter-Italic-Variable.ttf");

pub struct Image {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Pixels per document unit.
    pub scale: f64,
}

/// Lays out text as the host does. The font atlas egui builds is kept here,
/// since egui sends only its changes after the first image.
pub struct Looker {
    ctx: Option<egui::Context>,
    atlas: Option<ColorImage>,
    cache: draw::text::Cache,
}

impl Default for Looker {
    fn default() -> Self {
        Self::new()
    }
}

impl Looker {
    pub fn new() -> Self {
        Self {
            ctx: None,
            atlas: None,
            cache: draw::text::Cache::new(),
        }
    }

    fn context(&mut self) -> egui::Context {
        self.ctx
            .get_or_insert_with(|| {
                let ctx = egui::Context::default();
                ctx.set_fonts(fonts());
                ctx
            })
            .clone()
    }

    /// The marks inside `min`..`max`, in document units, scaled so the
    /// longer side is at most `max_side` pixels and a unit at most 4.
    pub fn image(&mut self, marks: Vec<SceneMark>, min: Point, max: Point, max_side: u32) -> Option<Image> {
        let span = [(max[0] - min[0]).max(1.0), (max[1] - min[1]).max(1.0)];
        let scale = (f64::from(max_side) / span[0].max(span[1])).min(4.0);
        let width = (span[0] * scale).ceil().max(1.0) as u32;
        let height = (span[1] * scale).ceil().max(1.0) as u32;
        let mut pixmap = Pixmap::new(width, height)?;
        pixmap.fill(tiny_skia::Color::from_rgba8(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255));
        paint_grid(&mut pixmap, min, max, scale);
        for mark in marks {
            match mark {
                SceneMark::Ink(paint) => {
                    if paint.eraser_preview || paint.triangles.is_empty() {
                        continue;
                    }
                    let mut path = PathBuilder::new();
                    for triangle in &paint.triangles {
                        let [a, b, c] = triangle.map(|point| to_pixel(point, min, scale));
                        path.move_to(a[0], a[1]);
                        path.line_to(b[0], b[1]);
                        path.line_to(c[0], c[1]);
                        path.close();
                    }
                    let Some(path) = path.finish() else {
                        continue;
                    };
                    let mut brush = Paint::default();
                    let alpha = (paint.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                    brush.set_color_rgba8(paint.rgb[0], paint.rgb[1], paint.rgb[2], alpha);
                    brush.anti_alias = true;
                    pixmap.fill_path(&path, &brush, FillRule::Winding, Transform::identity(), None);
                }
                SceneMark::Text(view) => self.paint_text(&mut pixmap, &view, min, scale),
            }
        }
        let png = pixmap.encode_png().ok()?;
        Some(Image {
            png,
            width,
            height,
            scale,
        })
    }

    /// One text, through the host's layout and paint, then filled in.
    fn paint_text(&mut self, pixmap: &mut Pixmap, view: &draw::document::TextView, min: Point, scale: f64) {
        let ctx = self.context();
        let rect = Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(pixmap.width() as f32, pixmap.height() as f32),
        );
        let input = egui::RawInput {
            screen_rect: Some(rect),
            max_texture_side: Some(4096),
            ..Default::default()
        };
        ctx.begin_pass(input);
        let laid = draw::text::lay(
            &ctx,
            &view.content,
            &view.style,
            &view.look,
            view.origin,
            view.box_width,
            view.box_height,
            view.world,
            scale,
            true,
            &mut self.cache,
        );
        let painter = ctx.layer_painter(egui::LayerId::background());
        draw::text::paint(&painter, rect, min, scale, view, &laid);
        let mut output = ctx.end_pass();
        for (id, deltas) in &output.textures_delta.set {
            if *id != TextureId::default() {
                continue;
            }
            for delta in deltas {
                let egui::epaint::ImageData::Color(image) = &delta.image;
                apply_delta(&mut self.atlas, image, delta.pos);
            }
        }
        // Applied above; the font atlas is the only texture text uses.
        output.textures_delta.clear();
        let shapes = std::mem::take(&mut output.shapes);
        let primitives = ctx.tessellate(shapes, output.pixels_per_point);
        if let Some(atlas) = &self.atlas {
            for primitive in &primitives {
                fill_primitive(pixmap, primitive, atlas);
            }
        }
    }
}

/// The host's faces, as its toolbar installs them, for text alone.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let faces = [
        (draw::text::INTER, INTER, 400.0),
        (draw::text::INTER_BOLD, INTER, 700.0),
        (draw::text::INTER_ITALIC, INTER_ITALIC, 400.0),
        (draw::text::INTER_BOLD_ITALIC, INTER_ITALIC, 700.0),
    ];
    for (name, bytes, weight) in faces {
        let mut data = FontData::from_static(bytes);
        data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
        fonts.font_data.insert(name.into(), Arc::new(data));
        fonts
            .families
            .insert(FontFamily::Name(name.into()), vec![name.into()]);
    }
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, draw::text::INTER.into());
    fonts
}

fn apply_delta(atlas: &mut Option<ColorImage>, image: &ColorImage, pos: Option<[usize; 2]>) {
    let Some(pos) = pos else {
        *atlas = Some(image.clone());
        return;
    };
    let Some(atlas) = atlas.as_mut() else {
        return;
    };
    for y in 0..image.size[1] {
        for x in 0..image.size[0] {
            let (ax, ay) = (pos[0] + x, pos[1] + y);
            if ax < atlas.size[0] && ay < atlas.size[1] {
                atlas.pixels[ay * atlas.size[0] + ax] = image.pixels[y * image.size[0] + x];
            }
        }
    }
}

fn to_pixel(point: Point, min: Point, scale: f64) -> [f32; 2] {
    [
        ((point[0] - min[0]) * scale) as f32,
        ((point[1] - min[1]) * scale) as f32,
    ]
}

fn paint_grid(pixmap: &mut Pixmap, min: Point, max: Point, scale: f64) {
    // Lines closer than 4 pixels would grey the image out.
    if GRID_SPACING * scale < 4.0 {
        return;
    }
    let mut paint = Paint::default();
    paint.set_color_rgba8(GRID[0], GRID[1], GRID[2], 255);
    paint.anti_alias = false;
    let (width, height) = (pixmap.width() as f32, pixmap.height() as f32);
    let mut x = (min[0] / GRID_SPACING).floor() * GRID_SPACING;
    while x <= max[0] {
        let px = ((x - min[0]) * scale) as f32;
        if let Some(rect) = tiny_skia::Rect::from_xywh(px.round(), 0.0, 1.0, height) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
        x += GRID_SPACING;
    }
    let mut y = (min[1] / GRID_SPACING).floor() * GRID_SPACING;
    while y <= max[1] {
        let py = ((y - min[1]) * scale) as f32;
        if let Some(rect) = tiny_skia::Rect::from_xywh(0.0, py.round(), width, 1.0) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
        y += GRID_SPACING;
    }
}

/// Fills egui's triangles as its renderer would: each vertex's color, times
/// the font atlas where the triangle samples it, blended over the image.
fn fill_primitive(pixmap: &mut Pixmap, primitive: &ClippedPrimitive, atlas: &ColorImage) {
    let Primitive::Mesh(mesh) = &primitive.primitive else {
        return;
    };
    let (width, height) = (pixmap.width() as i64, pixmap.height() as i64);
    let clip = primitive.clip_rect;
    let data = pixmap.data_mut();
    for triangle in mesh.indices.chunks_exact(3) {
        let v = [
            &mesh.vertices[triangle[0] as usize],
            &mesh.vertices[triangle[1] as usize],
            &mesh.vertices[triangle[2] as usize],
        ];
        let p = v.map(|vertex| [f64::from(vertex.pos.x), f64::from(vertex.pos.y)]);
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        let lo_x = p.iter().map(|q| q[0]).fold(f64::MAX, f64::min).max(f64::from(clip.min.x)).floor().max(0.0) as i64;
        let hi_x = p.iter().map(|q| q[0]).fold(f64::MIN, f64::max).min(f64::from(clip.max.x)).ceil().min(width as f64) as i64;
        let lo_y = p.iter().map(|q| q[1]).fold(f64::MAX, f64::min).max(f64::from(clip.min.y)).floor().max(0.0) as i64;
        let hi_y = p.iter().map(|q| q[1]).fold(f64::MIN, f64::max).min(f64::from(clip.max.y)).ceil().min(height as f64) as i64;
        for y in lo_y..hi_y {
            for x in lo_x..hi_x {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((p[1][0] - cx) * (p[2][1] - cy) - (p[2][0] - cx) * (p[1][1] - cy)) / area;
                let w1 = ((p[2][0] - cx) * (p[0][1] - cy) - (p[0][0] - cx) * (p[2][1] - cy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let weights = [w0, w1, w2];
                let mut color = [0.0f64; 4];
                let mut uv = [0.0f64; 2];
                for (vertex, weight) in v.iter().zip(weights) {
                    let rgba = vertex.color.to_array();
                    for (slot, channel) in color.iter_mut().zip(rgba) {
                        *slot += f64::from(channel) * weight;
                    }
                    uv[0] += f64::from(vertex.uv.x) * weight;
                    uv[1] += f64::from(vertex.uv.y) * weight;
                }
                let texel = sample(atlas, uv);
                let src: [f64; 4] = std::array::from_fn(|i| color[i] * texel[i] / 255.0);
                let alpha = src[3] / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let at = ((y * width + x) * 4) as usize;
                for i in 0..4 {
                    let dst = f64::from(data[at + i]);
                    data[at + i] = (src[i] + dst * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// The atlas at a normalized coordinate, bilinearly, premultiplied.
fn sample(atlas: &ColorImage, uv: [f64; 2]) -> [f64; 4] {
    let [w, h] = atlas.size;
    if w == 0 || h == 0 {
        return [255.0; 4];
    }
    let x = (uv[0] * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
    let y = (uv[1] * h as f64 - 0.5).clamp(0.0, (h - 1) as f64);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let at = |px: usize, py: usize| atlas.pixels[py * w + px].to_array();
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    std::array::from_fn(|i| {
        let top = f64::from(a[i]) * (1.0 - fx) + f64::from(b[i]) * fx;
        let bottom = f64::from(c[i]) * (1.0 - fx) + f64::from(d[i]) * fx;
        top * (1.0 - fy) + bottom * fy
    })
}
