//! A layered image: its size, a tree of layers and groups, each layer's
//! pixels, and the composite of them all.
//!
//! The tree holds only each node's settings, so a step of history can keep a
//! copy of it cheaply. Pixels live apart, by layer id, and stay there after a
//! layer is deleted, so undoing the delete brings them back untouched. A
//! mask's grey pixels live there too, by the mask's own id, and a text's
//! pixels are drawn there from its words, kept until they change.
//!
//! Within each group, from the bottom up, a pixel layer, a text, or a group
//! blends as layers do, its coverage first multiplied by its mask; an
//! adjustment changes everything already combined beneath it in its group,
//! mixed in by its opacity and its mask.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A rectangle of whole pixels. Empty when either side is not positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect::new(x, y, (right - x).max(0), (bottom - y).max(0))
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, right - x, bottom - y)
    }

    pub fn contains(&self, other: &Rect) -> bool {
        other.is_empty() || self.intersect(other) == *other
    }
}

/// How a layer mixes with what is under it: the standard separable modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
}

impl Blend {
    pub const ALL: [Blend; 6] = [
        Blend::Normal,
        Blend::Multiply,
        Blend::Screen,
        Blend::Overlay,
        Blend::Darken,
        Blend::Lighten,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Multiply => "Multiply",
            Blend::Screen => "Screen",
            Blend::Overlay => "Overlay",
            Blend::Darken => "Darken",
            Blend::Lighten => "Lighten",
        }
    }

    pub fn index(self) -> usize {
        Blend::ALL.iter().position(|blend| *blend == self).unwrap_or(0)
    }

    /// One channel: `b` the backdrop, `s` the source, both from 0 to 1.
    fn mix(self, b: f32, s: f32) -> f32 {
        match self {
            Blend::Normal => s,
            Blend::Multiply => b * s,
            Blend::Screen => b + s - b * s,
            // Hard light with the layers swapped.
            Blend::Overlay => {
                if b <= 0.5 {
                    2.0 * b * s
                } else {
                    1.0 - 2.0 * (1.0 - b) * (1.0 - s)
                }
            }
            Blend::Darken => b.min(s),
            Blend::Lighten => b.max(s),
        }
    }
}

/// A layer's four locks. `all` also counts as each of the other three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Locks {
    #[serde(default)]
    pub transparency: bool,
    #[serde(default)]
    pub pixels: bool,
    #[serde(default)]
    pub position: bool,
    #[serde(default)]
    pub all: bool,
}

impl Locks {
    pub fn get(&self, index: usize) -> bool {
        match index {
            0 => self.transparency,
            1 => self.pixels,
            2 => self.position,
            _ => self.all,
        }
    }

    pub fn toggle(&mut self, index: usize) {
        match index {
            0 => self.transparency = !self.transparency,
            1 => self.pixels = !self.pixels,
            2 => self.position = !self.position,
            _ => self.all = !self.all,
        }
    }

    /// These locks and a parent's, together.
    fn with(self, other: Locks) -> Locks {
        Locks {
            transparency: self.transparency || other.transparency,
            pixels: self.pixels || other.pixels,
            position: self.position || other.position,
            all: self.all || other.all,
        }
    }

    pub fn keeps_alpha(&self) -> bool {
        self.transparency || self.all
    }

    pub fn keeps_pixels(&self) -> bool {
        self.pixels || self.all
    }

    pub fn keeps_position(&self) -> bool {
        self.position || self.all
    }
}

/// What a layer is. A group is a node with `children`, of any kind.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Kind {
    /// Pixels of its own, by its id.
    #[default]
    Pixels,
    /// Words in a box, drawn to pixels by its id.
    Text(Text),
    /// An adjustment by curves.
    Curves(Curves),
    /// An adjustment of exposure, contrast, saturation, and temperature.
    Grade(Grade),
}

impl Kind {
    pub fn is_adjustment(&self) -> bool {
        matches!(self, Kind::Curves(_) | Kind::Grade(_))
    }
}

/// A mask that belongs to a node: grey pixels by `id`, at the node's
/// offset, the node's size. White shows and black hides; outside its pixels
/// it shows. `feather` blurs it, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMask {
    pub id: u64,
    pub name: String,
    pub feather: f32,
    /// A hidden mask hides nothing.
    pub visible: bool,
}

/// The curves of a Curves layer: one for all three channels, then red,
/// green, and blue, each as points from 0 to 1, left to right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curves {
    pub rgb: Vec<[f64; 2]>,
    pub red: Vec<[f64; 2]>,
    pub green: Vec<[f64; 2]>,
    pub blue: Vec<[f64; 2]>,
}

impl Default for Curves {
    /// Every curve straight, changing nothing.
    fn default() -> Self {
        let straight = vec![[0.0, 0.0], [1.0, 1.0]];
        Self { rgb: straight.clone(), red: straight.clone(), green: straight.clone(), blue: straight }
    }
}

impl Curves {
    /// The channels' names, in the order the Properties page shows them.
    pub const CHANNELS: [&'static str; 4] = ["rgb", "red", "green", "blue"];

    pub fn channel(&self, index: usize) -> &Vec<[f64; 2]> {
        match index {
            1 => &self.red,
            2 => &self.green,
            3 => &self.blue,
            _ => &self.rgb,
        }
    }

    pub fn channel_mut(&mut self, index: usize) -> &mut Vec<[f64; 2]> {
        match index {
            1 => &mut self.red,
            2 => &mut self.green,
            3 => &mut self.blue,
            _ => &mut self.rgb,
        }
    }
}

/// Points as a curve takes them: at most sixteen, inside 0 to 1, sorted
/// left to right, at least two.
pub fn clean_points(points: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
    let mut points: Vec<[f64; 2]> = points
        .iter()
        .filter(|point| point[0].is_finite() && point[1].is_finite())
        .map(|point| [point[0].clamp(0.0, 1.0), point[1].clamp(0.0, 1.0)])
        .collect();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    points.truncate(16);
    (points.len() >= 2).then_some(points)
}

/// A Color Grade layer's settings, each 0 at rest: exposure in stops from
/// −3 to +3, and contrast, saturation, and temperature from −100 to +100.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Grade {
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
}

impl Grade {
    pub fn clamped(self) -> Self {
        let keep = |value: f32, most: f32| if value.is_finite() { value.clamp(-most, most) } else { 0.0 };
        Self {
            exposure: keep(self.exposure, 3.0),
            contrast: keep(self.contrast, 100.0),
            saturation: keep(self.saturation, 100.0),
            temperature: keep(self.temperature, 100.0),
        }
    }
}

pub use crate::text::Text;

/// One layer or group. A group has `children`, bottom to top.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub blend: Blend,
    /// Scales the layer, or the whole group, from 0 to 1.
    pub opacity: f32,
    /// Scales only the layer's own pixels' alpha, from 0 to 1.
    pub fill: f32,
    pub locks: Locks,
    /// Where the layer's pixels sit in the image: a text's box corner, and
    /// where a node's mask sits.
    pub offset: [i32; 2],
    pub children: Option<Vec<Node>>,
    pub kind: Kind,
    pub mask: Option<LayerMask>,
}

impl Node {
    pub fn layer(id: u64, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            visible: true,
            blend: Blend::Normal,
            opacity: 1.0,
            fill: 1.0,
            locks: Locks::default(),
            offset: [0, 0],
            children: None,
            kind: Kind::Pixels,
            mask: None,
        }
    }

    /// A node of another kind than pixels.
    pub fn of_kind(id: u64, name: &str, kind: Kind) -> Self {
        Self { kind, ..Self::layer(id, name) }
    }

    pub fn group(id: u64, name: &str, children: Vec<Node>) -> Self {
        Self {
            children: Some(children),
            ..Self::layer(id, name)
        }
    }

    pub fn is_group(&self) -> bool {
        self.children.is_some()
    }

    /// A layer with pixels of its own to paint.
    pub fn has_pixels(&self) -> bool {
        !self.is_group() && self.kind == Kind::Pixels
    }

    pub fn text(&self) -> Option<&Text> {
        match &self.kind {
            Kind::Text(text) if !self.is_group() => Some(text),
            _ => None,
        }
    }
}

