//! What the assistant's tools do to the session. Each one takes the path the
//! same hand action takes: a stroke is a brush gesture, a property is the
//! panel's edit, an order change is the toolbar's command, a name is the
//! outliner's rename. So undo, the document the host is sent, and saving are
//! the ones a hand edit makes. The person's tools stay as they were: the
//! current tool, the brush color, the eraser's size, a shape tool's snap,
//! and the panel's page are put back after each call.

use super::{ColorOwner, Session, Subject};
use crate::registry::{CommandId, MenuAction, Tool};
use draw::document::{ColorId, NodeId, Point, Sample, ShapeKind, DrawingId, TextBox};
use draw::{PropId, PropValue};
use kurbo::Affine;

/// What a call leaves to put back: the person's tool settings and panel.
struct Kept {
    tool: Tool,
    brush_active: ColorId,
    eraser_size: u8,
    snaps: [bool; 3],
    subject: Subject,
    return_to: Option<Subject>,
    panel_open: bool,
}

/// One brush color, as the toolbar's swatches show it.
pub(crate) struct BrushColor {
    pub id: ColorId,
    pub name: String,
    pub rgb: [u8; 3],
    pub active: bool,
}

impl Session {
    fn keep(&self) -> Kept {
        Kept {
            tool: self.tool,
            brush_active: self.brush_active,
            eraser_size: self.eraser_size,
            snaps: [self.rectangle.snap, self.ellipse.snap, self.line.snap],
            subject: self.subject,
            return_to: self.return_to,
            panel_open: self.panel_open,
        }
    }

    fn put_back(&mut self, kept: Kept) {
        if self.tool != kept.tool {
            // As choosing the tool would, but leaving the selection as the
            // call left it.
            self.settle();
            self.tool = kept.tool;
        }
        self.brush_active = kept.brush_active;
        self.eraser_size = kept.eraser_size;
        self.rectangle.snap = kept.snaps[0];
        self.ellipse.snap = kept.snaps[1];
        self.line.snap = kept.snaps[2];
        self.subject = kept.subject;
        self.return_to = kept.return_to;
        self.panel_open = kept.panel_open;
    }

    /// Runs `work` and puts the person's tools and panel back after it.
    fn with_kept<R>(&mut self, work: impl FnOnce(&mut Self) -> R) -> R {
        let kept = self.keep();
        let result = work(self);
        self.put_back(kept);
        result
    }

    /// Before a request's call: what the person left open, such as text
    /// being typed, is committed as their own step, outside the request's.
    pub(crate) fn assist_settle(&mut self) {
        self.settle();
    }

    /// The changes on `storyboard` under `group` become one undo step.
    pub(crate) fn join_group(&mut self, storyboard: DrawingId, group: &str) {
        if let Some(index) = self.index_of(storyboard) {
            self.storyboards[index].join_group(group);
        }
    }

    /// A hand edit: every request's undo step is complete.
    pub(crate) fn close_groups(&mut self) {
        for storyboard in &mut self.storyboards {
            storyboard.close_group();
        }
    }

    pub(crate) fn brush_colors(&self) -> Vec<BrushColor> {
        self.brush_palette
            .iter()
            .map(|item| BrushColor {
                id: item.id(),
                name: item.color.name.clone(),
                rgb: item.color.rgb,
                active: item.id() == self.brush_active,
            })
            .collect()
    }

    /// The brush color a layer of `canvas` came from.
    fn layer_source(&self, canvas: NodeId, layer: ColorId) -> Option<ColorId> {
        self.brush_palette
            .iter()
            .map(|item| item.id())
            .find(|source| self.active_storyboard().layer_for_source(canvas, *source) == Some(layer))
    }

    fn is_node(&self, id: NodeId) -> bool {
        self.active_storyboard().layers().iter().any(|row| row.id == id)
    }

