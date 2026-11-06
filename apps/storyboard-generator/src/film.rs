//! The film a storyboard holds: its clips on the Shots track and, inside each
//! clip, layers of keyframes that hold until the next one. Frames count from
//! 0 here; people read them from 1. Each keyframe names a drawing the record
//! keeps, so this module edits structure only and never a drawing.
//!
//! Every edit is a plain function of the film, so the record can keep the
//! film before and after as one undo step. An edit that needs a drawing the
//! record does not have returns it as a `NewDrawing`: blank, or a copy.

use serde_json::{Map, Value, json};

/// The film's frame rate.
pub const FPS: u32 = 24;
/// A new clip's length: two seconds.
pub const NEW_CLIP_LENGTH: u32 = 2 * FPS;
/// The Paper row's choices, as the conventions guide names them.
pub const PAPER_NAMES: &[&str] = &["Japanese layout paper, 16:9"];
/// A new layer's colours, in turn, as the approved clip-layers view shows them.
pub const LAYER_COLORS: &[[u8; 3]] = &[
    [37, 99, 235],
    [249, 115, 22],
    [34, 197, 94],
    [236, 72, 153],
    [20, 184, 166],
    [168, 85, 247],
    [234, 179, 8],
    [148, 163, 184],
];

#[derive(Clone, Debug, PartialEq)]
pub struct Film {
    pub title: String,
    pub scene: String,
    pub size: [u32; 2],
    pub paper: u8,
    /// The Shots track's clips, in the order they start.
    pub clips: Vec<Clip>,
    /// Ids of clips, layers, keys, marks, and drawings. Never reused.
    pub next_id: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: u64,
    pub name: String,
    pub shot: String,
    pub start: u32,
    pub length: u32,
    pub action: String,
    pub dialogue: String,
    pub camera: String,
    pub animator: String,
    /// Top to bottom, as the layer list shows them.
    pub layers: Vec<Layer>,
    /// Where the field's top left lies in the clip's drawings: the origin,
    /// except for a storyboard from before films, whose field is placed on
    /// its drawing so the drawing is not moved.
    pub origin: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub opacity: f64,
    pub folder: bool,
    /// The folder it sits in.
    pub parent: Option<u64>,
    /// A folder's rows are shown.
    pub open: bool,
    pub visible: bool,
    pub locked: bool,
    pub outline: bool,
    /// By frame. A folder has none; every other layer has one at frame 0.
    pub keys: Vec<Key>,
    pub marks: Vec<Mark>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub id: u64,
    pub frame: u32,
    /// The drawing the record keeps for it. No two keys share one.
    pub drawing: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub id: u64,
    pub frame: u32,
    pub comment: bool,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Onion {
    pub on: bool,
    pub before: u8,
    pub after: u8,
    /// The nearest drawing's opacity, 0 to 1.
    pub opacity: f64,
    /// How much each drawing further away loses, 0 to 1.
    pub fade: f64,
    pub keys_only: bool,
}

impl Default for Onion {
    fn default() -> Self {
        Self {
            on: false,
            before: 2,
            after: 2,
            opacity: 0.4,
            fade: 0.1,
            keys_only: false,
        }
    }
}

/// A drawing a change needs that the record does not have yet: blank, or a
/// copy of another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewDrawing {
    pub id: u64,
    pub copy_of: Option<u64>,
}

/// Frames copied from a clip, as Copy frames keeps them.
#[derive(Clone, Debug, PartialEq)]
pub struct CopiedFrames {
    pub length: u32,
    /// One per copied layer, top to bottom: each key's frame from the copy's
    /// start and its drawing.
    pub layers: Vec<Vec<(u32, u64)>>,
}

fn take(next: &mut u64) -> u64 {
    let id = *next;
    *next += 1;
    id
}

