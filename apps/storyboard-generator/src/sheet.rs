//! The sheet a clip is drawn on, and what the stage shows around the drawing
//! being worked on. The paper, as the approved sheet views show it (peg
//! holes, the header's TITLE, SCENE, SHOT, and TIME, the scan field, the
//! field, the safe field, the centre cross, and the NOTES and ANIMATOR
//! boxes), is an ordinary drawing of shapes and text this program sends
//! under everything; the host draws it as any drawing and knows no paper.

use crate::film::{Clip, FPS, Film, Layer};
use crate::record::Record;
use protocol::{ActiveDrawing, Stage, Underlay};
use serde_json::{Value, json};

const INK: [u8; 3] = [20, 20, 20];
const PRINTED: [u8; 3] = [107, 114, 128];
const FAINT: [u8; 3] = [190, 194, 200];
const PAPER: [u8; 3] = [255, 255, 255];
/// Onion skin's earlier drawings are red and its later ones green, as the
/// approved sheet-onion-skin view shows them.
pub const EARLIER: [u8; 3] = [220, 38, 38];
pub const LATER: [u8; 3] = [22, 163, 74];

/// The field, `[x, y, width, height]` in the drawing's units: the film's
/// frame size, from the origin.
pub fn field(film: &Film) -> [f64; 4] {
    [0.0, 0.0, f64::from(film.size[0]), f64::from(film.size[1])]
}

/// A clip's field in its drawings' units: the film's field at the clip's origin.
pub fn field_in(film: &Film, clip: &Clip) -> [f64; 4] {
    let [_, _, width, height] = field(film);
    [clip.origin[0], clip.origin[1], width, height]
}

/// A clip's paper in its drawings' units.
pub fn paper_bounds_in(film: &Film, clip: &Clip) -> ([f64; 2], [f64; 2]) {
    let (min, max) = paper_bounds(film);
    let [x, y] = clip.origin;
    ([min[0] + x, min[1] + y], [max[0] + x, max[1] + y])
}

/// The paper's corners, around the scan field, the header, and the boxes
/// under it.
pub fn paper_bounds(film: &Film) -> ([f64; 2], [f64; 2]) {
    let [_, _, width, height] = field(film);
    let unit = height / 1080.0;
    (
        [-width * 0.05 - 80.0 * unit, -height * 0.05 - 300.0 * unit],
        [width * 1.05 + 80.0 * unit, height * 1.05 + 160.0 * unit],
    )
}

/// The time a clip runs, as the header writes it: seconds + frames.
pub fn time_text(frames: u32) -> String {
    format!("{} + {}", frames / FPS, frames % FPS)
}

/// The circled numbers of key marks, ① to ⑳; beyond that, the number in
/// parentheses.
pub fn circled(number: usize) -> String {
    match number {
        1..=20 => char::from_u32(0x2460 + number as u32 - 1).map(String::from).unwrap_or_default(),
        _ => format!("({number})"),
    }
}

/// Builds the paper's drawing, as the protocol's drawing JSON.
struct Paper {
    nodes: Vec<Value>,
    next: u64,
    unit: f64,
}

