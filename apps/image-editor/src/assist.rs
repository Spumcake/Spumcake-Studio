//! The assistant's "Image Editor" skill: it sees the tab's pictures and
//! their layers, arranges the gallery, and manages the shown picture's
//! layers, texts, masks, and adjustments. Painting, cloning, healing, and
//! moving pixels stay the person's. Each call goes through the same changes
//! the filmstrip and the panel make, so it honours the same locks. Every
//! layer call of one request is one undo step of the picture; each gallery
//! call is one step of the strip's history.

use crate::doc::Doc;
use crate::files;
use crate::gallery::Gallery;
use crate::picture::{Curves, Grade, Kind};
use crate::text::Style;
use protocol::{AppMessage, Skill, Tool, ToolResult};
use serde_json::{Value, json};

/// The longest side of the picture the assistant looks at.
const LOOK_SIZE: u32 = 1024;

pub fn skills() -> Vec<Skill> {
    let layer = json!({"type": "integer", "description": "The layer's or group's id, from list_layers."});
    let picture = json!({"type": "integer", "minimum": 0, "description": "The picture's position on the filmstrip, from list_pictures, 0 being the first."});
    let style = json!({
        "type": "object",
        "description": "A text's style. Leave out any part to keep it as it is, or as the default for a new text: Inter, 96 px, black, left, line height 1.2.",
        "properties": {
            "face": {"type": "string", "enum": ["inter", "mono"]},
            "size": {"type": "number", "description": "In picture pixels."},
            "bold": {"type": "boolean"},
            "italic": {"type": "boolean"},
            "underline": {"type": "boolean"},
            "color": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 3, "maxItems": 3, "description": "[red, green, blue]."},
            "align": {"type": "string", "enum": ["left", "center", "right", "justify"]},
            "line_height": {"type": "number", "description": "A multiple of the face's own line spacing."},
            "letter_spacing": {"type": "number", "description": "Pixels after every character but the last on a line."},
            "all_caps": {"type": "boolean"},
        },
        "additionalProperties": false,
    });
    let points = json!({"type": "array", "items": {"type": "array", "items": {"type": "number", "minimum": 0, "maximum": 1}, "minItems": 2, "maxItems": 2}, "minItems": 2, "maxItems": 16, "description": "[input, output] pairs from 0 to 1, left to right; the curve runs smoothly through them."});
    let object = |properties: Value, required: &[&str]| {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    };
    vec![Skill {
        id: "image".into(),
        name: "Image Editor".into(),
        description: "See the pictures in an Image Editor tab's gallery, arrange them on its filmstrip (show, add, duplicate, rename, reorder, and remove pictures), and manage the shown picture's layers: add, rename, hide or show, reorder, and delete them; add and set texts, masks, and Curves and Color Grade adjustments.".into(),
        instructions: "An Image Editor tab is a gallery: a filmstrip of pictures, one of them shown. Call list_pictures to see them in order, with which is shown; pictures are named by their position, 0 being the first, and positions change when pictures are added, moved, or removed. The layer tools act on the picture shown, so call show_picture first to work on another. Call list_layers first: it gives each layer's id, name, kind (pixels, group, text, curves, or grade), and settings, with a text's words and style, a Curves layer's curves, a Color Grade's values, and a layer's mask, top of the stack first, as the Layers panel shows them. Call look to see a picture itself. Positions in move_layer count from the top among the layer's siblings, 0 being the top. A layer whose lock is on refuses what it locks; renaming and deleting are always allowed. The gallery keeps at least one picture. A mask belongs to its layer: white shows the layer and black hides it. An adjustment changes everything beneath it within its group. You cannot paint, clone, heal, or move pixels; those stay the person's.".into(),
        tools: vec![
            Tool {
                name: "look".into(),
                description: "See a picture as it is composited now, scaled down to at most 1024 pixels on its longer side: the shown one, or another.".into(),
                input: object(json!({"picture": {"type": "integer", "minimum": 0, "description": "The picture's position, from list_pictures. Leave out for the picture shown."}}), &[]),
            },
            Tool {
                name: "list_pictures".into(),
                description: "List the gallery's pictures in filmstrip order, with each one's position, name, size, and number of layers, and which is shown.".into(),
                input: object(json!({}), &[]),
            },
            Tool {
                name: "show_picture".into(),
                description: "Show a picture, with its layers, as a click on its thumbnail does.".into(),
                input: object(json!({"picture": picture}), &["picture"]),
            },
            Tool {
                name: "add_picture".into(),
                description: "Add a new, empty 1920 × 1080 picture, one transparent layer, at the end of the filmstrip, and show it. Returns its position and name.".into(),
                input: object(
                    json!({"name": {"type": "string", "description": "The picture's name. Leave out for \"Untitled Asset\"; a name already used gets a number."}}),
                    &[],
                ),
            },
            Tool {
                name: "duplicate_picture".into(),
                description: "Copy a picture, with its layers, beside it, named \"… copy\", and show the copy. Returns its position and name.".into(),
                input: object(json!({"picture": picture}), &["picture"]),
            },
            Tool {
                name: "rename_picture".into(),
                description: "Rename a picture.".into(),
                input: object(json!({"picture": picture, "name": {"type": "string", "description": "The new name."}}), &["picture", "name"]),
            },
            Tool {
                name: "move_picture".into(),
                description: "Move a picture along the filmstrip.".into(),
                input: object(
                    json!({"picture": picture, "to": {"type": "integer", "minimum": 0, "description": "The picture's new position, 0 being the first."}}),
                    &["picture", "to"],
                ),
            },
            Tool {
                name: "remove_picture".into(),
                description: "Remove a picture from the gallery. The person can undo it from the filmstrip. The last picture cannot be removed.".into(),
                input: object(json!({"picture": picture}), &["picture"]),
            },
            Tool {
                name: "list_layers".into(),
                description: "List the picture's size and every layer and group, top first, with ids, names, kinds, visibility, blend mode, opacity, fill, locks, masks, and each text's, curve's, or grade's settings.".into(),
                input: object(json!({}), &[]),
            },
            Tool {
                name: "add_layer".into(),
                description: "Add a clear layer the picture's size at the top of the picture, or at the top of a group. Returns its id.".into(),
                input: object(
                    json!({
                        "name": {"type": "string", "description": "The layer's name."},
                        "group": {"type": "integer", "description": "A group's id to add it inside. Leave out for the top of the picture."},
                    }),
                    &["name"],
                ),
            },
            Tool {
                name: "rename_layer".into(),
                description: "Rename a layer or group.".into(),
                input: object(json!({"layer": layer, "name": {"type": "string", "description": "The new name."}}), &["layer", "name"]),
            },
            Tool {
                name: "set_visible".into(),
                description: "Show or hide a layer or group.".into(),
                input: object(json!({"layer": layer, "visible": {"type": "boolean"}}), &["layer", "visible"]),
            },
            Tool {
                name: "move_layer".into(),
                description: "Move a layer or group up or down among its siblings.".into(),
                input: object(
                    json!({"layer": layer, "to": {"type": "integer", "minimum": 0, "description": "The new position among its siblings, 0 being the top."}}),
                    &["layer", "to"],
                ),
            },
            Tool {
                name: "delete_layer".into(),
                description: "Delete a layer, or a group with everything in it. The picture keeps at least one layer.".into(),
                input: object(json!({"layer": layer}), &["layer"]),
            },
            Tool {
                name: "add_text".into(),
                description: "Add a text layer that stays editable, its box's top-left corner at x, y in picture pixels, above the active layer. Returns its id.".into(),
                input: object(
                    json!({
                        "text": {"type": "string", "description": "The words; a line break starts a new paragraph."},
                        "x": {"type": "number"},
                        "y": {"type": "number"},
                        "width": {"type": "number", "description": "The box's width the lines wrap in. Leave out for one line per paragraph."},
                        "style": style,
                    }),
                    &["text", "x", "y"],
                ),
            },
            Tool {
                name: "set_text".into(),
                description: "Change a text layer's words, style, or box.".into(),
                input: object(
                    json!({
                        "layer": layer,
                        "text": {"type": "string"},
                        "style": style,
                        "box": {
                            "type": "object",
                            "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "width": {"type": ["number", "null"], "description": "The width the lines wrap in, or null for one line per paragraph."}},
                            "required": ["x", "y"],
                            "additionalProperties": false,
                        },
                    }),
                    &["layer"],
                ),
            },
            Tool {
                name: "add_mask".into(),
                description: "Put a white mask on a layer, text, adjustment, or group, which shows all of it. The person paints it to hide parts. Returns the mask's id.".into(),
                input: object(json!({"layer": layer}), &["layer"]),
            },
            Tool {
                name: "set_mask".into(),
                description: "Set a layer's mask: its feather in pixels, and invert: true flips it once, white for black.".into(),
                input: object(
                    json!({"layer": layer, "feather": {"type": "number", "minimum": 0, "maximum": 250}, "invert": {"type": "boolean"}}),
                    &["layer"],
                ),
            },
            Tool {
                name: "add_adjustment".into(),
                description: "Add a Curves or Color Grade layer, which changes everything beneath it within its group, above a layer or above the active one. Returns its id.".into(),
                input: object(
                    json!({"kind": {"type": "string", "enum": ["curves", "grade"]}, "above": {"type": "integer", "description": "The layer to put it above. Leave out for above the active layer."}}),
                    &["kind"],
                ),
            },
            Tool {
                name: "set_curves".into(),
                description: "Set one curve of a Curves layer: rgb for all three channels, applied after each channel's own, or red, green, or blue.".into(),
                input: object(
                    json!({"layer": layer, "channel": {"type": "string", "enum": ["rgb", "red", "green", "blue"]}, "points": points}),
                    &["layer", "channel", "points"],
                ),
            },
            Tool {
                name: "set_grade".into(),
                description: "Set a Color Grade layer: exposure in stops from -3 to 3, and contrast, saturation, and temperature from -100 to 100, each 0 at rest. Leave out what stays.".into(),
                input: object(
                    json!({
                        "layer": layer,
                        "exposure": {"type": "number", "minimum": -3, "maximum": 3},
                        "contrast": {"type": "number", "minimum": -100, "maximum": 100},
                        "saturation": {"type": "number", "minimum": -100, "maximum": 100},
                        "temperature": {"type": "number", "minimum": -100, "maximum": 100},
                    }),
                    &["layer"],
                ),
            },
        ],
    }]
}