impl Film {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            scene: String::new(),
            size: [1920, 1080],
            paper: 0,
            clips: Vec::new(),
            next_id: 1,
        }
    }

    pub fn id(&mut self) -> u64 {
        take(&mut self.next_id)
    }

    /// Frames from the start to the last clip's end.
    pub fn length(&self) -> u32 {
        self.clips.iter().map(|clip| clip.start + clip.length).max().unwrap_or(0)
    }

    pub fn clip(&self, id: u64) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.id == id)
    }

    pub fn clip_mut(&mut self, id: u64) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|clip| clip.id == id)
    }

    /// The clip at a frame of the film, none over a gap.
    pub fn clip_at(&self, frame: u32) -> Option<&Clip> {
        self.clips.iter().find(|clip| frame >= clip.start && frame < clip.start + clip.length)
    }

    /// The clip and layer a key belongs to, and the key.
    pub fn find_drawing(&self, drawing: u64) -> Option<(&Clip, &Layer, Key)> {
        for clip in &self.clips {
            for layer in &clip.layers {
                if let Some(key) = layer.keys.iter().find(|key| key.drawing == drawing) {
                    return Some((clip, layer, *key));
                }
            }
        }
        None
    }

    /// Every drawing a key names.
    pub fn drawings(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for clip in &self.clips {
            for layer in &clip.layers {
                out.extend(layer.keys.iter().map(|key| key.drawing));
            }
        }
        out
    }

    /// The first free name of "Shot 1", "Shot 2", and on.
    pub fn free_clip_name(&self) -> String {
        let mut n = 1;
        loop {
            let name = format!("Shot {n}");
            if !self.clips.iter().any(|clip| clip.name == name) {
                return name;
            }
            n += 1;
        }
    }

    fn sort(&mut self) {
        self.clips.sort_by_key(|clip| (clip.start, clip.id));
    }

    /// A new clip at `start`, or after the last one, with one blank layer.
    /// Clips it lands on move later. Returns its id and its drawing.
    pub fn add_clip(&mut self, start: Option<u32>, length: u32) -> (u64, Vec<NewDrawing>) {
        let start = start.unwrap_or_else(|| self.length());
        let id = self.id();
        let name = self.free_clip_name();
        let shot = name.trim_start_matches("Shot ").to_string();
        let (layer, drawing) = self.blank_layer("Layer 1", 0);
        let clip = Clip {
            id,
            name,
            shot,
            start,
            length: length.max(1),
            action: String::new(),
            dialogue: String::new(),
            camera: String::new(),
            animator: String::new(),
            layers: vec![layer],
            origin: [0.0, 0.0],
        };
        self.insert_pushing(clip);
        (id, vec![drawing])
    }

    fn blank_layer(&mut self, name: &str, index: usize) -> (Layer, NewDrawing) {
        let id = self.id();
        let key = self.id();
        let drawing = self.id();
        let layer = Layer {
            id,
            name: name.to_string(),
            color: LAYER_COLORS[index % LAYER_COLORS.len()],
            opacity: 1.0,
            folder: false,
            parent: None,
            open: true,
            visible: true,
            locked: false,
            outline: false,
            keys: vec![Key { id: key, frame: 0, drawing }],
            marks: Vec::new(),
        };
        (layer, NewDrawing { id: drawing, copy_of: None })
    }

    /// Puts `clip` on the track. A clip it lands inside is cut where it
    /// starts; the clips from there on move later to make room.
    fn insert_pushing(&mut self, clip: Clip) {
        let end = clip.start + clip.length;
        for other in &mut self.clips {
            if other.start < clip.start && other.start + other.length > clip.start {
                other.length = clip.start - other.start;
            }
        }
        let first_after = self.clips.iter().filter(|other| other.start >= clip.start).map(|other| other.start).min();
        if let Some(first_after) = first_after
            && first_after < end
        {
            let push = end - first_after;
            for other in self.clips.iter_mut().filter(|other| other.start >= clip.start) {
                other.start += push;
            }
        }
        self.clips.push(clip);
        self.sort();
    }

    /// Puts `clip` on the track over what is there: clips it covers whole go,
    /// and those it covers in part are cut.
    fn insert_replacing(&mut self, clip: Clip) {
        let (start, end) = (clip.start, clip.start + clip.length);
        let mut kept = Vec::new();
        for mut other in std::mem::take(&mut self.clips) {
            let other_end = other.start + other.length;
            if other_end <= start || other.start >= end {
                kept.push(other);
            } else if other.start >= start && other_end <= end {
                // Covered whole: it goes.
            } else if other.start < start {
                other.length = start - other.start;
                kept.push(other);
            } else {
                other.length = other_end - end;
                other.start = end;
                kept.push(other);
            }
        }
        self.clips = kept;
        self.clips.push(clip);
        self.sort();
    }

    /// A copy of a clip right after it, with copies of its drawings; later
    /// clips move later.
    pub fn duplicate_clip(&mut self, id: u64) -> Option<(u64, Vec<NewDrawing>)> {
        let source = self.clip(id)?.clone();
        let mut copy = source.clone();
        let mut next = self.next_id;
        copy.id = take(&mut next);
        copy.name = format!("{} copy", source.name);
        let mut n = 2;
        while self.clips.iter().any(|clip| clip.name == copy.name) {
            copy.name = format!("{} copy {n}", source.name);
            n += 1;
        }
        copy.start = source.start + source.length;
        let mut drawings = Vec::new();
        let mut renamed = Vec::new();
        for layer in &mut copy.layers {
            let new = take(&mut next);
            renamed.push((layer.id, new));
            layer.id = new;
            for key in &mut layer.keys {
                key.id = take(&mut next);
                let drawing = take(&mut next);
                drawings.push(NewDrawing { id: drawing, copy_of: Some(key.drawing) });
                key.drawing = drawing;
            }
            for mark in &mut layer.marks {
                mark.id = take(&mut next);
            }
        }
        for layer in &mut copy.layers {
            if let Some(parent) = layer.parent {
                layer.parent = renamed.iter().find(|(old, _)| *old == parent).map(|(_, new)| *new);
            }
        }
        self.next_id = next;
        let new = copy.id;
        self.insert_pushing(copy);
        Some((new, drawings))
    }

    /// Takes a clip off the track. Its place stays empty.
    pub fn delete_clip(&mut self, id: u64) -> bool {
        let before = self.clips.len();
        self.clips.retain(|clip| clip.id != id);
        self.clips.len() != before
    }

    /// Moves one edge of a clip by `frames`. Moving its end moves every later
    /// clip with it, as the approved film-clip-retime shows; its start stops
    /// at the clip before.
    pub fn resize_clip(&mut self, id: u64, end_edge: bool, frames: i64) -> bool {
        let Some(index) = self.clips.iter().position(|clip| clip.id == id) else {
            return false;
        };
        let (start, length) = (self.clips[index].start, self.clips[index].length);
        if end_edge {
            let new_length = (i64::from(length) + frames).max(1) as u32;
            if new_length == length {
                return false;
            }
            let delta = i64::from(new_length) - i64::from(length);
            for other in self.clips.iter_mut().filter(|other| other.id != id && other.start >= start + length) {
                other.start = (i64::from(other.start) + delta).max(0) as u32;
            }
            self.clips[index].length = new_length;
        } else {
            let end = start + length;
            let before = self.clips[..index].iter().map(|other| other.start + other.length).max().unwrap_or(0);
            let new_start = (i64::from(start) + frames).clamp(i64::from(before), i64::from(end) - 1) as u32;
            if new_start == start {
                return false;
            }
            let shift = i64::from(new_start) - i64::from(start);
            let clip = &mut self.clips[index];
            clip.start = new_start;
            clip.length = end - new_start;
            for layer in clip.layers.iter_mut().filter(|layer| !layer.folder) {
                trim_head(layer, shift);
            }
        }
        self.sort();
        true
    }

    /// Moves clips by `frames`. With `push`, clips they land on move later;
    /// without, the clips they cover are cut or go, as an overwrite does.
    pub fn move_clips(&mut self, ids: &[u64], frames: i64, push: bool) -> bool {
        let mut moving: Vec<Clip> = self.clips.iter().filter(|clip| ids.contains(&clip.id)).cloned().collect();
        let Some(earliest) = moving.iter().map(|clip| clip.start).min() else {
            return false;
        };
        let frames = frames.max(-i64::from(earliest));
        if frames == 0 {
            return false;
        }
        self.clips.retain(|clip| !ids.contains(&clip.id));
        for clip in &mut moving {
            clip.start = (i64::from(clip.start) + frames) as u32;
        }
        moving.sort_by_key(|clip| clip.start);
        for clip in moving {
            if push {
                self.insert_pushing(clip);
            } else {
                self.insert_replacing(clip);
            }
        }
        true
    }

    /// A clip whose length changed from inside it, from `old`: the clips
    /// after it move by as much, as retiming its end on the film does.
    pub fn follow_length(&mut self, clip: u64, old: u32) {
        let Some(at) = self.clip(clip) else {
            return;
        };
        let (start, length) = (at.start, at.length);
        if length == old {
            return;
        }
        let delta = i64::from(length) - i64::from(old);
        for other in self.clips.iter_mut().filter(|other| other.id != clip && other.start >= start + old) {
            other.start = (i64::from(other.start) + delta).max(0) as u32;
        }
        self.sort();
    }

    /// The highest id the film holds.
    fn highest_id(&self) -> u64 {
        let mut highest = 0;
        for clip in &self.clips {
            highest = highest.max(clip.id);
            for layer in &clip.layers {
                highest = highest.max(layer.id);
                for key in &layer.keys {
                    highest = highest.max(key.id).max(key.drawing);
                }
                for mark in &layer.marks {
                    highest = highest.max(mark.id);
                }
            }
        }
        highest
    }
}