impl Paper {
    /// A printed line's width, so the sheet reads at the size it is shown.
    fn width(&self) -> u32 {
        (5.0 * self.unit).round().max(1.0) as u32
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn color(&mut self, name: &str, role: &str, rgb: [u8; 3], size: u32, visible: bool, dashed: bool) -> Value {
        json!({
            "id": self.id(),
            "name": name,
            "role": role,
            "rgb": rgb,
            "opacity": 1.0,
            "size": size,
            "cap": "round",
            "dash": if dashed { "dashed" } else { "solid" },
            "visible": visible,
        })
    }

    fn rect(&mut self, min: [f64; 2], max: [f64; 2], outline: Option<[u8; 3]>, fill: Option<[u8; 3]>, dashed: bool) {
        let outline_color = self.color("Outline", "outline", outline.unwrap_or(INK), self.width(), outline.is_some(), dashed);
        let fill_color = self.color("Fill", "fill", fill.unwrap_or(PAPER), 1, fill.is_some(), false);
        let id = self.id();
        self.nodes.push(json!({
            "id": id,
            "kind": "rectangle",
            "name": "Paper",
            "visible": true,
            "locked": true,
            "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "min": min,
            "max": max,
            "corner_radius": 0.0,
            "palette": [outline_color, fill_color],
        }));
    }

    fn rounded(&mut self, min: [f64; 2], max: [f64; 2], rgb: [u8; 3]) {
        let outline = self.color("Outline", "outline", rgb, 1, false, false);
        let fill = self.color("Fill", "fill", rgb, 1, true, false);
        let id = self.id();
        let radius = (max[1] - min[1]) / 2.0;
        self.nodes.push(json!({
            "id": id,
            "kind": "rectangle",
            "name": "Paper",
            "visible": true,
            "locked": true,
            "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "min": min,
            "max": max,
            "corner_radius": radius,
            "palette": [outline, fill],
        }));
    }

    fn circle(&mut self, centre: [f64; 2], radius: f64, rgb: [u8; 3]) {
        let outline = self.color("Outline", "outline", rgb, 1, false, false);
        let fill = self.color("Fill", "fill", rgb, 1, true, false);
        let id = self.id();
        self.nodes.push(json!({
            "id": id,
            "kind": "ellipse",
            "name": "Paper",
            "visible": true,
            "locked": true,
            "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "min": [centre[0] - radius, centre[1] - radius],
            "max": [centre[0] + radius, centre[1] + radius],
            "palette": [outline, fill],
        }));
    }

    fn line(&mut self, start: [f64; 2], end: [f64; 2], rgb: [u8; 3]) {
        let outline = self.color("Outline", "outline", rgb, self.width(), true, false);
        let fill = self.color("Fill", "fill", rgb, 1, false, false);
        let id = self.id();
        self.nodes.push(json!({
            "id": id,
            "kind": "line",
            "name": "Paper",
            "visible": true,
            "locked": true,
            "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "start": start,
            "end": end,
            "palette": [outline, fill],
        }));
    }

    fn text(&mut self, origin: [f64; 2], words: &str, size: f64, rgb: [u8; 3], bold: bool) {
        if words.is_empty() {
            return;
        }
        let color = self.color("Text", "text", rgb, 3, true, false);
        let background = self.color("Background", "background", PAPER, 3, false, false);
        let border = self.color("Border", "border", INK, 2, false, false);
        let id = self.id();
        self.nodes.push(json!({
            "id": id,
            "kind": "text",
            "name": "Paper",
            "visible": true,
            "locked": true,
            "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "content": words,
            "origin": origin,
            "box_width": null,
            "box_height": null,
            "fixed_width": 0.0,
            "style": {
                "align": "left",
                "all_caps": false,
                "bold": bold,
                "family": "inter",
                "italic": false,
                "letter_spacing": 0.0,
                "line_height": 1.2,
                "list": null,
                "list_kind": "bullets",
                "padding": 0.0,
                "radius": 0.0,
                "size": (size * self.unit).round().max(1.0) as u32,
                "underline": false,
                "valign": "top",
                "wrap": false,
            },
            "palette": [color, background, border],
        }));
    }

    /// A ruled box with its printed label and its value.
    fn boxed(&mut self, min: [f64; 2], max: [f64; 2], label: &str, value: &str) {
        self.rect(min, max, Some(PRINTED), None, false);
        let height = max[1] - min[1];
        let label_size = 30.0;
        self.text([min[0] + 14.0 * self.unit, min[1] + (height - label_size * self.unit) / 2.0], label, label_size, PRINTED, false);
        let value_size = 44.0;
        let value_x = min[0] + (14.0 + label.chars().count() as f64 * 21.0 + 24.0) * self.unit;
        self.text([value_x, min[1] + (height - value_size * self.unit) / 2.0], value, value_size, INK, false);
    }

    fn drawing(mut self, name: &str) -> Value {
        // A drawing lists its objects front first; the paper went in first,
        // so it goes to the back.
        self.nodes.reverse();
        json!({
            "format_version": 1,
            "id": 1,
            "name": name,
            "camera": { "origin": [0.0, 0.0], "zoom": 1.0 },
            "nodes": self.nodes,
        })
    }
}

/// The sheet of a clip, as a drawing: the paper and what is printed on it,
/// filled in from the storyboard, with `key` (such as "A③") at the field's
/// top left when the frame shows one.
pub fn paper(film: &Film, clip: &Clip, key: Option<&str>) -> Value {
    let [_, _, width, height] = field(film);
    let unit = height / 1080.0;
    let mut paper = Paper { nodes: Vec::new(), next: 0, unit };
    let (min, max) = paper_bounds(film);
    paper.rect(min, max, Some(FAINT), Some(PAPER), false);
    // Peg holes: a round hole between two long slots.
    let peg_y = min[1] + 50.0 * unit;
    paper.circle([width / 2.0, peg_y], 18.0 * unit, FAINT);
    for side in [-1.0, 1.0] {
        let x = width / 2.0 + side * width * 0.24;
        paper.rounded([x - 70.0 * unit, peg_y - 13.0 * unit], [x + 70.0 * unit, peg_y + 13.0 * unit], FAINT);
    }
    // The header: TITLE, SCENE, SHOT, and TIME.
    let scan = [-width * 0.05, -height * 0.05, width * 1.05, height * 1.05];
    let header = [scan[1] - 150.0 * unit, scan[1] - 60.0 * unit];
    let widths = [0.42, 0.18, 0.18, 0.22];
    let labels = ["TITLE:", "SCENE:", "SHOT:", "TIME:"];
    let values = [film.title.clone(), film.scene.clone(), clip.shot.clone(), time_text(clip.length)];
    let mut x = scan[0];
    let span = scan[2] - scan[0];
    for ((part, label), value) in widths.iter().zip(labels).zip(values) {
        let next = x + span * part;
        paper.boxed([x, header[0]], [next, header[1]], label, &value);
        x = next;
    }
    // The fields: scan at 110%, the field at 100%, and the safe field at 90%.
    paper.rect([scan[0], scan[1]], [scan[2], scan[3]], Some(FAINT), None, false);
    paper.rect([0.0, 0.0], [width, height], Some(INK), None, false);
    paper.rect([width * 0.05, height * 0.05], [width * 0.95, height * 0.95], Some(PRINTED), None, true);
    let label = 28.0;
    paper.text([scan[2] - 70.0 * unit, scan[1] - 28.0 * unit], "110%", label, PRINTED, false);
    paper.text([width - 70.0 * unit, 8.0 * unit], "100%", label, PRINTED, false);
    paper.text([width * 0.95 - 64.0 * unit, height * 0.05 + 8.0 * unit], "90%", label, PRINTED, false);
    // The centre cross, and a tick at the middle of each side.
    let (cx, cy) = (width / 2.0, height / 2.0);
    let arm = 22.0 * unit;
    paper.line([cx - arm, cy], [cx + arm, cy], PRINTED);
    paper.line([cx, cy - arm], [cx, cy + arm], PRINTED);
    for (start, end) in [
        ([cx, -arm], [cx, arm]),
        ([cx, height - arm], [cx, height + arm]),
        ([-arm, cy], [arm, cy]),
        ([width - arm, cy], [width + arm, cy]),
    ] {
        paper.line(start, end, PRINTED);
    }
    // NOTES and ANIMATOR under the fields.
    let footer = [scan[3] + 40.0 * unit, scan[3] + 130.0 * unit];
    let split = scan[0] + span * 0.74;
    let notes = [clip.action.as_str(), clip.dialogue.as_str(), clip.camera.as_str()]
        .into_iter()
        .filter(|note| !note.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    paper.boxed([scan[0], footer[0]], [split, footer[1]], "NOTES:", &notes);
    paper.boxed([split, footer[0]], [scan[2], footer[1]], "ANIMATOR:", &clip.animator);
    if let Some(key) = key {
        paper.text([width * 0.05 + 16.0 * unit, height * 0.05 + 12.0 * unit], key, 72.0, INK, false);
    }
    paper.drawing("Sheet")
}

/// A layer's drawing at a frame, as the protocol's drawing JSON.
fn layer_drawing(record: &Record, layer: &Layer, frame: u32) -> Option<Value> {
    let key = layer.key_at(frame)?;
    let drawing = record.drawing_of(key.drawing)?;
    Some(drawing.snapshot_value(draw::document::DrawingId::new(key.drawing.max(1)), "Layer"))
}

fn plain(drawing: Value, opacity: f64, above: bool, outline: Option<[u8; 3]>) -> Underlay {
    moved(drawing, opacity, above, outline, [0.0, 0.0])
}

fn moved(drawing: Value, opacity: f64, above: bool, outline: Option<[u8; 3]>, offset: [f64; 2]) -> Underlay {
    Underlay {
        drawing,
        tint: outline,
        opacity: opacity as f32,
        above,
        outline: outline.is_some(),
        offset,
        rotation: 0.0,
        tag: None,
    }
}

/// What the stage shows for a storyboard: inside a clip, the paper, every
/// other layer at the playhead above or below the active one, and the onion
/// skin; at the film's level, the clip under the playhead, or black over a
/// gap, with the field and the veil.
pub fn stage(record: &Record, tab: String) -> Stage {
    let film = record.film();
    let view = record.view();
    let mut underlays = Vec::new();
    let mut active = ActiveDrawing { visible: true, opacity: 1.0, outline: None };
    let mut fill = None;
    let mut field_rect = field(film);
    match view.clip.and_then(|clip| film.clip(clip)) {
        Some(clip) => {
            field_rect = field_in(film, clip);
            let active_layer = view.layer.and_then(|layer| clip.layer(layer));
            let key = active_layer.and_then(|layer| {
                let key = layer.key_at(view.frame)?;
                (!layer.folder).then(|| format!("{}{}", clip.letter(layer.id), circled(layer.key_number(key.id))))
            });
            underlays.push(moved(paper(film, clip, key.as_deref()), 1.0, false, None, clip.origin));
            if let Some(layer) = active_layer {
                active.visible = clip.shows(layer);
                active.opacity = layer.opacity as f32;
                active.outline = layer.outline.then_some(layer.color);
            }
            let active_index = active_layer.and_then(|layer| clip.layers.iter().position(|row| row.id == layer.id));
            // Bottom to top, so each list draws in order.
            for (index, layer) in clip.layers.iter().enumerate().rev() {
                if Some(index) == active_index || layer.folder || !clip.shows(layer) {
                    continue;
                }
                let above = active_index.is_some_and(|active| index < active);
                if let Some(drawing) = layer_drawing(record, layer, view.frame) {
                    underlays.push(plain(drawing, layer.opacity, above, layer.outline.then_some(layer.color)));
                }
            }
            // The onion skin lies over the layers below and under the drawing
            // worked on, so its tints read.
            if let Some(layer) = active_layer {
                onion(record, clip, layer, view.frame, &mut underlays);
            }
        }
        None => match film.clip_at(view.film_frame) {
            Some(clip) => {
                let frame = view.film_frame - clip.start;
                for layer in clip.layers.iter().rev() {
                    if layer.folder || !clip.shows(layer) {
                        continue;
                    }
                    // The clip's field lands on the film's field.
                    if let Some(drawing) = layer_drawing(record, layer, frame) {
                        underlays.push(moved(drawing, layer.opacity, false, None, [-clip.origin[0], -clip.origin[1]]));
                    }
                }
                underlays.insert(0, plain(blank_field(film), 1.0, false, None));
            }
            None => fill = Some(if film.clips.is_empty() { PAPER } else { [0, 0, 0] }),
        },
    }
    Stage {
        tab,
        image: None,
        fill,
        field: Some(field_rect),
        veil: view.clip.is_none(),
        underlays,
        overlays: Vec::new(),
        active,
    }
}

/// The white of the field, under a clip's drawings at the film's level.
fn blank_field(film: &Film) -> Value {
    let [_, _, width, height] = field(film);
    let mut paper = Paper { nodes: Vec::new(), next: 0, unit: height / 1080.0 };
    paper.rect([0.0, 0.0], [width, height], Some(INK), Some(PAPER), false);
    paper.drawing("Field")
}

/// The onion skin: the active layer's keys before and after the one shown,
/// tinted, fading with distance, under everything but the paper.
fn onion(record: &Record, clip: &Clip, layer: &Layer, frame: u32, underlays: &mut Vec<Underlay>) {
    let onion = record.onion();
    if !onion.on || layer.folder {
        return;
    }
    let Some(index) = layer.keys.iter().rposition(|key| key.frame <= frame) else {
        return;
    };
    let mut shown = Vec::new();
    for step in 1..=usize::from(onion.before) {
        let Some(key) = index.checked_sub(step).and_then(|at| layer.keys.get(at)) else {
            break;
        };
        shown.push((key.drawing, step, EARLIER));
    }
    for step in 1..=usize::from(onion.after) {
        let Some(key) = layer.keys.get(index + step).filter(|key| key.frame < clip.length) else {
            break;
        };
        shown.push((key.drawing, step, LATER));
    }
    // The furthest first, so the nearest lies on top.
    shown.sort_by(|a, b| b.1.cmp(&a.1));
    for (drawing, step, tint) in shown {
        let Some(found) = record.drawing_of(drawing) else {
            continue;
        };
        let opacity = (onion.opacity - onion.fade * (step as f64 - 1.0)).max(0.05);
        underlays.push(Underlay {
            drawing: found.snapshot_value(draw::document::DrawingId::new(drawing.max(1)), "Onion"),
            tint: Some(tint),
            opacity: opacity as f32,
            above: false,
            outline: false,
            offset: [0.0, 0.0],
            rotation: 0.0,
            tag: None,
        });
    }
}

/// Every mark the stage shows, in the order it paints them: what lies under
/// the drawing worked on, that drawing as its layer is set, and what lies
/// over it, as the host draws the stage. The look tool shows this, so the
/// assistant sees what the person sees.
pub fn stage_marks(record: &Record) -> Vec<draw::document::SceneMark> {
    use draw::document::SceneMark;
    let stage = stage(record, String::new());
    let mut below = Vec::new();
    let mut above = Vec::new();
    for underlay in &stage.underlays {
        let Some(found) = draw::document::snapshot::drawing_from_value(&underlay.drawing) else {
            continue;
        };
        let target = if underlay.above { &mut above } else { &mut below };
        let [dx, dy] = underlay.offset;
        for mark in found.drawing.scene(None) {
            target.push(match mark {
                SceneMark::Ink(mut paint) => {
                    for triangle in &mut paint.triangles {
                        for point in triangle.iter_mut() {
                            *point = [point[0] + dx, point[1] + dy];
                        }
                    }
                    paint.opacity *= f64::from(underlay.opacity);
                    if let Some(tint) = underlay.tint {
                        paint.rgb = tint;
                    }
                    if underlay.outline {
                        paint.opacity *= 0.35;
                    }
                    SceneMark::Ink(paint)
                }
                SceneMark::Text(mut view) => {
                    view.world = kurbo::Affine::translate((dx, dy)) * view.world;
                    SceneMark::Text(view)
                }
            });
        }
    }
    if stage.active.visible && record.editing().is_some() {
        for mark in record.drawing.scene(None) {
            below.push(match mark {
                SceneMark::Ink(mut paint) => {
                    paint.opacity *= f64::from(stage.active.opacity);
                    if let Some(rgb) = stage.active.outline {
                        paint.rgb = rgb;
                        paint.opacity *= 0.35;
                    }
                    SceneMark::Ink(paint)
                }
                text => text,
            });
        }
    }
    below.extend(above);
    below
}

/// Where the stage's sheet or field lies: the paper inside a clip, the
/// field on the film.
pub fn stage_bounds(record: &Record) -> ([f64; 2], [f64; 2]) {
    let film = record.film();
    if let Some(clip) = record.view().clip.and_then(|clip| film.clip(clip)) {
        paper_bounds_in(film, clip)
    } else {
        let [x, y, width, height] = field(film);
        ([x, y], [x + width, y + height])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_numbered_in_circles() {
        assert_eq!(circled(3), "③");
        assert_eq!(circled(21), "(21)");
        assert_eq!(time_text(60), "2 + 12");
    }

    #[test]
    fn the_paper_holds_the_headers_words_and_no_editable_object() {
        let mut film = Film::new("The Last Signal");
        film.scene = "3".into();
        let (clip, _) = film.add_clip(None, 60);
        let clip = film.clip(clip).unwrap().clone();
        let paper = paper(&film, &clip, Some("A③"));
        let text = paper.to_string();
        for words in ["TITLE:", "SCENE:", "SHOT:", "TIME:", "NOTES:", "ANIMATOR:", "The Last Signal", "2 + 12", "110%", "100%", "90%", "A③"] {
            assert!(text.contains(words), "{words} is missing");
        }
        let nodes = paper["nodes"].as_array().unwrap();
        assert!(nodes.iter().all(|node| node["locked"] == true));
        assert!(draw::document::snapshot::drawing_from_value(&paper).is_some(), "the paper is not a drawing");
    }
}
