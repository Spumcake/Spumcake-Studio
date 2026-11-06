//! Storyboard Generator's tool groups for the assistant: everything a person does to a
//! storyboard's contents by hand, as tools. They are built from the hand
//! surface, not from a list of shapes: the brush, eraser, shape, text, and
//! select gestures; the commands that change contents; every property the
//! panel and outliner edit on objects, text, and palette colors; and the
//! select tool's selection. Each tool runs the session's own path for that
//! hand action (see `session/assist.rs`), so its undo, document, and save
//! are a hand edit's. Undo and redo stay with the person.

use crate::look::{self, Looker};
use crate::registry::{CommandId, MenuAction};
use crate::session::Session;
use base64::Engine as _;
use draw::document::{ColorId, NodeId, Point, Sample, ShapeKind};
use draw::{PropId, PropValue};
use kurbo::Affine;
use protocol::{ToolGroup, Tool};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// What a tool did: `ok` false when nothing changed, and `output` says why.
pub struct Outcome {
    pub ok: bool,
    pub output: Value,
    pub images: Vec<String>,
}

fn failed(reason: impl Into<String>) -> Outcome {
    Outcome {
        ok: false,
        output: json!({ "error": reason.into() }),
        images: Vec::new(),
    }
}

/// The surface a tool works on.
pub struct Desk<'a> {
    pub session: &'a mut Session,
    pub looker: &'a mut Looker,
    /// The host's contents area, once known.
    pub viewport: Option<[f64; 2]>,
}

// ---------------------------------------------------------------------------
// The declaration.

const COORDINATES: &str = "Coordinates are document units, x to the right and y down, the same units the storyboard is drawn in at 100% zoom; the visible area is reported by read_storyboard. Ids are the document's own numbers: objects (canvases, shapes, text, groups), palette colors (a canvas's layers, a shape's or text's colors, and the brush's colors), and strokes. Ids stay the same while the person works, but an object made, deleted, or pasted changes the set, so read again when unsure.";

fn points_schema(pressure: bool) -> Value {
    let description = if pressure {
        "The path, in order, in document coordinates: each point [x, y] or [x, y, pressure], pressure from 0 to 1. Use points a few units apart; at least 2."
    } else {
        "The path, in order, in document coordinates: each point [x, y]. At least 2."
    };
    json!({
        "type": "array",
        "minItems": 2,
        "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 3},
        "description": description,
    })
}

fn objects_schema() -> Value {
    json!({
        "type": "array",
        "items": {"type": "integer"},
        "description": "Object ids to act on. They become the selection, as clicking them would. Leave out to act on the current selection, which may be what the person selected by hand.",
    })
}

fn point_schema(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": description})
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn tool(name: &str, description: &str, input: Value) -> Tool {
    Tool {
        name: name.to_string(),
        description: description.to_string(),
        input,
        implements: Vec::new(),
    }
}

/// Object properties as the panel's object page and the outliner edit them,
/// by their keys, with each value's form.
pub const OBJECT_PROPS: &[(PropId, &str)] = &[
    (PropId::Name, "text; canvases and groups only, as the outliner renames them"),
    (PropId::Visible, "true or false: shown or hidden, as the outliner's eye"),
    (PropId::Locked, "true or false: locked objects cannot be changed, as the outliner's lock"),
    (PropId::Padding, "number 0 to 128: a canvas's padding around its ink"),
    (PropId::CornerRadius, "number 0 to 64: a rectangle's corner radius"),
    (PropId::Family, "\"inter\" or \"monospace\": text's font"),
    (PropId::FontSize, "number 8 to 144: text's size"),
    (PropId::Bold, "true or false: text"),
    (PropId::Italic, "true or false: text"),
    (PropId::Underline, "true or false: text"),
    (PropId::Align, "\"left\", \"center\", \"right\", or \"justify\": text"),
    (PropId::VAlign, "\"top\", \"middle\", or \"bottom\": text in a box taller than its lines"),
    (PropId::List, "\"none\", \"bullets\", \"numbers\", or \"checklist\": every line of a text as a list item"),
    (PropId::LineHeight, "number 0.8 to 3: text's line height, times its size"),
    (PropId::LetterSpacing, "number -5 to 20: text's letter spacing"),
    (PropId::TextPadding, "number 0 to 48: space between a text box's edge and its words"),
    (PropId::TextRadius, "number 0 to 48: a text box's corner radius"),
    (PropId::AllCaps, "true or false: text shown in capitals"),
    (PropId::AutoSize, "true or false: the text box sizes itself to its words"),
    (PropId::WrapText, "true or false: words wrap at the box's width"),
];

/// Palette color properties, as a color's page and the outliner edit them.
pub const COLOR_PROPS: &[(PropId, &str)] = &[
    (PropId::Rgb, "\"#rrggbb\" or [r, g, b] from 0 to 255: the color"),
    (PropId::Hex, "\"#rrggbb\": the color, as the page's Hex field"),
    (PropId::Name, "text: a canvas layer's name; shape and text colors are named by their role"),
    (PropId::Opacity, "number 0 to 1"),
    (PropId::ColorSize, "number 1 to 32: line width, for colors that draw a line (layers, outlines, line strokes, text borders)"),
    (PropId::Dash, "\"solid\" or \"dashed\": for colors that draw a line"),
    (PropId::Cap, "\"round\" or \"flat\": line ends, for layers and line strokes"),
    (PropId::Visible, "true or false: whether the color is drawn; for a fill, text background, or text border, whether there is one"),
    (PropId::Locked, "true or false: a canvas layer only; a locked layer takes no new ink or erasing"),
];

fn props_schema(props: &[(PropId, &str)]) -> Value {
    let mut properties = Map::new();
    for (prop, description) in props {
        properties.insert(prop.key().to_string(), json!({ "description": description }));
    }
    json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
        "minProperties": 1,
        "description": "Property keys and their new values.",
    })
}

/// Commands that change contents, by the names the tools take them under.
pub const ARRANGE: &[(CommandId, &str)] = &[
    (CommandId::BringForward, "bring_forward"),
    (CommandId::SendBackward, "send_backward"),
    (CommandId::BringToFront, "bring_to_front"),
    (CommandId::SendToBack, "send_to_back"),
];
pub const ALIGN: &[(CommandId, &str)] = &[
    (CommandId::AlignLeft, "left"),
    (CommandId::AlignCenter, "center"),
    (CommandId::AlignRight, "right"),
];
pub const TURN: &[(CommandId, &str)] = &[
    (CommandId::RotateClockwise, "rotate_clockwise"),
    (CommandId::RotateCounterclockwise, "rotate_counterclockwise"),
    (CommandId::RotateHalf, "rotate_half"),
    (CommandId::FlipHorizontal, "flip_horizontal"),
    (CommandId::FlipVertical, "flip_vertical"),
];
pub const CLIPBOARD: &[(CommandId, &str)] = &[
    (CommandId::Cut, "cut"),
    (CommandId::Copy, "copy"),
    (CommandId::Paste, "paste"),
];
pub const NUDGE: &[(CommandId, &str)] = &[
    (CommandId::NudgeLeft, "left"),
    (CommandId::NudgeRight, "right"),
    (CommandId::NudgeUp, "up"),
    (CommandId::NudgeDown, "down"),
];