/// Runs one tool on a tab's gallery: a gallery tool on the gallery, `look`
/// on the picture it names, and a layer tool on the picture shown. Returns
/// what to tell the model, and a picture for it to look at.
pub fn run(gallery: &mut Gallery, tool: &str, input: &Value, group: &str) -> (Result<Value, String>, Option<String>) {
    let picture = || {
        input.get("picture").and_then(Value::as_u64).map(|index| index as usize).ok_or_else(|| String::from("Give the picture's position as `picture`."))
    };
    let named = |index: usize, gallery: &Gallery| json!({"picture": index, "name": gallery.frames[index].name});
    let outcome = match tool {
        "look" => {
            let index = match input.get("picture") {
                None | Some(Value::Null) => gallery.shown,
                Some(_) => match picture() {
                    Ok(index) if index < gallery.frames.len() => index,
                    Ok(index) => return (Err(format!("There is no picture {index}; the gallery holds {}.", gallery.frames.len())), None),
                    Err(reason) => return (Err(reason), None),
                },
            };
            let (outcome, look) = run_doc(&mut gallery.frames[index].doc, tool, input, group);
            let outcome = outcome.map(|mut output| {
                output["picture"] = json!(index);
                output["name"] = json!(gallery.frames[index].name);
                output
            });
            return (outcome, look);
        }
        "list_pictures" => Ok(json!({
            "shown": gallery.shown,
            "pictures": gallery
                .frames
                .iter()
                .enumerate()
                .map(|(index, frame)| json!({
                    "picture": index,
                    "name": frame.name,
                    "width": frame.doc.picture.width,
                    "height": frame.doc.picture.height,
                    "layers": frame.doc.picture.all_layers().len(),
                    "shown": index == gallery.shown,
                }))
                .collect::<Vec<_>>(),
        })),
        "show_picture" => picture().and_then(|index| gallery.select(index).map(|()| named(index, gallery))),
        "add_picture" => {
            let name = input.get("name").and_then(Value::as_str).filter(|name| !name.trim().is_empty()).unwrap_or(crate::session::UNTITLED);
            let index = gallery.add(name, crate::session::new_doc(), None);
            Ok(named(index, gallery))
        }
        "duplicate_picture" => picture().and_then(|index| gallery.duplicate(index)).map(|index| named(index, gallery)),
        "rename_picture" => match (picture(), input.get("name").and_then(Value::as_str)) {
            (Ok(index), Some(name)) => gallery.rename(index, name).map(|()| named(index, gallery)),
            (Err(reason), _) => Err(reason),
            (_, None) => Err("Give the new name as `name`.".into()),
        },
        "move_picture" => match (picture(), input.get("to").and_then(Value::as_u64)) {
            (Ok(index), Some(to)) => {
                let to = (to as usize).min(gallery.frames.len().saturating_sub(1));
                gallery.move_to(index, to).map(|()| named(to, gallery))
            }
            (Err(reason), _) => Err(reason),
            (_, None) => Err("Give the new position as `to`.".into()),
        },
        "remove_picture" => picture().and_then(|index| {
            let name = gallery.frames.get(index).map(|frame| frame.name.clone());
            gallery.remove(index).map(|()| json!({"removed": index, "name": name, "shown": gallery.shown}))
        }),
        _ => return run_doc(&mut gallery.frame_mut().doc, tool, input, group),
    };
    (outcome, None)
}