/// A layer's pixels: RGBA8, straight alpha, rows top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Pixels {
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            data.extend_from_slice(&rgba);
        }
        Self { width, height, data }
    }

    pub fn clear(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The bytes of `rect`, in this buffer's own pixels, row after row.
    pub fn read(&self, rect: Rect) -> Vec<u8> {
        let mut out = Vec::with_capacity(rect.w.max(0) as usize * rect.h.max(0) as usize * 4);
        for y in rect.y..rect.bottom() {
            let start = (y as usize * self.width as usize + rect.x as usize) * 4;
            out.extend_from_slice(&self.data[start..start + rect.w as usize * 4]);
        }
        out
    }

    /// Puts back bytes `read` took from `rect`.
    pub fn write(&mut self, rect: Rect, bytes: &[u8]) {
        let row = rect.w as usize * 4;
        for (index, y) in (rect.y..rect.bottom()).enumerate() {
            let start = (y as usize * self.width as usize + rect.x as usize) * 4;
            self.data[start..start + row].copy_from_slice(&bytes[index * row..(index + 1) * row]);
        }
    }

    /// A part of these pixels, with what falls outside them transparent.
    /// `rect` is in this buffer's own pixels and may reach past its edges.
    pub fn cut(&self, rect: Rect) -> Pixels {
        let mut out = Pixels::clear(rect.w.max(1) as u32, rect.h.max(1) as u32);
        let own = Rect::new(0, 0, self.width as i32, self.height as i32);
        let shared = own.intersect(&rect);
        if !shared.is_empty() {
            let bytes = self.read(shared);
            out.write(Rect::new(shared.x - rect.x, shared.y - rect.y, shared.w, shared.h), &bytes);
        }
        out
    }
}

/// The image: its size, the tree, and every layer's pixels.
#[derive(Debug, Clone)]
pub struct LayeredImage {
    pub width: u32,
    pub height: u32,
    /// Bottom to top.
    pub nodes: Vec<Node>,
    pub pixels: HashMap<u64, Pixels>,
    pub next_id: u64,
    /// What each text's pixels were drawn from, so they are drawn again only
    /// when its words, style, or box change.
    drawn: HashMap<u64, Text>,
    /// Each feathered mask blurred, by mask id, with the feather it was
    /// blurred by, until it changes: the image is put together in bands
    /// of rows, so a mask is blurred once rather than per band.
    feathered: HashMap<u64, (f32, Vec<u8>)>,
    /// Masks changed since they were blurred: the part changed, in the
    /// mask's own pixels, or none for all of it.
    stale_masks: HashMap<u64, Option<Rect>>,
}

/// Where a node sits: its parent group, or none at the top, and its index
/// among its siblings, bottom first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub parent: Option<u64>,
    pub index: usize,
}