fn names(table: &[(CommandId, &'static str)]) -> Vec<&'static str> {
    table.iter().map(|(_, name)| *name).collect()
}

/// Every tool group Storyboard Generator offers, as `tools` declares them.
pub fn tool_groups() -> Vec<ToolGroup> {
    vec![
        ToolGroup {
            id: "look".into(),
            name: "Read and look at a storyboard".into(),
            description: "Read what a storyboard holds, as data, and look at an image of it as it is drawn.".into(),
            instructions: format!(
                "Read before you change anything: the person may have changed the storyboard by hand since you last read it, so positions and ids you saw earlier may be stale. read_storyboard lists the canvases and the active one, each canvas's color layers and their freehand strokes, every object front to back, the selection, the brush's colors, and the visible area. A canvas holds freehand ink: strokes inside color layers, and a stroke's color, opacity, width, dash, and cap belong to its layer. Shapes (rectangles, ellipses, lines), text, and groups are objects beside the canvases, each with its own palette of colors by role (outline and fill; a line's stroke; text, background, and border). Freehand drawings are only strokes in boxes as data, so call look_at to see them, for example to tell what a drawing is or which strokes are a cat's ears; use its scale and region to turn image pixels into document coordinates. {COORDINATES}"
            ),
            tools: vec![
                tool(
                    "read_storyboard",
                    "Everything the storyboard holds, as JSON: canvases with their layers and strokes (ids, bounds, simplified points), objects front to back with their kind, name, bounds, transform, properties, palette, and words, the selection, the brush's colors, and the visible area.",
                    object(
                        json!({
                            "canvas": {"type": "integer", "description": "Give the strokes of this canvas only. Leave out for every canvas."},
                            "points": {"type": "boolean", "description": "Include each stroke's simplified points. Default true."},
                        }),
                        &[],
                    ),
                ),
                tool(
                    "look_at",
                    "A PNG image of the storyboard as it is drawn: everything visible in a region, or around one canvas or object, or the whole storyboard. The result says the region and the scale in pixels per unit.",
                    object(
                        json!({
                            "object": {"type": "integer", "description": "Show this canvas or object, with a margin. Leave out, with no region, for everything."},
                            "region": object(json!({"min": point_schema("Top-left corner [x, y]."), "max": point_schema("Bottom-right corner [x, y].")}), &["min", "max"]),
                            "visible_area": {"type": "boolean", "description": "Show what the person sees on screen now."},
                            "size": {"type": "integer", "minimum": 64, "maximum": 1536, "description": "The image's longer side in pixels. Default 1024."},
                        }),
                        &[],
                    ),
                ),
            ],
        },
        ToolGroup {
            id: "draw".into(),
            name: "Draw by hand".into(),
            description: "Draw as the person draws with the tools: freehand strokes, erasing, rectangles, ellipses, lines, text, and moving, scaling, or rotating objects.".into(),
            instructions: format!(
                "Each tool is one gesture of the person's own tools, made on the open storyboard. The person's tool settings stay as they were. Freehand strokes go on a canvas, into the layer of their color; a color is a layer id of that canvas or one of the brush's colors from read_storyboard, and a brush color the canvas has no layer for adds that layer. Strokes are smoothed and pressure-shaped like the person's own, so give points a few units apart along the path; to fill an area, draw several strokes side by side. Erasing removes ink along a path without deleting strokes. Shapes and text are drawn with the tools' current colors and sizes; change them afterwards with the Restyle tools, using the palette color ids the result gives. Put new work inside the visible area that read_storyboard gives, unless the request says where. Ink's color belongs to its layer, so to recolor only part of a freehand drawing, such as one part of a figure: find those strokes with read_storyboard and look_at, redraw them from their points in the new brush color, make sure that color's layer is above the old one with edit's move_layer (read_storyboard lists layers top to bottom), then erase the old ink along the same path. Each result names what was made. All the changes of one request are taken back by one undo. {COORDINATES}"
            ),
            tools: vec![
                tool(
                    "draw_stroke",
                    "One freehand brush stroke along points, in a palette color, on a canvas. Without a canvas, the highlighted canvas is used; with no canvas highlighted, the stroke makes a new canvas, as it does by hand.",
                    object(
                        json!({
                            "points": points_schema(true),
                            "color": {"type": "integer", "description": "A layer id of the canvas, or a brush color id. Default: the person's current brush color."},
                            "canvas": {"type": "integer", "description": "The canvas to draw on. Default: the highlighted canvas."},
                        }),
                        &["points"],
                    ),
                ),
                tool(
                    "erase",
                    "Erase along points on a canvas, as the eraser does: ink under the path disappears, but every stroke stays, so it can still be restyled.",
                    object(
                        json!({
                            "points": points_schema(false),
                            "size": {"type": "integer", "minimum": 1, "maximum": 32, "description": "The eraser's width. Default: the person's eraser size."},
                            "canvas": {"type": "integer", "description": "The canvas to erase on. Default: the highlighted canvas."},
                        }),
                        &["points"],
                    ),
                ),
                tool(
                    "add_shape",
                    "A rectangle or ellipse filling the box between two corners, or a line from one point to the other, as the shape tools draw them.",
                    object(
                        json!({
                            "kind": {"type": "string", "enum": ["rectangle", "ellipse", "line"]},
                            "from": point_schema("One corner, or the line's start [x, y]."),
                            "to": point_schema("The opposite corner, or the line's end [x, y]."),
                            "constrain": {"type": "boolean", "description": "Make a square or circle, or snap a line to 45 degrees, as Shift does. Default false."},
                            "properties": props_schema(OBJECT_PROPS),
                        }),
                        &["kind", "from", "to"],
                    ),
                ),
                tool(
                    "add_text",
                    "New text, typed with the text tool, its box's top-left corner at a point. Without a width the box fits the words; with one the words wrap. Use \\n for new lines.",
                    object(
                        json!({
                            "text": {"type": "string"},
                            "at": point_schema("The box's top-left corner [x, y]."),
                            "width": {"type": "number", "description": "The box's width, so the words wrap."},
                            "properties": props_schema(OBJECT_PROPS),
                        }),
                        &["text", "at"],
                    ),
                ),
                tool(
                    "transform",
                    "Move, scale, or rotate objects, as dragging them or their handles does. Scale and rotation are about the objects' center, or about a point. Text is never rotated.",
                    object(
                        json!({
                            "objects": objects_schema(),
                            "move": point_schema("Distance to move [dx, dy]."),
                            "scale": {"description": "A factor, or [x factor, y factor]; negative mirrors."},
                            "rotate": {"type": "number", "description": "Degrees, clockwise on screen."},
                            "about": point_schema("The point to scale and rotate about [x, y]. Default: the objects' center."),
                        }),
                        &[],
                    ),
                ),
                tool(
                    "set_text_box",
                    "Set a text box's width or height, as dragging its side handles does; the words re-wrap.",
                    object(
                        json!({
                            "object": {"type": "integer"},
                            "width": {"type": "number"},
                            "height": {"type": "number"},
                        }),
                        &["object"],
                    ),
                ),
            ],
        },
        ToolGroup {
            id: "edit".into(),
            name: "Edit and arrange".into(),
            description: "Select, cut, copy, paste, delete, nudge, group, ungroup, order, align, rotate, and flip objects; add canvases and choose the one drawn on; and order, duplicate, or delete a canvas's color layers.".into(),
            instructions: format!(
                "These are the person's own commands. Most act on objects: pass their ids, which become the selection as clicking them would, or leave them out to act on what is selected now, which is how to act on what the person selected by hand (\"these\", \"this\"). The selection stays afterwards, as it does by hand. Objects are listed front to back; position 0 is the front. A canvas's layers are listed top to bottom; position 0 paints on top. Pasting places copies a little below and right of the originals and selects them. A command that cannot apply, such as grouping one object, changes nothing and says why. {COORDINATES}"
            ),
            tools: vec![
                tool(
                    "select",
                    "Change the selection as the select tool does: replace it, add or remove objects as Shift-clicking does, take everything an area touches as a drag does, or clear it.",
                    object(
                        json!({
                            "mode": {"type": "string", "enum": ["replace", "add", "remove", "clear"]},
                            "objects": {"type": "array", "items": {"type": "integer"}},
                            "area": object(json!({"min": point_schema("Top-left [x, y]."), "max": point_schema("Bottom-right [x, y].")}), &["min", "max"]),
                        }),
                        &["mode"],
                    ),
                ),
                tool(
                    "clipboard",
                    "Cut, copy, or paste whole objects, within Storyboard Generator.",
                    object(
                        json!({"action": {"type": "string", "enum": names(CLIPBOARD)}, "objects": objects_schema()}),
                        &["action"],
                    ),
                ),
                tool("delete", "Delete objects, as the Delete key does.", object(json!({"objects": objects_schema()}), &[])),
                tool(
                    "nudge",
                    "Nudge objects one unit, or eight with large, per step, as the arrow keys do.",
                    object(
                        json!({
                            "direction": {"type": "string", "enum": names(NUDGE)},
                            "steps": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Default 1."},
                            "large": {"type": "boolean", "description": "Eight units per step, as Shift does."},
                            "objects": objects_schema(),
                        }),
                        &["direction"],
                    ),
                ),
                tool("group", "Group two or more objects into one.", object(json!({"objects": objects_schema()}), &[])),
                tool("ungroup", "Release the objects of the selected groups.", object(json!({"objects": objects_schema()}), &[])),
                tool(
                    "arrange",
                    "Bring objects forward or to the front, or send them backward or to the back.",
                    object(json!({"order": {"type": "string", "enum": names(ARRANGE)}, "objects": objects_schema()}), &["order"]),
                ),
                tool(
                    "align",
                    "Align two or more objects' left edges, centers, or right edges.",
                    object(json!({"edge": {"type": "string", "enum": names(ALIGN)}, "objects": objects_schema()}), &["edge"]),
                ),
                tool(
                    "turn",
                    "Rotate objects a quarter or half turn, or flip them, about their center. Text never turns.",
                    object(json!({"turn": {"type": "string", "enum": names(TURN)}, "objects": objects_schema()}), &["turn"]),
                ),
                tool(
                    "move_object",
                    "Move an object to a position among its siblings, as dragging its row in the layers list does. 0 is the front.",
                    object(json!({"object": {"type": "integer"}, "position": {"type": "integer", "minimum": 0}}), &["object", "position"]),
                ),
                tool("new_canvas", "Add a new empty canvas at the front, and make it the one drawn on.", object(json!({}), &[])),
                tool(
                    "set_active_canvas",
                    "Make a canvas the one strokes and erasing go on, as clicking it does.",
                    object(json!({"canvas": {"type": "integer"}}), &["canvas"]),
                ),
                tool(
                    "duplicate_layer",
                    "Copy a canvas's color layer, with its strokes, below it.",
                    object(json!({"canvas": {"type": "integer"}, "layer": {"type": "integer"}}), &["canvas", "layer"]),
                ),
                tool(
                    "delete_layer",
                    "Delete a canvas's color layer and its strokes. A canvas keeps at least one layer.",
                    object(json!({"canvas": {"type": "integer"}, "layer": {"type": "integer"}}), &["canvas", "layer"]),
                ),
                tool(
                    "move_layer",
                    "Move a canvas's color layer to a position; 0 paints on top.",
                    object(
                        json!({"canvas": {"type": "integer"}, "layer": {"type": "integer"}, "position": {"type": "integer", "minimum": 0}}),
                        &["canvas", "layer", "position"],
                    ),
                ),
            ],
        },
        ToolGroup {
            id: "style".into(),
            name: "Restyle".into(),
            description: "Change the properties of objects, text, and palette colors, as the properties panel and the layers list do, and change a text's words.".into(),
            instructions: format!(
                "Objects have their own properties: a canvas's padding, a rectangle's corner radius, text's font, size, alignment, and layout, and every object's visibility and lock. Colors are separate: a shape's outline and fill, a line's stroke, a text's text, background, and border, and each of a canvas's layers are palette colors with their own ids, from read_storyboard or the result that made them. To make a circle pink, set the rgb of its outline or fill color; to make freehand ink dashed or wider, set its layer's dash or color_size, which changes every stroke in that layer. To recolor only some strokes, redraw them in another color with the Draw by hand tools instead, as its instructions say; setting the layer's rgb recolors every stroke in it. A fill, a text background, or a text border is turned on with its color's visible. Leave objects out to change the selection, such as what the person selected by hand. A locked object must be unlocked before anything else of it changes. {COORDINATES}"
            ),
            tools: vec![
                tool(
                    "set_object_properties",
                    "Set properties of objects. Each object takes the properties it has; the result says which took each one.",
                    object(json!({"objects": objects_schema(), "properties": props_schema(OBJECT_PROPS)}), &["properties"]),
                ),
                tool(
                    "set_color_properties",
                    "Set properties of one palette color of an object: a canvas's layer, or a shape's or text's color.",
                    object(
                        json!({
                            "object": {"type": "integer", "description": "The canvas, shape, or text that owns the color."},
                            "color": {"type": "integer", "description": "The palette color's id."},
                            "properties": props_schema(COLOR_PROPS),
                        }),
                        &["object", "color", "properties"],
                    ),
                ),
                tool(
                    "set_text",
                    "Replace a text object's words, as editing it in place does. Empty words delete it.",
                    object(json!({"object": {"type": "integer"}, "text": {"type": "string"}}), &["object", "text"]),
                ),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Carrying out one call.

impl Desk<'_> {
    /// Runs `group`'s `tool` with `input` on the active storyboard.
    pub fn run(&mut self, group: &str, tool: &str, input: &Value) -> Outcome {
        let before = self.snapshot();
        let selection_before = self.session.selection();
        let done = match (group, tool) {
            ("look", "read_storyboard") => return self.read(input),
            // Its stored spelling in conversations kept before 0.14.
            ("look", "read_sketch") => return self.read(input), // The stored spelling.
            ("look", "look_at") => return self.look(input),
            ("draw", "draw_stroke") => self.stroke(input),
            ("draw", "erase") => self.erase(input),
            ("draw", "add_shape") => self.shape(input),
            ("draw", "add_text") => self.text(input),
            ("draw", "transform") => self.transform(input),
            ("draw", "set_text_box") => self.text_box(input),
            ("edit", "select") => return self.select(input, selection_before),
            ("edit", "clipboard") if input.get("action").and_then(Value::as_str) == Some("copy") => {
                // Copying changes no contents: it is done when it could be.
                return match self.command(input, CommandId::Copy, false, 1) {
                    Ok(()) => Outcome {
                        ok: true,
                        output: json!({ "copied": ids(&self.session.selection()) }),
                        images: Vec::new(),
                    },
                    Err(reason) => failed(reason),
                };
            }
            ("edit", "clipboard") => self.choice(input, "action", CLIPBOARD, false),
            ("edit", "delete") => self.command(input, CommandId::Delete, false, 1),
            ("edit", "nudge") => self.nudge(input),
            ("edit", "group") => self.command(input, CommandId::Group, false, 1),
            ("edit", "ungroup") => self.command(input, CommandId::Ungroup, false, 1),
            ("edit", "arrange") => self.choice(input, "order", ARRANGE, false),
            ("edit", "align") => self.choice(input, "edge", ALIGN, false),
            ("edit", "turn") => self.choice(input, "turn", TURN, false),
            ("edit", "move_object") => (|| {
                let id = node(input, "object")?;
                let to = count(input, "position")?;
                self.session.assist_reorder(id, to)
            })(),
            ("edit", "new_canvas") => {
                let viewport = self.viewport.unwrap_or([0.0, 0.0]);
                self.session.assist_command(None, CommandId::NewCanvas, false, viewport)
            }
            ("edit", "set_active_canvas") => {
                return match node(input, "canvas").and_then(|id| self.session.assist_highlight(id)) {
                    Ok(()) => Outcome {
                        ok: true,
                        output: json!({ "active_canvas": self.session.highlighted_id().map(NodeId::get) }),
                        images: Vec::new(),
                    },
                    Err(reason) => failed(reason),
                };
            }
            ("edit", "duplicate_layer") => self.layer(input, MenuAction::Duplicate),
            ("edit", "delete_layer") => self.layer(input, MenuAction::Delete),
            ("edit", "move_layer") => (|| {
                let canvas = node(input, "canvas")?;
                let layer = color(input, "layer")?;
                let to = count(input, "position")?;
                self.session.assist_move_layer(canvas, layer, to)
            })(),
            ("style", "set_object_properties") => {
                let outcome = self.object_props(input, &before);
                if !outcome.ok && self.session.selection() != selection_before {
                    self.session.replace_selection(selection_before);
                }
                return outcome;
            }
            ("style", "set_color_properties") => return self.color_props(input, &before),
            ("style", "set_text") => (|| {
                let id = node(input, "object")?;
                let words = text(input, "text")?;
                self.session.assist_set_text(id, words)
            })(),
            _ => return failed(format!("Storyboard Generator has no tool {group}__{tool}")),
        };
        match done {
            Ok(()) => self.changes(&before, None),
            Err(reason) => {
                // A refused call leaves the selection as it found it.
                if self.session.selection() != selection_before {
                    self.session.replace_selection(selection_before);
                }
                failed(reason)
            }
        }
    }

    fn snapshot(&self) -> Value {
        let record = self.session.active_storyboard();
        let mut value = record.drawing.snapshot_value(record.id(), record.name());
        if let Some(object) = value.as_object_mut() {
            object.remove("camera");
        }
        value
    }

    /// What a change made and removed, or why nothing changed.
    fn changes(&self, before: &Value, extra: Option<Value>) -> Outcome {
        let after = self.snapshot();
        if &after == before {
            return failed("nothing changed");
        }
        let was = inventory(before);
        let now = inventory(&after);
        let mut output = Map::new();
        let made: Vec<Value> = now
            .objects
            .iter()
            .filter(|id| !was.objects.contains(id))
            .filter_map(|id| self.describe(NodeId::new(*id), true))
            .collect();
        if !made.is_empty() {
            output.insert("made".into(), Value::Array(made));
        }
        let removed: Vec<u64> = was.objects.difference(&now.objects).copied().collect();
        if !removed.is_empty() {
            output.insert("removed_objects".into(), json!(removed));
        }
        let layers: Vec<u64> = now.layers.difference(&was.layers).copied().collect();
        if !layers.is_empty() {
            output.insert("new_layers".into(), json!(layers));
        }
        let strokes: Vec<u64> = now.strokes.difference(&was.strokes).copied().collect();
        if !strokes.is_empty() {
            output.insert("new_strokes".into(), json!(strokes));
        }
        let erased: Vec<u64> = now
            .erases
            .iter()
            .filter(|(id, count)| was.erases.get(id).is_some_and(|before| before < count))
            .map(|(id, _)| *id)
            .collect();
        if !erased.is_empty() {
            output.insert("erased_strokes".into(), json!(erased));
        }
        let gone: Vec<u64> = was.strokes.difference(&now.strokes).copied().collect();
        if !gone.is_empty() {
            output.insert("removed_strokes".into(), json!(gone));
        }
        if let Some(Value::Object(extra)) = extra {
            output.extend(extra);
        }
        output.insert("selection".into(), json!(ids(&self.session.selection())));
        output.insert(
            "active_canvas".into(),
            json!(self.session.highlighted_id().map(NodeId::get)),
        );
        Outcome {
            ok: true,
            output: Value::Object(output),
            images: Vec::new(),
        }
    }

    fn stroke(&mut self, input: &Value) -> Result<(), String> {
        let samples = samples(input, true)?;
        let canvas = optional_node(input, "canvas")?;
        let color = optional(input, "color", |value| value.as_u64().filter(|id| *id != 0).map(ColorId::new))?;
        self.session.assist_stroke(canvas, color, samples)
    }

    fn erase(&mut self, input: &Value) -> Result<(), String> {
        let samples = samples(input, false)?;
        let canvas = optional_node(input, "canvas")?;
        let size = optional(input, "size", |value| value.as_u64().map(|size| size.min(255) as u8))?;
        self.session.assist_erase(canvas, size, samples)
    }

    fn shape(&mut self, input: &Value) -> Result<(), String> {
        let kind = match text(input, "kind")? {
            "rectangle" => ShapeKind::Rectangle,
            "ellipse" => ShapeKind::Ellipse,
            "line" => ShapeKind::Line,
            other => return Err(format!("{other:?} is not rectangle, ellipse, or line")),
        };
        let from = point(input, "from")?;
        let to = point(input, "to")?;
        let constrain = input.get("constrain").and_then(Value::as_bool).unwrap_or(false);
        let props = object_props(input.get("properties"))?;
        let before = self.object_ids();
        self.session.assist_shape(kind, from, to, constrain)?;
        self.style_made(&before, &props)
    }

    fn text(&mut self, input: &Value) -> Result<(), String> {
        let words = text(input, "text")?;
        let at = point(input, "at")?;
        let width = optional(input, "width", Value::as_f64)?;
        let props = object_props(input.get("properties"))?;
        let before = self.object_ids();
        self.session.assist_text(at, width, words)?;
        self.style_made(&before, &props)
    }

    fn object_ids(&self) -> BTreeSet<u64> {
        self.session.active_storyboard().layers().into_iter().map(|row| row.id.get()).collect()
    }

    /// Properties given with a new shape or text, set on it as the panel would.
    fn style_made(&mut self, before: &BTreeSet<u64>, props: &[(PropId, PropValue)]) -> Result<(), String> {
        if props.is_empty() {
            return Ok(());
        }
        let made: Vec<NodeId> = self.object_ids().difference(before).map(|id| NodeId::new(*id)).collect();
        self.session.assist_object_props(&made, props);
        Ok(())
    }

    fn transform(&mut self, input: &Value) -> Result<(), String> {
        let objects = optional_nodes(input)?;
        let shift = optional(input, "move", |value| pair(value))?.unwrap_or([0.0, 0.0]);
        let scale = match input.get("scale") {
            None | Some(Value::Null) => [1.0, 1.0],
            Some(Value::Number(factor)) => {
                let factor = factor.as_f64().unwrap_or(1.0);
                [factor, factor]
            }
            Some(value) => pair(value).ok_or("scale is a number or [x, y]")?,
        };
        let degrees = optional(input, "rotate", Value::as_f64)?.unwrap_or(0.0);
        let about = optional(input, "about", |value| pair(value))?;
        if [shift[0], shift[1], scale[0], scale[1], degrees].iter().any(|n| !n.is_finite())
            || scale[0] == 0.0
            || scale[1] == 0.0
        {
            return Err("move, scale, and rotate must be finite, and a scale not zero".into());
        }
        let turns = degrees != 0.0 || scale[0] < 0.0 || scale[1] < 0.0;
        self.session.assist_transform(
            objects.as_deref(),
            |center| {
                let pivot = about.unwrap_or(center);
                let pivot = kurbo::Vec2::new(pivot[0], pivot[1]);
                Affine::translate(kurbo::Vec2::new(shift[0], shift[1]))
                    * Affine::translate(pivot)
                    * Affine::rotate(degrees.to_radians())
                    * Affine::scale_non_uniform(scale[0], scale[1])
                    * Affine::translate(-pivot)
            },
            turns,
        )
    }

    fn text_box(&mut self, input: &Value) -> Result<(), String> {
        let id = node(input, "object")?;
        let width = optional(input, "width", Value::as_f64)?;
        let height = optional(input, "height", Value::as_f64)?;
        if width.is_none() && height.is_none() {
            return Err("give a width, a height, or both".into());
        }
        self.session.assist_text_box(id, width, height)
    }

    fn command(&mut self, input: &Value, command: CommandId, shift: bool, times: usize) -> Result<(), String> {
        let objects = optional_nodes(input)?;
        let viewport = self.viewport.unwrap_or([0.0, 0.0]);
        self.session.assist_command(objects.as_deref(), command, shift, viewport)?;
        for _ in 1..times {
            self.session.assist_command(None, command, shift, viewport)?;
        }
        Ok(())
    }

    fn choice(&mut self, input: &Value, key: &str, table: &[(CommandId, &str)], shift: bool) -> Result<(), String> {
        let name = text(input, key)?;
        let Some((command, _)) = table.iter().find(|(_, option)| *option == name) else {
            return Err(format!("{key} {name:?} is not one of {:?}", table.iter().map(|(_, n)| *n).collect::<Vec<_>>()));
        };
        self.command(input, *command, shift, 1)
    }

    fn nudge(&mut self, input: &Value) -> Result<(), String> {
        let direction = text(input, "direction")?;
        let Some((command, _)) = NUDGE.iter().find(|(_, name)| *name == direction) else {
            return Err(format!("{direction:?} is not left, right, up, or down"));
        };
        let steps = optional(input, "steps", Value::as_u64)?.unwrap_or(1).clamp(1, 100) as usize;
        let large = input.get("large").and_then(Value::as_bool).unwrap_or(false);
        self.command(input, *command, large, steps)
    }

    fn layer(&mut self, input: &Value, action: MenuAction) -> Result<(), String> {
        let canvas = node(input, "canvas")?;
        let layer = color(input, "layer")?;
        self.session.assist_layer(canvas, layer, action)
    }

    fn select(&mut self, input: &Value, before: Vec<NodeId>) -> Outcome {
        let result = (|| {
            let mode = text(input, "mode")?;
            let objects = optional(input, "objects", |value| {
                value.as_array().map(|items| items.iter().filter_map(Value::as_u64).filter(|id| *id != 0).map(NodeId::new).collect::<Vec<_>>())
            })?;
            let area = match input.get("area") {
                None | Some(Value::Null) => None,
                Some(area) => Some((
                    area.get("min").and_then(pair).ok_or("area needs min [x, y]")?,
                    area.get("max").and_then(pair).ok_or("area needs max [x, y]")?,
                )),
            };
            if let Some(objects) = &objects
                && let Some(missing) = objects
                    .iter()
                    .find(|id| !self.session.active_storyboard().layers().iter().any(|row| row.id == **id))
            {
                return Err(format!("there is no object {}", missing.get()));
            }
            match mode {
                "clear" => self.session.clear_selection(),
                "replace" => {
                    if let Some((min, max)) = area {
                        self.session.marquee_select(min, max, false, &[]);
                    } else {
                        self.session.replace_selection(objects.unwrap_or_default());
                    }
                }
                "add" | "remove" => {
                    let adding = mode == "add";
                    if let Some((min, max)) = area.filter(|_| adding) {
                        let base = self.session.selection();
                        self.session.marquee_select(min, max, true, &base);
                    }
                    for id in objects.unwrap_or_default() {
                        if self.session.selection().contains(&id) != adding {
                            self.session.select_click(id, true);
                        }
                    }
                }
                other => return Err(format!("{other:?} is not replace, add, remove, or clear")),
            }
            Ok(())
        })();
        match result {
            Err(reason) => failed(reason),
            Ok(()) => {
                let after = self.session.selection();
                Outcome {
                    ok: true,
                    output: json!({
                        "selection": ids(&after),
                        "changed": after != before,
                        "active_canvas": self.session.highlighted_id().map(NodeId::get),
                    }),
                    images: Vec::new(),
                }
            }
        }
    }

    fn object_props(&mut self, input: &Value, before: &Value) -> Outcome {
        let props = match object_props(input.get("properties")) {
            Ok(props) if !props.is_empty() => props,
            Ok(_) => return failed("give at least one property"),
            Err(reason) => return failed(reason),
        };
        let objects = match optional_nodes(input) {
            Ok(Some(objects)) => {
                if let Err(reason) = self.session.assist_select_objects(Some(&objects)) {
                    return failed(reason);
                }
                objects
            }
            Ok(None) => self.session.selection(),
            Err(reason) => return failed(reason),
        };
        if objects.is_empty() {
            return failed("nothing is selected; pass objects, or select them first");
        }
        let took = self.session.assist_object_props(&objects, &props);
        let report: Map<String, Value> = took
            .iter()
            .map(|(prop, nodes)| (prop.key().to_string(), json!(ids(nodes))))
            .collect();
        let mut outcome = self.changes(before, Some(json!({ "took": report })));
        if !outcome.ok {
            outcome.output = json!({
                "error": "nothing changed: the objects already have those values, are locked, or do not have those properties",
                "took": report,
            });
        }
        outcome
    }

    fn color_props(&mut self, input: &Value, before: &Value) -> Outcome {
        let parsed = (|| {
            let owner = node(input, "object")?;
            let id = color(input, "color")?;
            let props = color_props(input.get("properties"))?;
            if props.is_empty() {
                return Err("give at least one property".to_string());
            }
            Ok((owner, id, props))
        })();
        let (owner, id, props) = match parsed {
            Ok(parsed) => parsed,
            Err(reason) => return failed(reason),
        };
        let took = match self.session.assist_color_props(owner, id, &props) {
            Ok(took) => took,
            Err(reason) => return failed(reason),
        };
        let refused: Vec<&str> = took.iter().filter(|(_, ok)| !ok).map(|(prop, _)| prop.key()).collect();
        let color = self
            .session
            .active_storyboard()
            .layers()
            .iter()
            .find(|row| row.id == owner)
            .map(|_| self.color_value(owner, id));
        let mut outcome = self.changes(before, Some(json!({ "refused": refused, "color": color })));
        if !outcome.ok {
            outcome.output = json!({
                "error": "nothing changed: the color already has those values, or does not have those properties",
                "refused": refused,
            });
        }
        outcome
    }

    fn color_value(&self, owner: NodeId, id: ColorId) -> Value {
        let storyboard = self.session.active_storyboard();
        let mut value = Map::new();
        value.insert("id".into(), json!(id.get()));
        for (prop, _) in COLOR_PROPS {
            if let Some(read) = storyboard.color_prop(owner, id, *prop) {
                value.insert(prop.key().into(), friendly(*prop, &read));
            }
        }
        Value::Object(value)
    }

    // -----------------------------------------------------------------------
    // Reading.

    fn read(&mut self, input: &Value) -> Outcome {
        let only = input.get("canvas").and_then(Value::as_u64);
        let points = input.get("points").and_then(Value::as_bool).unwrap_or(true);
        let snapshot = self.snapshot();
        let record = self.session.active_storyboard();
        let mut objects = Vec::new();
        for node in snapshot.get("nodes").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = node.get("id").and_then(Value::as_u64)
                && let Some(value) = self.describe_tree(node, NodeId::new(id), only, points)
            {
                objects.push(value);
            }
        }
        let canvases: Vec<Value> = record
            .layers()
            .into_iter()
            .filter(|row| row.kind == draw::document::LayerKind::Canvas)
            .map(|row| json!({ "id": row.id.get(), "name": row.name }))
            .collect();
        let brush: Vec<Value> = self
            .session
            .brush_colors()
            .into_iter()
            .map(|color| json!({ "id": color.id.get(), "name": color.name, "rgb": hex(color.rgb), "current": color.active }))
            .collect();
        let output = json!({
            "storyboard": record.name(),
            "saved_to": record.path().map(|path| path.display().to_string()),
            "canvases": canvases,
            "active_canvas": record.highlighted_id().map(NodeId::get),
            "selection": ids(record.selection()),
            "visible_area": self.visible_area().map(|(min, max)| json!({ "min": round_point(min), "max": round_point(max), "zoom": record.camera().zoom })),
            "brush_colors": brush,
            "objects_front_to_back": objects,
        });
        Outcome {
            ok: true,
            output,
            images: Vec::new(),
        }
    }

    fn visible_area(&self) -> Option<(Point, Point)> {
        let viewport = self.viewport?;
        let camera = self.session.active_storyboard().camera();
        let zoom = camera.zoom.max(1e-6);
        Some((
            camera.origin,
            [camera.origin[0] + viewport[0] / zoom, camera.origin[1] + viewport[1] / zoom],
        ))
    }

    /// One object of the snapshot, with its children, as the model reads it.
    fn describe_tree(&self, value: &Value, id: NodeId, only: Option<u64>, points: bool) -> Option<Value> {
        let mut described = self.describe(id, false)?;
        let object = described.as_object_mut()?;
        match value.get("kind").and_then(Value::as_str) {
            Some("group") => {
                let children: Vec<Value> = value
                    .get("children")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|child| {
                        let child_id = NodeId::new(child.get("id")?.as_u64()?);
                        self.describe_tree(child, child_id, only, points)
                    })
                    .collect();
                object.insert("children".into(), Value::Array(children));
            }
            Some("canvas") => {
                let world = self.session.active_storyboard().world_transform(id).unwrap_or(Affine::IDENTITY);
                let detailed = only.is_none_or(|only| only == id.get());
                let layers: Vec<Value> = value
                    .get("colors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|layer| self.describe_layer(id, layer, world, detailed, points))
                    .collect();
                object.insert("layers_top_to_bottom".into(), Value::Array(layers));
            }
            _ => {}
        }
        Some(described)
    }

    fn describe_layer(&self, canvas: NodeId, layer: &Value, world: Affine, detailed: bool, points: bool) -> Value {
        let color = layer.get("color").cloned().unwrap_or(Value::Null);
        let id = color.get("id").and_then(Value::as_u64).unwrap_or(0);
        let mut value = self.color_value(canvas, ColorId::new(id)).as_object().cloned().unwrap_or_default();
        let source = layer.get("source").and_then(Value::as_u64);
        let drawable = source.is_some_and(|source| {
            self.session.brush_colors().iter().any(|color| color.id.get() == source)
        });
        value.insert("brush_color".into(), json!(source.filter(|_| drawable)));
        let strokes = layer.get("strokes").and_then(Value::as_array).cloned().unwrap_or_default();
        value.insert("stroke_count".into(), json!(strokes.len()));
        if detailed {
            let size = value.get("color_size").and_then(Value::as_f64).unwrap_or(4.0);
            let budget = if strokes.len() > 200 { 8 } else { 40 };
            let listed: Vec<Value> = strokes
                .iter()
                .map(|stroke| describe_stroke(stroke, world, size, points.then_some(budget)))
                .collect();
            value.insert("strokes".into(), Value::Array(listed));
        }
        Value::Object(value)
    }

    /// One object's summary: kind, name, bounds, transform, properties, palette, words.
    fn describe(&self, id: NodeId, with_children: bool) -> Option<Value> {
        let storyboard = self.session.active_storyboard();
        let row = storyboard.layers().into_iter().find(|row| row.id == id)?;
        let kind = match row.kind {
            draw::document::LayerKind::Canvas => "canvas",
            draw::document::LayerKind::Group => "group",
            draw::document::LayerKind::Rectangle => "rectangle",
            draw::document::LayerKind::Ellipse => "ellipse",
            draw::document::LayerKind::Line => "line",
            draw::document::LayerKind::Text => "text",
        };
        let mut value = Map::new();
        value.insert("id".into(), json!(id.get()));
        value.insert("kind".into(), json!(kind));
        value.insert("name".into(), json!(row.name));
        value.insert("visible".into(), json!(row.visible));
        value.insert("locked".into(), json!(row.locked));
        value.insert("selected".into(), json!(row.selected));
        if let Some(parent) = row.parent {
            value.insert("in_group".into(), json!(parent.get()));
        }
        value.insert("bounds".into(), self.bounds(id).map_or(Value::Null, bounds_value));
        if let Some(world) = storyboard.world_transform(id)
            && world != Affine::IDENTITY
        {
            value.insert("transform".into(), transform_value(world));
        }
        let mut props = Map::new();
        for (prop, _) in OBJECT_PROPS {
            if matches!(prop, PropId::Name | PropId::Visible | PropId::Locked) {
                continue;
            }
            if let Some(read) = storyboard.node_prop(id, *prop) {
                props.insert(prop.key().into(), friendly(*prop, &read));
            }
        }
        if !props.is_empty() {
            value.insert("properties".into(), Value::Object(props));
        }
        if kind == "line"
            && let (Some(world), Some(ends)) = (storyboard.world_transform(id), self.line_ends(id))
        {
            let map = |point: Point| {
                let mapped = world * kurbo::Point::new(point[0], point[1]);
                round_point([mapped.x, mapped.y])
            };
            value.insert("from".into(), map(ends.0));
            value.insert("to".into(), map(ends.1));
        }
        if !matches!(kind, "canvas" | "group") {
            let palette: Vec<Value> = storyboard
                .node_palette(id)
                .unwrap_or_default()
                .iter()
                .map(|color| {
                    let mut entry = self.color_value(id, color.id);
                    if let Some(entry) = entry.as_object_mut() {
                        entry.insert("role".into(), json!(role_name(color.role)));
                    }
                    entry
                })
                .collect();
            value.insert("palette".into(), Value::Array(palette));
        }
        if kind == "text" {
            value.insert("text".into(), json!(storyboard.text_content(id).unwrap_or_default()));
            if let Some(frame) = self.session.assist_text_frame(id) {
                value.insert(
                    "box".into(),
                    json!({ "origin": round_point(frame.origin), "width": frame.width, "height": frame.height }),
                );
            }
        }
        if with_children && kind == "canvas" {
            let layers: Vec<Value> = row
                .colors
                .iter()
                .map(|color| self.color_value(id, color.id))
                .collect();
            value.insert("layers_top_to_bottom".into(), Value::Array(layers));
        }
        Some(Value::Object(value))
    }

    fn line_ends(&self, id: NodeId) -> Option<(Point, Point)> {
        let snapshot = self.snapshot();
        let node = find_value(snapshot.get("nodes")?, id.get())?;
        Some((pair(node.get("start")?)?, pair(node.get("end")?)?))
    }

    /// An object's box on the canvas, as its selection box would be.
    fn bounds(&self, id: NodeId) -> Option<(Point, Point)> {
        let storyboard = self.session.active_storyboard();
        if storyboard.is_text(id) {
            let fit = storyboard.text_box(id)?;
            let world = storyboard.world_transform(id)?;
            let origin = fit.frame.origin;
            let corners = [
                origin,
                [origin[0] + fit.width, origin[1]],
                [origin[0], origin[1] + fit.height],
                [origin[0] + fit.width, origin[1] + fit.height],
            ];
            return union(corners.iter().map(|corner| {
                let mapped = world * kurbo::Point::new(corner[0], corner[1]);
                ([mapped.x, mapped.y], [mapped.x, mapped.y])
            }));
        }
        let row = storyboard.layers().into_iter().find(|row| row.id == id)?;
        if row.kind == draw::document::LayerKind::Group {
            let children: Vec<NodeId> = storyboard
                .layers()
                .into_iter()
                .filter(|child| child.parent == Some(id))
                .map(|child| child.id)
                .collect();
            return union(children.into_iter().filter_map(|child| self.bounds(child)));
        }
        // The selection box of a copy, so the person's selection stays.
        let record = self.session.active_storyboard();
        let whole = record.drawing.snapshot_value(record.id(), record.name());
        let mut copy = draw::document::snapshot::drawing_from_value(&whole)?.drawing;
        copy.set_selection([id]);
        copy.selection_bounds()
    }

    // -----------------------------------------------------------------------
    // Looking.

    fn look(&mut self, input: &Value) -> Outcome {
        let size = input.get("size").and_then(Value::as_u64).unwrap_or(1024).clamp(64, 1536) as u32;
        let region = if let Some(region) = input.get("region").filter(|value| !value.is_null()) {
            match (region.get("min").and_then(pair), region.get("max").and_then(pair)) {
                (Some(min), Some(max)) if max[0] > min[0] && max[1] > min[1] => Some((min, max)),
                _ => return failed("region needs min and max [x, y], with max below and right of min"),
            }
        } else if input.get("visible_area").and_then(Value::as_bool) == Some(true) {
            match self.visible_area() {
                Some(area) => Some(area),
                None => return failed("the visible area is not known yet"),
            }
        } else if let Some(id) = input.get("object").and_then(Value::as_u64) {
            match self.bounds(NodeId::new(id)) {
                Some(bounds) => Some(margin(bounds)),
                None => return failed(format!("object {id} has nothing visible to show")),
            }
        } else {
            let storyboard = self.session.active_storyboard();
            let all = union(
                storyboard
                    .layers()
                    .into_iter()
                    .filter(|row| row.parent.is_none())
                    .filter_map(|row| self.bounds(row.id)),
            );
            all.map(margin).or_else(|| self.visible_area())
        };
        let Some((min, max)) = region else {
            return failed("the storyboard has nothing visible to show");
        };
        let marks = self.session.assist_scene();
        let Some(image) = self.looker.image(marks, min, max, size) else {
            return failed("the image could not be drawn");
        };
        Outcome {
            ok: true,
            output: json!({
                "region": { "min": round_point(min), "max": round_point(max) },
                "width": image.width,
                "height": image.height,
                "pixels_per_unit": (image.scale * 1000.0).round() / 1000.0,
                "to_document": "x = region.min[0] + pixel_x / pixels_per_unit, y = region.min[1] + pixel_y / pixels_per_unit",
                "notes": look::NOTES,
            }),
            images: vec![base64::engine::general_purpose::STANDARD.encode(&image.png)],
        }
    }
}

// ---------------------------------------------------------------------------
// Input.

fn node(input: &Value, key: &str) -> Result<NodeId, String> {
    input
        .get(key)
        .and_then(Value::as_u64)
        .filter(|id| *id != 0)
        .map(NodeId::new)
        .ok_or_else(|| format!("{key} must be an object id"))
}

fn color(input: &Value, key: &str) -> Result<ColorId, String> {
    input
        .get(key)
        .and_then(Value::as_u64)
        .filter(|id| *id != 0)
        .map(ColorId::new)
        .ok_or_else(|| format!("{key} must be a color id"))
}

fn count(input: &Value, key: &str) -> Result<usize, String> {
    input
        .get(key)
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .ok_or_else(|| format!("{key} must be a whole number from 0"))
}

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} must be text"))
}

fn optional<T>(input: &Value, key: &str, read: impl Fn(&Value) -> Option<T>) -> Result<Option<T>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => read(value).map(Some).ok_or_else(|| format!("{key} has the wrong form")),
    }
}

fn optional_node(input: &Value, key: &str) -> Result<Option<NodeId>, String> {
    optional(input, key, |value| value.as_u64().filter(|id| *id != 0).map(NodeId::new))
}

fn optional_nodes(input: &Value) -> Result<Option<Vec<NodeId>>, String> {
    optional(input, "objects", |value| {
        let items = value.as_array()?;
        items
            .iter()
            .map(|item| item.as_u64().filter(|id| *id != 0).map(NodeId::new))
            .collect::<Option<Vec<_>>>()
    })
    .map(|objects| objects.filter(|objects| !objects.is_empty()))
}

fn pair(value: &Value) -> Option<Point> {
    let items = value.as_array()?;
    let x = items.first()?.as_f64()?;
    let y = items.get(1)?.as_f64()?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

fn point(input: &Value, key: &str) -> Result<Point, String> {
    input.get(key).and_then(pair).ok_or_else(|| format!("{key} must be [x, y]"))
}

fn samples(input: &Value, pressure: bool) -> Result<Vec<Sample>, String> {
    let items = input
        .get("points")
        .and_then(Value::as_array)
        .ok_or("points must be a list of [x, y]")?;
    items
        .iter()
        .map(|item| {
            let xy = pair(item).ok_or("each point must be [x, y]")?;
            let p = item.get(2).and_then(Value::as_f64).filter(|_| pressure);
            if p.is_some_and(|p| !(0.0..=1.0).contains(&p)) {
                return Err("pressure must be from 0 to 1".to_string());
            }
            Ok(Sample { x: xy[0], y: xy[1], pressure: p })
        })
        .collect()
}

/// Choice values by name, in their option order.
fn options(prop: PropId) -> Option<&'static [&'static str]> {
    Some(match prop {
        PropId::Dash => &["solid", "dashed"],
        PropId::Cap => &["round", "flat"],
        PropId::Family => &["inter", "monospace"],
        PropId::Align => &["left", "center", "right", "justify"],
        PropId::VAlign => &["top", "middle", "bottom"],
        PropId::List => &["none", "bullets", "numbers", "checklist"],
        _ => return None,
    })
}

/// A property value as the model writes it, as the panel would send it.
fn prop_value(prop: PropId, value: &Value) -> Result<PropValue, String> {
    let key = prop.key();
    if let Some(names) = options(prop) {
        if let Some(name) = value.as_str() {
            let name = name.to_ascii_lowercase();
            return names
                .iter()
                .position(|option| *option == name)
                .map(|index| PropValue::Index(index as u8))
                .ok_or_else(|| format!("{key} is one of {names:?}"));
        }
        return value
            .as_u64()
            .filter(|index| (*index as usize) < names.len())
            .map(|index| PropValue::Index(index as u8))
            .ok_or_else(|| format!("{key} is one of {names:?}"));
    }
    match prop {
        PropId::Name | PropId::Hex => value
            .as_str()
            .map(|text| PropValue::Text(text.to_string()))
            .ok_or_else(|| format!("{key} must be text")),
        PropId::Rgb => {
            if let Some(text) = value.as_str() {
                return parse_hex(text).map(PropValue::Rgb).ok_or_else(|| format!("{key} must be \"#rrggbb\" or [r, g, b]"));
            }
            let channels = value.as_array().filter(|items| items.len() == 3).ok_or_else(|| format!("{key} must be \"#rrggbb\" or [r, g, b]"))?;
            let mut rgb = [0u8; 3];
            for (slot, channel) in rgb.iter_mut().zip(channels) {
                *slot = channel.as_u64().filter(|c| *c <= 255).ok_or_else(|| format!("{key} channels are 0 to 255"))? as u8;
            }
            Ok(PropValue::Rgb(rgb))
        }
        PropId::Visible
        | PropId::Locked
        | PropId::Bold
        | PropId::Italic
        | PropId::Underline
        | PropId::AllCaps
        | PropId::AutoSize
        | PropId::WrapText => value.as_bool().map(PropValue::Bool).ok_or_else(|| format!("{key} must be true or false")),
        _ => value
            .as_f64()
            .filter(|n| n.is_finite())
            .map(PropValue::Number)
            .ok_or_else(|| format!("{key} must be a number")),
    }
}

fn parse_props(value: Option<&Value>, allowed: &[(PropId, &str)]) -> Result<Vec<(PropId, PropValue)>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let object = value.as_object().ok_or("properties must be an object of keys and values")?;
    let mut props = Vec::new();
    for (key, value) in object {
        let Some((prop, _)) = allowed.iter().find(|(prop, _)| prop.key() == key) else {
            return Err(format!(
                "{key:?} is not one of {:?}",
                allowed.iter().map(|(prop, _)| prop.key()).collect::<Vec<_>>()
            ));
        };
        props.push((*prop, prop_value(*prop, value)?));
    }
    Ok(props)
}