/// Runs one tool on a picture's document.
pub fn run_doc(doc: &mut Doc, tool: &str, input: &Value, group: &str) -> (Result<Value, String>, Option<String>) {
    let layer = || input.get("layer").and_then(Value::as_u64).ok_or_else(|| String::from("Give the layer's id as `layer`."));
    let by = Some(group);
    let outcome = match tool {
        "look" => {
            let (width, height) = (doc.picture.width, doc.picture.height);
            let (small_w, small_h, small) = files::shrink(width, height, doc.composite(), LOOK_SIZE);
            return match files::encode_png(small_w, small_h, &small) {
                Ok(png) => (
                    Ok(json!({"width": width, "height": height, "shown_at": [small_w, small_h]})),
                    Some(files::base64(&png)),
                ),
                Err(reason) => (Err(reason), None),
            };
        }
        "list_layers" => Ok(list(doc)),
        "add_layer" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            let group = input.get("group").and_then(Value::as_u64);
            doc.add_layer(name, group, by).map(|id| json!({"id": id}))
        }
        "rename_layer" => match (layer(), input.get("name").and_then(Value::as_str)) {
            (Ok(id), Some(name)) => doc.rename(id, name, by).map(|()| json!({"id": id, "name": name.trim()})),
            (Err(reason), _) => Err(reason),
            (_, None) => Err("Give the new name as `name`.".into()),
        },
        "set_visible" => match (layer(), input.get("visible").and_then(Value::as_bool)) {
            (Ok(id), Some(visible)) => doc.set_visible(id, visible, by).map(|()| json!({"id": id, "visible": visible})),
            (Err(reason), _) => Err(reason),
            (_, None) => Err("Give `visible` as true or false.".into()),
        },
        "move_layer" => match (layer(), input.get("to").and_then(Value::as_u64)) {
            (Ok(id), Some(to)) => doc.reorder(id, to as usize, by).map(|()| json!({"id": id, "to": to})),
            (Err(reason), _) => Err(reason),
            (_, None) => Err("Give the new position as `to`.".into()),
        },
        "delete_layer" => layer().and_then(|id| doc.delete(id, by).map(|()| json!({"deleted": id}))),
        "add_text" => {
            let words = input.get("text").and_then(Value::as_str);
            let (x, y) = (input.get("x").and_then(Value::as_f64), input.get("y").and_then(Value::as_f64));
            match (words, x, y, style_of(input.get("style"), Style::default())) {
                (Some(words), Some(x), Some(y), Ok(style)) if x.is_finite() && y.is_finite() => {
                    let width = input.get("width").and_then(Value::as_f64).filter(|width| width.is_finite() && *width > 0.0).map(|width| width as f32);
                    doc.add_text(words, [x.round() as i32, y.round() as i32], width, style, by).map(|id| json!({"id": id}))
                }
                (_, _, _, Err(reason)) => Err(reason),
                (None, _, _, _) => Err("Give the words as `text`.".into()),
                _ => Err("Give the box's corner as `x` and `y`.".into()),
            }
        }
        "set_text" => layer().and_then(|id| {
            let text = doc.picture.find(id).and_then(|node| node.text()).cloned().ok_or_else(|| format!("Layer {id} is not a text."))?;
            let style = match input.get("style") {
                Some(given) => Some(style_of(Some(given), text.style)?),
                None => None,
            };
            let place = match input.get("box") {
                Some(given) => {
                    let (x, y) = (given.get("x").and_then(Value::as_f64), given.get("y").and_then(Value::as_f64));
                    let (Some(x), Some(y)) = (x, y) else {
                        return Err("Give the box's `x` and `y`.".into());
                    };
                    let width = match given.get("width") {
                        Some(Value::Null) => None,
                        Some(width) => Some(width.as_f64().filter(|width| width.is_finite() && *width > 0.0).ok_or("Give the box's `width` as a positive number, or null.")? as f32),
                        None => text.width,
                    };
                    Some(([x.round() as i32, y.round() as i32], width))
                }
                None => None,
            };
            doc.set_text(id, input.get("text").and_then(Value::as_str), style, place, by).map(|()| json!({"id": id}))
        }),
        "add_mask" => layer().and_then(|id| doc.add_mask(id, by).map(|mask| json!({"layer": id, "mask": mask}))),
        "set_mask" => layer().and_then(|id| {
            if let Some(feather) = input.get("feather").and_then(Value::as_f64) {
                doc.set_feather(id, feather, false, by)?;
            }
            if input.get("invert").and_then(Value::as_bool) == Some(true) {
                doc.invert_mask(id, by)?;
            }
            Ok(json!({"layer": id}))
        }),
        "add_adjustment" => {
            let kind = match input.get("kind").and_then(Value::as_str) {
                Some("curves") => Ok(Kind::Curves(Curves::default())),
                Some("grade") => Ok(Kind::Grade(Grade::default())),
                _ => Err(String::from("Give `kind` as curves or grade.")),
            };
            kind.and_then(|kind| doc.add_adjustment(kind, input.get("above").and_then(Value::as_u64), by).map(|id| json!({"id": id})))
        }
        "set_curves" => layer().and_then(|id| {
            let channel = input.get("channel").and_then(Value::as_str).and_then(|name| Curves::CHANNELS.iter().position(|own| *own == name)).ok_or("Give `channel` as rgb, red, green, or blue.")?;
            let points: Vec<[f64; 2]> = serde_json::from_value(input.get("points").cloned().unwrap_or(Value::Null)).map_err(|_| "Give `points` as [input, output] pairs.")?;
            doc.set_curve(id, channel, &points, false, by).map(|()| json!({"id": id}))
        }),
        "set_grade" => layer().and_then(|id| {
            let mut grade = match doc.picture.find(id).map(|node| &node.kind) {
                Some(Kind::Grade(grade)) => *grade,
                _ => return Err(format!("Layer {id} is not a Color Grade layer.")),
            };
            let value = |key: &str| input.get(key).and_then(Value::as_f64).map(|value| value as f32);
            grade.exposure = value("exposure").unwrap_or(grade.exposure);
            grade.contrast = value("contrast").unwrap_or(grade.contrast);
            grade.saturation = value("saturation").unwrap_or(grade.saturation);
            grade.temperature = value("temperature").unwrap_or(grade.temperature);
            doc.set_grade(id, grade, false, by).map(|()| json!({"id": id}))
        }),
        other => Err(format!("The Image Editor has no tool {other}.")),
    };
    (outcome, None)
}