    /// Selects `objects`, as clicking them with the select tool does. With
    /// none, the selection stays as the person left it.
    pub(crate) fn assist_select_objects(&mut self, objects: Option<&[NodeId]>) -> Result<(), String> {
        let Some(objects) = objects else {
            return Ok(());
        };
        if let Some(missing) = objects.iter().find(|id| !self.is_node(**id)) {
            return Err(format!("there is no object {} in this storyboard", missing.get()));
        }
        self.replace_selection(objects.to_vec());
        Ok(())
    }

    /// Makes `canvas` the canvas the brush and eraser work on, as clicking it does.
    pub(crate) fn assist_highlight(&mut self, canvas: NodeId) -> Result<(), String> {
        if !self.active_storyboard().is_canvas(canvas) {
            return Err(format!("{} is not a canvas", canvas.get()));
        }
        self.select_click(canvas, false);
        if self.active_storyboard().highlighted_id() != Some(canvas) {
            return Err(format!(
                "canvas {} is hidden or locked, so nothing can be drawn on it",
                canvas.get()
            ));
        }
        Ok(())
    }

    /// A brush stroke through `samples`, in document coordinates. `color` is
    /// a layer of the canvas or a brush color; without it, the person's
    /// current brush color. With no canvas highlighted, the stroke makes a
    /// new canvas, as it does by hand.
    pub(crate) fn assist_stroke(
        &mut self,
        canvas: Option<NodeId>,
        color: Option<ColorId>,
        samples: Vec<Sample>,
    ) -> Result<(), String> {
        if samples.len() < 2 {
            return Err("a stroke needs at least two points".into());
        }
        self.with_kept(|session| {
            if let Some(canvas) = canvas {
                session.assist_highlight(canvas)?;
            }
            session.set_tool(Tool::Brush);
            let target = session.active_storyboard().highlighted_id();
            if let Some(color) = color {
                let source = if session.brush_palette.iter().any(|item| item.id() == color) {
                    color
                } else if let Some(canvas) = target
                    && session.active_storyboard().has_color(canvas, color)
                {
                    session.layer_source(canvas, color).ok_or_else(|| {
                        format!(
                            "layer {} was not made from a brush color, so the brush cannot draw into it; use a brush color instead",
                            color.get()
                        )
                    })?
                } else {
                    return Err(format!(
                        "{} is neither a brush color nor a layer of the canvas being drawn on",
                        color.get()
                    ));
                };
                session.brush_active = source;
            }
            session
                .begin()
                .map_err(|_| "the layer for that color on this canvas is hidden or locked".to_string())?;
            for sample in samples {
                session
                    .add_sample(sample)
                    .map_err(|_| "a point is not a finite number, or its pressure is outside 0 to 1".to_string())?;
            }
            session.commit().map_err(|_| "the stroke could not be drawn".to_string())
        })
    }

    /// An eraser path through `samples`. It adds an erase to every stroke it
    /// crosses on the canvas, so the strokes stay and can be brought back by undo.
    pub(crate) fn assist_erase(
        &mut self,
        canvas: Option<NodeId>,
        size: Option<u8>,
        samples: Vec<Sample>,
    ) -> Result<(), String> {
        if samples.len() < 2 {
            return Err("an erase needs at least two points".into());
        }
        self.with_kept(|session| {
            if let Some(canvas) = canvas {
                session.assist_highlight(canvas)?;
            }
            session.set_tool(Tool::Eraser);
            if let Some(size) = size {
                session.eraser_size = size.clamp(draw::defaults::SIZE_MIN, draw::defaults::SIZE_MAX);
            }
            session
                .begin()
                .map_err(|_| "no canvas is highlighted to erase on; pass a canvas".to_string())?;
            for sample in samples {
                session
                    .add_sample(sample)
                    .map_err(|_| "a point is not a finite number".to_string())?;
            }
            session.commit().map_err(|_| "the erase could not be made".to_string())
        })
    }