fn object_props(value: Option<&Value>) -> Result<Vec<(PropId, PropValue)>, String> {
    parse_props(value, OBJECT_PROPS)
}

fn color_props(value: Option<&Value>) -> Result<Vec<(PropId, PropValue)>, String> {
    parse_props(value, COLOR_PROPS)
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(text.get(at..at + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

// ---------------------------------------------------------------------------
// Output.

fn friendly(prop: PropId, value: &PropValue) -> Value {
    match value {
        PropValue::Index(index) => match options(prop).and_then(|names| names.get(usize::from(*index))) {
            Some(name) => json!(name),
            None => json!(index),
        },
        PropValue::Rgb(rgb) => json!(hex(*rgb)),
        PropValue::Number(number) => json!((number * 1000.0).round() / 1000.0),
        PropValue::Bool(on) => json!(on),
        PropValue::Text(text) => json!(text),
        PropValue::Curve(_) | PropValue::Idle | PropValue::Mixed => Value::Null,
    }
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn role_name(role: draw::document::ColorRole) -> &'static str {
    use draw::document::ColorRole;
    match role {
        ColorRole::Layer => "layer",
        ColorRole::Outline => "outline",
        ColorRole::Stroke => "stroke",
        ColorRole::Fill => "fill",
        ColorRole::Text => "text",
        ColorRole::Background => "background",
        ColorRole::Border => "border",
    }
}

fn ids(nodes: &[NodeId]) -> Vec<u64> {
    nodes.iter().map(|id| id.get()).collect()
}

fn round(number: f64) -> f64 {
    (number * 10.0).round() / 10.0
}

fn round_point(point: Point) -> Value {
    json!([round(point[0]), round(point[1])])
}

fn bounds_value((min, max): (Point, Point)) -> Value {
    json!({ "min": round_point(min), "max": round_point(max) })
}

fn transform_value(world: Affine) -> Value {
    let [a, b, c, d, e, f] = world.as_coeffs();
    let sx = a.hypot(b);
    let sy = if sx > 1e-9 { (a * d - b * c) / sx } else { c.hypot(d) };
    json!({
        "translate": [round(e), round(f)],
        "rotation_degrees": round(b.atan2(a).to_degrees()),
        "scale": [(sx * 1000.0).round() / 1000.0, (sy * 1000.0).round() / 1000.0],
    })
}

fn union(boxes: impl Iterator<Item = (Point, Point)>) -> Option<(Point, Point)> {
    boxes.reduce(|(amin, amax), (bmin, bmax)| {
        (
            [amin[0].min(bmin[0]), amin[1].min(bmin[1])],
            [amax[0].max(bmax[0]), amax[1].max(bmax[1])],
        )
    })
}

/// A box with room around it, so its edges are not cut.
fn margin((min, max): (Point, Point)) -> (Point, Point) {
    let pad = ((max[0] - min[0]).max(max[1] - min[1]) * 0.05).max(8.0);
    ([min[0] - pad, min[1] - pad], [max[0] + pad, max[1] + pad])
}

fn find_value(nodes: &Value, id: u64) -> Option<Value> {
    for node in nodes.as_array()? {
        if node.get("id").and_then(Value::as_u64) == Some(id) {
            return Some(node.clone());
        }
        if let Some(found) = node.get("children").and_then(|children| find_value(children, id)) {
            return Some(found);
        }
    }
    None
}

/// A stroke's id, bounds, erase count, and its points simplified to at
/// most `budget`, in document coordinates.
fn describe_stroke(stroke: &Value, world: Affine, layer_size: f64, budget: Option<usize>) -> Value {
    let points: Vec<Point> = stroke
        .get("samples")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|sample| {
            let x = sample.get("x")?.as_f64()?;
            let y = sample.get("y")?.as_f64()?;
            let mapped = world * kurbo::Point::new(x, y);
            Some([mapped.x, mapped.y])
        })
        .collect();
    let half = stroke.get("size").and_then(Value::as_f64).unwrap_or(layer_size) / 2.0;
    let bounds = union(points.iter().map(|p| ([p[0] - half, p[1] - half], [p[0] + half, p[1] + half])));
    let mut value = json!({
        "id": stroke.get("id"),
        "bounds": bounds.map_or(Value::Null, bounds_value),
        "erased_by": stroke.get("erases").and_then(Value::as_array).map_or(0, Vec::len),
    });
    if let Some(budget) = budget {
        let simple = simplify(&points, budget);
        value["points"] = Value::Array(simple.into_iter().map(round_point).collect());
    }
    value
}

/// Fewer points along the same path: Ramer-Douglas-Peucker, loosened until
/// it fits the budget.
fn simplify(points: &[Point], budget: usize) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut tolerance = 0.75;
    loop {
        let mut keep = vec![false; points.len()];
        keep[0] = true;
        keep[points.len() - 1] = true;
        rdp(points, 0, points.len() - 1, tolerance, &mut keep);
        let kept: Vec<Point> = points.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect();
        if kept.len() <= budget.max(2) {
            return kept;
        }
        tolerance *= 1.6;
    }
}

fn rdp(points: &[Point], first: usize, last: usize, tolerance: f64, keep: &mut [bool]) {
    if last <= first + 1 {
        return;
    }
    let (a, b) = (points[first], points[last]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    let mut far = (0.0, first);
    for (index, p) in points.iter().enumerate().take(last).skip(first + 1) {
        let distance = if length < 1e-9 {
            (p[0] - a[0]).hypot(p[1] - a[1])
        } else {
            ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / length
        };
        if distance > far.0 {
            far = (distance, index);
        }
    }
    if far.0 > tolerance {
        keep[far.1] = true;
        rdp(points, first, far.1, tolerance, keep);
        rdp(points, far.1, last, tolerance, keep);
    }
}

/// Every object, layer, and stroke id in a snapshot.
struct Inventory {
    objects: BTreeSet<u64>,
    layers: BTreeSet<u64>,
    strokes: BTreeSet<u64>,
    /// Each stroke's erase count.
    erases: std::collections::BTreeMap<u64, usize>,
}

fn inventory(snapshot: &Value) -> Inventory {
    fn walk(nodes: &Value, out: &mut Inventory) {
        for node in nodes.as_array().into_iter().flatten() {
            if let Some(id) = node.get("id").and_then(Value::as_u64) {
                out.objects.insert(id);
            }
            for layer in node.get("colors").and_then(Value::as_array).into_iter().flatten() {
                if let Some(id) = layer.get("color").and_then(|color| color.get("id")).and_then(Value::as_u64) {
                    out.layers.insert(id);
                }
                for stroke in layer.get("strokes").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(id) = stroke.get("id").and_then(Value::as_u64) {
                        out.strokes.insert(id);
                        let erases = stroke.get("erases").and_then(Value::as_array).map_or(0, Vec::len);
                        out.erases.insert(id, erases);
                    }
                }
            }
            if let Some(children) = node.get("children") {
                walk(children, out);
            }
        }
    }
    let mut out = Inventory {
        objects: BTreeSet::new(),
        layers: BTreeSet::new(),
        strokes: BTreeSet::new(),
        erases: std::collections::BTreeMap::new(),
    };
    if let Some(nodes) = snapshot.get("nodes") {
        walk(nodes, &mut out);
    }
    out
}