impl LayeredImage {
    /// An image of no layers.
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            nodes: Vec::new(),
            pixels: HashMap::new(),
            next_id: 1,
            drawn: HashMap::new(),
            feathered: HashMap::new(),
            stale_masks: HashMap::new(),
        }
    }

    /// A new image with one empty layer, "Layer 1", as the owner set on
    /// 2026-11-01.
    pub fn clear(width: u32, height: u32) -> Self {
        let mut image = Self::empty(width, height);
        let id = image.take_id();
        image.nodes.push(Node::layer(id, "Layer 1"));
        image.pixels.insert(id, Pixels::clear(width, height));
        image
    }

    /// An image with one white "Background" layer.
    pub fn blank(width: u32, height: u32) -> Self {
        let mut image = Self::empty(width, height);
        let id = image.take_id();
        image.nodes.push(Node::layer(id, "Background"));
        image.pixels.insert(id, Pixels::filled(width, height, [255, 255, 255, 255]));
        image
    }

    /// An image made of one image, as its only layer.
    pub fn from_pixels(name: &str, pixels: Pixels) -> Self {
        let mut image = Self::empty(pixels.width, pixels.height);
        let id = image.take_id();
        image.nodes.push(Node::layer(id, name));
        image.pixels.insert(id, pixels);
        image
    }

    pub fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    pub fn find(&self, id: u64) -> Option<&Node> {
        find_in(&self.nodes, id)
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Node> {
        find_in_mut(&mut self.nodes, id)
    }

    pub fn place(&self, id: u64) -> Option<Place> {
        place_in(&self.nodes, None, id)
    }

    /// The siblings under `parent`, or the top level.
    pub fn siblings(&self, parent: Option<u64>) -> Option<&Vec<Node>> {
        match parent {
            None => Some(&self.nodes),
            Some(id) => self.find(id)?.children.as_ref(),
        }
    }

    pub fn siblings_mut(&mut self, parent: Option<u64>) -> Option<&mut Vec<Node>> {
        match parent {
            None => Some(&mut self.nodes),
            Some(id) => self.find_mut(id)?.children.as_mut(),
        }
    }

    /// The node's own locks and every enclosing group's, together.
    pub fn locks(&self, id: u64) -> Locks {
        let mut locks = Locks::default();
        let mut current = Some(id);
        while let Some(at) = current {
            let Some(node) = self.find(at) else {
                break;
            };
            locks = locks.with(node.locks);
            current = self.place(at).and_then(|place| place.parent);
        }
        locks
    }

    /// The node and every node inside it.
    pub fn ids_within(&self, id: u64) -> Vec<u64> {
        let mut out = Vec::new();
        if let Some(node) = self.find(id) {
            collect_ids(node, &mut out);
        }
        out
    }

    /// Every layer that has pixels, under `id`, or `id` itself.
    pub fn layers_within(&self, id: u64) -> Vec<u64> {
        self.ids_within(id)
            .into_iter()
            .filter(|id| self.find(*id).is_some_and(|node| !node.is_group()))
            .collect()
    }

    /// Every layer with pixels in the image.
    pub fn all_layers(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for node in &self.nodes {
            collect_ids(node, &mut out);
        }
        out.retain(|id| self.find(*id).is_some_and(|node| !node.is_group()));
        out
    }

    /// A layer's box in the image, or a group's: the union of its layers'.
    pub fn node_rect(&self, id: u64) -> Option<Rect> {
        let node = self.find(id)?;
        if node.is_group() {
            let mut rect = Rect::new(0, 0, 0, 0);
            for layer in self.layers_within(id) {
                if let Some(inner) = self.node_rect(layer) {
                    rect = rect.union(&inner);
                }
            }
            return (!rect.is_empty()).then_some(rect);
        }
        let pixels = self.pixels.get(&id)?;
        Some(Rect::new(node.offset[0], node.offset[1], pixels.width as i32, pixels.height as i32))
    }

    /// Takes a node out of the tree, and says where it was.
    pub fn remove(&mut self, id: u64) -> Option<(Node, Place)> {
        let place = self.place(id)?;
        let siblings = self.siblings_mut(place.parent)?;
        Some((siblings.remove(place.index), place))
    }

    /// Puts a node in at `place`, its index clamped to the siblings.
    pub fn insert(&mut self, node: Node, place: Place) -> bool {
        let Some(siblings) = self.siblings_mut(place.parent) else {
            return false;
        };
        let index = place.index.min(siblings.len());
        siblings.insert(index, node);
        true
    }

    /// Composites `rect` of the image into `out`, the whole image as
    /// RGBA8 with straight alpha. Masks are feathered and texts drawn first,
    /// by [`LayeredImage::prepare`].
    #[cfg(test)]
    pub fn composite_into(&self, rect: Rect, out: &mut [u8]) {
        self.composite_over(None, rect, out);
    }

    /// Composites `rect` of the image into `out`, the whole image as
    /// RGBA8 with straight alpha, starting from `beneath` when it fits: the
    /// image as the nodes beneath a node combine it, at each depth of the
    /// groups it lies in, so only that node and those above it are combined
    /// again. Each pixel is combined by itself, so the result is the same,
    /// to the bit, as combining every node.
    pub fn composite_over(&self, beneath: Option<&Beneath>, rect: Rect, out: &mut [u8]) {
        self.combine_into(&self.nodes, beneath.filter(|beneath| beneath.fits(self)), rect, out);
    }

    /// The image as the nodes beneath `path` combine it, for the document
    /// to keep while that node is changed: `path` is how many siblings lie
    /// beneath it at each depth, the top level first, then within each
    /// group it goes into. A group's level is its children's own, combined
    /// by themselves, as the group combines them. Made in bands of rows,
    /// one per core.
    pub fn beneath(&self, path: &[usize]) -> Beneath {
        let mut kept = Vec::new();
        let mut siblings: &[Node] = &self.nodes;
        for (depth, count) in path.iter().enumerate() {
            let count = (*count).min(siblings.len());
            kept.push(siblings[..count].to_vec());
            if depth + 1 == path.len() {
                break;
            }
            match siblings.get(count).and_then(|node| node.children.as_deref()) {
                Some(children) => siblings = children,
                None => break,
            }
        }
        let width = self.width as usize;
        let mut levels: Vec<Vec<[f32; 4]>> = kept.iter().map(|_| vec![[0.0f32; 4]; width * self.height as usize]).collect();
        let rect = self.bounds();
        if !rect.is_empty() {
            let adjusts = Adjusts::of(&self.nodes);
            let band_rows = (rect.h as usize).div_ceil(bands(rect));
            let tile = tile_rows(width).min(band_rows);
            let spare = tile * width * depth(&self.nodes);
            // Each band's rows of every level.
            let mut pieces: Vec<Vec<&mut [[f32; 4]]>> = Vec::new();
            for level in &mut levels {
                for (index, rows) in level.chunks_mut(band_rows * width).enumerate() {
                    if pieces.len() <= index {
                        pieces.push(Vec::new());
                    }
                    pieces[index].push(rows);
                }
            }
            std::thread::scope(|scope| {
                for (index, levels) in pieces.into_iter().enumerate() {
                    let (kept, adjusts) = (&kept, &adjusts);
                    scope.spawn(move || {
                        let mut spare = vec![[0.0f32; 4]; spare];
                        for (nodes, rows) in kept.iter().zip(levels) {
                            for (part, buffer) in rows.chunks_mut(tile * width).enumerate() {
                                let part = Rect::new(0, (index * band_rows + part * tile) as i32, self.width as i32, (buffer.len() / width) as i32);
                                self.composite_nodes(nodes, adjusts, part, buffer, &mut spare);
                            }
                        }
                    });
                }
            });
        }
        Beneath { kept, width: self.width, height: self.height, levels }
    }

    /// Composites `nodes`, a tree of this image's, into `rect` of `out`,
    /// from `beneath`, which lies beneath the image's own nodes, or from
    /// nothing.
    fn combine_into(&self, nodes: &[Node], beneath: Option<&Beneath>, rect: Rect, out: &mut [u8]) {
        let rect = rect.intersect(&self.bounds());
        if rect.is_empty() {
            return;
        }
        let adjusts = Adjusts::of(nodes);
        let row_bytes = self.width as usize * 4;
        let width = rect.w as usize;
        let rows = &mut out[rect.y as usize * row_bytes..rect.bottom() as usize * row_bytes];
        // A large area is split into bands of rows, one per core, as each
        // pixel is composited by itself. The last band is this thread's.
        let band_rows = (rect.h as usize).div_ceil(bands(rect));
        // Each band works in a few rows at a time, which stay in the core's
        // cache while every node is combined into them, with as many more
        // for the groups it goes into.
        let tile = tile_rows(width).min(band_rows);
        let scratch = tile * width * (1 + depth(nodes));
        with_scratch(scratch * (rect.h as usize).div_ceil(band_rows), |buffers| {
            std::thread::scope(|scope| {
                let mut pieces = rows.chunks_mut(band_rows * row_bytes).zip(buffers.chunks_mut(scratch)).enumerate().peekable();
                while let Some((index, (chunk, buffer))) = pieces.next() {
                    let band = Rect::new(rect.x, rect.y + (index * band_rows) as i32, rect.w, (chunk.len() / row_bytes) as i32);
                    let adjusts = &adjusts;
                    if pieces.peek().is_none() {
                        self.composite_band(nodes, beneath, adjusts, band, chunk, buffer, tile);
                    } else {
                        scope.spawn(move || self.composite_band(nodes, beneath, adjusts, band, chunk, buffer, tile));
                    }
                }
            });
        });
    }

    /// Composites `rect` into `rows`, the image's rows it covers, `tile`
    /// rows at a time at the start of `buffer`, which starts as the kept
    /// image beneath there or clear; the rest of `buffer` is for groups.
    #[allow(clippy::too_many_arguments)]
    fn composite_band(&self, nodes: &[Node], beneath: Option<&Beneath>, adjusts: &Adjusts, rect: Rect, rows: &mut [u8], buffer: &mut [[f32; 4]], tile: usize) {
        let width = rect.w as usize;
        let row_bytes = self.width as usize * 4;
        let (buffer, spare) = buffer.split_at_mut(tile * width);
        for first in (0..rect.h as usize).step_by(tile.max(1)) {
            let count = tile.min(rect.h as usize - first);
            let part = Rect::new(rect.x, rect.y + first as i32, rect.w, count as i32);
            let buffer = &mut buffer[..count * width];
            match beneath {
                Some(beneath) => {
                    self.copy_level(&beneath.levels[0], part, buffer);
                    self.composite_kept(nodes, beneath, 0, adjusts, part, buffer, spare);
                }
                None => {
                    buffer.fill([0.0; 4]);
                    self.composite_nodes(nodes, adjusts, part, buffer, spare);
                }
            }
            self.to_bytes(part, buffer, &mut rows[first * row_bytes..(first + count) * row_bytes]);
        }
    }

    /// `rect` of a kept level, the whole image premultiplied, into
    /// `buffer`.
    fn copy_level(&self, level: &[[f32; 4]], rect: Rect, buffer: &mut [[f32; 4]]) {
        let (stride, width) = (self.width as usize, rect.w as usize);
        for row in 0..rect.h as usize {
            let start = (rect.y as usize + row) * stride + rect.x as usize;
            buffer[row * width..(row + 1) * width].copy_from_slice(&level[start..start + width]);
        }
    }

    /// Blends `nodes` into `buffer`, which holds what `beneath` keeps at
    /// `depth` beneath them: the siblings above it, and, when the kept
    /// image goes into a group, that group from its children kept within
    /// it, as [`LayeredImage::composite_nodes`] combines a group.
    #[allow(clippy::too_many_arguments)]
    fn composite_kept(&self, nodes: &[Node], beneath: &Beneath, depth: usize, adjusts: &Adjusts, rect: Rect, buffer: &mut [[f32; 4]], spare: &mut [[f32; 4]]) {
        let count = beneath.kept[depth].len();
        let (Some(level), Some(group)) = (beneath.levels.get(depth + 1), nodes.get(count)) else {
            self.composite_nodes(&nodes[count.min(nodes.len())..], adjusts, rect, buffer, spare);
            return;
        };
        if group.visible && group.opacity > 0.0 {
            let (inner, spare) = spare.split_at_mut(buffer.len());
            self.copy_level(level, rect, inner);
            self.composite_kept(group.children.as_deref().unwrap_or_default(), beneath, depth + 1, adjusts, rect, inner, spare);
            self.blend_group(group, inner, rect, buffer);
        }
        self.composite_nodes(&nodes[count + 1..], adjusts, rect, buffer, spare);
    }

    /// `rect` of premultiplied `buffer` as RGBA8 with straight alpha, into
    /// `rows`, the image's rows it covers.
    fn to_bytes(&self, rect: Rect, buffer: &[[f32; 4]], rows: &mut [u8]) {
        let width = rect.w as usize;
        let row_bytes = self.width as usize * 4;
        for row in 0..rect.h as usize {
            let start = row * row_bytes + rect.x as usize * 4;
            let line = &mut rows[start..start + width * 4];
            for (column, pixel) in buffer[row * width..(row + 1) * width].iter().enumerate() {
                let alpha = pixel[3];
                let byte = &mut line[column * 4..column * 4 + 4];
                if alpha <= 0.0 {
                    byte.copy_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                // Dividing by one changes nothing, so it is not done.
                if alpha == 1.0 {
                    for channel in 0..3 {
                        byte[channel] = to_byte(pixel[channel]);
                    }
                    byte[3] = 255;
                    continue;
                }
                for channel in 0..3 {
                    byte[channel] = to_byte(pixel[channel] / alpha);
                }
                byte[3] = to_byte(alpha);
            }
        }
    }

    /// The whole composite. The editor keeps its own and redoes only what
    /// changed; the tests take it whole.
    #[cfg(test)]
    pub fn composite(&mut self) -> Vec<u8> {
        self.prepare();
        let mut out = vec![0; self.width as usize * self.height as usize * 4];
        self.composite_into(self.bounds(), &mut out);
        out
    }

    /// The image as the layers up to and including `id` make it, in the
    /// order they combine: what lies above it, adjustments too, is left
    /// out. It is what a stamp copies from.
    pub fn composite_through(&self, id: u64) -> Vec<u8> {
        let nodes = self.up_to(id, true);
        let mut out = vec![0; self.width as usize * self.height as usize * 4];
        self.combine_into(&nodes, None, self.bounds(), &mut out);
        out
    }

    /// Every `step`th row of the image as the layers beneath `id` make
    /// it, `id` left out, such as for the histogram a Curves layer is set
    /// against. Rows between are left clear. Bands of whole steps are made
    /// side by side.
    pub fn rows_beneath(&self, id: u64, step: usize) -> Vec<u8> {
        let nodes = self.up_to(id, false);
        let adjusts = Adjusts::of(&nodes);
        let (width, height) = (self.width as usize, self.height as usize);
        let mut out = vec![0; width * height * 4];
        if width == 0 || height == 0 {
            return out;
        }
        let step = step.max(1);
        let row_bytes = width * 4;
        let band_rows = height.div_ceil(step).div_ceil(bands(self.bounds())).max(1) * step;
        std::thread::scope(|scope| {
            for (index, chunk) in out.chunks_mut(band_rows * row_bytes).enumerate() {
                let (nodes, adjusts) = (&nodes, &adjusts);
                scope.spawn(move || {
                    let mut buffer = vec![[0.0f32; 4]; width * (1 + depth(nodes))];
                    for local in (0..chunk.len() / row_bytes).step_by(step) {
                        let row = Rect::new(0, (index * band_rows + local) as i32, width as i32, 1);
                        self.composite_band(nodes, None, adjusts, row, &mut chunk[local * row_bytes..(local + 1) * row_bytes], &mut buffer, 1);
                    }
                });
            }
        });
        out
    }

    /// The tree up to `id`, with `id` itself when `with`: what lies above it
    /// left out.
    pub fn up_to(&self, id: u64, with: bool) -> Vec<Node> {
        fn prune(nodes: &[Node], id: u64, with: bool) -> (Vec<Node>, bool) {
            let mut kept = Vec::new();
            for node in nodes {
                if node.id == id {
                    if with {
                        kept.push(node.clone());
                    }
                    return (kept, true);
                }
                if let Some(children) = &node.children {
                    let (inner, found) = prune(children, id, with);
                    if found {
                        kept.push(Node { children: Some(inner), ..node.clone() });
                        return (kept, true);
                    }
                }
                kept.push(node.clone());
            }
            (kept, false)
        }
        prune(&self.nodes, id, with).0
    }

    /// Blends `nodes`, bottom to top, into `buffer`: `rect` of the image,
    /// premultiplied. `adjusts` holds the tables of their adjustments, and
    /// `spare` a buffer's room for each depth of groups, so a group is
    /// combined without asking for fresh memory.
    fn composite_nodes(&self, nodes: &[Node], adjusts: &Adjusts, rect: Rect, buffer: &mut [[f32; 4]], spare: &mut [[f32; 4]]) {
        let table = unit_table();
        let at = |index: usize| (rect.x + (index % rect.w as usize) as i32, rect.y + (index / rect.w as usize) as i32);
        for node in nodes.iter().filter(|node| node.visible && node.opacity > 0.0) {
            let mask = self.mask_view(node);
            let shown = |index: usize| mask.as_ref().map_or(1.0, |mask| {
                let (x, y) = at(index);
                mask.at(x, y)
            });
            if let Some(children) = &node.children {
                // A group composites its children by themselves, then
                // blends the result as one layer, through its mask.
                let (inner, spare) = spare.split_at_mut(buffer.len());
                inner.fill([0.0; 4]);
                self.composite_nodes(children, adjusts, rect, inner, spare);
                self.blend_group(node, inner, rect, buffer);
                continue;
            }
            if let Some(adjust) = adjusts.get(node.id) {
                // An adjustment changes what is combined beneath it.
                for (index, dst) in buffer.iter_mut().enumerate() {
                    let alpha = dst[3];
                    if alpha <= 0.0 {
                        continue;
                    }
                    let amount = node.opacity * shown(index);
                    if amount <= 0.0 {
                        continue;
                    }
                    // Most pixels are opaque, and dividing by one changes
                    // nothing, so it is not done.
                    let rgb = if alpha == 1.0 { [dst[0], dst[1], dst[2]] } else { [dst[0] / alpha, dst[1] / alpha, dst[2] / alpha] };
                    let changed = adjust.apply(rgb);
                    for channel in 0..3 {
                        dst[channel] = (rgb[channel] + (changed[channel] - rgb[channel]) * amount) * alpha;
                    }
                }
                continue;
            }
            let Some(pixels) = self.pixels.get(&node.id) else {
                continue;
            };
            let layer = Rect::new(node.offset[0], node.offset[1], pixels.width as i32, pixels.height as i32);
            let shared = layer.intersect(&rect);
            if shared.is_empty() {
                continue;
            }
            let scale = node.opacity * node.fill;
            if scale <= 0.0 {
                continue;
            }
            for y in shared.y..shared.bottom() {
                let source_row = (y - layer.y) as usize * pixels.width as usize;
                let target_row = (y - rect.y) as usize * rect.w as usize;
                for x in shared.x..shared.right() {
                    let at = (source_row + (x - layer.x) as usize) * 4;
                    let alpha = pixels.data[at + 3];
                    if alpha == 0 {
                        continue;
                    }
                    let covered = mask.as_ref().map_or(1.0, |mask| mask.at(x, y));
                    if covered <= 0.0 {
                        continue;
                    }
                    let rgb = [
                        table[pixels.data[at] as usize],
                        table[pixels.data[at + 1] as usize],
                        table[pixels.data[at + 2] as usize],
                    ];
                    let dst = &mut buffer[target_row + (x - rect.x) as usize];
                    blend_pixel(dst, rgb, table[alpha as usize] * scale * covered, node.blend);
                }
            }
        }
    }

    /// Blends a group's children, combined by themselves in `inner`, into
    /// `buffer` as one layer, through its mask.
    fn blend_group(&self, node: &Node, inner: &[[f32; 4]], rect: Rect, buffer: &mut [[f32; 4]]) {
        let mask = self.mask_view(node);
        let shown = |index: usize| {
            mask.as_ref().map_or(1.0, |mask| mask.at(rect.x + (index % rect.w as usize) as i32, rect.y + (index / rect.w as usize) as i32))
        };
        for (index, (dst, src)) in buffer.iter_mut().zip(inner.iter()).enumerate() {
            if src[3] <= 0.0 {
                continue;
            }
            let alpha = src[3];
            let rgb = [src[0] / alpha, src[1] / alpha, src[2] / alpha];
            blend_pixel(dst, rgb, alpha * node.opacity * shown(index), node.blend);
        }
    }

    /// A node's mask as the composite reads it: blurred when feathered, and
    /// none when it has none or it is hidden.
    fn mask_view(&self, node: &Node) -> Option<MaskView<'_>> {
        let mask = node.mask.as_ref().filter(|mask| mask.visible)?;
        let pixels = self.pixels.get(&mask.id)?;
        let rect = Rect::new(node.offset[0], node.offset[1], pixels.width as i32, pixels.height as i32);
        match self.feathered.get(&mask.id) {
            Some((feather, values)) if *feather == mask.feather && mask.feather >= 0.5 => Some(MaskView { rect, data: values, stride: 1 }),
            _ => Some(MaskView { rect, data: &pixels.data, stride: 4 }),
        }
    }

    /// Brings what the composite reads up to date: each text drawn again
    /// when its words, style, or box changed, and each feathered mask
    /// blurred again where it or its feather changed. Returns the part of
    /// the image that changed with them: a redrawn text, and everything a
    /// mask's blur reaches from where it changed, so no seam is left.
    pub fn prepare(&mut self) -> Option<Rect> {
        let mut touched = self.sync_texts();
        let mut masks = Vec::new();
        for node in self.every_node() {
            if let Some(mask) = node.mask.as_ref().filter(|mask| mask.feather >= 0.5) {
                masks.push((mask.id, mask.feather, node.offset));
            }
        }
        let mut reach_into = |rect: Rect| touched = Some(touched.map_or(rect, |touched: Rect| touched.union(&rect)));
        for (id, feather, offset) in masks {
            let stale = self.stale_masks.remove(&id);
            let Some(pixels) = self.pixels.get(&id) else {
                continue;
            };
            let (width, height) = (pixels.width as usize, pixels.height as usize);
            let whole = Rect::new(0, 0, width as i32, height as i32);
            let kept = self.feathered.get(&id).filter(|(kept, values)| *kept == feather && values.len() == width * height).is_some();
            // How far one value of the blur reaches: three passes of a box.
            let reach = 3 * ((f64::from(feather) / 3.0).ceil() as i32).max(1);
            let at = |rect: Rect| Rect::new(rect.x + offset[0], rect.y + offset[1], rect.w, rect.h);
            match (kept, stale) {
                (true, None) => continue,
                // Only around the part that changed: blurred from twice the
                // reach around it, and kept within the reach.
                (true, Some(Some(changed))) => {
                    let write = Rect::new(changed.x - reach, changed.y - reach, changed.w + 2 * reach, changed.h + 2 * reach).intersect(&whole);
                    let read = Rect::new(changed.x - 2 * reach, changed.y - 2 * reach, changed.w + 4 * reach, changed.h + 4 * reach).intersect(&whole);
                    if write.is_empty() {
                        continue;
                    }
                    let (rw, rh) = (read.w as usize, read.h as usize);
                    let mut values = Vec::with_capacity(rw * rh);
                    for y in read.y..read.bottom() {
                        let row = &pixels.data[(y as usize * width + read.x as usize) * 4..(y as usize * width + read.right() as usize) * 4];
                        values.extend(row.chunks_exact(4).map(|pixel| f32::from(grey(pixel))));
                    }
                    blur(&mut values, rw, rh, f64::from(feather));
                    if let Some((_, bytes)) = self.feathered.get_mut(&id) {
                        for y in write.y..write.bottom() {
                            for x in write.x..write.right() {
                                let value = values[(y - read.y) as usize * rw + (x - read.x) as usize];
                                bytes[y as usize * width + x as usize] = value.round().clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                    reach_into(at(write));
                }
                // All of it, in bands of rows, into the memory the last
                // blur used, as a held Feather slider blurs it many times.
                _ => {
                    let band = height.div_ceil(bands(whole)) * width;
                    let mut values = std::mem::take(&mut *GREYS.lock().unwrap_or_else(PoisonError::into_inner));
                    values.resize(width * height, 0.0);
                    in_bands(&mut values, band, |index, out| {
                        for (value, pixel) in out.iter_mut().zip(pixels.data[index * band * 4..].chunks_exact(4)) {
                            *value = f32::from(grey(pixel));
                        }
                    });
                    blur(&mut values, width, height, f64::from(feather));
                    let mut bytes = match self.feathered.remove(&id) {
                        Some((_, bytes)) if bytes.len() == width * height => bytes,
                        _ => vec![0; width * height],
                    };
                    in_bands(&mut bytes, band, |index, out| {
                        for (byte, value) in out.iter_mut().zip(&values[index * band..]) {
                            *byte = value.round().clamp(0.0, 255.0) as u8;
                        }
                    });
                    *GREYS.lock().unwrap_or_else(PoisonError::into_inner) = values;
                    self.feathered.insert(id, (feather, bytes));
                    reach_into(at(whole));
                }
            }
        }
        touched
    }

    /// Forgets what was made from a buffer's pixels, as they all changed.
    pub fn forget(&mut self, id: u64) {
        self.stale_masks.insert(id, None);
    }

    /// Forgets what was made from part of a buffer's pixels, `rect` in the
    /// buffer's own pixels, as only it changed.
    pub fn forget_area(&mut self, id: u64, rect: Rect) {
        match self.stale_masks.get_mut(&id) {
            Some(None) => {}
            Some(Some(stale)) => *stale = stale.union(&rect),
            None => {
                self.stale_masks.insert(id, Some(rect));
            }
        }
    }

    /// Draws each text again whose words, style, or box changed since it
    /// was drawn. Returns the part of the image that changed.
    pub fn sync_texts(&mut self) -> Option<Rect> {
        let texts: Vec<(u64, Text)> = self.every_node().into_iter().filter_map(|node| node.text().map(|text| (node.id, text.clone()))).collect();
        let mut touched: Option<Rect> = None;
        for (id, text) in texts {
            if self.drawn.get(&id) == Some(&text) && self.pixels.contains_key(&id) {
                continue;
            }
            let before = self.node_rect(id);
            self.pixels.insert(id, crate::text::draw(&text));
            self.drawn.insert(id, text);
            for rect in [before, self.node_rect(id)].into_iter().flatten() {
                touched = Some(touched.map_or(rect, |touched| touched.union(&rect)));
            }
        }
        touched
    }

    /// Every node in the tree, groups and what they hold.
    pub fn every_node(&self) -> Vec<&Node> {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Node>) {
            for node in nodes {
                out.push(node);
                if let Some(children) = &node.children {
                    walk(children, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }

    /// The node a mask belongs to, by the mask's id.
    pub fn mask_owner(&self, mask: u64) -> Option<&Node> {
        self.every_node().into_iter().find(|node| node.mask.as_ref().is_some_and(|own| own.id == mask))
    }

    /// Where a buffer of pixels sits: a layer's at its offset, a mask's at
    /// its node's.
    pub fn buffer_offset(&self, id: u64) -> [i32; 2] {
        self.find(id).or_else(|| self.mask_owner(id)).map_or([0, 0], |node| node.offset)
    }

    /// The size a new mask on a node is: a pixel layer's own size, or the
    /// image's for a text, an adjustment, or a group.
    pub fn mask_size(&self, id: u64) -> (u32, u32) {
        match (self.find(id), self.pixels.get(&id)) {
            (Some(node), Some(pixels)) if node.has_pixels() => (pixels.width, pixels.height),
            _ => (self.width, self.height),
        }
    }

    /// The buffers a file keeps as PNGs: every pixel layer's and every
    /// mask's. A text keeps its words, not its pixels.
    pub fn buffers(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for node in self.every_node() {
            if node.has_pixels() {
                out.push(node.id);
            }
            if let Some(mask) = &node.mask {
                out.push(mask.id);
            }
        }
        out
    }
}

/// A mask as the composite reads it: grey values over `rect` of the image,
/// `stride` bytes apart. Outside it, everything shows.
struct MaskView<'a> {
    rect: Rect,
    data: &'a [u8],
    stride: usize,
}

impl MaskView<'_> {
    fn at(&self, x: i32, y: i32) -> f32 {
        if x < self.rect.x || y < self.rect.y || x >= self.rect.right() || y >= self.rect.bottom() {
            return 1.0;
        }
        let index = ((y - self.rect.y) as usize * self.rect.w as usize + (x - self.rect.x) as usize) * self.stride;
        let value = if self.stride == 4 { grey(&self.data[index..index + 4]) } else { self.data[index] };
        unit_table()[value as usize]
    }
}

/// A mask pixel's grey: its red, or white where it is clear.
pub fn grey(pixel: &[u8]) -> u8 {
    let alpha = u32::from(pixel[3]);
    ((u32::from(pixel[0]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
}

/// Blurs grey values by about `radius` pixels in place: three box blurs,
/// which come close to a Gaussian, each the average of the values inside
/// the image only, so an edge keeps its value. One thread per core takes
/// a band of rows, then a band of columns, in turn, each line summed in the
/// same order as [`blur_alone`] does, so the result is the same to the bit.
pub fn blur(values: &mut [f32], width: usize, height: usize, radius: f64) {
    if !radius.is_finite() || radius < 0.5 || width == 0 || height == 0 {
        return;
    }
    let pass = ((radius / 3.0).ceil() as usize).max(1);
    let cores = bands(Rect::new(0, 0, width as i32, height as i32));
    let (band_rows, band_columns) = (height.div_ceil(cores), width.div_ceil(cores));
    // Each band of columns is blurred into its own part of `columns`, row
    // by row, as wide as the band, and the next pass's rows are read back
    // from there.
    let mut columns = std::mem::take(&mut *BLURRED.lock().unwrap_or_else(PoisonError::into_inner));
    columns.resize(width * height, 0.0);
    {
        let rows: Vec<RwLock<&mut [f32]>> = values.chunks_mut(band_rows * width).map(RwLock::new).collect();
        let parts: Vec<RwLock<&mut [f32]>> = columns.chunks_mut(band_columns * height).map(RwLock::new).collect();
        let threads = rows.len().max(parts.len());
        let barrier = Barrier::new(threads);
        let (rows, parts, barrier) = (&rows, &parts, &barrier);
        // Row `y` of what the columns made, into `line`.
        let gather = |parts: &[RwLockReadGuard<&mut [f32]>], y: usize, line: &mut [f32]| {
            for (band, part) in parts.iter().enumerate() {
                let wide = part.len() / height;
                line[band * band_columns..band * band_columns + wide].copy_from_slice(&part[y * wide..(y + 1) * wide]);
            }
        };
        let work = move |index: usize| {
            let mut line = vec![0.0f32; width];
            for round in 0..3 {
                if let Some(band) = rows.get(index) {
                    let mut band = write(band);
                    let made: Vec<_> = if round > 0 { parts.iter().map(read).collect() } else { Vec::new() };
                    for (row, out) in band.chunks_exact_mut(width).enumerate() {
                        if made.is_empty() {
                            line.copy_from_slice(out);
                        } else {
                            gather(&made, index * band_rows + row, &mut line);
                        }
                        box_average(&line, pass, out);
                    }
                }
                barrier.wait();
                if let Some(part) = parts.get(index) {
                    let mut part = write(part);
                    let bands: Vec<_> = rows.iter().map(read).collect();
                    let x = index * band_columns;
                    let wide = part.len() / height;
                    let row = |y: usize| &bands[y / band_rows][(y % band_rows) * width + x..(y % band_rows) * width + x + wide];
                    box_average_columns(row, height, wide, pass, &mut part);
                }
                barrier.wait();
            }
            if let Some(band) = rows.get(index) {
                let mut band = write(band);
                let made: Vec<_> = parts.iter().map(read).collect();
                for (row, out) in band.chunks_exact_mut(width).enumerate() {
                    gather(&made, index * band_rows + row, out);
                }
            }
        };
        std::thread::scope(|scope| {
            for index in 1..threads {
                scope.spawn(move || work(index));
            }
            work(0);
        });
    }
    *BLURRED.lock().unwrap_or_else(PoisonError::into_inner) = columns;
}

fn read<'a, 'b>(lock: &'a RwLock<&'b mut [f32]>) -> RwLockReadGuard<'a, &'b mut [f32]> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<'a, 'b>(lock: &'a RwLock<&'b mut [f32]>) -> RwLockWriteGuard<'a, &'b mut [f32]> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

/// [`blur`] on one thread, a line at a time: what the bands must equal.
#[cfg(test)]
pub fn blur_alone(values: &mut [f32], width: usize, height: usize, radius: f64) {
    if !radius.is_finite() || radius < 0.5 || width == 0 || height == 0 {
        return;
    }
    let pass = ((radius / 3.0).ceil() as usize).max(1);
    let mut scratch = vec![0.0f32; width.max(height)];
    let mut line = vec![0.0f32; width.max(height)];
    for _ in 0..3 {
        for y in 0..height {
            line[..width].copy_from_slice(&values[y * width..(y + 1) * width]);
            box_average(&line[..width], pass, &mut scratch[..width]);
            values[y * width..(y + 1) * width].copy_from_slice(&scratch[..width]);
        }
        for x in 0..width {
            for y in 0..height {
                line[y] = values[y * width + x];
            }
            box_average(&line[..height], pass, &mut scratch[..height]);
            for y in 0..height {
                values[y * width + x] = scratch[y];
            }
        }
    }
}

/// [`box_average`] down `wide` columns side by side, `row(y)` being their
/// values in row `y`, into `out`, `wide` values a row: each column's sum
/// gains and loses its values in the same order as a line's.
fn box_average_columns<'a>(row: impl Fn(usize) -> &'a [f32], height: usize, wide: usize, radius: usize, out: &mut [f32]) {
    let mut sums = vec![0.0f32; wide];
    let mut count = 0usize;
    for y in 0..radius.min(height) {
        for (sum, value) in sums.iter_mut().zip(row(y)) {
            *sum += value;
        }
        count += 1;
    }
    for index in 0..height {
        if index + radius < height {
            for (sum, value) in sums.iter_mut().zip(row(index + radius)) {
                *sum += value;
            }
            count += 1;
        }
        if index > radius {
            for (sum, value) in sums.iter_mut().zip(row(index - radius - 1)) {
                *sum -= value;
            }
            count -= 1;
        }
        let count = count.max(1) as f32;
        for (out, sum) in out[index * wide..(index + 1) * wide].iter_mut().zip(&sums) {
            *out = sum / count;
        }
    }
}

/// Runs `work` on each `band` values of `data`, side by side, with the
/// band's number: the last band on this thread.
pub fn in_bands<T: Send>(data: &mut [T], band: usize, work: impl Fn(usize, &mut [T]) + Sync) {
    run_apart(data.chunks_mut(band.max(1)).collect(), work);
}

/// Runs `work` on each of `items`, with its number, on a thread per core,
/// this one among them: each thread takes the next item left, so one the
/// system holds back leaves its share to the others.
pub fn run_apart<T: Send>(items: Vec<T>, work: impl Fn(usize, T) + Sync) {
    let threads = cores().min(items.len());
    if threads <= 1 {
        for (index, item) in items.into_iter().enumerate() {
            work(index, item);
        }
        return;
    }
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|item| Mutex::new(Some(item))).collect();
    let next = AtomicUsize::new(0);
    let take = || loop {
        let index = next.fetch_add(1, Ordering::Relaxed);
        let Some(slot) = slots.get(index) else {
            break;
        };
        if let Some(item) = slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
            work(index, item);
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..threads {
            scope.spawn(&take);
        }
        take();
    });
}

/// A mask's grey values to blur, kept from one blur to the next.
static GREYS: Mutex<Vec<f32>> = Mutex::new(Vec::new());

/// Room for a blur's columns, kept from one blur to the next, so a held
/// Feather slider does not ask for fresh memory each time.
static BLURRED: Mutex<Vec<f32>> = Mutex::new(Vec::new());

/// Each value the average of those within `radius` of it, in the line.
fn box_average(line: &[f32], radius: usize, out: &mut [f32]) {
    let len = line.len();
    let mut sum = 0.0f32;
    let mut count = 0usize;
    for value in line.iter().take(radius.min(len)) {
        sum += value;
        count += 1;
    }
    for index in 0..len {
        if index + radius < len {
            sum += line[index + radius];
            count += 1;
        }
        if index > radius {
            sum -= line[index - radius - 1];
            count -= 1;
        }
        out[index] = sum / count.max(1) as f32;
    }
}

/// The image as the nodes beneath one node combine it, the whole image
/// premultiplied, at each depth of the groups that node lies in: the
/// top-level nodes beneath it, then within each group its siblings beneath
/// it, by themselves. The document keeps it while that node, or one above
/// it, changes, such as an adjustment's slider while it is held, and says
/// when it no longer holds.
pub struct Beneath {
    /// At each depth, the siblings it was made from.
    kept: Vec<Vec<Node>>,
    width: u32,
    height: u32,
    levels: Vec<Vec<[f32; 4]>>,
}

impl Beneath {
    /// How many siblings lie beneath the node at each depth.
    pub fn path(&self) -> Vec<usize> {
        self.kept.iter().map(Vec::len).collect()
    }

    /// Whether it is of this image's size, and its path goes through
    /// groups the image has.
    fn fits(&self, image: &LayeredImage) -> bool {
        if (self.width, self.height) != (image.width, image.height) {
            return false;
        }
        let mut siblings: &[Node] = &image.nodes;
        for (depth, kept) in self.kept.iter().enumerate() {
            if kept.len() > siblings.len() {
                return false;
            }
            if depth + 1 < self.kept.len() {
                match siblings.get(kept.len()).and_then(|node| node.children.as_deref()) {
                    Some(children) => siblings = children,
                    None => return false,
                }
            }
        }
        true
    }

    /// Whether it still lies beneath the image's nodes: it fits, and
    /// every sibling it was made from is as the image has it. The pixels
    /// are the document's to vouch for.
    pub fn holds(&self, image: &LayeredImage) -> bool {
        if !self.fits(image) {
            return false;
        }
        let mut siblings: &[Node] = &image.nodes;
        for kept in &self.kept {
            if siblings[..kept.len()] != kept[..] {
                return false;
            }
            siblings = siblings.get(kept.len()).and_then(|node| node.children.as_deref()).unwrap_or_default();
        }
        true
    }
}

/// How deep groups go within `nodes`: none, 0.
fn depth(nodes: &[Node]) -> usize {
    nodes.iter().filter_map(|node| node.children.as_deref()).map(|children| 1 + depth(children)).max().unwrap_or(0)
}

/// How many bands of rows `rect` is composited in: one per core, as each
/// pixel is composited by itself, and one for a small area.
pub fn bands(rect: Rect) -> usize {
    if (rect.w as usize) * (rect.h as usize) < 65_536 { 1 } else { cores().min(rect.h.max(1) as usize) }
}

/// How many cores the computer has.
pub fn cores() -> usize {
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, usize::from))
}

/// How many shares work on `rect` is split into, for threads that each
/// take the next share left: a few per core, and one for a small area.
pub fn shares(rect: Rect) -> usize {
    if bands(rect) == 1 { 1 } else { (2 * cores()).min(rect.h.max(1) as usize) }
}

/// How many rows of `width` pixels a band combines at a time: about 256 KB
/// of premultiplied pixels, which stay in a core's cache.
fn tile_rows(width: usize) -> usize {
    (16_384 / width.max(1)).max(1)
}

/// Premultiplied pixels to composite in, kept from one composite to the
/// next, so a large composite does not ask for fresh memory each time.
static SCRATCH: Mutex<Vec<[f32; 4]>> = Mutex::new(Vec::new());

fn with_scratch<T>(len: usize, work: impl FnOnce(&mut [[f32; 4]]) -> T) -> T {
    let mut buffer = std::mem::take(&mut *SCRATCH.lock().unwrap_or_else(PoisonError::into_inner));
    if buffer.len() < len {
        buffer = vec![[0.0; 4]; len];
    }
    let result = work(&mut buffer[..len]);
    *SCRATCH.lock().unwrap_or_else(PoisonError::into_inner) = buffer;
    result
}

/// The tables of every adjustment in a tree, found once for a composite
/// rather than made again for each band or row.
struct Adjusts(Vec<(u64, Arc<Adjust>)>);

impl Adjusts {
    fn of(nodes: &[Node]) -> Self {
        fn walk(nodes: &[Node], found: &mut Vec<(u64, Arc<Adjust>)>) {
            for node in nodes {
                match &node.children {
                    Some(children) => walk(children, found),
                    None => found.extend(adjust_for(&node.kind).map(|adjust| (node.id, adjust))),
                }
            }
        }
        let mut found = Vec::new();
        walk(nodes, &mut found);
        Self(found)
    }

    fn get(&self, id: u64) -> Option<&Adjust> {
        self.0.iter().find(|(own, _)| *own == id).map(|(_, adjust)| &**adjust)
    }
}

/// An adjustment's tables by its settings: the last few made are kept, so
/// an adjustment that did not change is not made again, and one whose
/// slider is held is made once for each value.
fn adjust_for(kind: &Kind) -> Option<Arc<Adjust>> {
    static KEPT: Mutex<Vec<(Kind, Arc<Adjust>)>> = Mutex::new(Vec::new());
    if !kind.is_adjustment() {
        return None;
    }
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(at) = kept.iter().position(|(own, _)| own == kind) {
        let entry = kept.remove(at);
        let adjust = entry.1.clone();
        kept.push(entry);
        return Some(adjust);
    }
    let adjust = Arc::new(Adjust::of(kind)?);
    kept.push((kind.clone(), adjust.clone()));
    if kept.len() > 32 {
        kept.remove(0);
    }
    Some(adjust)
}

/// An adjustment as the composite applies it: a 256-entry table for each
/// channel, read between entries, then a saturation scale.
struct Adjust {
    tables: [[f32; 256]; 3],
    saturation: f32,
}

impl Adjust {
    fn of(kind: &Kind) -> Option<Adjust> {
        let mut tables = [[0.0f32; 256]; 3];
        match kind {
            Kind::Curves(curves) => {
                let master = Curve::new(&curves.rgb);
                for (channel, table) in tables.iter_mut().enumerate() {
                    let own = Curve::new(curves.channel(channel + 1));
                    for (index, value) in table.iter_mut().enumerate() {
                        // The master after the channel's.
                        *value = master.y(own.y(index as f64 / 255.0)) as f32;
                    }
                }
                Some(Adjust { tables, saturation: 1.0 })
            }
            Kind::Grade(grade) => {
                let grade = grade.clamped();
                let gain = 2f32.powf(grade.exposure);
                let contrast = 1.0 + grade.contrast / 100.0;
                let warm = grade.temperature / 100.0 * 0.2;
                let tint = [1.0 + warm, 1.0, 1.0 - warm];
                for (channel, table) in tables.iter_mut().enumerate() {
                    for (index, value) in table.iter_mut().enumerate() {
                        // Exposure in linear light, then contrast about the
                        // middle grey, then the warmth's tint.
                        let linear = to_linear(index as f32 / 255.0) * gain;
                        let exposed = from_linear(linear.min(1.0));
                        let contrasted = (exposed - 0.5) * contrast + 0.5;
                        *value = (contrasted * tint[channel]).clamp(0.0, 1.0);
                    }
                }
                Some(Adjust { tables, saturation: 1.0 + grade.saturation / 100.0 })
            }
            Kind::Pixels | Kind::Text(_) => None,
        }
    }

    fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for channel in 0..3 {
            let place = rgb[channel].clamp(0.0, 1.0) * 255.0;
            // Never below 0, so the cast is the floor, without its call.
            let index = (place as usize).min(255);
            let next = (index + 1).min(255);
            let part = place - index as f32;
            let table = &self.tables[channel];
            out[channel] = table[index] + (table[next] - table[index]) * part;
        }
        if self.saturation != 1.0 {
            let luma = 0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2];
            for value in &mut out {
                *value = (luma + (*value - luma) * self.saturation).clamp(0.0, 1.0);
            }
        }
        out
    }
}

