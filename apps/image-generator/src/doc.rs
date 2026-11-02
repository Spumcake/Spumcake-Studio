//! One image as a tab edits it: the image, the active layer, the
//! selection, the composite, and the undo history, with every change a person
//! or the assistant can make. Each change is one step; a step stores the
//! region that changed, before and after, not the whole image.

use crate::paint::{self, Brush, Combine, Mask};
use crate::layered::{Beneath, Blend, Curves, Grade, Kind, LayerMask, Node, LayeredImage, Pixels, Place, Rect};
use std::cell::{Cell, RefCell};
use crate::text::{self, Style, Text};
use protocol::{TextInput, TextKey};

/// One part of a step, which undo puts back and redo puts forward.
#[derive(Debug, Clone)]
enum Change {
    /// A region of one layer's pixels, in its own pixels.
    Pixels { id: u64, rect: Rect, before: Vec<u8>, after: Vec<u8> },
    /// One layer's whole buffer, when its size changes.
    Buffer { id: u64, before: Pixels, after: Pixels },
    /// The tree's settings and order, which hold no pixels.
    Tree { before: Vec<Node>, after: Vec<Node> },
    Active { before: Option<u64>, after: Option<u64> },
    Selection { before: Option<Mask>, after: Option<Mask> },
    Size { before: [u32; 2], after: [u32; 2] },
}

/// A brush that copies from the image: the clone stamp, or the healing
/// brush with `heal`. `offset` is from where it paints to where it copies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sampler {
    pub brush: Brush,
    pub offset: [i32; 2],
    pub heal: bool,
}

/// What a stroke paints with.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tool {
    Brush(Brush),
    Sampler(Sampler),
}

impl Tool {
    fn brush(&self) -> Brush {
        match self {
            Tool::Brush(brush) => *brush,
            Tool::Sampler(sampler) => sampler.brush,
        }
    }
}

/// A stroke being drawn on the buffer `id`: the layer `owner`'s pixels, or
/// its mask's.
struct Live {
    id: u64,
    owner: u64,
    tool: Tool,
    points: Vec<[f64; 3]>,
    /// The layer's pixels before the stroke, and where they sit.
    before: Pixels,
    offset: [i32; 2],
    /// Each image pixel's coverage so far, row by row.
    coverage: Vec<f32>,
    /// The part painted so far, and the part the stroke reaches, in image
    /// pixels.
    painted: Rect,
    wanted: Rect,
    /// What a sampler copies from: the layers up to and including the
    /// active one as the stroke began, straight RGBA8 the image's size.
    source: Vec<u8>,
}

/// Selected pixels lifted off the layer `id` and moved by `delta`, until
/// they are set down or put back.
struct Float {
    id: u64,
    /// The layer as it was, and with the selected pixels taken off it.
    before: Pixels,
    lifted: Pixels,
    /// The selected pixels, over `rect` of the image.
    piece: Pixels,
    rect: Rect,
    selection: Mask,
    outlines: Vec<Vec<[f64; 2]>>,
    delta: [i32; 2],
}

/// A text being typed in: its caret and the other end of its selection,
/// as character places, and the tree and active layer before the editing
/// began, so it becomes one step.
struct Typing {
    id: u64,
    caret: usize,
    anchor: usize,
    before: Vec<Node>,
    active: Option<u64>,
    /// A new text takes its words as its name.
    named: bool,
}

/// A text being typed in, as the host draws it: its box, its caret, and
/// its selected spans, in image pixels.
pub struct Editing {
    pub rect: [f64; 4],
    pub caret: [f64; 4],
    pub selection: Vec<[f64; 4]>,
}

#[derive(Debug, Clone, Default)]
struct Step {
    changes: Vec<Change>,
    /// The assistant's request this step belongs to: every call under one
    /// request is one step.
    group: Option<String>,
}

/// Why a change was refused, in words a person or the assistant reads.
pub type Refusal = String;

pub struct Doc {
    pub image: LayeredImage,
    pub active: Option<u64>,
    pub selection: Option<Mask>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// The tree before a held slider began, until it is let go.
    pending: Option<(Vec<Node>, Option<u64>)>,
    /// The composite, RGBA8 straight alpha, and the part of it that is out
    /// of date.
    composite: Vec<u8>,
    stale: Option<Rect>,
    /// The part of the composite brought up to date since the host was last
    /// told, so only it is sent again.
    changed: Option<Rect>,
    /// The stroke being drawn, painted as its points come.
    live: Option<Live>,
    /// Selected pixels being moved.
    float: Option<Float>,
    /// The text being typed in.
    typing: Option<Typing>,
    /// The layer whose mask is chosen, rather than the layer itself: it
    /// counts while that layer stays active.
    mask_of: Option<u64>,
    /// Rises whenever the composite changes.
    pub revision: u64,
    /// Rises with every change to the image, so its tab knows it is
    /// unwritten.
    pub edits: u64,
    /// Changes, to a number no other image has had, whenever pixels may
    /// have changed rather than only the tree's settings. What was made from
    /// the pixels, such as the image beneath a held slider's layer, a
    /// Curves layer's histogram, or a file's copy of the layers, holds while
    /// it stays the same.
    pub generation: u64,
    /// The image as the nodes beneath a held slider's layer combine it,
    /// or beneath the active layer, made while the program was idle; and
    /// the generation and top-level nodes as last composited, to find the
    /// first that changed.
    beneath: Option<KeptBeneath>,
    composited: Option<(u64, Vec<Node>)>,
    /// How many times an image beneath was made, ahead or for a slider.
    pub beneaths_made: u64,
    /// A Curves layer's histograms, kept while what lies beneath it stays.
    histograms: RefCell<Option<Histograms>>,
    /// How many times histograms were made.
    pub histograms_made: Cell<u64>,
    outlines: Option<Vec<Vec<[f64; 2]>>>,
    /// How many layers and groups have been made, for their names.
    made_layers: u32,
    made_groups: u32,
}

impl Doc {
    pub fn new(image: LayeredImage) -> Self {
        let active = image.nodes.last().map(|node| node.id);
        let made_layers = image.all_layers().len() as u32;
        let mut doc = Self {
            image,
            active,
            selection: None,
            undo: Vec::new(),
            redo: Vec::new(),
            pending: None,
            composite: Vec::new(),
            stale: None,
            changed: None,
            live: None,
            float: None,
            typing: None,
            mask_of: None,
            revision: 0,
            edits: 0,
            generation: next_generation(),
            beneath: None,
            composited: None,
            beneaths_made: 0,
            histograms: RefCell::new(None),
            histograms_made: Cell::new(0),
            outlines: None,
            made_layers,
            made_groups: 0,
        };
        doc.made_groups = doc.count_groups();
        doc.image.sync_texts();
        doc.stale = Some(doc.image.bounds());
        doc
    }