impl Clip {
    pub fn layer(&self, id: u64) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    pub fn layer_mut(&mut self, id: u64) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|layer| layer.id == id)
    }

    /// The first layer that holds drawings.
    pub fn first_drawn_layer(&self) -> Option<u64> {
        self.layers.iter().find(|layer| !layer.folder).map(|layer| layer.id)
    }

    /// A layer shows when it and every folder it sits in show.
    pub fn shows(&self, layer: &Layer) -> bool {
        self.up_from(layer).all(|at| at.visible)
    }

    /// The layer is locked, or a folder it sits in is.
    pub fn locked(&self, layer: &Layer) -> bool {
        self.up_from(layer).any(|at| at.locked)
    }

    /// A closed folder above hides the layer's row.
    pub fn folded(&self, layer: &Layer) -> bool {
        self.up_from(layer).skip(1).any(|at| !at.open)
    }

    /// How many folders the layer sits in.
    pub fn depth(&self, layer: &Layer) -> u8 {
        (self.up_from(layer).count() - 1).min(255) as u8
    }

    fn up_from<'a>(&'a self, layer: &'a Layer) -> impl Iterator<Item = &'a Layer> + 'a {
        let mut current = Some(layer);
        let mut steps = 0;
        std::iter::from_fn(move || {
            let at = current?;
            steps += 1;
            current = if steps > self.layers.len() { None } else { at.parent.and_then(|parent| self.layer(parent)) };
            Some(at)
        })
    }

    /// A layer's letter for key numbers: A for the bottom drawn layer, then B,
    /// and on, as genga sheets letter their cels.
    pub fn letter(&self, layer: u64) -> String {
        let index = self
            .layers
            .iter()
            .rev()
            .filter(|layer| !layer.folder)
            .position(|row| row.id == layer)
            .unwrap_or(0);
        let mut letter = String::new();
        let mut n = index;
        loop {
            letter.insert(0, char::from(b'A' + (n % 26) as u8));
            if n < 26 {
                break;
            }
            n = n / 26 - 1;
        }
        letter
    }
}

impl Layer {
    /// The key a frame shows: the last at or before it.
    pub fn key_at(&self, frame: u32) -> Option<&Key> {
        self.keys.iter().rev().find(|key| key.frame <= frame)
    }

    /// The key's number on the layer, from 1, as A③ counts it.
    pub fn key_number(&self, key: u64) -> usize {
        self.keys.iter().position(|at| at.id == key).map_or(1, |index| index + 1)
    }

    fn sort(&mut self) {
        self.keys.sort_by_key(|key| key.frame);
        self.marks.sort_by_key(|mark| (mark.frame, mark.id));
    }
}

/// Edits inside a clip. Each returns `None` when nothing changed, or the
/// drawings the record must make for it.
impl Film {
    /// Adds a layer, or a folder, above `above`, or at the top.
    pub fn add_layer(&mut self, clip: u64, above: Option<u64>, folder: bool) -> Option<(u64, Vec<NewDrawing>)> {
        let count = self.clip(clip)?.layers.len();
        let word = if folder { "Folder" } else { "Layer" };
        let mut n = count + 1;
        while self.clip(clip)?.layers.iter().any(|layer| layer.name == format!("{word} {n}")) {
            n += 1;
        }
        let (mut layer, drawing) = self.blank_layer(&format!("{word} {n}"), count);
        let mut drawings = vec![drawing];
        if folder {
            layer.folder = true;
            layer.keys.clear();
            drawings.clear();
        }
        let id = layer.id;
        let clip = self.clip_mut(clip)?;
        let at = above.and_then(|above| clip.layers.iter().position(|row| row.id == above)).unwrap_or(0);
        layer.parent = above.and_then(|above| clip.layer(above)).and_then(|row| row.parent);
        clip.layers.insert(at, layer);
        Some((id, drawings))
    }