fn to_linear(value: f32) -> f32 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

fn from_linear(value: f32) -> f32 {
    if value <= 0.003_130_8 { value * 12.92 } else { 1.055 * value.powf(1.0 / 2.4) - 0.055 }
}

/// A curve's point, as `draw::document::CurvePoint` has it, so the curve
/// below is `draw::ink`'s, line for line, and the host's drawn curve is the
/// one applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

/// A curve read at many places: its points, and their tangents worked out
/// once rather than at each place.
pub struct Curve {
    points: Vec<CurvePoint>,
    tangents: Vec<f64>,
}

impl Curve {
    pub fn new(points: &[[f64; 2]]) -> Self {
        let points: Vec<CurvePoint> = points.iter().map(|point| CurvePoint { x: point[0], y: point[1] }).collect();
        let tangents = if points.len() >= 2 { monotone_tangents(&points) } else { Vec::new() };
        Self { points, tangents }
    }

    /// The curve at `x`, as [`curve_y`] gives it.
    pub fn y(&self, x: f64) -> f64 {
        curve_y_with(&self.points, &self.tangents, x)
    }
}

/// The monotone cubic Hermite through the points (Fritsch–Carlson), at `x`:
/// copied from `draw/src/ink.rs`.
pub fn curve_y(points: &[CurvePoint], x: f64) -> f64 {
    let tangents = if points.len() >= 2 { monotone_tangents(points) } else { Vec::new() };
    curve_y_with(points, &tangents, x)
}

