//! The assistant's tools for the film and the inside of its clips: one for
//! each hand action on the timeline, the clip's and layer's and
//! storyboard's pages, and the onion skin's settings. Each runs the session
//! call the hand runs, so the person and the assistant change the same
//! storyboard the same way, and every change of one request is one undo.
//!
//! Frames are counted from 1, as people read them and the ruler shows them.

use crate::film::{Film, NEW_CLIP_LENGTH};
use crate::session::Session;
use crate::session::film::{ClipText, LayerFlag};
use crate::tools::Outcome;
use protocol::{Tool, ToolGroup};
use serde_json::{Value, json};

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Tool {
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        input: json!({ "type": "object", "properties": properties, "required": required }),
        implements: Vec::new(),
    }
}

fn integer(description: &str) -> Value {
    json!({ "type": "integer", "description": description })
}

fn layers_schema() -> Value {
    json!({ "type": "array", "items": { "type": "integer" }, "description": "Layer ids. Leave out for the selected frames' layers, or the active layer." })
}

pub fn group() -> ToolGroup {
    ToolGroup {
        id: "clips".into(),
        name: "Film and clips".into(),
        description: "Make and time the film's clips, and inside a clip its layers, keyframes, holds, labels, and onion skin, as the timeline and its pages do.".into(),
        instructions: "A storyboard holds a film: clips on the Shots track, each one shot, played in order. Inside a clip are layers, top to bottom, each with keyframes: a keyframe's drawing holds until the next keyframe, and a blank keyframe holds nothing. Frames count from 1, on the film from its start and in a clip from the clip's start. Drawing happens inside a clip, on the active layer at the playhead: open a clip with edit_clip, choose a frame with go_to_frame and a layer with select_layer, then use the Draw by hand tools; go back with leave_clip. read_film says where the person is, every clip with its layers and keys, and each clip's item, which other workspaces name its first and last frames by. Retiming a clip's end moves the clips after it; moving clips with push moves those they land on, and without it covers them. Moving frames with push moves the frames after them; without it, they replace the frames they land on. All the changes of one request are taken back by one undo.".into(),
        tools: vec![
            tool("read_film", "The film as data: where the person is, the storyboard's settings, the onion skin, the selection, and every clip with its timing, notes, items, layers, keys, and marks.", json!({}), &[]),
            tool("add_clip", "Add a clip, with one blank layer. Clips it lands on move later.", json!({ "start": integer("The film's frame it starts on. Leave out for after the last clip."), "length": integer("Frames. Default 48, two seconds.") }), &[]),
            tool("duplicate_clip", "Copy a clip, drawings and all, right after it.", json!({ "clip": integer("The clip's id.") }), &["clip"]),
            tool("delete_clip", "Take a clip off the film. Its place stays empty.", json!({ "clip": integer("The clip's id.") }), &["clip"]),
            tool("retime_clip", "Move a clip's start or end edge by a number of frames. Moving its end moves the clips after it.", json!({ "clip": integer("The clip's id."), "edge": { "type": "string", "enum": ["start", "end"] }, "frames": integer("Frames, negative to shorten from the end or lengthen from the start.") }), &["clip", "edge", "frames"]),
            tool("move_clips", "Move clips by a number of frames.", json!({ "clips": { "type": "array", "items": { "type": "integer" } }, "frames": integer("Frames, negative for earlier."), "push": { "type": "boolean", "description": "Move the clips they land on later instead of covering them. Default true." } }), &["clips", "frames"]),
            tool("set_clip", "Set a clip's name, shot number, notes, or animator, as its page in the panel does.", json!({ "clip": integer("The clip's id."), "name": { "type": "string" }, "shot": { "type": "string" }, "action": { "type": "string" }, "dialogue": { "type": "string" }, "camera": { "type": "string" }, "animator": { "type": "string" } }), &["clip"]),
            tool("select_clips", "Select clips on the film, as clicking them does.", json!({ "clips": { "type": "array", "items": { "type": "integer" } } }), &["clips"]),
            tool("edit_clip", "Open a clip on its sheet, to draw inside it, as Edit clip does.", json!({ "clip": integer("The clip's id.") }), &["clip"]),
            tool("leave_clip", "Go back from a clip to the film.", json!({}), &[]),
            tool("go_to_frame", "Move the playhead: inside a clip, to its frame; on the film, to the film's frame.", json!({ "frame": integer("Frame, from 1.") }), &["frame"]),
            tool("select_layer", "Make a layer of the open clip the one drawn on.", json!({ "layer": integer("The layer's id.") }), &["layer"]),
            tool("select_frames", "Select a block of frames on layers of the open clip, as dragging across them does.", json!({ "layers": layers_schema(), "start": integer("First frame, from 1."), "end": integer("Last frame, from 1.") }), &["start", "end"]),
            tool("add_layer", "Add a layer, or a folder, above the active layer of the open clip.", json!({ "folder": { "type": "boolean" } }), &[]),
            tool("delete_layer", "Delete a layer of the open clip, or a folder with what is in it. A clip keeps one layer.", json!({ "layer": integer("The layer's id.") }), &["layer"]),
            tool("move_layer", "Move a layer to a position in the list, 0 at the top, and into a folder or out of one.", json!({ "layer": integer("The layer's id."), "position": integer("0 is the top."), "folder": integer("The folder it goes into. Leave out for none.") }), &["layer", "position"]),
            tool("set_layer", "Set a layer's name, colour, opacity, eye, lock, outline, or a folder's open state.", json!({ "layer": integer("The layer's id."), "name": { "type": "string" }, "color": { "type": "string", "description": "#rrggbb" }, "opacity": { "type": "number", "minimum": 0, "maximum": 1 }, "visible": { "type": "boolean" }, "locked": { "type": "boolean" }, "outline": { "type": "boolean" }, "open": { "type": "boolean" } }), &["layer"]),
            tool("insert_frames", "Insert frames after a frame on layers, as Insert frame does: their later keys move later and the clip grows.", json!({ "layers": layers_schema(), "frame": integer("Frame, from 1."), "count": integer("Frames. Default 1.") }), &["frame"]),
            tool("remove_frames", "Remove frames from layers, as Remove frames does: keys there go and later ones move earlier.", json!({ "layers": layers_schema(), "frame": integer("First frame, from 1."), "count": integer("Frames. Default 1.") }), &["frame"]),
            tool("insert_keyframe", "Make a frame of a layer a keyframe: a copy of what it showed, or blank.", json!({ "layer": integer("The layer's id."), "frame": integer("Frame, from 1."), "blank": { "type": "boolean" } }), &["layer", "frame"]),
            tool("clear_keyframe", "Clear a keyframe: the one before holds on.", json!({ "layer": integer("The layer's id."), "frame": integer("Frame, from 1.") }), &["layer", "frame"]),
            tool("move_frames", "Move frames start..end of layers by a number of frames.", json!({ "layers": layers_schema(), "start": integer("First frame, from 1."), "end": integer("Last frame, from 1."), "frames": integer("Frames, negative for earlier."), "push": { "type": "boolean", "description": "Move the frames after them along instead of replacing what they land on." } }), &["start", "end", "frames"]),
            tool("stretch_hold", "Lengthen or shorten the hold of the keyframe at a frame, moving the layer's later keys with it.", json!({ "layer": integer("The layer's id."), "frame": integer("The keyframe's frame, from 1."), "frames": integer("Frames to add, negative to take away.") }), &["layer", "frame", "frames"]),
            tool("copy_frames", "Copy frames start..end of layers, for paste_frames.", json!({ "layers": layers_schema(), "start": integer("First frame, from 1."), "end": integer("Last frame, from 1.") }), &["start", "end"]),
            tool("paste_frames", "Paste the copied frames at a frame, replacing what is there.", json!({ "layers": layers_schema(), "frame": integer("Frame, from 1.") }), &["frame"]),
            tool("reverse_frames", "Reverse frames start..end of layers: their holds come in the opposite order.", json!({ "layers": layers_schema(), "start": integer("First frame, from 1."), "end": integer("Last frame, from 1.") }), &["start", "end"]),
            tool("add_mark", "Add a label or a comment at a frame of a layer.", json!({ "layer": integer("The layer's id."), "frame": integer("Frame, from 1."), "kind": { "type": "string", "enum": ["label", "comment"] }, "text": { "type": "string" } }), &["layer", "frame", "kind", "text"]),
            tool("remove_mark", "Remove a label or a comment.", json!({ "layer": integer("The layer's id."), "mark": integer("The mark's id.") }), &["layer", "mark"]),
            tool("set_onion_skin", "Turn onion skin on or off, and set how many keys before and after it shows, its starting opacity, how much it fades, and whether it counts keys only. A view aid: not an undo step.", json!({ "on": { "type": "boolean" }, "before": integer("0 to 9."), "after": integer("0 to 9."), "opacity": { "type": "number", "minimum": 0, "maximum": 1 }, "fade": { "type": "number", "minimum": 0, "maximum": 1 }, "keys_only": { "type": "boolean" } }), &[]),
            tool("set_storyboard", "Set the storyboard's title and scene, which the sheet's header shows, and the film's frame size and paper.", json!({ "title": { "type": "string" }, "scene": { "type": "string" }, "width": integer("Frame width."), "height": integer("Frame height."), "paper": integer("0: Japanese layout paper, 16:9.") }), &[]),
        ],
    }
}