    /// Takes a layer off, with the rows inside it when it is a folder. A clip
    /// keeps at least one layer that holds drawings.
    pub fn delete_layer(&mut self, clip: u64, layer: u64) -> bool {
        let Some(clip) = self.clip_mut(clip) else {
            return false;
        };
        if clip.layer(layer).is_none() {
            return false;
        }
        let mut gone = vec![layer];
        loop {
            let more: Vec<u64> = clip
                .layers
                .iter()
                .filter(|row| !gone.contains(&row.id) && row.parent.is_some_and(|parent| gone.contains(&parent)))
                .map(|row| row.id)
                .collect();
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        if !clip.layers.iter().any(|row| !row.folder && !gone.contains(&row.id)) {
            return false;
        }
        clip.layers.retain(|row| !gone.contains(&row.id));
        true
    }

    /// Moves a layer to `index` in the list, top first, and into `parent`.
    pub fn move_layer(&mut self, clip: u64, layer: u64, index: usize, parent: Option<u64>) -> bool {
        let Some(clip) = self.clip_mut(clip) else {
            return false;
        };
        let Some(from) = clip.layers.iter().position(|row| row.id == layer) else {
            return false;
        };
        if let Some(parent) = parent {
            let Some(folder) = clip.layer(parent).filter(|row| row.folder) else {
                return false;
            };
            // Not into itself, or into a folder inside it.
            if clip.up_from(folder).any(|at| at.id == layer) {
                return false;
            }
        }
        let mut row = clip.layers.remove(from);
        let to = index.min(clip.layers.len());
        let changed = from != to || row.parent != parent;
        row.parent = parent;
        clip.layers.insert(to, row);
        changed
    }

    /// The frame becomes a key: Insert keyframe copies what it showed,
    /// Insert blank keyframe starts empty.
    pub fn insert_key(&mut self, clip: u64, layer: u64, frame: u32, blank: bool) -> Option<Vec<NewDrawing>> {
        let mut next = self.next_id;
        let (key, drawing) = (take(&mut next), take(&mut next));
        let clip = self.clip_mut(clip)?;
        let row = clip.layer_mut(layer).filter(|row| !row.folder)?;
        if row.keys.iter().any(|at| at.frame == frame) {
            return None;
        }
        let copy_of = if blank { None } else { row.key_at(frame).map(|at| at.drawing) };
        row.keys.push(Key { id: key, frame, drawing });
        row.sort();
        clip.length = clip.length.max(frame + 1);
        self.next_id = next;
        Some(vec![NewDrawing { id: drawing, copy_of }])
    }

    /// The key at the frame goes, and the one before holds on. The first key
    /// of a layer becomes blank instead.
    pub fn clear_key(&mut self, clip: u64, layer: u64, frame: u32) -> Option<Vec<NewDrawing>> {
        let mut next = self.next_id;
        let row = self.clip_mut(clip)?.layer_mut(layer)?;
        let index = row.keys.iter().position(|at| at.frame == frame)?;
        let mut drawings = Vec::new();
        if frame == 0 {
            let drawing = take(&mut next);
            row.keys[index].drawing = drawing;
            drawings.push(NewDrawing { id: drawing, copy_of: None });
        } else {
            row.keys.remove(index);
        }
        self.next_id = next;
        Some(drawings)
    }

    /// Inserts `count` frames after `frame` on these layers: their later keys
    /// and marks move later, and the clip grows.
    pub fn insert_frames(&mut self, clip: u64, layers: &[u64], frame: u32, count: u32) -> bool {
        let Some(clip) = self.clip_mut(clip) else {
            return false;
        };
        if count == 0 || layers.is_empty() {
            return false;
        }
        for row in clip.layers.iter_mut().filter(|row| layers.contains(&row.id)) {
            for key in row.keys.iter_mut().filter(|key| key.frame > frame) {
                key.frame += count;
            }
            for mark in row.marks.iter_mut().filter(|mark| mark.frame > frame) {
                mark.frame += count;
            }
        }
        clip.length += count;
        true
    }

    /// Removes frames `frame .. frame + count` from these layers: keys there go,
    /// later ones move earlier, and the clip shrinks.
    pub fn remove_frames(&mut self, clip: u64, layers: &[u64], frame: u32, count: u32) -> Option<Vec<NewDrawing>> {
        let mut next = self.next_id;
        let clip = self.clip_mut(clip)?;
        let count = count.min(clip.length.saturating_sub(frame)).min(clip.length.saturating_sub(1));
        if count == 0 {
            return None;
        }
        let end = frame + count;
        let mut drawings = Vec::new();
        for row in clip.layers.iter_mut().filter(|row| layers.contains(&row.id) && !row.folder) {
            row.keys.retain(|key| key.frame < frame || key.frame >= end);
            row.marks.retain(|mark| mark.frame < frame || mark.frame >= end);
            for key in row.keys.iter_mut().filter(|key| key.frame >= end) {
                key.frame -= count;
            }
            for mark in row.marks.iter_mut().filter(|mark| mark.frame >= end) {
                mark.frame -= count;
            }
            keep_first_key(row, &mut next, &mut drawings);
        }
        clip.length -= count;
        self.next_id = next;
        Some(drawings)
    }

    /// Moves frames `start .. end` of these layers by `frames`. With `push`,
    /// the frames after them move along and the clip grows; without, what they
    /// land on is replaced.
    pub fn move_frames(
        &mut self,
        clip: u64,
        layers: &[u64],
        start: u32,
        end: u32,
        frames: i64,
        push: bool,
    ) -> Option<Vec<NewDrawing>> {
        if end <= start {
            return None;
        }
        let frames = frames.max(-i64::from(start));
        if frames == 0 {
            return None;
        }
        let mut next = self.next_id;
        let clip = self.clip_mut(clip)?;
        let shift = |frame: u32| (i64::from(frame) + frames) as u32;
        let (to_start, to_end) = (shift(start), shift(end));
        let mut drawings = Vec::new();
        let mut longest = 0;
        for row in clip.layers.iter_mut().filter(|row| layers.contains(&row.id) && !row.folder) {
            let (moving, mut rest): (Vec<Key>, Vec<Key>) =
                row.keys.drain(..).partition(|key| key.frame >= start && key.frame < end);
            let (moving_marks, mut rest_marks): (Vec<Mark>, Vec<Mark>) =
                row.marks.drain(..).partition(|mark| mark.frame >= start && mark.frame < end);
            if push && frames > 0 {
                for key in rest.iter_mut().filter(|key| key.frame >= end) {
                    key.frame = shift(key.frame);
                }
                for mark in rest_marks.iter_mut().filter(|mark| mark.frame >= end) {
                    mark.frame = shift(mark.frame);
                }
            } else {
                rest.retain(|key| key.frame < to_start || key.frame >= to_end);
                rest_marks.retain(|mark| mark.frame < to_start || mark.frame >= to_end);
            }
            rest.extend(moving.into_iter().map(|key| Key { frame: shift(key.frame), ..key }));
            rest_marks.extend(moving_marks.into_iter().map(|mark| Mark { frame: shift(mark.frame), ..mark }));
            row.keys = rest;
            row.marks = rest_marks;
            row.sort();
            keep_first_key(row, &mut next, &mut drawings);
            longest = longest.max(row.keys.last().map_or(0, |key| key.frame + 1));
        }
        clip.length = if push && frames > 0 { clip.length + frames as u32 } else { clip.length.max(longest) };
        self.next_id = next;
        Some(drawings)
    }

    /// Stretches the hold that starts at `key_frame` by `frames`: the later
    /// keys of the layer move along, and the clip grows or shrinks with it.
    pub fn stretch_span(&mut self, clip: u64, layer: u64, key_frame: u32, frames: i64) -> bool {
        let Some(clip) = self.clip_mut(clip) else {
            return false;
        };
        let length = clip.length;
        let Some(row) = clip.layer_mut(layer) else {
            return false;
        };
        let Some(index) = row.keys.iter().position(|key| key.frame == key_frame) else {
            return false;
        };
        let span_end = row.keys.get(index + 1).map_or(length, |key| key.frame);
        let frames = frames.max(1 - i64::from(span_end - key_frame));
        if frames == 0 {
            return false;
        }
        let shift = |frame: u32| (i64::from(frame) + frames) as u32;
        for key in row.keys.iter_mut().skip(index + 1) {
            key.frame = shift(key.frame);
        }
        for mark in row.marks.iter_mut().filter(|mark| mark.frame >= span_end) {
            mark.frame = shift(mark.frame);
        }
        clip.length = (i64::from(clip.length) + frames).max(1) as u32;
        true
    }

    /// The frames `start .. end` of these layers, for Paste frames.
    pub fn copy_frames(&self, clip: u64, layers: &[u64], start: u32, end: u32) -> Option<CopiedFrames> {
        let clip = self.clip(clip)?;
        if end <= start {
            return None;
        }
        let mut copied = Vec::new();
        for row in clip.layers.iter().filter(|row| layers.contains(&row.id) && !row.folder) {
            let mut keys: Vec<(u32, u64)> = row
                .keys
                .iter()
                .filter(|key| key.frame > start && key.frame < end)
                .map(|key| (key.frame - start, key.drawing))
                .collect();
            if let Some(held) = row.key_at(start) {
                keys.insert(0, (0, held.drawing));
            }
            copied.push(keys);
        }
        (!copied.is_empty()).then_some(CopiedFrames { length: end - start, layers: copied })
    }

    /// Pastes copied frames at `frame`, the first copied layer on the first of
    /// `layers`, replacing what was there. What followed still shows after.
    pub fn paste_frames(&mut self, clip: u64, layers: &[u64], frame: u32, copied: &CopiedFrames) -> Option<Vec<NewDrawing>> {
        let mut next = self.next_id;
        let clip = self.clip_mut(clip)?;
        let end = frame + copied.length;
        let length = clip.length;
        let mut drawings = Vec::new();
        let targets: Vec<u64> =
            clip.layers.iter().filter(|row| layers.contains(&row.id) && !row.folder).map(|row| row.id).collect();
        if targets.is_empty() {
            return None;
        }
        for (target, keys) in targets.iter().zip(&copied.layers) {
            let row = clip.layer_mut(*target)?;
            let after = row.key_at(end).copied();
            row.keys.retain(|key| key.frame < frame || key.frame >= end);
            for (offset, source) in keys {
                let drawing = take(&mut next);
                row.keys.push(Key { id: take(&mut next), frame: frame + offset, drawing });
                drawings.push(NewDrawing { id: drawing, copy_of: Some(*source) });
            }
            if let Some(after) = after
                && end < length
                && !row.keys.iter().any(|key| key.frame == end)
            {
                let drawing = take(&mut next);
                row.keys.push(Key { id: take(&mut next), frame: end, drawing });
                drawings.push(NewDrawing { id: drawing, copy_of: Some(after.drawing) });
            }
            row.sort();
        }
        clip.length = length.max(end);
        self.next_id = next;
        Some(drawings)
    }

    /// Reverses frames `start .. end` of these layers: their holds come in the
    /// opposite order. What followed still shows after.
    pub fn reverse_frames(&mut self, clip: u64, layers: &[u64], start: u32, end: u32) -> Option<Vec<NewDrawing>> {
        let mut next = self.next_id;
        let clip = self.clip_mut(clip)?;
        let end = end.min(clip.length);
        if end <= start + 1 {
            return None;
        }
        let length = clip.length;
        let mut drawings = Vec::new();
        let mut changed = false;
        for row in clip.layers.iter_mut().filter(|row| layers.contains(&row.id) && !row.folder) {
            // The holds inside the range, as (key, frames).
            let mut holds: Vec<(Key, u32)> = Vec::new();
            let mut frame = start;
            while frame < end {
                let Some(key) = row.key_at(frame).copied() else {
                    break;
                };
                let until = row.keys.iter().find(|at| at.frame > frame).map_or(end, |at| at.frame.min(end));
                holds.push((key, until - frame));
                frame = until;
            }
            if holds.len() < 2 {
                continue;
            }
            let after = row.key_at(end).copied();
            row.keys.retain(|key| key.frame < start || key.frame >= end);
            let mut frame = start;
            for (key, frames) in holds.into_iter().rev() {
                // A key held in from before the range is shown again, as a copy.
                let drawing = if row.keys.iter().any(|at| at.drawing == key.drawing) {
                    let copy = take(&mut next);
                    drawings.push(NewDrawing { id: copy, copy_of: Some(key.drawing) });
                    copy
                } else {
                    key.drawing
                };
                row.keys.push(Key { id: take(&mut next), frame, drawing });
                frame += frames;
            }
            if let Some(after) = after
                && end < length
                && !row.keys.iter().any(|key| key.frame == end)
            {
                let copy = take(&mut next);
                drawings.push(NewDrawing { id: copy, copy_of: Some(after.drawing) });
                row.keys.push(Key { id: take(&mut next), frame: end, drawing: copy });
            }
            row.sort();
            changed = true;
        }
        self.next_id = next;
        changed.then_some(drawings)
    }

    /// A label or comment at a frame of a layer.
    pub fn add_mark(&mut self, clip: u64, layer: u64, frame: u32, comment: bool, text: &str) -> Option<u64> {
        let mut next = self.next_id;
        let id = take(&mut next);
        let row = self.clip_mut(clip)?.layer_mut(layer).filter(|row| !row.folder)?;
        row.marks.push(Mark { id, frame, comment, text: text.to_string() });
        row.sort();
        self.next_id = next;
        Some(id)
    }

    pub fn remove_mark(&mut self, clip: u64, layer: u64, mark: u64) -> bool {
        let Some(row) = self.clip_mut(clip).and_then(|clip| clip.layer_mut(layer)) else {
            return false;
        };
        let before = row.marks.len();
        row.marks.retain(|at| at.id != mark);
        row.marks.len() != before
    }
}

/// Moves a layer's frames `shift` earlier, as when its clip's start moves
/// later by as much: what the new first frame showed stays, by its key, and
/// moving the start earlier holds the first key from there.
fn trim_head(layer: &mut Layer, shift: i64) {
    if shift > 0 {
        let held = layer.keys.iter().rposition(|key| i64::from(key.frame) <= shift).unwrap_or(0);
        layer.keys.drain(..held);
        for key in &mut layer.keys {
            key.frame = (i64::from(key.frame) - shift).max(0) as u32;
        }
        layer.marks.retain(|mark| i64::from(mark.frame) >= shift);
        for mark in &mut layer.marks {
            mark.frame = (i64::from(mark.frame) - shift) as u32;
        }
    } else if shift < 0 {
        for key in layer.keys.iter_mut().skip(1) {
            key.frame = (i64::from(key.frame) - shift) as u32;
        }
        for mark in &mut layer.marks {
            mark.frame = (i64::from(mark.frame) - shift) as u32;
        }
    }
}

/// A layer that lost its first frame's key starts with a blank one.
fn keep_first_key(row: &mut Layer, next: &mut u64, drawings: &mut Vec<NewDrawing>) {
    if row.keys.iter().any(|key| key.frame == 0) {
        return;
    }
    let drawing = take(next);
    row.keys.insert(0, Key { id: take(next), frame: 0, drawing });
    drawings.push(NewDrawing { id: drawing, copy_of: None });
}

/// The film as `.smx` version 2 writes it, without its drawings.
impl Film {
    pub fn to_value(&self) -> Value {
        json!({
            "title": self.title,
            "scene": self.scene,
            "size": self.size,
            "paper": self.paper,
            "next_id": self.next_id,
            "tracks": [{
                "kind": "shots",
                "clips": self.clips.iter().map(clip_value).collect::<Vec<_>>(),
            }],
        })
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let mut film = Film::new(object.get("title")?.as_str()?);
        film.scene = text(object, "scene");
        if let Some(size) = object.get("size").and_then(Value::as_array)
            && let [width, height] = size.as_slice()
        {
            film.size = [width.as_u64()?.max(1) as u32, height.as_u64()?.max(1) as u32];
        }
        film.paper = number(object, "paper").unwrap_or(0).min(PAPER_NAMES.len() as u64 - 1) as u8;
        for track in object.get("tracks")?.as_array()? {
            if track.get("kind").and_then(Value::as_str) != Some("shots") {
                continue;
            }
            for clip in track.get("clips")?.as_array()? {
                film.clips.push(clip_from(clip)?);
            }
        }
        film.sort();
        film.next_id = number(object, "next_id").unwrap_or(1).max(film.highest_id() + 1);
        Some(film)
    }
}

fn clip_value(clip: &Clip) -> Value {
    json!({
        "id": clip.id,
        "name": clip.name,
        "shot": clip.shot,
        "start": clip.start,
        "length": clip.length,
        "notes": { "action": clip.action, "dialogue": clip.dialogue, "camera": clip.camera },
        "animator": clip.animator,
        "origin": clip.origin,
        "layers": clip.layers.iter().map(layer_value).collect::<Vec<_>>(),
    })
}

fn layer_value(layer: &Layer) -> Value {
    json!({
        "id": layer.id,
        "name": layer.name,
        "color": layer.color,
        "opacity": layer.opacity,
        "folder": layer.folder,
        "parent": layer.parent,
        "open": layer.open,
        "visible": layer.visible,
        "locked": layer.locked,
        "outline": layer.outline,
        "keys": layer.keys.iter().map(|key| json!({ "id": key.id, "frame": key.frame, "drawing": key.drawing })).collect::<Vec<_>>(),
        "marks": layer.marks.iter().map(|mark| json!({
            "id": mark.id,
            "frame": mark.frame,
            "kind": if mark.comment { "comment" } else { "label" },
            "text": mark.text,
        })).collect::<Vec<_>>(),
    })
}

fn text(object: &Map<String, Value>, key: &str) -> String {
    object.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn number(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

fn flag(object: &Map<String, Value>, key: &str, default: bool) -> bool {
    object.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn unit(object: &Map<String, Value>, key: &str, default: f64) -> f64 {
    object.get(key).and_then(Value::as_f64).unwrap_or(default).clamp(0.0, 1.0)
}

fn clip_from(value: &Value) -> Option<Clip> {
    let object = value.as_object()?;
    let notes = object.get("notes").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut layers = Vec::new();
    for layer in object.get("layers")?.as_array()? {
        layers.push(layer_from(layer)?);
    }
    if !layers.iter().any(|layer| !layer.folder) {
        return None;
    }
    Some(Clip {
        id: number(object, "id")?,
        name: text(object, "name"),
        shot: text(object, "shot"),
        start: number(object, "start")? as u32,
        length: (number(object, "length")? as u32).max(1),
        action: text(&notes, "action"),
        dialogue: text(&notes, "dialogue"),
        camera: text(&notes, "camera"),
        animator: text(object, "animator"),
        layers,
        origin: object
            .get("origin")
            .and_then(Value::as_array)
            .and_then(|origin| Some([origin.first()?.as_f64()?, origin.get(1)?.as_f64()?]))
            .filter(|origin| origin.iter().all(|value| value.is_finite()))
            .unwrap_or([0.0, 0.0]),
    })
}

fn layer_from(value: &Value) -> Option<Layer> {
    let layer = value.as_object()?;
    let color = layer.get("color").and_then(Value::as_array)?;
    let channel = |index: usize| color.get(index).and_then(Value::as_u64).map(|value| value.min(255) as u8);
    let mut keys = Vec::new();
    for key in layer.get("keys").and_then(Value::as_array).cloned().unwrap_or_default() {
        let key = key.as_object()?;
        keys.push(Key {
            id: number(key, "id")?,
            frame: number(key, "frame")? as u32,
            drawing: number(key, "drawing")?,
        });
    }
    let mut marks = Vec::new();
    for mark in layer.get("marks").and_then(Value::as_array).cloned().unwrap_or_default() {
        let mark = mark.as_object()?;
        marks.push(Mark {
            id: number(mark, "id")?,
            frame: number(mark, "frame")? as u32,
            comment: mark.get("kind").and_then(Value::as_str) == Some("comment"),
            text: text(mark, "text"),
        });
    }
    let mut row = Layer {
        id: number(layer, "id")?,
        name: text(layer, "name"),
        color: [channel(0)?, channel(1)?, channel(2)?],
        opacity: unit(layer, "opacity", 1.0),
        folder: flag(layer, "folder", false),
        parent: number(layer, "parent"),
        open: flag(layer, "open", true),
        visible: flag(layer, "visible", true),
        locked: flag(layer, "locked", false),
        outline: flag(layer, "outline", false),
        keys,
        marks,
    };
    row.sort();
    if !row.folder && !row.keys.iter().any(|key| key.frame == 0) {
        return None;
    }
    Some(row)
}

impl Onion {
    pub fn to_value(&self) -> Value {
        json!({
            "on": self.on,
            "before": self.before,
            "after": self.after,
            "opacity": self.opacity,
            "fade": self.fade,
            "keys_only": self.keys_only,
        })
    }

    pub fn from_value(value: Option<&Value>) -> Self {
        let default = Onion::default();
        let Some(onion) = value.and_then(Value::as_object) else {
            return default;
        };
        Onion {
            on: flag(onion, "on", default.on),
            before: number(onion, "before").map_or(default.before, |value| value.min(9) as u8),
            after: number(onion, "after").map_or(default.after, |value| value.min(9) as u8),
            opacity: unit(onion, "opacity", default.opacity),
            fade: unit(onion, "fade", default.fade),
            keys_only: flag(onion, "keys_only", default.keys_only),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn film_with(clips: u32) -> Film {
        let mut film = Film::new("The Last Signal");
        for _ in 0..clips {
            film.add_clip(None, NEW_CLIP_LENGTH);
        }
        film
    }

    #[test]
    fn clips_are_named_and_follow_one_another() {
        let film = film_with(3);
        let names: Vec<&str> = film.clips.iter().map(|clip| clip.name.as_str()).collect();
        assert_eq!(names, ["Shot 1", "Shot 2", "Shot 3"]);
        let starts: Vec<u32> = film.clips.iter().map(|clip| clip.start).collect();
        assert_eq!(starts, [0, 48, 96]);
        assert_eq!(film.length(), 144);
    }

    #[test]
    fn retiming_a_clip_moves_the_later_ones() {
        let mut film = film_with(3);
        let second = film.clips[1].id;
        assert!(film.resize_clip(second, true, 12));
        let starts: Vec<u32> = film.clips.iter().map(|clip| clip.start).collect();
        assert_eq!(starts, [0, 48, 108]);
        assert_eq!(film.clips[1].length, 60);
    }

    #[test]
    fn deleting_a_clip_leaves_its_gap() {
        let mut film = film_with(3);
        let second = film.clips[1].id;
        assert!(film.delete_clip(second));
        assert!(film.clip_at(60).is_none());
        assert_eq!(film.clips[1].start, 96);
    }

    #[test]
    fn moving_a_clip_keeps_its_id_and_pushes_or_replaces() {
        let mut film = film_with(3);
        let (first, third) = (film.clips[0].id, film.clips[2].id);
        // The third clip goes between the first and the second.
        assert!(film.move_clips(&[third], -48, true));
        let order: Vec<u64> = film.clips.iter().map(|clip| clip.id).collect();
        assert_eq!(order[0], first);
        assert_eq!(order[1], third);
        assert_eq!(film.clip(third).unwrap().start, 48);
        // Replacing covers the clip it lands on.
        let mut film = film_with(3);
        let third = film.clips[2].id;
        assert!(film.move_clips(&[third], -48, false));
        assert_eq!(film.clips.len(), 2);
    }

    #[test]
    fn duplicates_take_new_ids_and_copy_their_drawings() {
        let mut film = film_with(1);
        let first = film.clips[0].id;
        let (copy, drawings) = film.duplicate_clip(first).unwrap();
        assert_ne!(copy, first);
        assert_eq!(film.clip(copy).unwrap().name, "Shot 1 copy");
        assert_eq!(drawings.len(), 1);
        assert_eq!(drawings[0].copy_of, Some(film.clips[0].layers[0].keys[0].drawing));
        let all = film.drawings();
        let mut unique = all.clone();
        unique.dedup();
        assert_eq!(all.len(), unique.len());
    }

    #[test]
    fn keys_hold_until_the_next_and_frames_move_them() {
        let mut film = film_with(1);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        let made = film.insert_key(clip, layer, 12, false).unwrap();
        let first = film.clips[0].layers[0].keys[0].drawing;
        assert_eq!(made[0].copy_of, Some(first));
        assert_eq!(film.clips[0].layers[0].key_at(11).unwrap().drawing, first);
        assert!(film.insert_frames(clip, &[layer], 5, 2));
        assert_eq!(film.clips[0].layers[0].keys[1].frame, 14);
        assert_eq!(film.clips[0].length, 50);
        film.remove_frames(clip, &[layer], 0, 3).unwrap();
        let row = &film.clips[0].layers[0];
        assert_eq!(row.keys[0].frame, 0);
        assert_eq!(row.keys[1].frame, 11);
    }

    #[test]
    fn a_shift_drag_pushes_and_a_plain_one_replaces() {
        let mut film = film_with(1);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        for frame in [12, 24, 36] {
            film.insert_key(clip, layer, frame, true).unwrap();
        }
        let mut pushed = film.clone();
        pushed.move_frames(clip, &[layer], 12, 24, 8, true).unwrap();
        let frames: Vec<u32> = pushed.clips[0].layers[0].keys.iter().map(|key| key.frame).collect();
        assert_eq!(frames, [0, 20, 32, 44]);
        film.move_frames(clip, &[layer], 12, 24, 8, false).unwrap();
        let frames: Vec<u32> = film.clips[0].layers[0].keys.iter().map(|key| key.frame).collect();
        assert_eq!(frames, [0, 20, 36]);
    }

    #[test]
    fn stretching_a_hold_doubles_it() {
        let mut film = film_with(1);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        film.insert_key(clip, layer, 12, true).unwrap();
        film.insert_key(clip, layer, 24, true).unwrap();
        assert!(film.stretch_span(clip, layer, 12, 12));
        let frames: Vec<u32> = film.clips[0].layers[0].keys.iter().map(|key| key.frame).collect();
        assert_eq!(frames, [0, 12, 36]);
        assert_eq!(film.clips[0].length, 60);
    }

    #[test]
    fn reversing_puts_the_holds_in_the_other_order() {
        let mut film = film_with(1);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        film.insert_key(clip, layer, 4, true).unwrap();
        film.insert_key(clip, layer, 10, true).unwrap();
        let before: Vec<u64> = film.clips[0].layers[0].keys.iter().map(|key| key.drawing).collect();
        film.reverse_frames(clip, &[layer], 0, 10).unwrap();
        let row = &film.clips[0].layers[0];
        assert_eq!(row.key_at(0).unwrap().drawing, before[1]);
        assert_eq!(row.key_at(6).unwrap().drawing, before[0]);
        assert_eq!(row.key_at(10).unwrap().drawing, before[2]);
    }

    #[test]
    fn copied_frames_paste_as_copies() {
        let mut film = film_with(1);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        film.insert_key(clip, layer, 6, true).unwrap();
        let copied = film.copy_frames(clip, &[layer], 0, 12).unwrap();
        let made = film.paste_frames(clip, &[layer], 24, &copied).unwrap();
        assert!(made.iter().all(|drawing| drawing.copy_of.is_some()));
        let frames: Vec<u32> = film.clips[0].layers[0].keys.iter().map(|key| key.frame).collect();
        assert_eq!(frames, [0, 6, 24, 30, 36]);
    }

    #[test]
    fn folders_hide_and_lock_what_is_inside() {
        let mut film = film_with(1);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        let (folder, _) = film.add_layer(clip, None, true).unwrap();
        assert!(film.move_layer(clip, layer, 1, Some(folder)));
        film.clip_mut(clip).unwrap().layer_mut(folder).unwrap().visible = false;
        film.clip_mut(clip).unwrap().layer_mut(folder).unwrap().locked = true;
        let at = film.clip(clip).unwrap();
        let row = at.layer(layer).unwrap();
        assert!(!at.shows(row));
        assert!(at.locked(row));
        assert_eq!(at.depth(row), 1);
        // The last drawn layer cannot go.
        assert!(!film.delete_layer(clip, folder));
    }

    #[test]
    fn the_film_survives_a_round_trip() {
        let mut film = film_with(2);
        let clip = film.clips[0].id;
        let layer = film.clips[0].layers[0].id;
        film.insert_key(clip, layer, 12, false).unwrap();
        film.add_mark(clip, layer, 3, false, "radio on").unwrap();
        film.add_layer(clip, None, true).unwrap();
        film.clip_mut(clip).unwrap().dialogue = "VOICE: Mayday".into();
        let back = Film::from_value(&film.to_value()).unwrap();
        assert_eq!(back, film);
    }
}