/// [`curve_y`] with the points' tangents already worked out.
fn curve_y_with(points: &[CurvePoint], tangents: &[f64], x: f64) -> f64 {
    if points.is_empty() {
        return x.clamp(0.0, 1.0);
    }
    let x = x.clamp(points[0].x, points[points.len() - 1].x);
    if points.len() == 1 {
        return points[0].y.clamp(0.0, 1.0);
    }
    let mut index = 0;
    while index + 1 < points.len() && points[index + 1].x < x {
        index += 1;
    }
    if index + 1 >= points.len() {
        return points[points.len() - 1].y.clamp(0.0, 1.0);
    }
    hermite(
        points[index],
        points[index + 1],
        tangents[index],
        tangents[index + 1],
        x,
    )
}

fn monotone_tangents(points: &[CurvePoint]) -> Vec<f64> {
    let count = points.len();
    let mut secants = vec![0.0; count - 1];
    for index in 0..count - 1 {
        let dx = points[index + 1].x - points[index].x;
        secants[index] = if dx == 0.0 {
            0.0
        } else {
            (points[index + 1].y - points[index].y) / dx
        };
    }
    let mut slopes = vec![0.0; count];
    slopes[0] = secants[0];
    slopes[count - 1] = secants[count - 2];
    for index in 1..count - 1 {
        if secants[index - 1] * secants[index] <= 0.0 {
            slopes[index] = 0.0;
        } else {
            slopes[index] = (secants[index - 1] + secants[index]) / 2.0;
        }
    }
    for index in 0..count - 1 {
        if secants[index] == 0.0 {
            slopes[index] = 0.0;
            slopes[index + 1] = 0.0;
            continue;
        }
        let a = slopes[index] / secants[index];
        let b = slopes[index + 1] / secants[index];
        let sum = a * a + b * b;
        if sum > 9.0 {
            let scale = 3.0 / sum.sqrt();
            slopes[index] = scale * a * secants[index];
            slopes[index + 1] = scale * b * secants[index];
        }
    }
    slopes
}