/// The film as the read tool and the results give it.
pub fn describe(session: &Session) -> Value {
    let record = session.active_storyboard();
    let film = record.film();
    let view = record.view();
    let picked = session.picked();
    let onion = session.onion();
    let path = record.path().map(|path| path.display().to_string());
    json!({
        "title": film.title,
        "scene": film.scene,
        "frame_size": film.size,
        "paper": crate::film::PAPER_NAMES[usize::from(film.paper)],
        "length": film.length(),
        "where": {
            "clip": view.clip,
            "frame": view.clip.map(|_| view.frame + 1),
            "film_frame": view.film_frame + 1,
            "layer": view.layer,
        },
        "can_draw": session.can_draw(),
        "selected_clips": picked.clips,
        "selected_frames": picked.cells.map(|cells| json!({ "layers": cells.layers, "start": cells.start + 1, "end": cells.end + 1 })),
        "onion_skin": { "on": onion.on, "before": onion.before, "after": onion.after, "opacity": onion.opacity, "fade": onion.fade, "keys_only": onion.keys_only },
        "clips": film.clips.iter().map(|clip| clip_json(session, clip, path.as_deref())).collect::<Vec<_>>(),
    })
}

fn clip_json(session: &Session, clip: &crate::film::Clip, path: Option<&str>) -> Value {
    json!({
        "id": clip.id,
        "name": clip.name,
        "shot": clip.shot,
        "start": clip.start + 1,
        "length": clip.length,
        "action": clip.action,
        "dialogue": clip.dialogue,
        "camera": clip.camera,
        "animator": clip.animator,
        "items": path.map(|path| json!({
            "first_frame": { "path": path, "item": crate::board::item_id(clip.id, crate::board::Which::First) },
            "last_frame": { "path": path, "item": crate::board::item_id(clip.id, crate::board::Which::Last) },
        })),
        "layers": clip.layers.iter().map(|layer| layer_json(session, clip, layer)).collect::<Vec<_>>(),
    })
}