    fn count_groups(&self) -> u32 {
        let mut ids = Vec::new();
        for node in &self.image.nodes {
            ids.extend(self.image.ids_within(node.id));
        }
        ids.iter().filter(|id| self.image.find(**id).is_some_and(Node::is_group)).count() as u32
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The composite, brought up to date.
    pub fn composite(&mut self) -> &[u8] {
        self.refresh();
        &self.composite
    }

    /// Recomposites only the part that changed. Returns whether anything did.
    pub fn refresh(&mut self) -> bool {
        let size = self.image.width as usize * self.image.height as usize * 4;
        if self.composite.len() != size {
            self.composite = vec![0; size];
            self.stale = Some(self.image.bounds());
        }
        // A mask blurred again or a text drawn again changes only what is
        // made from its node's settings, which the image beneath compares
        // for itself, so the pixels' generation stays.
        if let Some(touched) = self.image.prepare() {
            self.touch_made(touched);
        }
        let Some(rect) = self.stale.take() else {
            self.let_slider_beneath_go();
            return false;
        };
        match self.keep_beneath() {
            Some(path) if rect == self.image.bounds() => {
                let beneath = self.image.beneath(&path);
                self.beneaths_made += 1;
                self.image.composite_over(Some(&beneath), rect, &mut self.composite);
                self.beneath = Some(KeptBeneath { generation: self.generation, beneath, ahead: false });
            }
            _ => self.image.composite_over(self.beneath.as_ref().map(|kept| &kept.beneath), rect, &mut self.composite),
        }
        self.composited = Some((self.generation, self.image.nodes.clone()));
        self.let_slider_beneath_go();
        self.changed = Some(self.changed.map_or(rect, |changed| changed.union(&rect)));
        self.revision += 1;
        true
    }

    /// Let go, a slider's image beneath is not needed any more: one made
    /// for it is kept only while it is held, as it is the image's size in
    /// floating point. One made ahead stays for the next.
    fn let_slider_beneath_go(&mut self) {
        if self.pending.is_none() && self.beneath.as_ref().is_some_and(|kept| !kept.ahead) {
            self.beneath = None;
        }
    }

    /// Whether the image beneath that is kept still holds: its nodes and
    /// the pixels are as they were.
    fn beneath_holds(&self) -> bool {
        self.beneath.as_ref().is_some_and(|kept| kept.generation == self.generation && kept.beneath.holds(&self.image))
    }

    /// Keeps the image beneath what a held slider changes, while it
    /// holds. Without one, when only the tree's settings changed since the
    /// last composite, returns the path to the first node that changed, for
    /// a new one.
    fn keep_beneath(&mut self) -> Option<Vec<usize>> {
        if !self.beneath_holds() {
            self.beneath = None;
        }
        if self.beneath.is_some() || self.pending.is_none() {
            return None;
        }
        let (made, before) = self.composited.as_ref()?;
        if *made != self.generation {
            return None;
        }
        let path = changed_path(&self.image.nodes, before);
        (path[0] < self.image.nodes.len() && worth_keeping(&path)).then_some(path)
    }

    /// The path to the active layer the image beneath it would be kept
    /// at, when the program is idle and none that holds is kept there.
    fn ahead_path(&self) -> Option<Vec<usize>> {
        if self.pending.is_some() || self.drawing() || self.typing.is_some() || self.stale.is_some() || self.composite.is_empty() {
            return None;
        }
        let path = path_to(&self.image.nodes, self.active?).filter(|path| worth_keeping(path))?;
        let kept = self.beneath.as_ref().is_some_and(|kept| kept.beneath.path() == path) && self.beneath_holds();
        (!kept).then_some(path)
    }

    /// Whether the program, when idle, has the image beneath the active
    /// layer to make.
    pub fn ahead_due(&self) -> bool {
        self.ahead_path().is_some()
    }

    /// While the program is idle: makes the image beneath the active
    /// layer, at each depth of the groups it lies in, so the first value of
    /// a slider held on it, a group, or its mask's feather combines only it
    /// and what lies above, as every later value does. Returns whether it
    /// made one.
    pub fn keep_ahead(&mut self) -> bool {
        let Some(path) = self.ahead_path() else {
            return false;
        };
        let beneath = self.image.beneath(&path);
        self.beneaths_made += 1;
        self.beneath = Some(KeptBeneath { generation: self.generation, beneath, ahead: true });
        true
    }

    /// Lets go of the image kept beneath, for an image not shown.
    pub fn let_beneath_go(&mut self) {
        if self.pending.is_none() {
            self.beneath = None;
        }
    }

    /// A slider begins on `id`: the image beneath kept for another node
    /// is let go, so it keeps its own.
    fn begin_slider(&mut self, id: u64) {
        self.pending = Some((self.image.nodes.clone(), self.active));
        if self.beneath.as_ref().is_some_and(|kept| path_to(&self.image.nodes, id) != Some(kept.beneath.path())) {
            self.beneath = None;
        }
    }

    /// The part of the composite that changed since this was last asked.
    pub fn take_changed(&mut self) -> Option<Rect> {
        self.changed.take()
    }

    /// A slider is held: its changes wait to become one step.
    pub fn held(&self) -> bool {
        self.pending.is_some()
    }

    /// Part of the composite is out of date.
    pub fn out_of_date(&self) -> bool {
        self.stale.is_some()
    }

    /// How many top-level nodes the image kept beneath a held slider holds.
    #[cfg(test)]
    fn beneath_count(&self) -> Option<usize> {
        self.beneath.as_ref().map(|kept| kept.beneath.path()[0])
    }

    /// A Curves layer's histogram of what lies beneath it, for `channel`:
    /// made again only when that changes, so dragging the curve never
    /// makes it again.
    pub fn histogram(&self, id: u64, channel: usize) -> Vec<f32> {
        let beneath = self.image.up_to(id, false);
        let size = (self.image.width, self.image.height);
        let mut kept = self.histograms.borrow_mut();
        let holds = kept.as_ref().is_some_and(|kept| kept.id == id && kept.generation == self.generation && kept.size == size && kept.beneath == beneath);
        if !holds {
            let rows = self.image.rows_beneath(id, 8);
            let channels = [0, 1, 2, 3].map(|channel| histogram(&rows, channel));
            *kept = Some(Histograms { id, generation: self.generation, size, beneath, channels });
            self.histograms_made.set(self.histograms_made.get() + 1);
        }
        kept.as_ref().map(|kept| kept.channels[channel.min(3)].clone()).unwrap_or_default()
    }

    fn touch(&mut self, rect: Rect) {
        self.generation = next_generation();
        let rect = rect.intersect(&self.image.bounds());
        if rect.is_empty() {
            return;
        }
        self.stale = Some(match self.stale {
            Some(stale) => stale.union(&rect),
            None => rect,
        });
    }

    /// Only what is made from the tree's settings changed in `rect`, such
    /// as a mask blurred again or a text drawn again: the pixels' generation
    /// stays.
    fn touch_made(&mut self, rect: Rect) {
        let rect = rect.intersect(&self.image.bounds());
        if !rect.is_empty() {
            self.stale = Some(self.stale.map_or(rect, |stale| stale.union(&rect)));
        }
    }

    fn touch_all(&mut self) {
        self.generation = next_generation();
        self.stale = Some(self.image.bounds());
    }

    /// Only the tree's settings or order changed, not any pixels: all of
    /// the image is composited again, and what was made from the pixels
    /// holds.
    fn touch_tree(&mut self) {
        self.stale = Some(self.image.bounds());
    }

    /// The selection's outlines, for the marching edge.
    pub fn outlines(&mut self) -> Vec<Vec<[f64; 2]>> {
        if self.outlines.is_none() {
            self.outlines = Some(self.selection.as_ref().map(Mask::outlines).unwrap_or_default());
        }
        self.outlines.clone().unwrap_or_default()
    }

    fn commit(&mut self, changes: Vec<Change>, group: Option<&str>) {
        if changes.is_empty() {
            return;
        }
        self.edits += 1;
        self.redo.clear();
        if let Some(group) = group
            && let Some(last) = self.undo.last_mut()
            && last.group.as_deref() == Some(group)
        {
            last.changes.extend(changes);
            return;
        }
        self.undo.push(Step {
            changes,
            group: group.map(String::from),
        });
    }

    pub fn undo(&mut self) -> bool {
        self.settle();
        let Some(step) = self.undo.pop() else {
            return false;
        };
        for change in step.changes.iter().rev() {
            self.put(change, false);
        }
        self.edits += 1;
        self.redo.push(step);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.settle();
        let Some(step) = self.redo.pop() else {
            return false;
        };
        for change in &step.changes {
            self.put(change, true);
        }
        self.edits += 1;
        self.undo.push(step);
        true
    }

    /// Puts one change's after, or its before.
    fn put(&mut self, change: &Change, forward: bool) {
        match change {
            Change::Pixels { id, rect, before, after } => {
                let bytes = if forward { after } else { before };
                if let Some(pixels) = self.image.pixels.get_mut(id) {
                    pixels.write(*rect, bytes);
                }
                self.image.forget_area(*id, *rect);
                let offset = self.image.buffer_offset(*id);
                self.touch(Rect::new(rect.x + offset[0], rect.y + offset[1], rect.w, rect.h));
            }
            Change::Buffer { id, before, after } => {
                let pixels = if forward { after } else { before };
                self.image.pixels.insert(*id, pixels.clone());
                self.image.forget(*id);
                self.touch_all();
            }
            Change::Tree { before, after } => {
                self.image.nodes = if forward { after.clone() } else { before.clone() };
                self.image.sync_texts();
                self.touch_all();
            }
            Change::Active { before, after } => {
                self.active = if forward { *after } else { *before };
            }
            Change::Selection { before, after } => {
                self.selection = if forward { after.clone() } else { before.clone() };
                self.outlines = None;
            }
            Change::Size { before, after } => {
                let size = if forward { after } else { before };
                self.image.width = size[0];
                self.image.height = size[1];
                self.touch_all();
            }
        }
    }

    /// The tree changed by `edit`, as one step; nothing when `edit` refuses
    /// or changes nothing.
    fn edit_tree(
        &mut self,
        group: Option<&str>,
        edit: impl FnOnce(&mut Self) -> Result<(), Refusal>,
    ) -> Result<(), Refusal> {
        self.settle();
        let before = self.image.nodes.clone();
        let active = self.active;
        if let Err(refusal) = edit(self) {
            self.image.nodes = before;
            self.active = active;
            return Err(refusal);
        }
        self.image.sync_texts();
        let mut changes = Vec::new();
        if self.image.nodes != before {
            changes.push(Change::Tree { before, after: self.image.nodes.clone() });
            self.touch_tree();
        }
        if self.active != active {
            changes.push(Change::Active { before: active, after: self.active });
        }
        self.commit(changes, group);
        Ok(())
    }

    // Painting.

    /// A brush or eraser stroke on the active layer, inside the selection
    /// when there is one. One step.
    pub fn stroke(&mut self, points: &[[f64; 3]], brush: &Brush) -> Result<(), Refusal> {
        // The stroke already painted as it was drawn becomes the step as it
        // is, unless the layer must grow to take it.
        let tool = Tool::Brush(*brush);
        let painted = self.live.as_ref().is_some_and(|live| {
            let layer = Rect::new(live.offset[0], live.offset[1], live.before.width as i32, live.before.height as i32);
            live.points == points && live.tool == tool && self.active == Some(live.owner) && (live.id != live.owner || layer.contains(&live.wanted))
        });
        if painted {
            self.finish_live();
            return Ok(());
        }
        self.end_live();
        self.settle();
        if let Some(refusal) = self.paint_refusal(brush.erase) {
            return Err(refusal);
        }
        let Some(id) = self.active else {
            return Ok(());
        };
        // A mask is painted as it would be drawn, all at once.
        if self.target_mask().is_some() {
            self.live_part(0, points, tool);
            self.finish_live();
            return Ok(());
        }
        let locks = self.image.locks(id);
        let clip = self.image.bounds();
        let Some(coverage) = paint::stroke_coverage_within(points, brush, clip) else {
            return Ok(());
        };
        let wanted = coverage.rect.intersect(&clip);
        if wanted.is_empty() {
            return Ok(());
        }
        let mut changes = Vec::new();
        // A layer moved off part of the image grows to cover it again, so
        // a stroke there is not lost.
        let layer_rect = self.image.node_rect(id).unwrap_or(clip);
        if !layer_rect.contains(&wanted) {
            let grown = layer_rect.union(&clip);
            let cut = Rect::new(grown.x - layer_rect.x, grown.y - layer_rect.y, grown.w, grown.h);
            let before_tree = self.image.nodes.clone();
            let old = self.image.pixels.get(&id).cloned().unwrap_or_else(|| Pixels::clear(1, 1));
            let new = old.cut(cut);
            if let Some(node) = self.image.find_mut(id) {
                node.offset = [grown.x, grown.y];
            }
            self.image.pixels.insert(id, new.clone());
            changes.push(Change::Tree { before: before_tree, after: self.image.nodes.clone() });
            changes.push(Change::Buffer { id, before: old, after: new });
            // Its mask grows with it, showing where it is new.
            if let Some(mask) = self.image.find(id).and_then(|node| node.mask.clone())
                && let Some(old) = self.image.pixels.get(&mask.id).cloned()
            {
                let new = old.cut(cut);
                self.image.pixels.insert(mask.id, new.clone());
                self.image.forget(mask.id);
                changes.push(Change::Buffer { id: mask.id, before: old, after: new });
            }
        }
        let offset = self.image.find(id).map(|node| node.offset).unwrap_or([0, 0]);
        let pixels = self.image.pixels.get_mut(&id).ok_or("The active layer has no pixels.")?;
        let local = Rect::new(wanted.x - offset[0], wanted.y - offset[1], wanted.w, wanted.h);
        let before = pixels.read(local);
        let changed = paint::apply(pixels, offset, &coverage, clip, self.selection.as_ref(), brush, locks.keeps_alpha());
        if let Some(changed) = changed {
            // Only the part that changed is kept.
            let after = pixels.read(changed);
            let mut kept = Vec::with_capacity(after.len());
            for y in 0..changed.h {
                let start = (((changed.y - local.y + y) * local.w + (changed.x - local.x)) * 4) as usize;
                kept.extend_from_slice(&before[start..start + changed.w as usize * 4]);
            }
            changes.push(Change::Pixels { id, rect: changed, before: kept, after });
            self.touch(Rect::new(changed.x + offset[0], changed.y + offset[1], changed.w, changed.h));
        }
        self.image.forget(id);
        if !changes.is_empty() {
            self.touch_all_if_grown(&changes);
        }
        self.commit(changes, None);
        Ok(())
    }

    /// Paints the newest points of a stroke still being drawn, exactly as
    /// the finished stroke will paint them, with no step of history: `from`
    /// is how many points came before. Each pixel keeps the most coverage
    /// any piece of the stroke gave it, and is painted again from the
    /// layer as it was before the stroke, so pieces never build up.
    pub fn paint_live(&mut self, from: usize, points: &[[f64; 3]], brush: &Brush) {
        self.live_part(from, points, Tool::Brush(*brush));
    }

    /// The clone stamp's or the healing brush's newest points, painted at
    /// once as the brush's are.
    pub fn sample_live(&mut self, from: usize, points: &[[f64; 3]], sampler: &Sampler) {
        self.live_part(from, points, Tool::Sampler(*sampler));
    }

    /// A finished clone stamp or healing stroke, one step. The stamp's
    /// stroke painted as it was drawn becomes the step without painting it
    /// again; healing heals it again with the whole stroke left out of the
    /// light around it, and that is its step.
    pub fn sample_stroke(&mut self, points: &[[f64; 3]], sampler: &Sampler) -> Result<(), Refusal> {
        let tool = Tool::Sampler(*sampler);
        let painted = self.live.as_ref().is_some_and(|live| live.points == points && live.tool == tool && self.active == Some(live.owner));
        if !painted {
            self.end_live();
            self.settle();
            if let Some(refusal) = self.sample_refusal() {
                return Err(refusal);
            }
            self.live_part(0, points, tool);
        }
        self.finish_live();
        Ok(())
    }

    fn live_part(&mut self, from: usize, points: &[[f64; 3]], tool: Tool) {
        if points.is_empty() {
            return;
        }
        if from == 0 {
            self.end_live();
            self.settle();
            let refused = match tool {
                Tool::Brush(brush) => self.paint_refusal(brush.erase),
                Tool::Sampler(_) => self.sample_refusal(),
            };
            if refused.is_some() {
                return;
            }
            let Some(owner) = self.active else {
                return;
            };
            let id = self.target_mask().unwrap_or(owner);
            let Some(before) = self.image.pixels.get(&id).cloned() else {
                return;
            };
            let offset = self.image.buffer_offset(id);
            // A sampler copies from the image as it is now, so the stroke
            // never copies itself.
            let source = match tool {
                Tool::Sampler(_) => {
                    self.image.prepare();
                    self.image.composite_through(owner)
                }
                Tool::Brush(_) => Vec::new(),
            };
            let size = self.image.width as usize * self.image.height as usize;
            let empty = Rect::new(0, 0, 0, 0);
            self.live = Some(Live { id, owner, tool, points: Vec::new(), before, offset, coverage: vec![0.0; size], painted: empty, wanted: empty, source });
        }
        let clip = self.image.bounds();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.points.len() != from || live.tool != tool || self.active != Some(live.owner) {
            return;
        }
        // The pieces the new points add, joined to the last point painted,
        // raise the stroke's coverage so far, kept for the whole image.
        let start = from.saturating_sub(1);
        live.points.extend_from_slice(points);
        let brush = tool.brush();
        let layer = Rect::new(live.offset[0], live.offset[1], live.before.width as i32, live.before.height as i32);
        if let Some(bounds) = paint::stroke_bounds(&live.points[start..], &brush) {
            live.wanted = live.wanted.union(&bounds.intersect(&clip));
        }
        let area = paint::cover(&live.points[start..], &brush, clip, layer, &mut live.coverage);
        if area.is_empty() {
            return;
        }
        self.paint_area(area);
    }

    /// Paints `area` of the stroke being drawn again from the layer as it
    /// was before the stroke, with the coverage so far.
    fn paint_area(&mut self, area: Rect) {
        let keep_alpha = self.live.as_ref().is_some_and(|live| live.id == live.owner && self.image.locks(live.owner).keeps_alpha());
        let (width, height) = (self.image.width, self.image.height);
        let clip = self.image.bounds();
        let Doc { live, image, selection, .. } = self;
        let Some(live) = live.as_mut() else {
            return;
        };
        let Some(pixels) = image.pixels.get_mut(&live.id) else {
            return;
        };
        let layer = Rect::new(live.offset[0], live.offset[1], live.before.width as i32, live.before.height as i32);
        let area = area.intersect(&layer).intersect(&clip);
        if area.is_empty() {
            return;
        }
        let mut values = Vec::with_capacity(area.w as usize * area.h as usize);
        for y in area.y..area.bottom() {
            let row = y as usize * width as usize + area.x as usize;
            values.extend_from_slice(&live.coverage[row..row + area.w as usize]);
        }
        let local = Rect::new(area.x - live.offset[0], area.y - live.offset[1], area.w, area.h);
        pixels.write(local, &live.before.read(local));
        match live.tool {
            Tool::Brush(brush) => {
                // A mask takes the lightness of the brush's colour, and the
                // eraser paints it black.
                let brush = if live.id != live.owner { mask_brush(brush) } else { brush };
                let coverage = paint::Coverage { rect: area, values };
                paint::apply(pixels, live.offset, &coverage, clip, selection.as_ref(), &brush, keep_alpha);
            }
            Tool::Sampler(sampler) if sampler.heal => {
                // The light comes from within the brush's radius, so a
                // nearby edge, such as a hairline, is not pulled in.
                let radius = (sampler.brush.size / 2.0).round().max(1.0) as i32;
                let healer = paint::Healer::new(&live.source, width, height, &live.coverage, area, sampler.offset, radius);
                paint::apply_with(pixels, live.offset, area, &values, |x, y| healer.colour(x, y), selection.as_ref(), 1.0, keep_alpha);
            }
            Tool::Sampler(sampler) => {
                let colours = paint::stamp_colours(&live.source, width, height, area, sampler.offset);
                let opacity = sampler.brush.opacity.clamp(0.0, 1.0) as f32;
                paint::apply_colours(pixels, live.offset, area, &values, &colours, selection.as_ref(), opacity, keep_alpha);
            }
        }
        live.painted = live.painted.union(&area);
        image.forget_area(live.id, Rect::new(area.x - live.offset[0], area.y - live.offset[1], area.w, area.h));
        self.touch(area);
    }

    /// The stroke painted as it was drawn becomes one step. Healing heals
    /// the whole stroke again first, with all of it left out of the light.
    fn finish_live(&mut self) {
        if let Some(Live { tool: Tool::Sampler(Sampler { heal: true, .. }), painted, .. }) = &self.live {
            let painted = *painted;
            self.paint_area(painted);
        }
        let Some(live) = self.live.take() else {
            return;
        };
        let local = Rect::new(live.painted.x - live.offset[0], live.painted.y - live.offset[1], live.painted.w, live.painted.h);
        if let Some(pixels) = self.image.pixels.get(&live.id)
            && !local.is_empty()
        {
            let before = live.before.read(local);
            let after = pixels.read(local);
            if before != after {
                self.commit(vec![Change::Pixels { id: live.id, rect: local, before, after }], None);
            }
        }
    }

    /// A stroke is being drawn on it, or selected pixels moved.
    pub fn drawing(&self) -> bool {
        self.live.is_some() || self.float.is_some()
    }

    /// Takes the stroke being drawn off its layer, if there is one, and
    /// puts back selected pixels being moved.
    pub fn end_live(&mut self) {
        if let Some(float) = self.float.take() {
            let rect = float.rect.union(&Rect::new(float.rect.x + float.delta[0], float.rect.y + float.delta[1], float.rect.w, float.rect.h));
            self.image.pixels.insert(float.id, float.before);
            self.image.forget(float.id);
            self.selection = Some(float.selection);
            self.outlines = None;
            self.touch(rect);
        }
        let Some(live) = self.live.take() else {
            return;
        };
        let local = Rect::new(live.painted.x - live.offset[0], live.painted.y - live.offset[1], live.painted.w, live.painted.h);
        if let Some(pixels) = self.image.pixels.get_mut(&live.id)
            && !local.is_empty()
        {
            pixels.write(local, &live.before.read(local));
            self.image.forget_area(live.id, local);
            self.touch(live.painted);
        }
    }

    /// Why a brush, or the eraser with `erase`, cannot paint on the active
    /// target, in plain words; nothing when it can.
    pub fn paint_refusal(&self, erase: bool) -> Option<Refusal> {
        let Some(node) = self.active.and_then(|id| self.image.find(id)) else {
            return Some("Choose a layer to paint on.".into());
        };
        // A mask is painted whatever its layer is, unless it is locked.
        if self.target_mask().is_some() {
            return self.image.locks(node.id).all.then(|| self.locked_by(node.id, |locks| locks.all)).flatten().map(|name| format!("{name} is locked."));
        }
        if node.is_group() {
            return Some("Choose a layer to paint on; a group holds layers.".into());
        }
        if node.text().is_some() {
            return Some(format!("{} is text: use the T tool.", node.name));
        }
        if node.kind.is_adjustment() {
            return Some(format!("{} is an adjustment: nothing to paint.", node.name));
        }
        if !node.visible {
            return Some(format!("{} is hidden.", node.name));
        }
        let id = node.id;
        let locks = self.image.locks(id);
        if locks.keeps_pixels() {
            return Some(match self.locked_by(id, |locks| locks.all) {
                Some(name) => format!("{name} is locked."),
                None => format!("{}'s pixels are locked.", self.locked_by(id, |locks| locks.pixels)?),
            });
        }
        if erase && locks.keeps_alpha() {
            let name = self.locked_by(id, |locks| locks.transparency || locks.all)?;
            return Some(format!("{name}'s transparency is locked, so it cannot be erased."));
        }
        None
    }

    /// Why the clone stamp or the healing brush cannot paint on the active
    /// target; nothing when it can.
    pub fn sample_refusal(&self) -> Option<Refusal> {
        if self.target_mask().is_some() {
            return Some("The stamp and healing do not act on a mask; choose its layer.".into());
        }
        self.paint_refusal(false)
    }

    /// Why the Move tool cannot move the selected pixels of the active
    /// layer; nothing when it can.
    pub fn move_pixels_refusal(&self) -> Option<Refusal> {
        if self.target_mask().is_some() {
            return Some("Selected pixels move on a layer, not a mask; choose its layer.".into());
        }
        let node = self.active.and_then(|id| self.image.find(id))?;
        if node.text().is_some() {
            return Some(format!("{} is text: it moves whole.", node.name));
        }
        if node.kind.is_adjustment() {
            return Some(format!("{} is an adjustment: nothing to move.", node.name));
        }
        if node.is_group() {
            return Some("Choose a layer to move its selected pixels; a group holds layers.".into());
        }
        if self.image.locks(node.id).keeps_alpha() {
            let name = self.locked_by(node.id, |locks| locks.transparency || locks.all)?;
            return Some(format!("{name}'s transparency is locked, so its pixels cannot leave a hole."));
        }
        self.paint_refusal(false).or_else(|| self.move_refusal())
    }

    /// Why the Move tool cannot move the active layer or group, in plain
    /// words; nothing when it can.
    pub fn move_refusal(&self) -> Option<Refusal> {
        let Some(id) = self.active.filter(|id| self.image.find(*id).is_some()) else {
            return Some("Choose a layer to move.".into());
        };
        let layer = self.image.layers_within(id).into_iter().find(|layer| self.image.locks(*layer).keeps_position())?;
        Some(match self.locked_by(layer, |locks| locks.all) {
            Some(name) => format!("{name} is locked."),
            None => format!("{}'s position is locked.", self.locked_by(layer, |locks| locks.position)?),
        })
    }

    /// The name of the node, or the nearest group around it, whose own locks
    /// hold `lock`.
    fn locked_by(&self, id: u64, lock: impl Fn(&crate::layered::Locks) -> bool) -> Option<String> {
        let mut current = Some(id);
        while let Some(at) = current {
            let node = self.image.find(at)?;
            if lock(&node.locks) {
                return Some(node.name.clone());
            }
            current = self.image.place(at).and_then(|place| place.parent);
        }
        None
    }

    fn touch_all_if_grown(&mut self, changes: &[Change]) {
        if changes.iter().any(|change| matches!(change, Change::Buffer { .. })) {
            self.touch_all();
        }
    }

    /// The visible composite's colour at an image pixel.
    pub fn pick(&mut self, at: [f64; 2]) -> Option<[u8; 4]> {
        let x = at[0].floor();
        let y = at[1].floor();
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        let width = self.image.width as usize;
        if x >= width || y >= self.image.height as usize {
            return None;
        }
        let composite = self.composite();
        let at = (y * width + x) * 4;
        Some([composite[at], composite[at + 1], composite[at + 2], composite[at + 3]])
    }

    // Selecting.

    /// Joins a shape into the selection, feathered. One step.
    pub fn select(&mut self, mut shape: Mask, how: Combine, feather: f64) {
        self.settle();
        shape.feather(feather);
        let before = self.selection.clone();
        let mut next = match (&self.selection, how) {
            (_, Combine::New) | (None, Combine::Add) => Some(shape),
            (None, _) => None,
            (Some(current), _) => {
                let mut current = current.clone();
                current.combine(&shape, how);
                Some(current)
            }
        };
        if next.as_ref().is_some_and(Mask::is_clear) {
            next = None;
        }
        self.set_selection(before, next);
    }

    pub fn deselect(&mut self) {
        self.settle();
        let before = self.selection.clone();
        self.set_selection(before, None);
    }

    pub fn invert(&mut self) {
        self.settle();
        let before = self.selection.clone();
        let Some(mut next) = before.clone() else {
            return;
        };
        next.invert();
        let next = (!next.is_clear()).then_some(next);
        self.set_selection(before, next);
    }

    fn set_selection(&mut self, before: Option<Mask>, after: Option<Mask>) {
        if before == after {
            return;
        }
        self.selection = after.clone();
        self.outlines = None;
        self.commit(vec![Change::Selection { before, after }], None);
    }

    // Moving and cropping.

    /// Moves the active layer, or every layer of the active group. One step.
    pub fn move_active(&mut self, dx: i32, dy: i32) -> Result<(), Refusal> {
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        if let Some(refusal) = self.move_refusal() {
            return Err(refusal);
        }
        let Some(id) = self.active else {
            return Ok(());
        };
        // Every node inside moves, so a group's and a text's masks and boxes
        // go with them.
        let layers = self.image.ids_within(id);
        self.edit_tree(None, |doc| {
            for layer in layers {
                if let Some(node) = doc.image.find_mut(layer) {
                    node.offset = [node.offset[0] + dx, node.offset[1] + dy];
                }
            }
            Ok(())
        })
    }

    /// Crops the image and every layer to `rect`, in image pixels. One
    /// step. A box that covers the whole image changes nothing.
    pub fn crop(&mut self, rect: [f64; 4]) -> Result<(), Refusal> {
        self.settle();
        if rect.iter().any(|value| !value.is_finite()) {
            return Err("The crop box is not a box.".into());
        }
        let x0 = rect[0].round().max(0.0) as i32;
        let y0 = rect[1].round().max(0.0) as i32;
        let x1 = ((rect[0] + rect[2]).round() as i32).min(self.image.width as i32);
        let y1 = ((rect[1] + rect[3]).round() as i32).min(self.image.height as i32);
        let crop = Rect::new(x0, y0, x1 - x0, y1 - y0);
        if crop.w < 1 || crop.h < 1 {
            return Err("The crop box is empty.".into());
        }
        if crop == self.image.bounds() {
            return Ok(());
        }
        let mut changes = Vec::new();
        let before_tree = self.image.nodes.clone();
        let nodes: Vec<(u64, bool, Option<u64>, [i32; 2])> = self
            .image
            .every_node()
            .into_iter()
            .map(|node| (node.id, node.has_pixels(), node.mask.as_ref().map(|mask| mask.id), node.offset))
            .collect();
        for (id, has_pixels, mask, offset) in nodes {
            // A text, an adjustment, or a group keeps its own pixels, if
            // any, and its mask's, and only shifts with the crop's corner.
            if !has_pixels {
                if let Some(node) = self.image.find_mut(id) {
                    node.offset = [offset[0] - crop.x, offset[1] - crop.y];
                }
                continue;
            }
            let Some(layer) = self.image.node_rect(id) else {
                continue;
            };
            let kept = layer.intersect(&crop);
            let (cut, offset) = if kept.is_empty() {
                (Rect::new(0, 0, 1, 1), [0, 0])
            } else {
                (Rect::new(kept.x - layer.x, kept.y - layer.y, kept.w, kept.h), [kept.x - crop.x, kept.y - crop.y])
            };
            let Some(old) = self.image.pixels.get(&id).cloned() else {
                continue;
            };
            let mut new = old.cut(cut);
            if kept.is_empty() {
                new = Pixels::clear(1, 1);
            }
            self.image.pixels.insert(id, new.clone());
            if let Some(node) = self.image.find_mut(id) {
                node.offset = offset;
            }
            changes.push(Change::Buffer { id, before: old, after: new });
            // A layer's mask is cut with it.
            if let Some(mask) = mask
                && let Some(old) = self.image.pixels.get(&mask).cloned()
            {
                let new = if kept.is_empty() { Pixels::filled(1, 1, [255, 255, 255, 255]) } else { old.cut(cut) };
                self.image.pixels.insert(mask, new.clone());
                self.image.forget(mask);
                changes.push(Change::Buffer { id: mask, before: old, after: new });
            }
        }
        changes.push(Change::Tree { before: before_tree, after: self.image.nodes.clone() });
        changes.push(Change::Size {
            before: [self.image.width, self.image.height],
            after: [crop.w as u32, crop.h as u32],
        });
        let selection = self.selection.as_ref().map(|mask| mask.cut(crop)).filter(|mask| !mask.is_clear());
        changes.push(Change::Selection { before: self.selection.clone(), after: selection.clone() });
        self.image.width = crop.w as u32;
        self.image.height = crop.h as u32;
        self.selection = selection;
        self.outlines = None;
        self.touch_all();
        self.commit(changes, None);
        Ok(())
    }

    // Layers.

    /// Where a new layer goes: above the active layer among its siblings,
    /// at the top of the active group, or at the top of the image.
    fn new_place(&self) -> Place {
        let top = Place { parent: None, index: self.image.nodes.len() };
        let Some(id) = self.active else {
            return top;
        };
        match self.image.find(id) {
            Some(node) if node.is_group() => Place {
                parent: Some(id),
                index: node.children.as_ref().map_or(0, Vec::len),
            },
            Some(_) => self.image.place(id).map(|place| Place { index: place.index + 1, ..place }).unwrap_or(top),
            None => top,
        }
    }

    fn refuse_inside_locked(&self, parent: Option<u64>) -> Result<(), Refusal> {
        if let Some(parent) = parent
            && self.image.locks(parent).all
        {
            return Err("That group is locked.".into());
        }
        Ok(())
    }

    /// A clear layer the image's size, active. `name` empty names it
    /// "Layer N". `group` puts it at the top of that group. One step.
    pub fn add_layer(&mut self, name: &str, group: Option<u64>, by: Option<&str>) -> Result<u64, Refusal> {
        let place = match group {
            Some(group) => match self.image.find(group) {
                Some(node) if node.is_group() => Place {
                    parent: Some(group),
                    index: node.children.as_ref().map_or(0, Vec::len),
                },
                Some(_) => return Err(format!("Layer {group} is not a group.")),
                None => return Err(format!("There is no group {group}.")),
            },
            None if by.is_some() => Place { parent: None, index: self.image.nodes.len() },
            None => self.new_place(),
        };
        self.refuse_inside_locked(place.parent)?;
        let pixels = Pixels::clear(self.image.width, self.image.height);
        self.add_node(name, pixels, [0, 0], place, by)
    }

    /// An image as a new layer, centred, above the active layer. One step.
    pub fn add_image(&mut self, name: &str, pixels: Pixels) -> Result<u64, Refusal> {
        let place = self.new_place();
        self.refuse_inside_locked(place.parent)?;
        let offset = [
            (self.image.width as i32 - pixels.width as i32) / 2,
            (self.image.height as i32 - pixels.height as i32) / 2,
        ];
        self.add_node(name, pixels, offset, place, None)
    }

    fn add_node(&mut self, name: &str, pixels: Pixels, offset: [i32; 2], place: Place, by: Option<&str>) -> Result<u64, Refusal> {
        let id = self.image.take_id();
        self.made_layers += 1;
        let name = if name.trim().is_empty() { format!("Layer {}", self.made_layers) } else { name.trim().to_string() };
        self.image.pixels.insert(id, pixels);
        let mut node = Node::layer(id, &name);
        node.offset = offset;
        self.edit_tree(by, |doc| {
            if !doc.image.insert(node, place) {
                return Err("There is nowhere to put the layer.".into());
            }
            doc.active = Some(id);
            Ok(())
        })?;
        Ok(id)
    }

    /// Wraps the active layer in a new group, which keeps it active, or
    /// makes an empty group, active, when none is active. One step.
    pub fn group_active(&mut self) -> Result<u64, Refusal> {
        let id = self.image.take_id();
        self.made_groups += 1;
        let name = format!("Group {}", self.made_groups);
        let active = self.active.filter(|active| self.image.find(*active).is_some());
        if let Some(place) = active.and_then(|active| self.image.place(active)) {
            self.refuse_inside_locked(place.parent)?;
        }
        self.edit_tree(None, |doc| {
            match active {
                Some(active) => {
                    let (node, place) = doc.image.remove(active).ok_or("The active layer is gone.")?;
                    doc.image.insert(Node::group(id, &name, vec![node]), place);
                }
                None => {
                    let at = doc.image.nodes.len();
                    doc.image.insert(Node::group(id, &name, Vec::new()), Place { parent: None, index: at });
                    doc.active = Some(id);
                }
            }
            Ok(())
        })?;
        Ok(id)
    }

    /// Puts a group's layers where the group was. One step.
    pub fn ungroup(&mut self, id: u64) -> Result<(), Refusal> {
        if !self.image.find(id).is_some_and(Node::is_group) {
            return Err(format!("Layer {id} is not a group."));
        }
        self.edit_tree(None, |doc| {
            let (node, place) = doc.image.remove(id).ok_or("The group is gone.")?;
            let children = node.children.unwrap_or_default();
            doc.active = children.last().map(|child| child.id);
            for (offset, child) in children.into_iter().enumerate() {
                doc.image.insert(child, Place { parent: place.parent, index: place.index + offset });
            }
            Ok(())
        })
    }

    /// Deletes a layer or a group with everything in it, even when locked.
    /// The image keeps at least one layer. One step.
    pub fn delete(&mut self, id: u64, by: Option<&str>) -> Result<(), Refusal> {
        let place = self.image.place(id).ok_or_else(|| format!("There is no layer {id}."))?;
        if place.parent.is_none() && self.image.nodes.len() == 1 {
            return Err("The image keeps at least one layer.".into());
        }
        self.edit_tree(by, |doc| {
            doc.image.remove(id);
            if doc.active.is_some_and(|active| doc.image.find(active).is_none()) {
                let siblings = doc.image.siblings(place.parent).cloned().unwrap_or_default();
                doc.active = siblings
                    .get(place.index.saturating_sub(1))
                    .or_else(|| siblings.first())
                    .map(|node| node.id)
                    .or(place.parent)
                    .or_else(|| doc.image.nodes.last().map(|node| node.id));
            }
            Ok(())
        })
    }

    /// Renames a layer, even when locked. One step.
    pub fn rename(&mut self, id: u64, name: &str, by: Option<&str>) -> Result<(), Refusal> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A layer's name cannot be empty.".into());
        }
        self.edit_tree(by, |doc| {
            doc.image.find_mut(id).ok_or_else(|| format!("There is no layer {id}."))?.name = name.to_string();
            Ok(())
        })
    }

    pub fn set_visible(&mut self, id: u64, visible: bool, by: Option<&str>) -> Result<(), Refusal> {
        self.edit_tree(by, |doc| {
            doc.image.find_mut(id).ok_or_else(|| format!("There is no layer {id}."))?.visible = visible;
            Ok(())
        })
    }

    pub fn activate(&mut self, id: u64) {
        self.mask_of = None;
        if self.image.find(id).is_some() && self.active != Some(id) {
            self.settle();
            // Choosing a layer is not a step of its own.
            self.active = Some(id);
        }
    }

    /// Chooses a layer's mask, to paint and to set on the Properties page.
    pub fn choose_mask(&mut self, id: u64) {
        if self.image.find(id).is_some_and(|node| node.mask.is_some()) {
            self.activate(id);
            self.mask_of = Some(id);
        }
    }

    /// The chosen mask's id, when the active layer's mask is chosen.
    pub fn target_mask(&self) -> Option<u64> {
        let owner = self.mask_of.filter(|owner| self.active == Some(*owner))?;
        self.image.find(owner)?.mask.as_ref().map(|mask| mask.id)
    }

    /// Moves a layer among its siblings: `to` counts from the top, as the
    /// Layers list shows them. One step.
    pub fn reorder(&mut self, id: u64, to: usize, by: Option<&str>) -> Result<(), Refusal> {
        let place = self.image.place(id).ok_or_else(|| format!("There is no layer {id}."))?;
        self.move_node(id, place.parent, to, by)
    }

    /// Moves a layer or group into `parent`, a group or the top level, `to`
    /// rows from the top of its children as the Layers list shows them, or
    /// among its siblings when `parent` is where it is. "Lock all" on it or
    /// around it keeps it where it is, and a group locked so takes no new
    /// layers. A group never goes into itself. One step.
    pub fn move_node(&mut self, id: u64, parent: Option<u64>, to: usize, by: Option<&str>) -> Result<(), Refusal> {
        let place = self.image.place(id).ok_or_else(|| format!("There is no layer {id}."))?;
        if self.image.locks(id).all {
            return Err("The layer is locked.".into());
        }
        if let Some(group) = parent {
            match self.image.find(group) {
                Some(node) if node.is_group() => {}
                Some(_) => return Err(format!("Layer {group} is not a group.")),
                None => return Err(format!("There is no group {group}.")),
            }
            if self.image.ids_within(id).contains(&group) {
                return Err("A group cannot go inside itself.".into());
            }
            if parent != place.parent {
                self.refuse_inside_locked(parent)?;
            }
        }
        let count = self.image.siblings(parent).map_or(0, Vec::len);
        // The siblings it goes among, without itself.
        let others = if parent == place.parent { count - 1 } else { count };
        let index = others - to.min(others);
        if parent == place.parent && index == place.index {
            return Ok(());
        }
        self.edit_tree(by, |doc| {
            let (node, _) = doc.image.remove(id).ok_or("The layer is gone.")?;
            doc.image.insert(node, Place { parent, index });
            Ok(())
        })
    }

    /// Turns one of the active layer's locks on or off. One step.
    pub fn toggle_lock(&mut self, id: u64, lock: usize) -> Result<(), Refusal> {
        self.edit_tree(None, |doc| {
            doc.image.find_mut(id).ok_or_else(|| format!("There is no layer {id}."))?.locks.toggle(lock);
            Ok(())
        })
    }

    /// Sets the active layer's blend mode, opacity, or fill. While a slider
    /// is held (`preview`), the changes wait to become one step when it is
    /// let go.
    pub fn set_setting(&mut self, setting: Setting, preview: bool) -> Result<(), Refusal> {
        let id = self.active.ok_or("There is no active layer.")?;
        self.end_typing();
        if self.image.locks(id).all {
            return Err("The layer is locked.".into());
        }
        let mut node = self.image.find(id).ok_or("There is no active layer.")?.clone();
        match setting {
            Setting::Blend(blend) => node.blend = blend,
            Setting::Opacity(value) => node.opacity = value.clamp(0.0, 1.0) as f32,
            Setting::Fill(value) => node.fill = value.clamp(0.0, 1.0) as f32,
        }
        // The value it already has changes nothing, and is no edit.
        if self.image.find(id) == Some(&node) {
            if !preview {
                self.finish_edit();
            }
            return Ok(());
        }
        if self.pending.is_none() {
            self.begin_slider(id);
        }
        *self.image.find_mut(id).ok_or("There is no active layer.")? = node;
        self.edits += 1;
        self.touch_tree();
        if !preview {
            self.finish_edit();
        }
        Ok(())
    }

    /// Ends what is open before another change: the text being typed in
    /// becomes its step, and a held slider its own.
    fn settle(&mut self) {
        self.end_typing();
        self.finish_edit();
    }

    /// Changes a node's settings: while a slider is held (`preview`), the
    /// changes wait to become one step when it is let go.
    /// The settings a node already has change nothing, and are no edit.
    fn set_node(&mut self, id: u64, preview: bool, by: Option<&str>, change: impl FnOnce(&mut Node) -> Result<(), Refusal>) -> Result<(), Refusal> {
        let Some(node) = self.image.find(id) else {
            return Err(format!("There is no layer {id}."));
        };
        if self.image.locks(id).all {
            return Err("The layer is locked.".into());
        }
        let mut node = node.clone();
        change(&mut node)?;
        if self.image.find(id) == Some(&node) {
            if !preview {
                self.settle();
            }
            return Ok(());
        }
        if !preview && self.pending.is_none() {
            return self.edit_tree(by, |doc| {
                *doc.image.find_mut(id).ok_or("The layer is gone.")? = node;
                Ok(())
            });
        }
        self.end_typing();
        if self.pending.is_none() {
            self.begin_slider(id);
        }
        *self.image.find_mut(id).ok_or("The layer is gone.")? = node;
        self.image.sync_texts();
        self.edits += 1;
        self.touch_tree();
        if !preview {
            self.finish_edit();
        }
        Ok(())
    }

    // Masks.

    /// Puts a white mask on a layer, text, adjustment, or group, and
    /// chooses it. One step.
    pub fn add_mask(&mut self, id: u64, by: Option<&str>) -> Result<u64, Refusal> {
        let node = self.image.find(id).ok_or_else(|| format!("There is no layer {id}."))?;
        if node.mask.is_some() {
            return Err(format!("{} already has a mask.", node.name));
        }
        if self.image.locks(id).all {
            return Err(format!("{} is locked.", node.name));
        }
        let name = format!("{} Mask", node.name);
        let (width, height) = self.image.mask_size(id);
        let mask = self.image.take_id();
        self.image.pixels.insert(mask, Pixels::filled(width, height, [255, 255, 255, 255]));
        self.edit_tree(by, |doc| {
            let node = doc.image.find_mut(id).ok_or("The layer is gone.")?;
            node.mask = Some(LayerMask { id: mask, name, feather: 0.0, visible: true });
            doc.active = Some(id);
            Ok(())
        })?;
        self.mask_of = Some(id);
        Ok(mask)
    }

    /// The node a mask or a layer id names, for a mask's settings.
    fn mask_node(&self, id: u64) -> Option<u64> {
        match self.image.find(id) {
            Some(node) => node.mask.as_ref().map(|_| node.id),
            None => self.image.mask_owner(id).map(|node| node.id),
        }
    }

    /// Sets a mask's feather, held as a slider is. `id` is its layer's or
    /// its own.
    pub fn set_feather(&mut self, id: u64, feather: f64, preview: bool, by: Option<&str>) -> Result<(), Refusal> {
        let owner = self.mask_node(id).ok_or("That layer has no mask.")?;
        let feather = if feather.is_finite() { feather.clamp(0.0, 250.0) as f32 } else { 0.0 };
        self.set_node(owner, preview, by, |node| {
            if let Some(mask) = &mut node.mask {
                mask.feather = feather;
            }
            Ok(())
        })
    }

    /// Flips a mask's pixels, white for black. One step.
    pub fn invert_mask(&mut self, id: u64, by: Option<&str>) -> Result<(), Refusal> {
        self.settle();
        let owner = self.mask_node(id).ok_or("That layer has no mask.")?;
        if self.image.locks(owner).all {
            return Err("The layer is locked.".into());
        }
        let mask = self.image.find(owner).and_then(|node| node.mask.as_ref()).map(|mask| mask.id).ok_or("That layer has no mask.")?;
        let before = self.image.pixels.get(&mask).cloned().ok_or("The mask has no pixels.")?;
        let mut after = before.clone();
        for pixel in after.data.chunks_exact_mut(4) {
            let value = 255 - crate::layered::grey(pixel);
            pixel.copy_from_slice(&[value, value, value, 255]);
        }
        self.image.pixels.insert(mask, after.clone());
        self.image.forget(mask);
        self.touch_all();
        self.commit(vec![Change::Buffer { id: mask, before, after }], by);
        Ok(())
    }

    /// Shows or hides a mask: a hidden mask hides nothing. One step.
    pub fn set_mask_visible(&mut self, id: u64, visible: bool) -> Result<(), Refusal> {
        let owner = self.mask_node(id).ok_or("That layer has no mask.")?;
        self.edit_tree(None, |doc| {
            if let Some(mask) = doc.image.find_mut(owner).and_then(|node| node.mask.as_mut()) {
                mask.visible = visible;
            }
            Ok(())
        })
    }

    /// Renames a mask. One step.
    pub fn rename_mask(&mut self, id: u64, name: &str) -> Result<(), Refusal> {
        let owner = self.mask_node(id).ok_or("That layer has no mask.")?;
        let name = name.trim();
        if name.is_empty() {
            return Err("A mask's name cannot be empty.".into());
        }
        self.edit_tree(None, |doc| {
            if let Some(mask) = doc.image.find_mut(owner).and_then(|node| node.mask.as_mut()) {
                mask.name = name.to_string();
            }
            Ok(())
        })
    }

    /// Takes a layer's mask off. One step.
    pub fn delete_mask(&mut self, id: u64, by: Option<&str>) -> Result<(), Refusal> {
        let owner = self.mask_node(id).ok_or("That layer has no mask.")?;
        self.edit_tree(by, |doc| {
            doc.image.find_mut(owner).ok_or("The layer is gone.")?.mask = None;
            Ok(())
        })?;
        self.mask_of = None;
        Ok(())
    }

    // Adjustments.

    /// A Curves or Color Grade layer above `above`, or above the active
    /// layer, chosen. One step.
    pub fn add_adjustment(&mut self, kind: Kind, above: Option<u64>, by: Option<&str>) -> Result<u64, Refusal> {
        let place = match above {
            Some(above) => {
                let place = self.image.place(above).ok_or_else(|| format!("There is no layer {above}."))?;
                Place { index: place.index + 1, ..place }
            }
            None if by.is_some() && self.active.is_none() => Place { parent: None, index: self.image.nodes.len() },
            None => self.new_place(),
        };
        self.refuse_inside_locked(place.parent)?;
        let name = match &kind {
            Kind::Curves(_) => "Curves",
            _ => "Color Grade",
        };
        let id = self.image.take_id();
        self.edit_tree(by, |doc| {
            if !doc.image.insert(Node::of_kind(id, name, kind), place) {
                return Err("There is nowhere to put the layer.".into());
            }
            doc.active = Some(id);
            Ok(())
        })?;
        Ok(id)
    }

    /// Sets one curve of a Curves layer: `channel` 0 for all three, then
    /// red, green, and blue. Held as a slider is.
    pub fn set_curve(&mut self, id: u64, channel: usize, points: &[[f64; 2]], preview: bool, by: Option<&str>) -> Result<(), Refusal> {
        let points = crate::layered::clean_points(points).ok_or("A curve needs at least two points.")?;
        self.set_node(id, preview, by, |node| match &mut node.kind {
            Kind::Curves(curves) => {
                *curves.channel_mut(channel.min(3)) = points;
                Ok(())
            }
            _ => Err(format!("{} is not a Curves layer.", node.name)),
        })
    }

    /// Puts every curve of a Curves layer back straight. One step.
    pub fn reset_adjustment(&mut self, id: u64) -> Result<(), Refusal> {
        self.set_node(id, false, None, |node| match &mut node.kind {
            Kind::Curves(curves) => {
                *curves = Curves::default();
                Ok(())
            }
            Kind::Grade(grade) => {
                *grade = Grade::default();
                Ok(())
            }
            _ => Err(format!("{} is not an adjustment.", node.name)),
        })
    }

    /// Sets a Color Grade layer's settings. Held as a slider is.
    pub fn set_grade(&mut self, id: u64, grade: Grade, preview: bool, by: Option<&str>) -> Result<(), Refusal> {
        self.set_node(id, preview, by, |node| match &mut node.kind {
            Kind::Grade(own) => {
                *own = grade.clamped();
                Ok(())
            }
            _ => Err(format!("{} is not a Color Grade layer.", node.name)),
        })
    }

    // Text.

    /// A new text above the active layer, its box's corner at `at`, being
    /// typed in. Nothing is a step until editing ends with words in it.
    pub fn start_text(&mut self, at: [f64; 2], width: Option<f32>, style: Style) -> Option<u64> {
        self.end_live();
        self.settle();
        if !at[0].is_finite() || !at[1].is_finite() {
            return None;
        }
        let place = self.new_place();
        if self.refuse_inside_locked(place.parent).is_err() {
            return None;
        }
        let before = self.image.nodes.clone();
        let active = self.active;
        let id = self.image.take_id();
        let mut node = Node::of_kind(id, "Text", Kind::Text(Text { words: String::new(), style: style.clamped(), width }));
        node.offset = [at[0].round() as i32, at[1].round() as i32];
        if !self.image.insert(node, place) {
            return None;
        }
        self.active = Some(id);
        self.mask_of = None;
        self.typing = Some(Typing { id, caret: 0, anchor: 0, before, active, named: true });
        self.image.sync_texts();
        self.touch_all();
        Some(id)
    }

    /// Edits a text again, its caret at its end.
    pub fn edit_text(&mut self, id: u64) -> Option<Style> {
        self.end_live();
        self.settle();
        let node = self.image.find(id)?;
        let text = node.text()?.clone();
        if self.image.locks(id).all {
            return None;
        }
        let end = text.words.chars().count();
        self.typing = Some(Typing { id, caret: end, anchor: end, before: self.image.nodes.clone(), active: self.active, named: false });
        self.active = Some(id);
        self.mask_of = None;
        Some(text.style)
    }

    /// The text being typed in, if any.
    pub fn typing(&self) -> Option<u64> {
        self.typing.as_ref().map(|typing| typing.id)
    }

    /// The topmost shown text whose box holds a point.
    pub fn text_at(&self, at: [f64; 2]) -> Option<u64> {
        let mut found = None;
        for node in self.image.every_node() {
            let Some(text) = node.text() else {
                continue;
            };
            if !node.visible {
                continue;
            }
            let layout = text::lay(text);
            let x = at[0] - f64::from(node.offset[0]);
            let y = at[1] - f64::from(node.offset[1]);
            if x >= 0.0 && y >= 0.0 && x <= f64::from(layout.width.max(8.0)) && y <= f64::from(layout.height) {
                found = Some(node.id);
            }
        }
        found
    }

    /// A click with the text tool: outside the text being typed in, it ends
    /// editing; on a text's box, it edits that text, and returns its style;
    /// elsewhere it makes a new text there.
    pub fn text_click(&mut self, at: [f64; 2], style: Style) -> Option<Style> {
        if self.typing.is_some() {
            self.end_typing();
            return None;
        }
        if let Some(id) = self.text_at(at) {
            return self.edit_text(id);
        }
        self.start_text(at, None, style);
        None
    }

    /// The text being typed in, with its words changed: its name follows
    /// them while it is new.
    fn change_words(&mut self, words: String) {
        let Some(typing) = &self.typing else {
            return;
        };
        let (id, named) = (typing.id, typing.named);
        let Some(node) = self.image.find_mut(id) else {
            return;
        };
        if let Kind::Text(text) = &mut node.kind {
            text.words = words;
            if named {
                let first: String = text.words.lines().next().unwrap_or("").trim().chars().take(40).collect();
                node.name = if first.is_empty() { "Text".into() } else { first };
            }
        }
        self.edits += 1;
        if let Some(touched) = self.image.sync_texts() {
            self.touch(touched);
        }
    }

    /// What the keyboard and the pointer do to the text being typed in.
    /// Returns the words for the clipboard, for Copy and Cut.
    pub fn text_input(&mut self, input: &TextInput) -> Option<String> {
        let typing = self.typing.as_ref()?;
        let node = self.image.find(typing.id)?;
        let text = node.text()?.clone();
        let offset = node.offset;
        let layout = text::lay(&text);
        let chars: Vec<char> = text.words.chars().collect();
        let (mut caret, mut anchor) = (typing.caret.min(chars.len()), typing.anchor.min(chars.len()));
        let (low, high) = (caret.min(anchor), caret.max(anchor));
        let local = |at: [f64; 2]| [(at[0] - f64::from(offset[0])) as f32, (at[1] - f64::from(offset[1])) as f32];
        let selected: String = chars[low..high].iter().collect();
        let mut words: Option<Vec<char>> = None;
        let mut clipboard = None;
        let replace = |from: usize, to: usize, with: &[char]| {
            let mut next = chars[..from].to_vec();
            next.extend_from_slice(with);
            next.extend_from_slice(&chars[to..]);
            next
        };
        match input {
            TextInput::Insert { text } => {
                let typed: Vec<char> = text.chars().filter(|c| *c == '\n' || !c.is_control()).map(|c| if c == '\t' { ' ' } else { c }).collect();
                words = Some(replace(low, high, &typed));
                caret = low + typed.len();
                anchor = caret;
            }
            TextInput::Place { at, extend } => {
                caret = layout.hit(local(*at));
                if !extend {
                    anchor = caret;
                }
            }
            TextInput::Select { at } => caret = layout.hit(local(*at)),
            TextInput::Word { at } => {
                let place = layout.hit(local(*at));
                let (start, end) = text::word_at(&text.words, place);
                anchor = start;
                caret = end;
            }
            TextInput::Key { key, shift, word } => {
                let step = |at: usize, forward: bool| {
                    if *word {
                        text::word_step(&text.words, at, forward)
                    } else if forward {
                        (at + 1).min(chars.len())
                    } else {
                        at.saturating_sub(1)
                    }
                };
                match key {
                    TextKey::Backspace | TextKey::Delete => {
                        let forward = *key == TextKey::Delete;
                        let (from, to) = if low < high { (low, high) } else if forward { (caret, step(caret, true)) } else { (step(caret, false), caret) };
                        if from < to {
                            words = Some(replace(from, to, &[]));
                        }
                        caret = from;
                        anchor = from;
                    }
                    TextKey::Left | TextKey::Right => {
                        let forward = *key == TextKey::Right;
                        caret = if low < high && !shift { if forward { high } else { low } } else { step(caret, forward) };
                        if !shift {
                            anchor = caret;
                        }
                    }
                    TextKey::Up | TextKey::Down => {
                        let row = layout.line_of(caret);
                        let x = layout.x_of(caret);
                        caret = match key {
                            TextKey::Up if row == 0 => 0,
                            TextKey::Up => layout.place_on(row - 1, x),
                            _ if row + 1 >= layout.lines.len() => chars.len(),
                            _ => layout.place_on(row + 1, x),
                        };
                        if !shift {
                            anchor = caret;
                        }
                    }
                    TextKey::Home | TextKey::End => {
                        let line = &layout.lines[layout.line_of(caret)];
                        caret = if *key == TextKey::Home { line.start } else { line.end };
                        if !shift {
                            anchor = caret;
                        }
                    }
                    TextKey::Enter => {
                        words = Some(replace(low, high, &['\n']));
                        caret = low + 1;
                        anchor = caret;
                    }
                    TextKey::Escape => {
                        self.end_typing();
                        return None;
                    }
                    TextKey::SelectAll => {
                        anchor = 0;
                        caret = chars.len();
                    }
                    TextKey::Copy => clipboard = (low < high).then(|| selected.clone()),
                    TextKey::Cut => {
                        if low < high {
                            clipboard = Some(selected.clone());
                            words = Some(replace(low, high, &[]));
                            caret = low;
                            anchor = low;
                        }
                    }
                }
            }
        }
        if let Some(typing) = &mut self.typing {
            typing.caret = caret;
            typing.anchor = anchor;
        }
        if let Some(words) = words {
            self.change_words(words.into_iter().collect());
        }
        clipboard
    }

    /// A change to the text being typed in, other than its words: the
    /// typing so far becomes its step first, then the change its own, so
    /// Undo takes them back one at a time. A new text with no words yet
    /// keeps the change in the step that makes it.
    fn typing_step(&mut self, change: impl FnOnce(&mut Text, &mut [i32; 2])) {
        self.settle_typing();
        let Some(id) = self.typing() else {
            return;
        };
        let Some(mut node) = self.image.find(id).cloned() else {
            return;
        };
        if let Kind::Text(text) = &mut node.kind {
            change(text, &mut node.offset);
        }
        // A style or box it already has changes nothing, and is no edit.
        if self.image.find(id) == Some(&node) {
            return;
        }
        if let Some(own) = self.image.find_mut(id) {
            *own = node;
        }
        self.edits += 1;
        self.image.sync_texts();
        self.touch_all();
        self.settle_typing();
    }

    /// The style of the text being typed in, as the settings row changes.
    /// A text layer's style, from its Properties: one step, or held while a
    /// slider is dragged, as other settings are.
    pub fn set_text_style(&mut self, id: u64, style: Style, preview: bool) -> Result<(), Refusal> {
        self.set_node(id, preview, None, |node| {
            let Kind::Text(text) = &mut node.kind else {
                return Err(format!("{} is not a text.", node.name));
            };
            text.style = style.clamped();
            Ok(())
        })
    }

    pub fn set_typing_style(&mut self, style: Style) {
        self.typing_step(|text, _| text.style = style.clamped());
    }

    /// The box of the text being typed in, after a side handle's drag:
    /// `[x, y, width, height]`, the height following the words.
    pub fn text_box(&mut self, rect: [f64; 4]) {
        if rect.iter().any(|value| !value.is_finite()) {
            return;
        }
        self.typing_step(|text, offset| {
            *offset = [rect[0].round() as i32, rect[1].round() as i32];
            text.width = Some(rect[2].abs().max(4.0) as f32);
        });
    }

    /// The typing so far becomes one step, and editing goes on.
    fn settle_typing(&mut self) {
        let Some(typing) = &self.typing else {
            return;
        };
        let empty = self.image.find(typing.id).and_then(Node::text).is_none_or(|text| text.words.is_empty());
        if empty || self.image.nodes == typing.before {
            return;
        }
        let after = self.image.nodes.clone();
        let Some(typing) = self.typing.as_mut() else {
            return;
        };
        let before = std::mem::replace(&mut typing.before, after.clone());
        let active = std::mem::replace(&mut typing.active, self.active);
        self.commit(vec![Change::Tree { before, after }, Change::Active { before: active, after: self.active }], None);
    }

    /// Ends editing: the typing becomes its step, and a text left empty is
    /// taken away.
    pub fn end_typing(&mut self) {
        let Some(typing) = self.typing.take() else {
            return;
        };
        let empty = self.image.find(typing.id).and_then(Node::text).is_some_and(|text| text.words.is_empty());
        if empty {
            self.image.remove(typing.id);
            if self.active == Some(typing.id) {
                self.active = typing.active.filter(|id| self.image.find(*id).is_some()).or_else(|| self.image.nodes.last().map(|node| node.id));
            }
        }
        let mut changes = Vec::new();
        if self.image.nodes != typing.before {
            changes.push(Change::Tree { before: typing.before, after: self.image.nodes.clone() });
        }
        if self.active != typing.active {
            changes.push(Change::Active { before: typing.active, after: self.active });
        }
        self.touch_all();
        self.commit(changes, None);
    }

    /// The text being typed in, as the host draws it.
    pub fn editing(&self) -> Option<Editing> {
        let typing = self.typing.as_ref()?;
        let node = self.image.find(typing.id)?;
        let text = node.text()?;
        let layout = text::lay(text);
        let (x, y) = (f64::from(node.offset[0]), f64::from(node.offset[1]));
        let place = |rect: [f32; 4]| [x + f64::from(rect[0]), y + f64::from(rect[1]), f64::from(rect[2]), f64::from(rect[3])];
        let (low, high) = (typing.caret.min(typing.anchor), typing.caret.max(typing.anchor));
        Some(Editing {
            rect: [x, y, f64::from(layout.width.max(1.0)), f64::from(layout.height)],
            caret: place(layout.caret(typing.caret, text.style.size)),
            selection: if low < high { layout.spans(low, high).into_iter().map(place).collect() } else { Vec::new() },
        })
    }

    /// A text with its words, as the assistant adds one: above the active
    /// layer, or at the top of the image. One step.
    pub fn add_text(&mut self, words: &str, at: [i32; 2], width: Option<f32>, style: Style, by: Option<&str>) -> Result<u64, Refusal> {
        let place = if by.is_some() && self.active.is_none() { Place { parent: None, index: self.image.nodes.len() } } else { self.new_place() };
        self.refuse_inside_locked(place.parent)?;
        let id = self.image.take_id();
        let first: String = words.lines().next().unwrap_or("").trim().chars().take(40).collect();
        let name = if first.is_empty() { "Text".to_string() } else { first };
        let mut node = Node::of_kind(id, &name, Kind::Text(Text { words: words.to_string(), style: style.clamped(), width }));
        node.offset = at;
        self.edit_tree(by, |doc| {
            if !doc.image.insert(node, place) {
                return Err("There is nowhere to put the text.".into());
            }
            doc.active = Some(id);
            Ok(())
        })?;
        Ok(id)
    }

    /// Changes a text's words, style, or box. One step.
    pub fn set_text(&mut self, id: u64, words: Option<&str>, style: Option<Style>, place: Option<([i32; 2], Option<f32>)>, by: Option<&str>) -> Result<(), Refusal> {
        self.set_node(id, false, by, |node| {
            let Kind::Text(text) = &mut node.kind else {
                return Err(format!("{} is not a text.", node.name));
            };
            if let Some(words) = words {
                text.words = words.to_string();
            }
            if let Some(style) = style {
                text.style = style.clamped();
            }
            if let Some((corner, width)) = place {
                node.offset = corner;
                text.width = width;
            }
            Ok(())
        })
    }

    // Moving selected pixels.

    /// The Move tool's drag while it is open, with a selection: the first
    /// lifts the selected pixels off the active layer, leaving it clear
    /// there, and each draws them, and the selection, moved.
    pub fn drag_part(&mut self, from: [f64; 2], to: [f64; 2]) -> Result<(), Refusal> {
        let delta = [(to[0] - from[0]).round(), (to[1] - from[1]).round()];
        if !delta[0].is_finite() || !delta[1].is_finite() {
            return Ok(());
        }
        let delta = [delta[0] as i32, delta[1] as i32];
        if self.float.is_none() {
            self.end_live();
            self.settle();
            if let Some(refusal) = self.move_pixels_refusal() {
                return Err(refusal);
            }
            let (Some(id), Some(selection)) = (self.active, self.selection.clone()) else {
                return Ok(());
            };
            let Some(layer) = self.image.node_rect(id) else {
                return Ok(());
            };
            let rect = selection.bounds().intersect(&layer);
            if rect.is_empty() {
                return Ok(());
            }
            let Some(before) = self.image.pixels.get(&id).cloned() else {
                return Ok(());
            };
            let mut lifted = before.clone();
            let mut piece = Pixels::clear(rect.w as u32, rect.h as u32);
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    let share = f32::from(selection.at(x, y)) / 255.0;
                    if share <= 0.0 {
                        continue;
                    }
                    let at = ((y - layer.y) as usize * before.width as usize + (x - layer.x) as usize) * 4;
                    let alpha = f32::from(before.data[at + 3]);
                    let into = ((y - rect.y) as usize * rect.w as usize + (x - rect.x) as usize) * 4;
                    piece.data[into..into + 3].copy_from_slice(&before.data[at..at + 3]);
                    piece.data[into + 3] = (alpha * share).round() as u8;
                    lifted.data[at + 3] = (alpha * (1.0 - share)).round() as u8;
                }
            }
            let outlines = self.outlines();
            self.image.pixels.insert(id, lifted.clone());
            self.float = Some(Float { id, before, lifted, piece, rect, selection, outlines, delta: [0, 0] });
            self.touch(rect);
            self.place_float(delta, true);
            return Ok(());
        }
        self.place_float(delta, false);
        Ok(())
    }

    /// Draws the lifted pixels moved by `delta` on the layer, and moves the
    /// selection with them.
    fn place_float(&mut self, delta: [i32; 2], first: bool) {
        let Some(float) = self.float.as_mut() else {
            return;
        };
        if float.delta == delta && !first {
            return;
        }
        let offset = self.image.find(float.id).map_or([0, 0], |node| node.offset);
        let Some(pixels) = self.image.pixels.get_mut(&float.id) else {
            return;
        };
        let layer = Rect::new(offset[0], offset[1], pixels.width as i32, pixels.height as i32);
        let moved = |delta: [i32; 2]| Rect::new(float.rect.x + delta[0], float.rect.y + delta[1], float.rect.w, float.rect.h);
        let old = moved(float.delta).intersect(&layer);
        let new = moved(delta).intersect(&layer);
        if !old.is_empty() {
            let local = Rect::new(old.x - layer.x, old.y - layer.y, old.w, old.h);
            pixels.write(local, &float.lifted.read(local));
        }
        for y in new.y..new.bottom() {
            for x in new.x..new.right() {
                let from = ((y - delta[1] - float.rect.y) as usize * float.rect.w as usize + (x - delta[0] - float.rect.x) as usize) * 4;
                let source = &float.piece.data[from..from + 4];
                let amount = f32::from(source[3]) / 255.0;
                if amount <= 0.0 {
                    continue;
                }
                let at = ((y - layer.y) as usize * pixels.width as usize + (x - layer.x) as usize) * 4;
                let below = f32::from(pixels.data[at + 3]) / 255.0;
                let alpha = amount + below * (1.0 - amount);
                for channel in 0..3 {
                    let mixed = f32::from(source[channel]) * amount + f32::from(pixels.data[at + channel]) * below * (1.0 - amount);
                    pixels.data[at + channel] = (mixed / alpha).round().clamp(0.0, 255.0) as u8;
                }
                pixels.data[at + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        float.delta = delta;
        let shift = [f64::from(delta[0]), f64::from(delta[1])];
        let outlines = float.outlines.iter().map(|outline| outline.iter().map(|point| [point[0] + shift[0], point[1] + shift[1]]).collect()).collect();
        self.selection = Some(float.selection.shifted(delta[0], delta[1]));
        self.outlines = Some(outlines);
        let id = float.id;
        self.image.forget(id);
        self.touch(old.union(&new));
    }

    /// The Move tool's drag let go, with a selection: the lifted pixels are
    /// set down where they are, and the move is one step.
    pub fn drop_pixels(&mut self, from: [f64; 2], to: [f64; 2]) -> Result<(), Refusal> {
        self.drag_part(from, to)?;
        let Some(float) = self.float.take() else {
            return Ok(());
        };
        let Some(pixels) = self.image.pixels.get(&float.id) else {
            return Ok(());
        };
        let offset = self.image.find(float.id).map_or([0, 0], |node| node.offset);
        let layer = Rect::new(offset[0], offset[1], pixels.width as i32, pixels.height as i32);
        let moved = Rect::new(float.rect.x + float.delta[0], float.rect.y + float.delta[1], float.rect.w, float.rect.h);
        let area = float.rect.union(&moved).intersect(&layer);
        let local = Rect::new(area.x - layer.x, area.y - layer.y, area.w, area.h);
        let mut changes = Vec::new();
        if !local.is_empty() {
            let (before, after) = (float.before.read(local), pixels.read(local));
            if before != after {
                changes.push(Change::Pixels { id: float.id, rect: local, before, after });
            }
        }
        if self.selection.as_ref() != Some(&float.selection) {
            changes.push(Change::Selection { before: Some(float.selection), after: self.selection.clone() });
        }
        self.commit(changes, None);
        Ok(())
    }

    /// A held slider was let go: its changes become one step.
    pub fn finish_edit(&mut self) {
        let Some((before, active)) = self.pending.take() else {
            return;
        };
        if before != self.image.nodes {
            self.commit(
                vec![
                    Change::Tree { before, after: self.image.nodes.clone() },
                    Change::Active { before: active, after: self.active },
                ],
                None,
            );
        }
    }
}

/// A Curves layer's histograms, one for each channel, and what they were
/// made of: the layer, the pixels' generation, the image's size, and the
/// tree beneath the layer.
struct Histograms {
    id: u64,
    generation: u64,
    size: (u32, u32),
    beneath: Vec<Node>,
    channels: [Vec<f32>; 4],
}

/// The image's histogram for a curve's channel, of every few pixels of
/// `rows`: the share of pixels at each of 128 levels, the fullest drawn as
/// 1.
fn histogram(rows: &[u8], channel: usize) -> Vec<f32> {
    let mut counts = [0u32; 128];
    // Every few pixels, of every few rows, are enough for its shape.
    for pixel in rows.chunks_exact(4).step_by(3) {
        if pixel[3] == 0 {
            continue;
        }
        let value = match channel {
            1 => pixel[0],
            2 => pixel[1],
            3 => pixel[2],
            _ => ((u32::from(pixel[0]) * 54 + u32::from(pixel[1]) * 183 + u32::from(pixel[2]) * 19) / 256) as u8,
        };
        counts[usize::from(value / 2)] += 1;
    }
    let most = counts.iter().copied().max().unwrap_or(0).max(1);
    counts.iter().map(|count| (*count as f32 / most as f32).sqrt()).collect()
}

/// The image kept beneath a node, the pixels' generation it was made
/// from, and whether it was made ahead, while the program was idle, rather
/// than for a held slider.
struct KeptBeneath {
    generation: u64,
    beneath: Beneath,
    ahead: bool,
}

/// The path to the first node of `now` that differs from `then`: how many
/// siblings lie beneath it at each depth, going into a group that changed
/// as far as its children are the same.
fn changed_path(now: &[Node], then: &[Node]) -> Vec<usize> {
    let same = now.iter().zip(then).take_while(|(now, then)| now == then).count();
    let mut path = vec![same];
    if let (Some(Some(now)), Some(Some(then))) = (now.get(same).map(|node| node.children.as_deref()), then.get(same).map(|node| node.children.as_deref())) {
        path.extend(changed_path(now, then));
    }
    path
}

/// The path to node `id`: how many siblings lie beneath it at each depth.
/// A group's goes on into it past all its children, so its own settings
/// change only how it blends.
fn path_to(nodes: &[Node], id: u64) -> Option<Vec<usize>> {
    for (index, node) in nodes.iter().enumerate() {
        if node.id == id {
            return Some(match &node.children {
                Some(children) => vec![index, children.len()],
                None => vec![index],
            });
        }
        if let Some(mut path) = node.children.as_deref().and_then(|children| path_to(children, id)) {
            path.insert(0, index);
            return Some(path);
        }
    }
    None
}

/// Whether anything lies beneath a path, so keeping it saves work.
fn worth_keeping(path: &[usize]) -> bool {
    path.iter().any(|count| *count > 0)
}

/// A generation no image has had: images are copied between galleries,
/// so one image's never matches another's by chance.
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The brush as a mask takes it: the lightness of its colour, and black
/// for the eraser, which hides.
fn mask_brush(brush: Brush) -> Brush {
    let [r, g, b] = brush.rgb;
    let lightness = ((u16::from(r.max(g).max(b)) + u16::from(r.min(g).min(b))) / 2) as u8;
    let grey = if brush.erase { 0 } else { lightness };
    Brush { rgb: [grey; 3], erase: false, ..brush }
}

/// A setting of the Layers page.
#[derive(Debug, Clone, Copy)]
pub enum Setting {
    Blend(Blend),
    Opacity(f64),
    Fill(f64),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush() -> Brush {
        Brush { rgb: [0, 0, 255], size: 8.0, hardness: 1.0, opacity: 1.0, pressure: false, erase: false }
    }

    fn eraser() -> Brush {
        Brush { erase: true, ..brush() }
    }

    fn line() -> Vec<[f64; 3]> {
        vec![[10.0, 20.0, 1.0], [50.0, 20.0, 1.0]]
    }

    /// Catches a stroke that paints outside the selection, or not under its
    /// path, and an undo that does not put every byte back.
    #[test]
    fn a_stroke_paints_under_it_inside_the_selection_and_undoes_exactly() {
        let mut doc = Doc::new(LayeredImage::blank(64, 40));
        let original = doc.composite().to_vec();
        doc.select(Mask::rect(64, 40, [0.0, 0.0], [30.0, 40.0]), Combine::New, 0.0);
        doc.stroke(&line(), &brush()).unwrap();
        let after = doc.composite().to_vec();
        let at = |bytes: &[u8], x: usize, y: usize| bytes[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].to_vec();
        assert_eq!(at(&after, 20, 20), vec![0, 0, 255, 255], "nothing painted under the stroke");
        for y in 0..40 {
            for x in 30..64 {
                assert_eq!(at(&after, x, y), at(&original, x, y), "painted outside the selection at {x},{y}");
            }
        }
        assert_eq!(at(&after, 20, 2), at(&original, 20, 2), "painted far from the stroke");
        assert!(doc.undo());
        assert_eq!(doc.composite(), &original[..], "undo left a difference");
        assert!(doc.undo(), "the selection was not its own step");
        assert!(doc.selection.is_none());
        assert!(doc.redo() && doc.redo());
        assert_eq!(doc.composite(), &after[..]);
    }

    /// Catches a stroke that looks different while it is drawn than when it
    /// is laid, or that builds up where its parts meet: a soft, see-through
    /// stroke crossing itself, painted part by part as the host sends it,
    /// is byte for byte the same stroke painted at once, one undo step, and
    /// the composite tells only the part it changed. Let go with Escape, it
    /// leaves nothing.
    #[test]
    fn a_stroke_painted_as_it_is_drawn_is_the_stroke_laid() {
        let soft = Brush { size: 14.0, hardness: 0.2, opacity: 0.5, ..brush() };
        let points: Vec<[f64; 3]> = (0..40)
            .map(|i| {
                let t = f64::from(i) / 39.0 * std::f64::consts::TAU;
                [40.0 + 25.0 * t.sin(), 30.0 + 18.0 * (2.0 * t).sin(), 1.0]
            })
            .collect();
        let mut whole = Doc::new(LayeredImage::blank(80, 60));
        whole.stroke(&points, &soft).unwrap();
        let laid = whole.composite().to_vec();

        let mut doc = Doc::new(LayeredImage::blank(80, 60));
        let original = doc.composite().to_vec();
        doc.take_changed();
        let mut sent = 0;
        for chunk in points.chunks(7) {
            doc.paint_live(sent, chunk, &soft);
            sent += chunk.len();
        }
        assert_eq!(doc.composite(), &laid[..], "the stroke drawn differs from the stroke laid");
        let changed = doc.take_changed().expect("the stroke changed nothing");
        assert!(changed.w < 80 && changed.h < 60, "the whole image was sent again: {changed:?}");
        doc.stroke(&points, &soft).unwrap();
        assert_eq!(doc.composite(), &laid[..], "letting go changed the stroke");
        assert!(doc.undo());
        assert_eq!(doc.composite(), &original[..], "the stroke was more than one step");
        assert!(!doc.can_undo());

        doc.paint_live(0, &points, &soft);
        assert_ne!(doc.composite(), &original[..]);
        doc.end_live();
        assert_eq!(doc.composite(), &original[..], "a stroke let go stayed on the image");
        assert!(!doc.can_undo());
    }

    /// Catches each lock not refusing what it locks, and refusing what it
    /// does not.
    #[test]
    fn each_lock_refuses_what_it_locks() {
        let mut doc = Doc::new(LayeredImage::blank(32, 32));
        let id = doc.active.unwrap();
        let set = |doc: &mut Doc, lock: usize| {
            doc.image.find_mut(id).unwrap().locks = Default::default();
            doc.toggle_lock(id, lock).unwrap();
        };
        let points = [[16.0, 16.0, 1.0]];

        // Transparency: painting keeps alpha, erasing is refused.
        set(&mut doc, 0);
        let pixel = (16 * 32 + 16) * 4;
        doc.image.pixels.get_mut(&id).unwrap().data[pixel + 3] = 0;
        doc.stroke(&points, &brush()).unwrap();
        assert_eq!(doc.image.pixels[&id].data[pixel + 3], 0, "painting made a clear pixel show");
        assert_eq!(doc.image.pixels[&id].data[pixel], 0, "painting changed no colour");
        assert!(doc.stroke(&points, &eraser()).is_err());
        assert!(doc.move_active(3, 0).is_ok());

        // Pixels: no painting, no erasing; moving is fine.
        set(&mut doc, 1);
        assert!(doc.stroke(&points, &brush()).is_err());
        assert!(doc.stroke(&points, &eraser()).is_err());
        assert!(doc.move_active(3, 0).is_ok());

        // Position: no moving; painting is fine.
        set(&mut doc, 2);
        assert!(doc.move_active(3, 0).is_err());
        assert!(doc.stroke(&points, &brush()).is_ok());

        // All: none of the three, nor settings; renaming and deleting stay.
        set(&mut doc, 3);
        assert!(doc.stroke(&points, &brush()).is_err());
        assert!(doc.stroke(&points, &eraser()).is_err());
        assert!(doc.move_active(3, 0).is_err());
        assert!(doc.set_setting(Setting::Opacity(0.5), false).is_err());
        assert!(doc.rename(id, "Kept", None).is_ok());
        doc.add_layer("", None, None).unwrap();
        assert!(doc.delete(id, None).is_ok());
    }

    /// Catches a move that does not change the offset, a crop that leaves a
    /// size or offset behind, and a crop that cannot be undone.
    #[test]
    fn move_and_crop_change_offsets_and_sizes() {
        let mut doc = Doc::new(LayeredImage::blank(100, 80));
        let background = doc.active.unwrap();
        let layer = doc.add_layer("Top", None, None).unwrap();
        doc.move_active(7, -3).unwrap();
        assert_eq!(doc.image.find(layer).unwrap().offset, [7, -3]);
        assert_eq!(doc.image.find(background).unwrap().offset, [0, 0]);
        let before = doc.composite().to_vec();
        doc.crop([10.0, 20.0, 50.0, 40.0]).unwrap();
        assert_eq!((doc.image.width, doc.image.height), (50, 40));
        assert_eq!(doc.composite().len(), 50 * 40 * 4);
        let pixels = &doc.image.pixels[&background];
        assert_eq!((pixels.width, pixels.height), (50, 40));
        assert_eq!(doc.image.find(background).unwrap().offset, [0, 0]);
        assert_eq!(doc.image.find(layer).unwrap().offset, [0, 0]);
        doc.undo();
        assert_eq!((doc.image.width, doc.image.height), (100, 80));
        assert_eq!(doc.image.find(layer).unwrap().offset, [7, -3]);
        assert_eq!(doc.composite(), &before[..]);
    }

    /// How long a stroke and its composite take on a new 1920 × 1080 image
    /// with a few layers. Run with `--release --ignored`; a debug build is
    /// many times slower.
    #[test]
    #[ignore]
    fn a_stroke_composites_quickly() {
        let mut doc = Doc::new(LayeredImage::blank(1920, 1080));
        doc.add_layer("A", None, None).unwrap();
        doc.add_layer("B", None, None).unwrap();
        doc.refresh();
        let points: Vec<[f64; 3]> = (0..200).map(|step| [200.0 + f64::from(step) * 6.0, 500.0 + f64::from(step % 40), 0.8]).collect();
        let started = std::time::Instant::now();
        doc.stroke(&points, &Brush { size: 40.0, ..brush() }).unwrap();
        doc.refresh();
        let stroke = started.elapsed();
        let started = std::time::Instant::now();
        doc.touch_all();
        doc.refresh();
        let whole = started.elapsed();
        eprintln!("stroke and its composite: {stroke:?}; the whole composite: {whole:?}");
        if !cfg!(debug_assertions) {
            assert!(stroke < std::time::Duration::from_millis(100), "{stroke:?}");
        }
    }

    /// Catches a layer moved off part of the image that can no longer be
    /// painted there.
    #[test]
    fn a_moved_layer_still_takes_paint_everywhere() {
        let mut doc = Doc::new(LayeredImage::blank(40, 40));
        let id = doc.add_layer("Top", None, None).unwrap();
        doc.move_active(20, 0).unwrap();
        doc.stroke(&[[5.0, 5.0, 1.0]], &brush()).unwrap();
        assert_eq!(&doc.composite()[(5 * 40 + 5) * 4..(5 * 40 + 5) * 4 + 4], &[0, 0, 255, 255]);
        assert_eq!(doc.image.find(id).unwrap().offset, [0, 0]);
        doc.undo();
        assert_eq!(doc.image.find(id).unwrap().offset, [20, 0]);
        assert_eq!(&doc.composite()[(5 * 40 + 5) * 4..(5 * 40 + 5) * 4 + 4], &[255, 255, 255, 255]);
    }

    /// Catches a held slider that makes one step per value, and a group of
    /// the assistant's calls that does not undo as one.
    #[test]
    fn a_held_slider_and_a_request_are_one_step_each() {
        let mut doc = Doc::new(LayeredImage::blank(8, 8));
        for value in [0.9, 0.7, 0.5] {
            doc.set_setting(Setting::Opacity(value), true).unwrap();
        }
        doc.finish_edit();
        assert!(doc.undo());
        assert!(!doc.can_undo());
        assert_eq!(doc.image.nodes[0].opacity, 1.0);

        let a = doc.add_layer("A", None, Some("request-1")).unwrap();
        doc.rename(a, "B", Some("request-1")).unwrap();
        doc.add_layer("C", None, Some("request-2")).unwrap();
        assert!(doc.undo());
        assert_eq!(doc.image.nodes.len(), 2);
        assert!(doc.undo());
        assert_eq!(doc.image.nodes.len(), 1, "one request took two undos");
    }

    /// Catches reorder counting from the bottom when the list counts from
    /// the top, and groups that lose their layers.
    #[test]
    fn layers_reorder_group_and_ungroup() {
        let mut doc = Doc::new(LayeredImage::blank(8, 8));
        let background = doc.active.unwrap();
        let a = doc.add_layer("A", None, None).unwrap();
        let b = doc.add_layer("B", None, None).unwrap();
        let order = |doc: &Doc| doc.image.nodes.iter().map(|node| node.id).collect::<Vec<_>>();
        assert_eq!(order(&doc), vec![background, a, b]);
        doc.reorder(background, 0, None).unwrap();
        assert_eq!(order(&doc), vec![a, b, background]);
        doc.activate(a);
        let group = doc.group_active().unwrap();
        assert_eq!(doc.image.find(group).unwrap().children.as_ref().unwrap()[0].id, a);
        let inside = doc.add_layer("Inside", None, None).unwrap();
        assert_eq!(doc.image.place(inside).unwrap().parent, Some(group));
        doc.ungroup(group).unwrap();
        assert_eq!(order(&doc), vec![a, inside, b, background]);
        doc.delete(b, None).unwrap();
        assert!(doc.image.find(b).is_none());
        doc.undo();
        assert!(doc.image.find(b).is_some());
    }
    /// Catches a layer that cannot go into a group or come out of one, a
    /// move that takes more than one undo, and the folder button taking the
    /// active layer away from the layer it wrapped.
    #[test]
    fn layers_go_into_and_out_of_groups_in_one_step() {
        let mut doc = Doc::new(LayeredImage::blank(8, 8));
        let background = doc.active.unwrap();
        let a = doc.add_layer("A", None, None).unwrap();
        let b = doc.add_layer("B", None, None).unwrap();
        doc.activate(a);
        let group = doc.group_active().unwrap();
        assert_eq!(doc.active, Some(a), "the folder button took the wrapped layer's place as active");
        let order = |doc: &Doc, parent: Option<u64>| doc.image.siblings(parent).unwrap().iter().map(|node| node.id).collect::<Vec<_>>();
        assert_eq!(order(&doc, None), vec![background, group, b]);

        // Into the group, at its top.
        doc.move_node(background, Some(group), 0, None).unwrap();
        assert_eq!(order(&doc, Some(group)), vec![a, background]);
        assert_eq!(order(&doc, None), vec![group, b]);
        assert!(doc.undo());
        assert_eq!(order(&doc, None), vec![background, group, b], "moving into a group took more than one undo");
        assert_eq!(order(&doc, Some(group)), vec![a]);
        assert!(doc.redo());

        // Out of the group, between the two top-level rows: B at the top,
        // then it, then the group.
        doc.move_node(a, None, 1, None).unwrap();
        assert_eq!(order(&doc, None), vec![group, a, b]);
        assert_eq!(order(&doc, Some(group)), vec![background]);
        assert!(doc.undo());
        assert_eq!(order(&doc, Some(group)), vec![a, background]);
        assert_eq!(order(&doc, None), vec![group, b]);
    }

    /// Catches a group put inside itself or its own group, and locks that do
    /// not keep layers where they are.
    #[test]
    fn moving_into_a_group_honours_locks_and_never_nests_a_group_in_itself() {
        let mut doc = Doc::new(LayeredImage::blank(8, 8));
        let background = doc.active.unwrap();
        let outer = doc.group_active().unwrap();
        doc.activate(outer);
        let inner = doc.add_layer("", None, None).unwrap();
        doc.activate(inner);
        let inner_group = doc.group_active().unwrap();
        let before = doc.image.nodes.clone();
        assert!(doc.move_node(outer, Some(outer), 0, None).is_err(), "a group went into itself");
        assert!(doc.move_node(outer, Some(inner_group), 0, None).is_err(), "a group went into its own group");
        assert_eq!(doc.image.nodes, before);

        let other = doc.add_layer("Other", None, Some("top")).unwrap();
        // A group locked whole takes no new layers.
        doc.toggle_lock(inner_group, 3).unwrap();
        assert!(doc.move_node(other, Some(inner_group), 0, None).is_err(), "a locked group took a layer");
        // Nor does a layer in it leave.
        assert!(doc.move_node(inner, None, 0, None).is_err(), "a layer left a locked group");
        doc.toggle_lock(inner_group, 3).unwrap();
        // A layer locked whole stays where it is.
        doc.toggle_lock(other, 3).unwrap();
        assert!(doc.move_node(other, Some(outer), 0, None).is_err(), "a locked layer moved");
        doc.toggle_lock(other, 3).unwrap();
        doc.move_node(other, Some(outer), 0, None).unwrap();
        assert_eq!(doc.image.place(other).unwrap().parent, Some(outer));
        assert_eq!(doc.image.place(background).unwrap().parent, Some(outer));
    }

    /// Catches a refused stroke or move with no reason to show, or one that
    /// names the wrong thing.
    #[test]
    fn refusals_say_why_in_plain_words() {
        let mut doc = Doc::new(LayeredImage::blank(8, 8));
        let background = doc.active.unwrap();
        assert_eq!(doc.paint_refusal(false), None);
        assert_eq!(doc.move_refusal(), None);
        doc.toggle_lock(background, 3).unwrap();
        assert_eq!(doc.paint_refusal(false).as_deref(), Some("Background is locked."));
        assert_eq!(doc.move_refusal().as_deref(), Some("Background is locked."));
        doc.toggle_lock(background, 3).unwrap();
        doc.toggle_lock(background, 0).unwrap();
        assert_eq!(doc.paint_refusal(false), None);
        assert_eq!(doc.paint_refusal(true).as_deref(), Some("Background's transparency is locked, so it cannot be erased."));
        doc.toggle_lock(background, 0).unwrap();
        doc.toggle_lock(background, 2).unwrap();
        assert_eq!(doc.move_refusal().as_deref(), Some("Background's position is locked."));
        assert_eq!(doc.paint_refusal(true), None);
        let group = doc.group_active().unwrap();
        doc.activate(group);
        assert_eq!(doc.paint_refusal(false).as_deref(), Some("Choose a layer to paint on; a group holds layers."));
        assert_eq!(doc.move_refusal().as_deref(), Some("Background's position is locked."));
        doc.activate(background);
        doc.set_visible(background, false, None).unwrap();
        assert_eq!(doc.paint_refusal(false).as_deref(), Some("Background is hidden."));
    }

    /// An image whose background is `fill` with `paint` run on each
    /// pixel, `x` and `y` given.
    fn painted(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 4]) -> Doc {
        let mut image = LayeredImage::blank(width, height);
        let id = image.nodes[0].id;
        let pixels = image.pixels.get_mut(&id).unwrap();
        for y in 0..height {
            for x in 0..width {
                let at = ((y * width + x) * 4) as usize;
                pixels.data[at..at + 4].copy_from_slice(&paint(x, y));
            }
        }
        Doc::new(image)
    }

    fn pixel_of(doc: &mut Doc, x: usize, y: usize) -> [u8; 4] {
        let width = doc.image.width as usize;
        let composite = doc.composite();
        let at = (y * width + x) * 4;
        [composite[at], composite[at + 1], composite[at + 2], composite[at + 3]]
    }

    fn sampler(size: f64, offset: [i32; 2], heal: bool) -> Sampler {
        Sampler { brush: Brush { size, hardness: 1.0, ..brush() }, offset, heal }
    }

    /// Catches a stamp that only looks like one: stamping from a red square
    /// onto an empty layer makes red pixels there; under a Curves layer it
    /// copies the grey beneath, not the graded grey, so the curve applies
    /// once; and painted part by part as the host sends it, the stroke never
    /// copies its own new pixels.
    #[test]
    fn the_stamp_copies_the_image_beneath_and_never_itself() {
        let mut doc = painted(60, 40, |x, y| if (5..15).contains(&x) && (5..15).contains(&y) { [255, 0, 0, 255] } else { [255, 255, 255, 255] });
        let retouch = doc.add_layer("Retouch", None, None).unwrap();
        doc.sample_stroke(&[[40.0, 10.0, 1.0]], &sampler(6.0, [-30, 0], false)).unwrap();
        assert_eq!(pixel_of(&mut doc, 40, 10), [255, 0, 0, 255], "the stamp made no red");
        let at = ((10 * 60 + 40) * 4) as usize;
        assert_eq!(&doc.image.pixels[&retouch].data[at..at + 4], &[255, 0, 0, 255], "the red is not on the retouch layer");
        assert_eq!(pixel_of(&mut doc, 40, 30), [255, 255, 255, 255], "the stamp painted outside its stroke");
        assert!(doc.undo());
        assert_eq!(pixel_of(&mut doc, 40, 10), [255, 255, 255, 255], "the stamp was more than one step");

        // Grey under a Curves layer that brightens it.
        let mut doc = painted(60, 40, |_, _| [128, 128, 128, 255]);
        let retouch = doc.add_layer("Retouch", None, None).unwrap();
        let curves = doc.add_adjustment(Kind::Curves(Curves::default()), None, None).unwrap();
        doc.set_curve(curves, 0, &[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]], false, None).unwrap();
        doc.activate(retouch);
        doc.sample_stroke(&[[40.0, 10.0, 1.0]], &sampler(6.0, [-30, 0], false)).unwrap();
        let at = ((10 * 60 + 40) * 4) as usize;
        assert_eq!(&doc.image.pixels[&retouch].data[at..at + 4], &[128, 128, 128, 255], "the stamp copied the graded grey");
        let graded = pixel_of(&mut doc, 40, 10);
        assert!((190..=193).contains(&graded[0]), "the curve applied {} times over", graded[0]);

        // A red bar, and a stroke beside it copying from 10 pixels left:
        // only its first pixels lie over the bar's copy.
        let mut doc = painted(80, 40, |x, _| if (8..12).contains(&x) { [255, 0, 0, 255] } else { [255, 255, 255, 255] });
        doc.add_layer("Retouch", None, None).unwrap();
        let points: Vec<[f64; 3]> = (18..60).map(|x| [f64::from(x), 20.0, 1.0]).collect();
        let tool = sampler(4.0, [-10, 0], false);
        let mut sent = 0;
        for chunk in points.chunks(2) {
            doc.sample_live(sent, chunk, &tool);
            sent += chunk.len();
        }
        doc.sample_stroke(&points, &tool).unwrap();
        assert_eq!(pixel_of(&mut doc, 20, 20), [255, 0, 0, 255]);
        for x in [26, 32, 40, 50] {
            assert_eq!(pixel_of(&mut doc, x, 20), [255, 255, 255, 255], "the stroke copied its own red at {x}");
        }
    }

    /// The mean and spread of a channel over a square.
    fn spread(doc: &mut Doc, center: [usize; 2], half: usize) -> (f64, f64) {
        let mut values = Vec::new();
        for y in center[1] - half..=center[1] + half {
            for x in center[0] - half..=center[0] + half {
                values.push(f64::from(pixel_of(doc, x, y)[1]));
            }
        }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let deviation = (values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt();
        (mean, deviation)
    }

    /// Catches healing that is only cloning: from a dark textured source
    /// into a light flat area, the healed pixels take the light around
    /// them, near the destination's mean, and keep the source's texture.
    #[test]
    fn healing_takes_the_light_where_it_lands_and_the_detail_it_copies() {
        let texture = |x: u32, y: u32| ((x * 7 + y * 13) % 5) as i32 * 10 - 20;
        let mut doc = painted(120, 60, |x, y| {
            let value = if x < 60 { (60 + texture(x, y)) as u8 } else { 200 };
            [value, value, value, 255]
        });
        let (source_mean, source_detail) = spread(&mut doc, [30, 30], 3);
        assert!(source_mean < 80.0 && source_detail > 8.0);
        let points: Vec<[f64; 3]> = (84..=96).map(|x| [f64::from(x), 30.0, 1.0]).collect();
        doc.sample_stroke(&points, &sampler(12.0, [-60, 0], true)).unwrap();
        let (mean, detail) = spread(&mut doc, [90, 30], 3);
        assert!((mean - 200.0).abs() < 12.0, "the healed pixels' mean is {mean}, not the destination's 200");
        assert!(detail > source_detail * 0.6, "the source's detail was lost: {detail} against {source_detail}");
        assert!(doc.undo());
        assert!(!doc.can_undo(), "healing was more than one step");
    }

    /// Catches healing that brings the blemish back: a dark spot on a light
    /// textured area is gone after one dab, its contrast with the ring
    /// around it small, as the spot's own pixels are left out of the light.
    #[test]
    fn healing_removes_a_dark_spot() {
        let texture = |x: u32, y: u32| ((x * 7 + y * 13) % 5) as i32 * 5 - 10;
        let spot = |x: u32, y: u32| (f64::from(x) - 70.0).powi(2) + (f64::from(y) - 30.0).powi(2) <= 36.0;
        let mut doc = painted(110, 60, |x, y| {
            let value = if spot(x, y) { 20 } else { (190 + texture(x, y)) as u8 };
            [value, value, value, 255]
        });
        let contrast = |doc: &mut Doc| {
            let mut inside = Vec::new();
            let mut ring = Vec::new();
            for y in 14..46usize {
                for x in 54..86usize {
                    let distance = ((x as f64 - 70.0).powi(2) + (y as f64 - 30.0).powi(2)).sqrt();
                    let value = f64::from(pixel_of(doc, x, y)[1]);
                    if distance <= 6.0 {
                        inside.push(value);
                    } else if (11.0..=15.0).contains(&distance) {
                        ring.push(value);
                    }
                }
            }
            let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
            (mean(&inside) - mean(&ring)).abs()
        };
        assert!(contrast(&mut doc) > 100.0);
        doc.sample_stroke(&[[70.0, 30.0, 1.0]], &sampler(20.0, [-45, 0], true)).unwrap();
        let after = contrast(&mut doc);
        assert!(after < 4.0, "the spot is still {after} darker than around it");
    }

    /// Catches healing that pulls in a nearby edge's colour: a dark spot
    /// 0.9 brush sizes from a dark band on a light field heals to within a
    /// few levels of the light around it, not darker.
    #[test]
    fn healing_near_an_edge_keeps_the_light_side() {
        let texture = |x: u32, y: u32| ((x * 7 + y * 13) % 5) as i32 * 5 - 10;
        let spot = |x: u32, y: u32| (f64::from(x) - 68.0).powi(2) + (f64::from(y) - 30.0).powi(2) <= 36.0;
        let mut doc = painted(130, 60, |x, y| {
            let value = if x < 50 { 40 } else if spot(x, y) { 20 } else { (190 + texture(x, y)) as u8 };
            [value, value, value, 255]
        });
        doc.sample_stroke(&[[68.0, 30.0, 1.0]], &sampler(20.0, [35, 0], true)).unwrap();
        let mut inside = Vec::new();
        for y in 24..37usize {
            for x in 62..75usize {
                if (x as f64 - 68.0).powi(2) + (y as f64 - 30.0).powi(2) <= 25.0 {
                    inside.push(f64::from(pixel_of(&mut doc, x, y)[1]));
                }
            }
        }
        let mean = inside.iter().sum::<f64>() / inside.len() as f64;
        assert!((mean - 190.0).abs() <= 8.0, "the healed spot is {mean}, pulled from the light side's 190 by the dark edge");
    }

    /// Catches a move that copies, or moves the whole layer: after a drag
    /// the selection's old place is clear, the pixels are at the new place,
    /// the rest of the layer is as it was, the selection moved with them,
    /// and one undo restores it all. Escape puts lifted pixels back.
    #[test]
    fn moving_selected_pixels_leaves_a_hole_and_undoes_in_one_step() {
        let mut doc = Doc::new(LayeredImage::blank(40, 30));
        let top = doc.add_layer("Top", None, None).unwrap();
        let blue = [0, 0, 255, 255];
        let red = [255, 0, 0, 255];
        {
            let pixels = doc.image.pixels.get_mut(&top).unwrap();
            for y in 5..10 {
                for x in 5..10 {
                    let at = (y * 40 + x) * 4;
                    pixels.data[at..at + 4].copy_from_slice(&blue);
                }
            }
            let at = (25 * 40 + 35) * 4;
            pixels.data[at..at + 4].copy_from_slice(&red);
        }
        doc.touch_all();
        let original = doc.image.pixels[&top].clone();
        let before = doc.composite().to_vec();
        doc.select(Mask::rect(40, 30, [4.0, 4.0], [12.0, 12.0]), Combine::New, 0.0);
        let layer = |doc: &Doc, x: usize, y: usize| doc.image.pixels[&top].data[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4].to_vec();

        let steps = doc.undo.len();
        doc.drag_part([6.0, 6.0], [10.0, 6.0]).unwrap();
        doc.drag_part([6.0, 6.0], [26.0, 11.0]).unwrap();
        assert_eq!(layer(&doc, 27, 12), blue.to_vec(), "the pixels do not follow the drag");
        doc.drop_pixels([6.0, 6.0], [26.0, 11.0]).unwrap();
        assert_eq!(doc.undo.len(), steps + 1, "the move was more than one step");
        assert_eq!(layer(&doc, 7, 7)[3], 0, "the old place is not clear");
        assert_eq!(layer(&doc, 27, 12), blue.to_vec(), "the pixels are not at the new place");
        assert_eq!(layer(&doc, 35, 25), red.to_vec(), "the rest of the layer changed");
        assert_eq!(pixel_of(&mut doc, 7, 7), [255, 255, 255, 255]);
        let selection = doc.selection.clone().unwrap();
        assert_eq!((selection.at(26, 11), selection.at(5, 5)), (255, 0), "the selection did not move with the pixels");
        assert!(doc.undo());
        assert_eq!(doc.image.pixels[&top], original, "one undo did not put the pixels back");
        assert_eq!(doc.selection.as_ref().unwrap().at(5, 5), 255);
        assert_eq!(doc.composite(), &before[..]);

        // Escape puts them back before they are set down.
        doc.drag_part([6.0, 6.0], [20.0, 6.0]).unwrap();
        assert_ne!(doc.image.pixels[&top], original);
        doc.end_live();
        assert_eq!(doc.image.pixels[&top], original, "Escape left the pixels moved");
        assert_eq!(doc.selection.as_ref().unwrap().at(5, 5), 255);
    }

    /// Catches a mask that does nothing, or masks the wrong layer: a white
    /// mask shows its layer, black painted into it hides only that layer,
    /// Invert flips it, the eraser paints black, and adding a layer while
    /// it is chosen, then moving its layer, keeps it on its layer.
    #[test]
    fn a_mask_hides_its_own_layer_where_it_is_black() {
        let mut doc = painted(40, 30, |_, _| [255, 255, 255, 255]);
        let red = doc.add_layer("Red", None, None).unwrap();
        doc.image.pixels.insert(red, Pixels::filled(40, 30, [255, 0, 0, 255]));
        doc.touch_all();
        let mask = doc.add_mask(red, None).unwrap();
        assert_eq!(doc.target_mask(), Some(mask));
        assert_eq!(pixel_of(&mut doc, 10, 10), [255, 0, 0, 255], "a white mask hid its layer");
        let black = Brush { rgb: [0, 0, 0], ..brush() };
        doc.stroke(&[[10.0, 10.0, 1.0], [30.0, 10.0, 1.0]], &black).unwrap();
        assert_eq!(pixel_of(&mut doc, 20, 10), [255, 255, 255, 255], "black did not hide the layer");
        assert_eq!(pixel_of(&mut doc, 20, 25), [255, 0, 0, 255], "the mask hid more than was painted");
        assert_eq!(doc.image.pixels[&red].data[(10 * 40 + 20) * 4], 255, "painting the mask painted the layer");
        doc.invert_mask(red, None).unwrap();
        assert_eq!(pixel_of(&mut doc, 20, 10), [255, 0, 0, 255], "Invert did not flip the mask");
        assert_eq!(pixel_of(&mut doc, 20, 25), [255, 255, 255, 255]);
        assert!(doc.undo());
        // The eraser paints black, which hides.
        doc.stroke(&[[5.0, 25.0, 1.0]], &eraser()).unwrap();
        assert_eq!(pixel_of(&mut doc, 5, 25), [255, 255, 255, 255], "the eraser did not hide");

        // A layer added while the mask is chosen goes above, without it,
        // and the mask moves with its own layer.
        let added = doc.add_layer("Added", None, None).unwrap();
        assert!(doc.image.find(added).unwrap().mask.is_none(), "the new layer took the mask");
        assert_eq!(doc.image.find(red).unwrap().mask.as_ref().map(|mask| mask.id), Some(mask));
        assert_eq!(doc.target_mask(), None);
        doc.activate(red);
        doc.move_active(0, 4).unwrap();
        assert_eq!(pixel_of(&mut doc, 20, 14), [255, 255, 255, 255], "the mask stayed behind its layer");
        assert_eq!(pixel_of(&mut doc, 20, 9), [255, 0, 0, 255]);
    }

    /// Catches a feathered mask painted with a seam: with Feather at 10, a
    /// stroke painted part by part on the mask, and its release, leave the
    /// image as one put together afresh from the whole mask blurred again.
    #[test]
    fn a_feathered_mask_paints_without_a_seam() {
        let mut doc = painted(120, 80, |_, _| [255, 255, 255, 255]);
        let red = doc.add_layer("Red", None, None).unwrap();
        doc.image.pixels.insert(red, Pixels::filled(120, 80, [255, 0, 0, 255]));
        doc.touch_all();
        doc.add_mask(red, None).unwrap();
        doc.set_feather(red, 10.0, false, None).unwrap();
        doc.composite();
        let black = Brush { rgb: [0, 0, 0], size: 12.0, ..brush() };
        let points = [[30.0, 40.0, 1.0], [50.0, 42.0, 1.0], [70.0, 38.0, 1.0], [90.0, 40.0, 1.0]];
        for (from, point) in points.iter().enumerate() {
            doc.paint_live(from, &[*point], &black);
            doc.composite();
        }
        doc.stroke(&points, &black).unwrap();
        let stepwise = doc.composite().to_vec();
        let mask = doc.image.find(red).unwrap().mask.as_ref().unwrap().id;
        doc.image.forget(mask);
        doc.image.prepare();
        let fresh = doc.image.composite();
        let worst = stepwise.iter().zip(&fresh).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "the image differs from one put together afresh by {worst}: a seam");
        assert!(stepwise[(40 * 120 + 60) * 4 + 1] > 200, "the painted mask did not hide the layer");
    }

    /// Catches adjustments that misapply: a curve taking 0.5 to 0.75
    /// brightens mid-grey to about 191 beneath it, only within its group;
    /// hidden, it does nothing; and Grade with saturation −100 makes grey.
    #[test]
    fn adjustments_change_only_what_is_beneath_them_in_their_group() {
        let mut doc = painted(2, 1, |_, _| [128, 128, 128, 255]);
        let background = doc.active.unwrap();
        let inner = doc.add_layer("Inner", None, None).unwrap();
        doc.image.pixels.insert(inner, Pixels::filled(2, 1, [0, 0, 0, 0]));
        doc.image.pixels.get_mut(&inner).unwrap().data[4..8].copy_from_slice(&[128, 128, 128, 255]);
        doc.activate(inner);
        let group = doc.group_active().unwrap();
        let curves = doc.add_adjustment(Kind::Curves(Curves::default()), Some(inner), None).unwrap();
        assert_eq!(doc.image.place(curves).unwrap().parent, Some(group));
        doc.set_curve(curves, 0, &[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]], false, None).unwrap();
        let inside = pixel_of(&mut doc, 1, 0);
        assert!((189..=193).contains(&inside[0]), "mid-grey became {}", inside[0]);
        assert_eq!(pixel_of(&mut doc, 0, 0), [128, 128, 128, 255], "the curve reached past its group");
        doc.set_visible(curves, false, None).unwrap();
        assert_eq!(pixel_of(&mut doc, 1, 0), [128, 128, 128, 255], "a hidden adjustment changed the image");

        // At the top, a curve reaches the background too.
        doc.activate(group);
        let top = doc.add_adjustment(Kind::Curves(Curves::default()), Some(group), None).unwrap();
        doc.set_curve(top, 0, &[[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]], false, None).unwrap();
        assert!((189..=193).contains(&pixel_of(&mut doc, 0, 0)[0]));
        let _ = background;

        let mut doc = painted(1, 1, |_, _| [220, 40, 60, 255]);
        let grade = doc.add_adjustment(Kind::Grade(Grade::default()), None, None).unwrap();
        doc.set_grade(grade, Grade { saturation: -100.0, ..Grade::default() }, false, None).unwrap();
        let grey = pixel_of(&mut doc, 0, 0);
        assert!(grey[0] == grey[1] && grey[1] == grey[2], "saturation −100 left colour: {grey:?}");
        doc.set_grade(grade, Grade { exposure: 1.0, ..Grade::default() }, false, None).unwrap();
        assert!(pixel_of(&mut doc, 0, 0)[1] > 40, "exposure did not brighten");
    }

    /// Catches a held slider's image that differs from the image put
    /// together whole: over a mask, a group with its own adjustment, and a
    /// curve above, each value of a held Color Grade, of the group's grade,
    /// and of the curve composites to the same bytes as every layer
    /// combined again, from the image kept beneath what changed; and a
    /// stroke while it is held is not lost under the kept image.
    #[test]
    fn a_held_slider_composites_as_the_whole_image_does() {
        let mut doc = painted(300, 260, |x, y| [(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x + y) % 251) as u8, 255]);
        let masked = doc.add_layer("Masked", None, None).unwrap();
        doc.image.pixels.insert(masked, Pixels::filled(300, 260, [200, 30, 90, 140]));
        let mask = doc.add_mask(masked, None).unwrap();
        for (index, pixel) in doc.image.pixels.get_mut(&mask).unwrap().data.chunks_exact_mut(4).enumerate() {
            let grey = (index % 300 * 255 / 300) as u8;
            pixel.copy_from_slice(&[grey, grey, grey, 255]);
        }
        doc.image.forget(mask);
        let inner = doc.add_layer("Inner", None, None).unwrap();
        doc.image.pixels.insert(inner, Pixels::filled(300, 260, [20, 200, 120, 100]));
        doc.touch_all();
        doc.activate(inner);
        let group = doc.group_active().unwrap();
        let inside = doc.add_adjustment(Kind::Grade(Grade { contrast: 20.0, ..Grade::default() }), Some(inner), None).unwrap();
        doc.activate(group);
        let grade = doc.add_adjustment(Kind::Grade(Grade { saturation: 30.0, ..Grade::default() }), Some(group), None).unwrap();
        let curve = doc.add_adjustment(Kind::Curves(Curves::default()), Some(grade), None).unwrap();
        doc.refresh();
        let whole = |doc: &mut Doc| {
            doc.refresh();
            let expected = doc.image.clone().composite();
            assert!(doc.composite() == expected.as_slice(), "the held slider's image differs from the whole image's");
        };
        for step in 1..=3 {
            doc.set_grade(grade, Grade { exposure: 0.3 * step as f32, saturation: 30.0, ..Grade::default() }, true, None).unwrap();
            whole(&mut doc);
            assert_eq!(doc.beneath_count(), Some(3), "the image beneath the grade was not kept");
        }
        doc.finish_edit();
        whole(&mut doc);
        assert_eq!(doc.beneath_count(), None, "the image beneath was kept after the slider was let go");
        for step in 1..=3 {
            doc.set_grade(inside, Grade { contrast: -20.0 * step as f32, ..Grade::default() }, true, None).unwrap();
            whole(&mut doc);
            assert_eq!(doc.beneath_count(), Some(2), "the image beneath the group was not kept");
        }
        doc.finish_edit();
        for step in 1..=3 {
            doc.set_curve(curve, 1, &[[0.0, 0.0], [0.4, 0.4 + 0.1 * f64::from(step)], [1.0, 1.0]], true, None).unwrap();
            whole(&mut doc);
            assert_eq!(doc.beneath_count(), Some(4));
        }
        doc.activate(masked);
        doc.stroke(&[[150.0, 130.0, 1.0], [160.0, 130.0, 1.0]], &Brush { size: 30.0, ..brush() }).unwrap();
        whole(&mut doc);
        doc.set_curve(curve, 1, &[[0.0, 0.0], [0.4, 0.9], [1.0, 1.0]], true, None).unwrap();
        whole(&mut doc);
    }

    /// Catches a slider's first value that combines every layer again:
    /// with the program idle, the image beneath the active layer is made
    /// once, ahead, for a grade over two translucent layers, a grade within
    /// a group, and a group's own opacity, and every held value, the first
    /// too, combines from it, to the same bytes as the whole image; let
    /// go, it stays for the next; and a stroke beneath has it made again.
    #[test]
    fn a_sliders_first_value_starts_from_the_image_made_ahead() {
        let mut doc = painted(300, 260, |x, y| [(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x + y) % 251) as u8, 255]);
        let lower = doc.add_layer("Lower", None, None).unwrap();
        doc.image.pixels.insert(lower, Pixels::filled(300, 260, [200, 30, 90, 140]));
        let upper = doc.add_layer("Upper", None, None).unwrap();
        doc.image.pixels.insert(upper, Pixels::filled(300, 260, [20, 200, 120, 100]));
        doc.touch_all();
        let grade = doc.add_adjustment(Kind::Grade(Grade::default()), Some(upper), None).unwrap();
        let whole = |doc: &mut Doc| {
            doc.refresh();
            let expected = doc.image.clone().composite();
            assert!(doc.composite() == expected.as_slice(), "the held slider's image differs from the whole image's");
        };
        // Held values on `id`, from the image made ahead and only it.
        let held = |doc: &mut Doc, id: u64, change: &dyn Fn(&mut Doc, f32)| {
            doc.refresh();
            assert!(doc.keep_ahead(), "nothing was made ahead");
            assert!(!doc.ahead_due(), "the image made ahead was not kept");
            let made = doc.beneaths_made;
            for step in 1..=3 {
                change(doc, step as f32);
                whole(doc);
                assert_eq!(doc.beneaths_made, made, "held value {step} on {id} made the image beneath again");
            }
            doc.finish_edit();
            whole(doc);
            assert!(!doc.ahead_due(), "the image made ahead was let go with the slider");
        };
        held(&mut doc, grade, &|doc, step| doc.set_grade(grade, Grade { exposure: 0.2 * step, ..Grade::default() }, true, None).unwrap());
        assert_eq!(doc.beneath_count(), Some(3));
        doc.activate(upper);
        let group = doc.group_active().unwrap();
        let inside = doc.add_adjustment(Kind::Grade(Grade::default()), Some(upper), None).unwrap();
        held(&mut doc, inside, &|doc, step| doc.set_grade(inside, Grade { contrast: 15.0 * step, ..Grade::default() }, true, None).unwrap());
        doc.activate(group);
        held(&mut doc, group, &|doc, step| doc.set_setting(Setting::Opacity(1.0 - 0.2 * f64::from(step)), true).unwrap());
        doc.activate(lower);
        doc.stroke(&[[150.0, 130.0, 1.0], [160.0, 130.0, 1.0]], &Brush { size: 30.0, ..brush() }).unwrap();
        whole(&mut doc);
        doc.activate(group);
        assert!(doc.ahead_due(), "a stroke beneath left the old image made ahead");
    }

    /// The pixels of the text layer `id` that have ink.
    fn ink(doc: &Doc, id: u64) -> usize {
        doc.image.pixels.get(&id).map_or(0, |pixels| pixels.data.chunks(4).filter(|pixel| pixel[3] > 0).count())
    }

    /// Catches text that is not text: typing "Hi" makes a text layer with
    /// ink inside its box, its words kept; re-editing it to "Hello" draws
    /// it again, as one more step; and Undo while typing takes back the
    /// typing, a new text with it.
    #[test]
    fn typing_makes_an_editable_text() {
        let mut doc = Doc::new(LayeredImage::blank(300, 120));
        let style = Style { size: 40.0, ..Style::default() };
        let id = doc.start_text([20.0, 20.0], None, style).unwrap();
        doc.text_input(&TextInput::Insert { text: "Hi".into() });
        let editing = doc.editing().expect("no box while typing");
        assert!(editing.rect[2] > 10.0 && editing.caret[0] > 20.0);
        doc.end_typing();
        assert!(doc.typing().is_none());
        let node = doc.image.find(id).expect("the text is gone");
        assert_eq!(node.text().unwrap().words, "Hi");
        assert_eq!(node.name, "Hi");
        let hi = ink(&doc, id);
        assert!(hi > 50, "Hi drew nothing");
        let dark = (20..80).any(|x| (20..70).any(|y| pixel_of(&mut doc, x, y)[0] < 128));
        assert!(dark, "no ink in the box on the image");
        assert!(doc.undo() && !doc.can_undo() && doc.image.find(id).is_none(), "the new text was not one step");
        assert!(doc.redo());

        assert!(doc.edit_text(id).is_some());
        doc.text_input(&TextInput::Key { key: TextKey::SelectAll, shift: false, word: false });
        doc.text_input(&TextInput::Insert { text: "Hello".into() });
        doc.end_typing();
        assert_eq!(doc.image.find(id).unwrap().text().unwrap().words, "Hello");
        assert!(ink(&doc, id) > hi, "Hello drew no more than Hi");
        assert!(doc.undo());
        assert_eq!(doc.image.find(id).unwrap().text().unwrap().words, "Hi", "re-editing was not its own step");

        // Undo mid-typing takes back the typing.
        let other = doc.start_text([20.0, 80.0], None, style).unwrap();
        doc.text_input(&TextInput::Insert { text: "Gold".into() });
        doc.text_input(&TextInput::Insert { text: "en".into() });
        assert!(doc.undo());
        assert!(doc.typing().is_none(), "Undo left the text being typed in");
        assert!(doc.image.find(other).is_none(), "Undo did not take back the typing");
        assert_eq!(doc.image.find(id).unwrap().text().unwrap().words, "Hi");

        // A text left empty is taken away, as no step.
        let steps = doc.undo.len();
        let empty = doc.start_text([5.0, 5.0], None, style).unwrap();
        doc.end_typing();
        assert!(doc.image.find(empty).is_none());
        assert_eq!(doc.undo.len(), steps);
    }

    /// Catches typing keys that land in the wrong place: a word selected
    /// and copied comes back, Cut takes it out, and Backspace by word.
    #[test]
    fn keys_and_the_clipboard_edit_the_words() {
        let mut doc = Doc::new(LayeredImage::blank(400, 100));
        let id = doc.start_text([0.0, 0.0], None, Style { size: 30.0, ..Style::default() }).unwrap();
        let key = |key: TextKey, shift: bool, word: bool| TextInput::Key { key, shift, word };
        doc.text_input(&TextInput::Insert { text: "Golden hour".into() });
        doc.text_input(&key(TextKey::Left, true, true));
        assert_eq!(doc.text_input(&key(TextKey::Copy, false, false)).as_deref(), Some("hour"));
        doc.text_input(&key(TextKey::End, false, false));
        doc.text_input(&TextInput::Insert { text: " hour".into() });
        let words = |doc: &Doc| doc.image.find(id).unwrap().text().unwrap().words.clone();
        assert_eq!(words(&doc), "Golden hour hour");
        doc.text_input(&key(TextKey::Backspace, false, true));
        assert_eq!(words(&doc), "Golden hour ");
        doc.text_input(&key(TextKey::Home, false, false));
        doc.text_input(&key(TextKey::Right, true, true));
        assert_eq!(doc.text_input(&key(TextKey::Cut, false, false)).as_deref(), Some("Golden"));
        assert_eq!(words(&doc), " hour ");
        doc.text_input(&key(TextKey::Enter, false, false));
        assert_eq!(words(&doc), "\n hour ");
    }
}