fn hermite(start: CurvePoint, end: CurvePoint, slope0: f64, slope1: f64, x: f64) -> f64 {
    let span = end.x - start.x;
    if span == 0.0 {
        return start.y.clamp(0.0, 1.0);
    }
    let t = ((x - start.x) / span).clamp(0.0, 1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    (h00 * start.y + h10 * span * slope0 + h01 * end.y + h11 * span * slope1).clamp(0.0, 1.0)
}

/// One source pixel, straight colour and alpha, over a premultiplied
/// backdrop. Where the backdrop is opaque the blend function takes the
/// source's place; where it is clear the source shows as itself.
fn blend_pixel(dst: &mut [f32; 4], rgb: [f32; 3], alpha: f32, blend: Blend) {
    if alpha <= 0.0 {
        return;
    }
    let backdrop = dst[3];
    let mut mixed = rgb;
    if blend != Blend::Normal && backdrop > 0.0 {
        for channel in 0..3 {
            let b = dst[channel] / backdrop;
            mixed[channel] = (1.0 - backdrop) * rgb[channel] + backdrop * blend.mix(b, rgb[channel]);
        }
    }
    for channel in 0..3 {
        dst[channel] = alpha * mixed[channel] + dst[channel] * (1.0 - alpha);
    }
    dst[3] = alpha + backdrop * (1.0 - alpha);
}

/// A fraction as a byte: `(value * 255).round().clamp(0, 255)`, rounding
/// halves up, without the call `round` costs for every channel of every
/// pixel. The part after the point is exact for any value below 256.
fn to_byte(value: f32) -> u8 {
    let scaled = value * 255.0;
    if !(scaled > 0.0) {
        return 0;
    }
    if scaled >= 255.0 {
        return 255;
    }
    let whole = scaled as u32;
    whole as u8 + u8::from(scaled - whole as f32 >= 0.5)
}

/// Each byte as a fraction, so the composite does not divide per pixel.
fn unit_table() -> &'static [f32; 256] {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0; 256];
        for (index, value) in table.iter_mut().enumerate() {
            *value = index as f32 / 255.0;
        }
        table
    })
}