    /// A rectangle or ellipse in the box from `from` to `to`, or a line
    /// between them, drawn as the shape tool's drag draws it. Snapping is
    /// off, so the points are used as given; `constrain` makes a square,
    /// circle, or a line at a multiple of 45 degrees, as Shift does.
    pub(crate) fn assist_shape(
        &mut self,
        kind: ShapeKind,
        from: Point,
        to: Point,
        constrain: bool,
    ) -> Result<(), String> {
        self.with_kept(|session| {
            let tool = match kind {
                ShapeKind::Rectangle => Tool::Rectangle,
                ShapeKind::Ellipse => Tool::Ellipse,
                ShapeKind::Line => Tool::Line,
            };
            session.set_tool(tool);
            let settings = session.shape_tool_mut(tool).expect("a shape tool");
            settings.snap = false;
            let shift = settings.constrain ^ constrain;
            session
                .begin_shape(from)
                .map_err(|_| "a point is not a finite number".to_string())?;
            session
                .move_shape(to, shift)
                .map_err(|_| "a point is not a finite number".to_string())?;
            session.commit().map_err(|_| "the shape could not be drawn".to_string())
        })
    }

    /// New text at `at`, its box's top-left corner, typed and committed as
    /// the text tool does. `width` is a dragged box's width, so the words
    /// wrap; without it the box sizes itself to the words.
    pub(crate) fn assist_text(
        &mut self,
        at: Point,
        width: Option<f64>,
        content: &str,
    ) -> Result<(), String> {
        if content.is_empty() {
            return Err("the text is empty".into());
        }
        if let Some(hit) = self
            .active_storyboard()
            .hit_test(at)
            .filter(|id| self.active_storyboard().is_text(*id))
        {
            return Err(format!(
                "text {} is already at that point, and a click there would edit it; use set_text to change its words, or place the new text elsewhere",
                hit.get()
            ));
        }
        self.with_kept(|session| {
            session.set_tool(Tool::Text);
            session
                .begin_text(at)
                .map_err(|_| "the point is not a finite number".to_string())?;
            let width = width.filter(|width| width.is_finite() && *width >= 1.0);
            let end = [at[0] + width.unwrap_or(0.0), at[1]];
            session
                .move_text(end)
                .map_err(|_| "the width is not a finite number".to_string())?;
            session
                .end_text(width.is_some())
                .map_err(|_| "the text could not be placed".to_string())?;
            session.set_text_draft(content.to_string());
            session.commit_text();
            Ok(())
        })
    }

    /// Replaces a text object's words, as editing it in place does. Empty
    /// words remove it.
    pub(crate) fn assist_set_text(&mut self, id: NodeId, content: &str) -> Result<(), String> {
        if !self.active_storyboard().is_text(id) {
            return Err(format!("{} is not a text object", id.get()));
        }
        if !self.edit_text(id) {
            return Err(format!("text {} is locked", id.get()));
        }
        self.set_text_draft(content.to_string());
        self.commit_text();
        Ok(())
    }

    /// Moves, scales, or rotates `objects`, or the selection, as dragging
    /// them or their handles does: `map` is applied in document space.
    pub(crate) fn assist_transform(
        &mut self,
        objects: Option<&[NodeId]>,
        map: impl Fn(Point) -> Affine,
        rotates: bool,
    ) -> Result<(), String> {
        self.assist_select_objects(objects)?;
        if rotates && !self.active_storyboard().selection_turns() {
            return Err("text is never rotated or flipped, so a selection holding text cannot turn".into());
        }
        let before = self.changeable_locals();
        if before.is_empty() {
            return Err("nothing selected can be moved: select objects first, or unlock them".into());
        }
        let Some((min, max)) = self.selection_bounds() else {
            return Err("the selection has nothing visible to move".into());
        };
        let center = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
        let affine = map(center);
        self.preview_world(&before, |world| affine * world);
        if !self.commit_preview(&before) {
            return Err("that change leaves the objects where they are".into());
        }
        Ok(())
    }