/// A style given as JSON over `base`: what it leaves out stays.
fn style_of(given: Option<&Value>, base: Style) -> Result<Style, String> {
    let Some(given) = given else {
        return Ok(base);
    };
    let Some(given) = given.as_object() else {
        return Err("Give `style` as an object.".into());
    };
    let mut merged = serde_json::to_value(base).map_err(|error| error.to_string())?;
    for (key, value) in given {
        merged[key] = value.clone();
    }
    serde_json::from_value::<Style>(merged).map(Style::clamped).map_err(|error| format!("The style is not one: {error}."))
}

/// Every layer, top first, as the Layers panel lists them.
fn list(doc: &Doc) -> Value {
    fn rows(doc: &Doc, nodes: &[crate::picture::Node], parent: Option<u64>, out: &mut Vec<Value>) {
        let count = nodes.len();
        for (index, node) in nodes.iter().enumerate().rev() {
            let size = doc.picture.pixels.get(&node.id).map(|pixels| [pixels.width, pixels.height]);
            let mut row = json!({
                "id": node.id,
                "name": node.name,
                "kind": match &node.kind {
                    _ if node.is_group() => "group",
                    Kind::Pixels => "pixels",
                    Kind::Text(_) => "text",
                    Kind::Curves(_) => "curves",
                    Kind::Grade(_) => "grade",
                },
                "parent": parent,
                "position": count - 1 - index,
                "visible": node.visible,
                "active": doc.active == Some(node.id),
                "blend": node.blend.label(),
                "opacity": (node.opacity * 100.0).round(),
                "locks": {
                    "transparency": node.locks.transparency,
                    "pixels": node.locks.pixels,
                    "position": node.locks.position,
                    "all": node.locks.all,
                },
            });
            if !node.is_group() {
                row["fill"] = json!((node.fill * 100.0).round());
                row["offset"] = json!(node.offset);
                if node.has_pixels() {
                    row["size"] = json!(size);
                }
            }
            match &node.kind {
                Kind::Text(text) if !node.is_group() => row["text"] = json!({"words": text.words, "style": text.style, "width": text.width}),
                Kind::Curves(curves) if !node.is_group() => row["curves"] = json!(curves),
                Kind::Grade(grade) if !node.is_group() => row["grade"] = json!(grade),
                _ => {}
            }
            if let Some(mask) = &node.mask {
                row["mask"] = json!({"id": mask.id, "name": mask.name, "feather": mask.feather, "visible": mask.visible});
            }
            out.push(row);
            if let Some(children) = &node.children {
                rows(doc, children, Some(node.id), out);
            }
        }
    }
    let mut out = Vec::new();
    rows(doc, &doc.picture.nodes, None, &mut out);
    json!({"width": doc.picture.width, "height": doc.picture.height, "layers": out})
}