fn collect_ids(node: &Node, out: &mut Vec<u64>) {
    out.push(node.id);
    if let Some(children) = &node.children {
        for child in children {
            collect_ids(child, out);
        }
    }
}

fn find_in(nodes: &[Node], id: u64) -> Option<&Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = node.children.as_deref().and_then(|children| find_in(children, id)) {
            return Some(found);
        }
    }
    None
}

fn find_in_mut(nodes: &mut [Node], id: u64) -> Option<&mut Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = node.children.as_deref_mut().and_then(|children| find_in_mut(children, id)) {
            return Some(found);
        }
    }
    None
}

fn place_in(nodes: &[Node], parent: Option<u64>, id: u64) -> Option<Place> {
    for (index, node) in nodes.iter().enumerate() {
        if node.id == id {
            return Some(Place { parent, index });
        }
        if let Some(found) = node.children.as_deref().and_then(|children| place_in(children, Some(node.id), id)) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-layer image of one pixel: an opaque backdrop and a source
    /// with the blend mode and alpha given.
    fn pair(backdrop: [u8; 4], source: [u8; 4], blend: Blend) -> [u8; 4] {
        let mut image = LayeredImage::from_pixels("under", Pixels::filled(1, 1, backdrop));
        let id = image.take_id();
        let mut node = Node::layer(id, "over");
        node.blend = blend;
        image.nodes.push(node);
        image.pixels.insert(id, Pixels::filled(1, 1, source));
        let out = image.composite();
        [out[0], out[1], out[2], out[3]]
    }

    /// Catches a blend mode computed with the wrong formula, or the layers
    /// swapped: each mode on known pixels, against the standard formulas
    /// worked by hand.
    #[test]
    fn each_blend_mode_on_known_pixels() {
        let b = [200, 100, 50, 255];
        let s = [100, 150, 250, 255];
        assert_eq!(pair(b, s, Blend::Normal), [100, 150, 250, 255]);
        // 200·100/255 = 78.4, 100·150/255 = 58.8, 50·250/255 = 49.0.
        assert_eq!(pair(b, s, Blend::Multiply), [78, 59, 49, 255]);
        // b + s − bs: 200+100−78.4 = 221.6, 100+150−58.8 = 191.2, 50+250−49 = 251.
        assert_eq!(pair(b, s, Blend::Screen), [222, 191, 251, 255]);
        // Overlay: b > ½ screens, b ≤ ½ multiplies, both doubled.
        // R: 1 − 2(1−200/255)(1−100/255) = 0.7378 → 188.
        // G: 2·(100/255)(150/255) = 0.4614 → 118.
        // B: 2·(50/255)(250/255) = 0.3845 → 98.
        assert_eq!(pair(b, s, Blend::Overlay), [188, 118, 98, 255]);
        assert_eq!(pair(b, s, Blend::Darken), [100, 100, 50, 255]);
        assert_eq!(pair(b, s, Blend::Lighten), [200, 150, 250, 255]);
        // A half-clear multiply source mixes halfway: 200·0.5 + 78.4·0.5.
        let half = pair(b, [100, 150, 250, 128], Blend::Multiply);
        assert_eq!(half[0], 139);
        // Over a clear backdrop a blend mode shows the source as itself.
        assert_eq!(pair([0, 0, 0, 0], s, Blend::Multiply), [100, 150, 250, 255]);
    }

    /// Catches opacity and fill ignored, and a group that blends its
    /// children one by one instead of as one: a Multiply group of two
    /// Normal layers, the top hiding the bottom, multiplies once.
    #[test]
    fn opacity_fill_and_groups() {
        let mut image = LayeredImage::from_pixels("under", Pixels::filled(1, 1, [200, 200, 200, 255]));
        let a = image.take_id();
        let b = image.take_id();
        let group = image.take_id();
        image.pixels.insert(a, Pixels::filled(1, 1, [0, 0, 0, 255]));
        image.pixels.insert(b, Pixels::filled(1, 1, [255, 255, 255, 255]));
        let mut node = Node::group(group, "group", vec![Node::layer(a, "a"), Node::layer(b, "b")]);
        node.blend = Blend::Multiply;
        image.nodes.push(node);
        // White multiplied over grey leaves grey; black, had it been
        // multiplied by itself, would make black.
        assert_eq!(&image.composite()[..4], &[200, 200, 200, 255]);
        image.find_mut(b).unwrap().visible = false;
        assert_eq!(&image.composite()[..4], &[0, 0, 0, 255]);
        image.find_mut(group).unwrap().opacity = 0.5;
        assert_eq!(&image.composite()[..4], &[100, 100, 100, 255]);
        image.find_mut(group).unwrap().opacity = 1.0;
        image.find_mut(a).unwrap().fill = 0.25;
        assert_eq!(&image.composite()[..4], &[150, 150, 150, 255]);
        image.find_mut(a).unwrap().fill = 1.0;
        image.find_mut(a).unwrap().opacity = 0.0;
        assert_eq!(&image.composite()[..4], &[200, 200, 200, 255]);
    }

    /// Catches a group's lock not reaching the layers inside it.
    #[test]
    fn a_group_lock_reaches_its_layers() {
        let mut image = LayeredImage::blank(4, 4);
        let inner = image.take_id();
        let group = image.take_id();
        image.pixels.insert(inner, Pixels::clear(4, 4));
        let mut node = Node::group(group, "group", vec![Node::layer(inner, "inner")]);
        node.locks.position = true;
        image.nodes.push(node);
        assert!(image.locks(inner).keeps_position());
        assert!(!image.locks(inner).keeps_pixels());
    }

    /// Catches a blur in bands that differs from the blur a line at a time:
    /// over a busy mask large enough to be split among the cores, of a size
    /// that leaves the last bands narrower, each feather's bands give the
    /// same values, to the bit.
    #[test]
    fn the_blur_in_bands_is_the_blur_alone() {
        let (width, height) = (517, 389);
        let values: Vec<f32> = (0..width * height).map(|index| ((index * 7919 + index / width * 31) % 256) as f32).collect();
        for radius in [0.6, 4.0, 13.0, 40.0] {
            let mut banded = values.clone();
            blur(&mut banded, width, height, radius);
            let mut alone = values.clone();
            blur_alone(&mut alone, width, height, radius);
            let differ = banded.iter().zip(&alone).filter(|(banded, alone)| banded.to_bits() != alone.to_bits()).count();
            assert_eq!(differ, 0, "{differ} values of a blur of {radius} differ from the blur alone");
        }
    }
}