    /// Sets a text object's box, as dragging its side handles does.
    pub(crate) fn assist_text_box(
        &mut self,
        id: NodeId,
        width: Option<f64>,
        height: Option<f64>,
    ) -> Result<(), String> {
        let Some(fit) = self.active_storyboard().text_box(id) else {
            return Err(format!("{} is not a text object", id.get()));
        };
        if !self.active_storyboard().transform_target(id) {
            return Err(format!("text {} is locked", id.get()));
        }
        let before = fit.frame;
        let em = self.active_storyboard().text_size(id).map_or(8.0, f64::from);
        let mut after = before;
        if let Some(width) = width {
            after.width = Some(width.max(em.max(8.0)));
        }
        if let Some(height) = height {
            after.height = Some(height.max(fit.natural));
        }
        if after == before {
            return Err("the box already has that size".into());
        }
        self.restore_text_box(id, after);
        if !self.commit_text_box(id, before) {
            return Err("the box could not be changed".into());
        }
        Ok(())
    }

    /// Runs a command on `objects`, or the selection, as its toolbar button
    /// or shortcut does. `shift` is the larger nudge.
    pub(crate) fn assist_command(
        &mut self,
        objects: Option<&[NodeId]>,
        command: CommandId,
        shift: bool,
        viewport: Point,
    ) -> Result<(), String> {
        self.assist_select_objects(objects)?;
        if !self.command_enabled(command) {
            return Err(disabled_reason(command, self.selection().is_empty()));
        }
        self.with_kept(|session| {
            session.perform_shifted(command, viewport, shift);
        });
        Ok(())
    }

    /// Edits object properties as the panel's object page does, with the
    /// outliner's eye, lock, and rename for visibility, locking, and names.
    /// Returns, for each property, the objects that took it.
    pub(crate) fn assist_object_props(
        &mut self,
        objects: &[NodeId],
        props: &[(PropId, PropValue)],
    ) -> Vec<(PropId, Vec<NodeId>)> {
        let mut took = Vec::new();
        // Unlocking first and locking last, so the other edits are not refused.
        let mut ordered: Vec<&(PropId, PropValue)> = props.iter().collect();
        ordered.sort_by_key(|(prop, value)| match (prop, value) {
            (PropId::Locked, PropValue::Bool(false)) => 0,
            (PropId::Locked, _) => 3,
            (PropId::Visible, _) => 2,
            _ => 1,
        });
        for (prop, value) in ordered {
            let mut changed = Vec::new();
            match prop {
                PropId::Visible | PropId::Locked => {
                    for id in objects {
                        let now = self.active_storyboard().node_prop(*id, *prop);
                        let ok = if now.as_ref() == Some(value) {
                            true
                        } else {
                            self.toggle_node(*id, *prop)
                        };
                        if ok {
                            changed.push(*id);
                        }
                    }
                }
                PropId::Name => {
                    for id in objects {
                        let renamable = self
                            .active_storyboard()
                            .layers()
                            .iter()
                            .find(|row| row.id == *id)
                            .is_some_and(|row| {
                                matches!(
                                    row.kind,
                                    draw::document::LayerKind::Canvas | draw::document::LayerKind::Group
                                )
                            });
                        let PropValue::Text(name) = value else {
                            continue;
                        };
                        if renamable
                            && (self.active_storyboard().node_prop(*id, PropId::Name).as_ref() == Some(value)
                                || self.rename_node(*id, name))
                        {
                            changed.push(*id);
                        }
                    }
                }
                _ => {
                    let targets: Vec<NodeId> = objects
                        .iter()
                        .copied()
                        .filter(|id| {
                            self.active_storyboard().node_prop(*id, *prop).is_some()
                                && self.active_storyboard().can_edit(*id, *prop)
                        })
                        .collect();
                    if !targets.is_empty() {
                        self.with_kept(|session| {
                            session.replace_selection(targets.clone());
                            session.subject = Subject::Objects;
                            session.set_panel_property(*prop, value.clone());
                        });
                        changed = targets;
                    }
                }
            }
            took.push((*prop, changed));
        }
        took
    }