pub fn result(call: String, outcome: Result<Value, String>) -> AppMessage {
    result_with(call, outcome, None)
}

pub fn result_with(call: String, outcome: Result<Value, String>, image: Option<String>) -> AppMessage {
    let (ok, output) = match outcome {
        Ok(output) => (true, output),
        Err(error) => (false, json!({"error": error})),
    };
    AppMessage::ToolResult(ToolResult { call, ok, output, images: image.into_iter().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::Picture;

    fn call(doc: &mut Doc, tool: &str, input: Value, group: &str) -> Result<Value, String> {
        run_doc(doc, tool, &input, group).0
    }

    /// Catches a tool that does nothing, one that ignores a lock, and calls
    /// of one request that undo one by one.
    #[test]
    fn the_assistant_manages_layers_and_honours_locks() {
        let mut doc = Doc::new(Picture::blank(64, 36));
        let background = doc.active.unwrap();
        let made = call(&mut doc, "add_layer", json!({"name": "Sky"}), "r1").unwrap();
        let sky = made["id"].as_u64().unwrap();
        call(&mut doc, "rename_layer", json!({"layer": sky, "name": "Clouds"}), "r1").unwrap();
        call(&mut doc, "set_visible", json!({"layer": sky, "visible": false}), "r1").unwrap();
        call(&mut doc, "move_layer", json!({"layer": sky, "to": 1}), "r1").unwrap();
        let listed = call(&mut doc, "list_layers", json!({}), "r1").unwrap();
        let layers = listed["layers"].as_array().unwrap();
        assert_eq!(layers[0]["id"], background, "the move did not put Clouds under the background");
        assert_eq!(layers[1]["name"], "Clouds");
        assert_eq!(layers[1]["visible"], false);
        // The four calls are one step.
        assert!(doc.undo());
        assert_eq!(doc.picture.nodes.len(), 1);
        assert!(!doc.can_undo());
        assert!(doc.redo());

        // Lock all refuses a reorder and a layer added into the group it
        // locks; renaming and deleting stay allowed.
        let group = doc.group_active().unwrap();
        doc.toggle_lock(group, 3).unwrap();
        assert!(call(&mut doc, "add_layer", json!({"name": "In", "group": group}), "r2").is_err());
        assert!(call(&mut doc, "move_layer", json!({"layer": group, "to": 1}), "r2").is_err());
        assert!(call(&mut doc, "rename_layer", json!({"layer": group, "name": "Kept"}), "r2").is_ok());
        assert!(call(&mut doc, "delete_layer", json!({"layer": group}), "r2").is_ok());
        assert!(call(&mut doc, "delete_layer", json!({"layer": background}), "r3").is_err(), "the last layer was deleted");
        assert!(call(&mut doc, "paint", json!({}), "r3").is_err());
    }

    /// Catches a look that sends no picture, or one that is not a PNG.
    #[test]
    fn look_sends_the_picture() {
        let mut doc = Doc::new(Picture::blank(2048, 1024));
        let (outcome, image) = run_doc(&mut doc, "look", &json!({}), "r1");
        let outcome = outcome.unwrap();
        assert_eq!(outcome["shown_at"], json!([1024, 512]));
        let image = image.expect("no picture");
        // A PNG's signature, in base64.
        assert!(image.starts_with("iVBORw0KGgo"), "{}", &image[..20]);
        let names: Vec<String> = skills()[0].tools.iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(
            names,
            [
                "look", "list_pictures", "show_picture", "add_picture", "duplicate_picture", "rename_picture",
                "move_picture", "remove_picture", "list_layers", "add_layer", "rename_layer", "set_visible", "move_layer",
                "delete_layer", "add_text", "set_text", "add_mask", "set_mask", "add_adjustment", "set_curves", "set_grade",
            ]
        );
    }

    /// Catches a gallery tool that does nothing, one that is more or less
    /// than one step of the strip's history, a look that ignores `picture`,
    /// and a layer tool that acts on a picture not shown.
    #[test]
    fn the_assistant_arranges_the_gallery_one_step_each() {
        let mut gallery = Gallery::one("Untitled Asset", Doc::new(Picture::blank(64, 36)));
        let call = |gallery: &mut Gallery, tool: &str, input: Value| run(gallery, tool, &input, "r1").0;
        let added = call(&mut gallery, "add_picture", json!({"name": "Beach"})).unwrap();
        assert_eq!(added, json!({"picture": 1, "name": "Beach"}));
        let copy = call(&mut gallery, "duplicate_picture", json!({"picture": 1})).unwrap();
        assert_eq!(copy, json!({"picture": 2, "name": "Beach copy"}));
        call(&mut gallery, "rename_picture", json!({"picture": 0, "name": "Sky"})).unwrap();
        call(&mut gallery, "move_picture", json!({"picture": 0, "to": 2})).unwrap();
        call(&mut gallery, "add_picture", json!({})).unwrap();
        call(&mut gallery, "remove_picture", json!({"picture": 3})).unwrap();
        let listed = call(&mut gallery, "list_pictures", json!({})).unwrap();
        let names: Vec<&str> = listed["pictures"].as_array().unwrap().iter().map(|row| row["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Beach", "Beach copy", "Sky"]);
        assert_eq!(listed["pictures"][0]["width"], 1920, "an added picture is not 1920 wide");

        // A layer tool acts on the picture shown, and look on the one named.
        call(&mut gallery, "show_picture", json!({"picture": 2})).unwrap();
        assert_eq!(gallery.frame().name, "Sky");
        call(&mut gallery, "add_layer", json!({"name": "Notes"})).unwrap();
        assert_eq!(gallery.frames[2].doc.picture.nodes.len(), 2);
        assert_eq!(gallery.frames[0].doc.picture.nodes.len(), 1, "a layer tool reached a picture not shown");
        let (looked, image) = run(&mut gallery, "look", &json!({"picture": 0}), "r1");
        assert_eq!(looked.unwrap()["width"], 1920);
        assert!(image.is_some());
        assert!(run(&mut gallery, "look", &json!({"picture": 9}), "r1").0.is_err());
        assert!(call(&mut gallery, "show_picture", json!({"picture": 9})).is_err());

        // Six gallery calls, six steps: each undo takes back exactly one.
        let mut counts = Vec::new();
        while gallery.undo() {
            counts.push(gallery.frames.len());
        }
        assert_eq!(counts, [4, 3, 3, 3, 2, 1]);
        assert_eq!(gallery.frames[0].name, "Untitled Asset");
        // The last picture stays.
        assert!(call(&mut gallery, "remove_picture", json!({"picture": 0})).is_err());
    }

    /// Catches the assistant's text, mask, and adjustment tools doing
    /// nothing, more than one step a request, or a list that does not say
    /// what each layer is: it adds a text and sets its words and style, puts
    /// a mask on a layer and inverts it, adds Curves and Color Grade and
    /// sets them, and list_layers reports every kind with its settings.
    #[test]
    fn the_assistant_adds_and_sets_texts_masks_and_adjustments() {
        let mut doc = Doc::new(Picture::blank(64, 36));
        let background = doc.active.unwrap();
        let before = doc.composite().to_vec();
        let text = call(&mut doc, "add_text", json!({"text": "Hi", "x": 4, "y": 4, "style": {"size": 20, "color": [200, 0, 0]}}), "r1").unwrap()["id"].as_u64().unwrap();
        call(&mut doc, "set_text", json!({"layer": text, "text": "Hello", "style": {"bold": true}, "box": {"x": 2, "y": 3, "width": 40}}), "r1").unwrap();
        assert!(doc.undo() && !doc.can_undo() && doc.picture.find(text).is_none(), "one request took more than one step");
        assert!(doc.redo());
        let node = doc.picture.find(text).unwrap();
        let words = node.text().unwrap();
        assert_eq!((words.words.as_str(), words.style.bold, words.style.size, words.width), ("Hello", true, 20.0, Some(40.0)));
        assert_eq!(node.offset, [2, 3]);
        assert_ne!(doc.composite(), &before[..], "the text drew nothing");

        let mask = call(&mut doc, "add_mask", json!({"layer": background}), "r2").unwrap()["mask"].as_u64().unwrap();
        call(&mut doc, "set_mask", json!({"layer": background, "feather": 4, "invert": true}), "r2").unwrap();
        assert_eq!(doc.picture.find(background).unwrap().mask.as_ref().map(|mask| mask.feather), Some(4.0));
        assert_eq!(doc.picture.pixels[&mask].data[0], 0, "invert did not flip the mask");
        let curves = call(&mut doc, "add_adjustment", json!({"kind": "curves", "above": background}), "r3").unwrap()["id"].as_u64().unwrap();
        call(&mut doc, "set_curves", json!({"layer": curves, "channel": "red", "points": [[0, 0], [0.5, 0.8], [1, 1]]}), "r3").unwrap();
        let grade = call(&mut doc, "add_adjustment", json!({"kind": "grade"}), "r4").unwrap()["id"].as_u64().unwrap();
        call(&mut doc, "set_grade", json!({"layer": grade, "saturation": -100, "exposure": 9}), "r4").unwrap();
        assert!(call(&mut doc, "set_grade", json!({"layer": curves, "contrast": 10}), "r5").is_err());
        assert!(call(&mut doc, "set_curves", json!({"layer": curves, "channel": "red", "points": [[0.5, 0.5]]}), "r5").is_err());

        let listed = call(&mut doc, "list_layers", json!({}), "r6").unwrap();
        let layers = listed["layers"].as_array().unwrap();
        let kinds: Vec<&str> = layers.iter().map(|row| row["kind"].as_str().unwrap()).collect();
        // The grade went above the active layer, the curves.
        assert_eq!(kinds, ["text", "grade", "curves", "pixels"]);
        assert_eq!(layers[1]["grade"]["saturation"], -100.0);
        assert_eq!(layers[1]["grade"]["exposure"], 3.0, "exposure was not kept within its range");
        assert_eq!(layers[0]["text"]["words"], "Hello");
        assert_eq!(layers[2]["curves"]["red"][1], json!([0.5, 0.8]));
        assert_eq!(layers[3]["mask"]["id"], mask);
    }
}