fn layer_json(session: &Session, clip: &crate::film::Clip, layer: &crate::film::Layer) -> Value {
    let record = session.active_storyboard();
    let keys: Vec<Value> = layer
        .keys
        .iter()
        .filter(|key| key.frame < clip.length)
        .map(|key| {
            let blank = record.drawing_of(key.drawing).is_none_or(|drawing| drawing.model_is_blank());
            json!({ "frame": key.frame + 1, "blank": blank })
        })
        .collect();
    let marks: Vec<Value> = layer
        .marks
        .iter()
        .map(|mark| json!({ "id": mark.id, "frame": mark.frame + 1, "kind": if mark.comment { "comment" } else { "label" }, "text": mark.text }))
        .collect();
    json!({
        "id": layer.id,
        "name": layer.name,
        "letter": (!layer.folder).then(|| clip.letter(layer.id)),
        "folder": layer.folder,
        "in_folder": layer.parent,
        "color": format!("#{:02x}{:02x}{:02x}", layer.color[0], layer.color[1], layer.color[2]),
        "opacity": layer.opacity,
        "visible": layer.visible,
        "locked": layer.locked,
        "outline": layer.outline,
        "open": layer.open,
        "keys": keys,
        "marks": marks,
    })
}

fn number(input: &Value, key: &str) -> Result<i64, String> {
    input.get(key).and_then(Value::as_i64).ok_or_else(|| format!("{key} is missing or not a whole number"))
}

fn id(input: &Value, key: &str) -> Result<u64, String> {
    input.get(key).and_then(Value::as_u64).ok_or_else(|| format!("{key} is missing or not an id"))
}

/// A frame from 1, as 0-based.
fn frame(input: &Value, key: &str) -> Result<u32, String> {
    let value = number(input, key)?;
    if value < 1 {
        return Err(format!("{key} counts from 1"));
    }
    Ok((value - 1) as u32)
}

fn flag(input: &Value, key: &str) -> Option<bool> {
    input.get(key).and_then(Value::as_bool)
}

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn ids(input: &Value, key: &str) -> Option<Vec<u64>> {
    Some(input.get(key)?.as_array()?.iter().filter_map(Value::as_u64).collect())
}