    /// Edits one palette color as its color page does, with the outliner's
    /// eye, lock, and rename for a canvas's layers.
    pub(crate) fn assist_color_props(
        &mut self,
        owner: NodeId,
        color: ColorId,
        props: &[(PropId, PropValue)],
    ) -> Result<Vec<(PropId, bool)>, String> {
        if !self.active_storyboard().has_color(owner, color) {
            return Err(format!("object {} has no color {}", owner.get(), color.get()));
        }
        if !self.active_storyboard().can_edit(owner, PropId::Visible) {
            return Err(format!("object {} is locked; unlock it first", owner.get()));
        }
        let layer = self.active_storyboard().is_canvas(owner);
        let mut took = Vec::new();
        for (prop, value) in props {
            let now = self.active_storyboard().color_prop(owner, color, *prop);
            let ok = match prop {
                _ if now.as_ref() == Some(value) => true,
                PropId::Visible | PropId::Locked if layer => {
                    self.toggle_color_flag(owner, color, *prop)
                }
                PropId::Locked => false,
                PropId::Name if layer => match value {
                    PropValue::Text(name) => self.rename_color(owner, color, name),
                    _ => false,
                },
                _ => self.with_kept(|session| {
                    session.subject = Subject::Color {
                        owner: ColorOwner::Node(owner),
                        color,
                    };
                    session.set_panel_property(*prop, value.clone())
                }),
            };
            took.push((*prop, ok));
        }
        Ok(took)
    }

    /// Duplicates or deletes a canvas layer, as its row's menu does.
    pub(crate) fn assist_layer(
        &mut self,
        canvas: NodeId,
        color: ColorId,
        action: MenuAction,
    ) -> Result<(), String> {
        if !self.active_storyboard().is_canvas(canvas) || !self.active_storyboard().has_color(canvas, color) {
            return Err(format!("canvas {} has no layer {}", canvas.get(), color.get()));
        }
        if self.layer_action(canvas, color, action) {
            Ok(())
        } else if action == MenuAction::Delete {
            Err("a canvas keeps at least one layer, and a locked canvas keeps all of them".into())
        } else {
            Err("the layer could not be duplicated; the canvas may be locked".into())
        }
    }

    /// Moves a canvas layer, as dragging its row does. 0 paints on top.
    pub(crate) fn assist_move_layer(
        &mut self,
        canvas: NodeId,
        color: ColorId,
        to: usize,
    ) -> Result<(), String> {
        self.assist_highlight(canvas)?;
        match self.move_color(color, to) {
            Ok(()) => Ok(()),
            Err(_) => Err(format!(
                "canvas {} has no layer {} or no position {}",
                canvas.get(),
                color.get(),
                to
            )),
        }
    }

    /// Moves an object among its siblings, as dragging its outliner row does.
    pub(crate) fn assist_reorder(&mut self, id: NodeId, to: usize) -> Result<(), String> {
        if !self.is_node(id) {
            return Err(format!("there is no object {}", id.get()));
        }
        if self.reorder_node(id, to) {
            Ok(())
        } else {
            Err("it is already there, the position is past its last sibling, or it is locked".into())
        }
    }

    /// A text box's frame, for a result.
    pub(crate) fn assist_text_frame(&self, id: NodeId) -> Option<TextBox> {
        self.active_storyboard().text_box(id).map(|fit| fit.frame)
    }
}

fn disabled_reason(command: CommandId, nothing_selected: bool) -> String {
    if nothing_selected && command != CommandId::Paste {
        return "nothing is selected; pass objects, or select them first".into();
    }
    match command {
        CommandId::Paste => "nothing has been copied or cut yet".into(),
        CommandId::Group => "grouping needs two or more unlocked objects with the same parent".into(),
        CommandId::Ungroup => "no selected object is a group".into(),
        CommandId::AlignLeft | CommandId::AlignCenter | CommandId::AlignRight => {
            "aligning needs two or more unlocked, visible objects".into()
        }
        CommandId::BringForward
        | CommandId::SendBackward
        | CommandId::BringToFront
        | CommandId::SendToBack => "the selection is already there, or is locked".into(),
        CommandId::RotateClockwise
        | CommandId::RotateCounterclockwise
        | CommandId::RotateHalf
        | CommandId::FlipHorizontal
        | CommandId::FlipVertical => {
            "the selection holds text, which never turns, or nothing unlocked and visible".into()
        }
        _ => "the selected objects are locked".into(),
    }
}