fn in_clip(session: &Session) -> Result<(), String> {
    match session.open_clip() {
        Some(_) => Ok(()),
        None => Err("no clip is open; open one with edit_clip".to_string()),
    }
}

fn layers(session: &Session, input: &Value) -> Vec<u64> {
    ids(input, "layers").filter(|layers| !layers.is_empty()).unwrap_or_else(|| session.acting_layers())
}

fn hex(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn changed(done: bool, what: &str) -> Result<(), String> {
    if done { Ok(()) } else { Err(format!("nothing changed: {what}")) }
}

/// Runs one film tool. The result is the film as it is afterwards.
pub fn run(session: &mut Session, tool: &str, input: &Value) -> Outcome {
    let before: Film = session.film().clone();
    let result: Result<Value, String> = (|| {
        let mut made = json!(null);
        match tool {
            "read_film" => return Ok(describe(session)),
            "add_clip" => {
                let start = match input.get("start") {
                    Some(_) => Some(frame(input, "start")?),
                    None => None,
                };
                let length = input.get("length").and_then(Value::as_u64).map_or(NEW_CLIP_LENGTH, |length| length.max(1) as u32);
                made = json!(session.add_clip(start, length).ok_or("the clip could not be added")?);
            }
            "duplicate_clip" => {
                made = json!(session.duplicate_clip(id(input, "clip")?).ok_or("no such clip")?);
            }
            "delete_clip" => changed(session.delete_clip(id(input, "clip")?), "no such clip")?,
            "retime_clip" => {
                let end = match text(input, "edge") {
                    Some("end") => true,
                    Some("start") => false,
                    _ => return Err("edge is start or end".to_string()),
                };
                changed(session.resize_clip(id(input, "clip")?, end, number(input, "frames")?), "the clip is already that length or the edge cannot move")?;
            }
            "move_clips" => {
                let clips = ids(input, "clips").ok_or("clips is missing")?;
                changed(session.move_clips(&clips, number(input, "frames")?, flag(input, "push").unwrap_or(true)), "no clip moved")?;
            }
            "set_clip" => {
                let clip = id(input, "clip")?;
                if session.film().clip(clip).is_none() {
                    return Err("no such clip".to_string());
                }
                for (key, which) in [
                    ("name", ClipText::Name),
                    ("shot", ClipText::Shot),
                    ("action", ClipText::Action),
                    ("dialogue", ClipText::Dialogue),
                    ("camera", ClipText::Camera),
                    ("animator", ClipText::Animator),
                ] {
                    if let Some(value) = text(input, key) {
                        session.set_clip_text(clip, which, value);
                    }
                }
            }
            "select_clips" => session.select_clips(ids(input, "clips").ok_or("clips is missing")?),
            "edit_clip" => changed(session.enter_clip(id(input, "clip")?), "no such clip")?,
            "leave_clip" => changed(session.leave_clip(), "no clip is open")?,
            "go_to_frame" => {
                let at = frame(input, "frame")?;
                if session.open_clip().is_some() {
                    session.seek_clip(at);
                } else {
                    session.seek_film(at);
                }
            }
            "select_layer" => {
                in_clip(session)?;
                changed(session.select_layer(id(input, "layer")?), "no such layer in the open clip")?;
            }
            "select_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                session.select_cells(layers, frame(input, "start")?, frame(input, "end")?);
            }
            "add_layer" => {
                in_clip(session)?;
                made = json!(session.add_layer(flag(input, "folder").unwrap_or(false)).ok_or("the layer could not be added")?);
            }
            "delete_layer" => {
                in_clip(session)?;
                changed(session.delete_layer(id(input, "layer")?), "no such layer, or it is the clip's last")?;
            }
            "move_layer" => {
                in_clip(session)?;
                let folder = input.get("folder").and_then(Value::as_u64);
                changed(session.move_layer(id(input, "layer")?, number(input, "position")?.max(0) as usize, folder), "the layer is already there, or that is not a folder")?;
            }
            "set_layer" => {
                in_clip(session)?;
                let layer = id(input, "layer")?;
                if let Some(name) = text(input, "name") {
                    session.rename_layer(layer, name);
                }
                if let Some(color) = text(input, "color") {
                    session.set_layer_color(layer, hex(color).ok_or("color is #rrggbb")?);
                }
                if let Some(opacity) = input.get("opacity").and_then(Value::as_f64) {
                    session.set_layer_opacity(layer, opacity);
                }
                for (key, which) in [
                    ("visible", LayerFlag::Visible),
                    ("locked", LayerFlag::Locked),
                    ("outline", LayerFlag::Outline),
                    ("open", LayerFlag::Open),
                ] {
                    if let Some(on) = flag(input, key) {
                        session.set_layer_flag(layer, which, on);
                    }
                }
            }
            "insert_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                let count = input.get("count").and_then(Value::as_u64).unwrap_or(1).max(1) as u32;
                changed(session.insert_frames(&layers, frame(input, "frame")?, count), "no layer")?;
            }
            "remove_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                let count = input.get("count").and_then(Value::as_u64).unwrap_or(1).max(1) as u32;
                changed(session.remove_frames(&layers, frame(input, "frame")?, count), "those frames are not in the clip")?;
            }
            "insert_keyframe" => {
                in_clip(session)?;
                changed(
                    session.insert_key(id(input, "layer")?, frame(input, "frame")?, flag(input, "blank").unwrap_or(false)),
                    "that frame is already a keyframe, or the layer is a folder",
                )?;
            }
            "clear_keyframe" => {
                in_clip(session)?;
                changed(session.clear_key(id(input, "layer")?, frame(input, "frame")?), "that frame is not a keyframe")?;
            }
            "move_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                let (start, end) = (frame(input, "start")?, frame(input, "end")?);
                changed(session.move_frames(&layers, start.min(end), start.max(end) + 1, number(input, "frames")?, flag(input, "push").unwrap_or(false)), "nothing moved")?;
            }
            "stretch_hold" => {
                in_clip(session)?;
                changed(session.stretch_span(id(input, "layer")?, frame(input, "frame")?, number(input, "frames")?), "that frame is not a keyframe, or the hold cannot change so")?;
            }
            "copy_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                changed(session.copy_frames(&layers, frame(input, "start")?, frame(input, "end")?), "nothing to copy")?;
                return Ok(json!({ "copied": true }));
            }
            "paste_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                changed(session.paste_frames(&layers, frame(input, "frame")?), "nothing has been copied")?;
            }
            "reverse_frames" => {
                in_clip(session)?;
                let layers = layers(session, input);
                changed(session.reverse_frames(&layers, frame(input, "start")?, frame(input, "end")?), "those frames hold one drawing")?;
            }
            "add_mark" => {
                in_clip(session)?;
                let comment = text(input, "kind") == Some("comment");
                made = json!(session.add_mark(id(input, "layer")?, frame(input, "frame")?, comment, text(input, "text").unwrap_or_default()).ok_or("the mark could not be added")?);
            }
            "remove_mark" => {
                in_clip(session)?;
                changed(session.remove_mark(id(input, "layer")?, id(input, "mark")?), "no such mark")?;
            }
            "set_onion_skin" => {
                session.set_onion(|onion| {
                    if let Some(on) = flag(input, "on") {
                        onion.on = on;
                    }
                    if let Some(before) = input.get("before").and_then(Value::as_u64) {
                        onion.before = before.min(9) as u8;
                    }
                    if let Some(after) = input.get("after").and_then(Value::as_u64) {
                        onion.after = after.min(9) as u8;
                    }
                    if let Some(opacity) = input.get("opacity").and_then(Value::as_f64) {
                        onion.opacity = opacity;
                    }
                    if let Some(fade) = input.get("fade").and_then(Value::as_f64) {
                        onion.fade = fade;
                    }
                    if let Some(keys_only) = flag(input, "keys_only") {
                        onion.keys_only = keys_only;
                    }
                });
            }
            "set_storyboard" => {
                let mut size = session.film().size;
                if let Some(width) = input.get("width").and_then(Value::as_u64) {
                    size[0] = width as u32;
                }
                if let Some(height) = input.get("height").and_then(Value::as_u64) {
                    size[1] = height as u32;
                }
                let paper = input.get("paper").and_then(Value::as_u64).map(|paper| paper as u8);
                session.set_film_settings(text(input, "title"), text(input, "scene"), Some(size), paper);
            }
            _ => return Err(format!("Storyboard Generator has no tool clips__{tool}")),
        }
        let mut output = describe(session);
        if !made.is_null() {
            output["made"] = made;
        }
        output["changed"] = json!(*session.film() != before);
        Ok(output)
    })();
    match result {
        Ok(output) => Outcome { ok: true, output, images: Vec::new() },
        Err(reason) => Outcome { ok: false, output: json!({ "error": reason }), images: Vec::new() },
    }
}
