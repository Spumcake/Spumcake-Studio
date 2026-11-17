//! A tab's rounds: what each of its cells' requests made, as the host
//! sends them with `takes`, and never kept in the `.ipx`. Each round is a
//! run of the filmstrip: its takes, each read from its file, and a place
//! for each take still being made, or one for a round that failed. A take
//! shown in the working area is read from its file as a linked layer. The
//! round views are drawn here, with the app's own fonts: the round grid,
//! two rounds one above the other, and the references sheet.

use crate::files;
use crate::gallery::Frame;
use crate::notes;
use crate::text::{self, Style, Text};
use layers::Rgba;
use layers::label;
use layers::layout::{self, Rect};
use protocol::{FilmPending, Item, Round, RoundState, SentReference, TakeFile, Thumbnail};
use std::collections::HashMap;
use std::path::Path;

/// What a take or a reference can be, as this workspace reads them.
pub const IMAGE_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

/// The roles a reference can have, as the Prompt page's menu offers them,
/// the guide's words for 0.15.
pub const ROLES: [&str; 4] = ["Reference", "Sketch", "Start frame", "Control"];

/// How an item of the tab's file names a take: this, then its path.
const TAKE_ITEM: &str = "take ";

/// The round views' space around everything, between takes, and the size
/// of their words, in the view's pixels, which the host draws at 100%.
const MARGIN: u32 = 24;
const GAP: u32 = 16;
const TITLE: f32 = 15.0;
const LABEL: f32 = 13.0;
const INK: [u8; 3] = [28, 28, 30];
const MUTED: [u8; 3] = [110, 110, 118];
const PAPER: [u8; 4] = [255, 255, 255, 255];
const LINE: [u8; 4] = [226, 226, 230, 255];
const WAITING: [u8; 4] = [236, 236, 240, 255];
const BADGE: [u8; 4] = [30, 30, 34, 210];
/// The chosen take's outline and star, the strip's blue.
const BLUE: [u8; 4] = [37, 99, 235, 255];
/// The longer side a take or a reference is read at for the round views.
const PREVIEW_MOST: u32 = 960;
/// How many of a prompt's first words label a round side by side.
const FIRST_WORDS: usize = 9;

/// One tile of the filmstrip a round made: a take, or a place for one.
pub struct Tile {
    pub cell: String,
    pub request: String,
    /// The round's number among its cell's rounds.
    pub number: u32,
    /// The take's place in its round, from 0.
    pub take: usize,
    /// Its file, or none for a take still being made or a round that failed.
    pub file: Option<TakeFile>,
    pub pending: Option<FilmPending>,
    /// The take as the working area shows it, once it has been shown.
    pub frame: Option<Box<Frame>>,
}

impl Tile {
    pub fn path(&self) -> Option<&str> {
        self.file.as_ref().map(|file| file.path.as_str())
    }

    /// Whether its frame holds something of the person's, such as notes,
    /// and so is kept while the program runs.
    fn keeps(&self) -> bool {
        self.frame.as_ref().is_some_and(|frame| !frame.doc.notes.is_empty() || frame.doc.edits > 0)
    }
}

/// A take's file as the strip shows it: read once, and again when it
/// changes on disk.
struct Read {
    stamp: Option<(u64, std::time::SystemTime)>,
    size: (u32, u32),
    /// `protocol::fingerprint` of its bytes, as its item declares it.
    fingerprint: String,
    /// Its filmstrip thumbnail's bytes and size, and where they were
    /// written for the host.
    film: Vec<u8>,
    film_size: (u32, u32),
    thumb: Option<Thumbnail>,
    /// Names its thumbnail's file.
    number: u64,
    failed: bool,
}

/// What the working area shows while a take is shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum View {
    /// The take, as any image is shown.
    #[default]
    Single,
    /// Its round whole, as numbered takes beside its references.
    Grid,
    /// Its round and the round of that request, one above the other.
    Pair(String),
    /// The take beside its round's references, and those not sent.
    Sheet,
}

/// What a round view was drawn from, so it is drawn again only when that
/// changes: the view, the tile shown, the rounds' generation, and the
/// view's width and height.
pub type Drawn = (View, Option<usize>, u64, u32, u32);

#[derive(Default)]
pub struct Rounds {
    /// Each cell's rounds, oldest first, as the host last sent them, in
    /// the order of the tab's cells.
    by_cell: Vec<(String, Vec<Round>)>,
    /// The strip's tiles: every round's in turn.
    pub tiles: Vec<Tile>,
    /// The tile shown in the working area in place of an image of the
    /// gallery.
    pub shown: Option<usize>,
    pub view: View,
    reads: HashMap<String, Read>,
    next_read: u64,
    /// Takes and references read small for the round views, by path.
    previews: HashMap<String, Option<Rgba>>,
    /// The round view last written for the host, and its revision.
    pub drawn: Option<(Drawn, u64)>,
    /// Rises with every `takes`, and every file read again.
    generation: u64,
    /// The notes marked on takes, as the tab's file keeps them, by each
    /// take's file: brought back when that take is shown again.
    pub kept: Vec<files::TakeNotes>,
}

impl Rounds {
    /// The notes a take's file was marked with, when they were marked on
    /// these bytes, as `sha256` says.
    pub fn kept_notes(&self, path: &str, sha256: &str) -> Option<layers::notes::Notes> {
        self.kept.iter().find(|kept| kept.link.path == path && kept.link.sha256 == sha256).map(|kept| kept.notes.clone())
    }

    /// The notes on takes to keep in the tab's file: those it kept, each
    /// replaced by its take's notes as they are now when the program holds
    /// that take, in the order of the takes' paths.
    pub fn notes_to_keep(&self) -> Vec<files::TakeNotes> {
        let mut kept = self.kept.clone();
        for frame in self.frames() {
            let Some(link) = frame.doc.image.nodes.first().and_then(|node| node.link.as_ref()).map(|linked| linked.link.clone()) else {
                continue;
            };
            kept.retain(|own| own.link.path != link.path);
            if !frame.doc.notes.is_empty() {
                kept.push(files::TakeNotes { link, notes: frame.doc.notes.clone() });
            }
        }
        kept.sort_by(|a, b| a.link.path.cmp(&b.link.path));
        kept
    }

    /// Whether a round view, not one image, fills the working area.
    pub fn viewing(&self) -> bool {
        self.shown.is_some() && self.view != View::Single
    }

    pub fn shown_frame(&self) -> Option<&Frame> {
        self.tiles.get(self.shown?)?.frame.as_deref()
    }

    pub fn shown_frame_mut(&mut self) -> Option<&mut Frame> {
        let shown = self.shown?;
        self.tiles.get_mut(shown)?.frame.as_deref_mut()
    }

    /// Every take's frame the program holds.
    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.tiles.iter().filter_map(|tile| tile.frame.as_deref())
    }

    pub fn frames_mut(&mut self) -> impl Iterator<Item = &mut Frame> {
        self.tiles.iter_mut().filter_map(|tile| tile.frame.as_deref_mut())
    }

    /// Replaces one cell's rounds, as `takes` sends them, and lays the
    /// strip out again: the cells' rounds in the order of `cells`, each
    /// cell's oldest first. A take keeps its frame, notes and all, and
    /// stays shown while its file is still among them. Returns the numbers
    /// of the frames let go, for their files to go too.
    pub fn set(&mut self, cell: &str, rounds: Vec<Round>, cells: &[String]) -> Vec<u64> {
        match self.by_cell.iter_mut().find(|(id, _)| id == cell) {
            Some((_, kept)) => *kept = rounds,
            None => self.by_cell.push((cell.to_string(), rounds)),
        }
        let place = |id: &str| cells.iter().position(|cell| cell == id).unwrap_or(usize::MAX);
        self.by_cell.sort_by_key(|(id, _)| place(id.as_str()));
        let shown = self.shown.and_then(|index| self.tiles.get(index)).and_then(|tile| tile.path().map(str::to_string));
        let mut frames: HashMap<String, Box<Frame>> = HashMap::new();
        for tile in self.tiles.drain(..) {
            if let (Some(file), Some(frame)) = (tile.file, tile.frame) {
                frames.insert(file.path, frame);
            }
        }
        let mut tiles = Vec::new();
        for (cell, rounds) in &self.by_cell {
            for round in rounds {
                let images: Vec<&TakeFile> = round.takes.iter().filter(|take| IMAGE_TYPES.contains(&take.media_type.as_str())).collect();
                // A round being made shows a place for each take still to
                // come; one that failed with nothing shows one.
                let waiting = match round.state {
                    RoundState::Running | RoundState::Pending => {
                        (round.count.unwrap_or(0) as usize).saturating_sub(images.len()).max(usize::from(images.is_empty()))
                    }
                    RoundState::Failed => usize::from(images.is_empty()),
                    RoundState::Done | RoundState::Cancelled => 0,
                };
                let pending = match round.state {
                    RoundState::Failed => FilmPending::Failed,
                    _ => FilmPending::Running { since: round.started },
                };
                for (take, file) in images.iter().enumerate() {
                    tiles.push(Tile {
                        cell: cell.clone(),
                        request: round.request.clone(),
                        number: round.number,
                        take,
                        file: Some((*file).clone()),
                        pending: None,
                        frame: frames.remove(&file.path),
                    });
                }
                for more in 0..waiting {
                    tiles.push(Tile {
                        cell: cell.clone(),
                        request: round.request.clone(),
                        number: round.number,
                        take: images.len() + more,
                        file: None,
                        pending: Some(pending.clone()),
                        frame: None,
                    });
                }
            }
        }
        self.tiles = tiles;
        self.shown = shown.and_then(|path| self.tiles.iter().position(|tile| tile.path() == Some(path.as_str())));
        if self.shown.is_none() {
            self.view = View::Single;
        }
        if let View::Pair(request) = &self.view
            && !self.tiles.iter().any(|tile| tile.request == *request)
        {
            self.view = View::Single;
        }
        self.reads.retain(|path, _| self.tiles.iter().any(|tile| tile.path() == Some(path.as_str())));
        self.generation += 1;
        frames.into_values().map(|frame| frame.uid).collect()
    }

    /// Reads each take's file the strip does not hold yet, or that changed
    /// on disk: its size, its fingerprint, and its thumbnail, written for
    /// the host as `<tab>-take-<number>.rgba` in `cache`. A path is read
    /// from `root`, the project, unless it is whole.
    pub fn read_files(&mut self, root: &Path, tab: &str, cache: &Path, revision: &mut u64) {
        let paths: Vec<String> = self.tiles.iter().filter_map(|tile| tile.path().map(str::to_string)).collect();
        for path in paths {
            let file = files::resolve(root, &path);
            let stamp = std::fs::metadata(&file).ok().and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
            let number = match self.reads.get(&path) {
                Some(read) if read.stamp == stamp => continue,
                Some(read) => read.number,
                None => {
                    self.next_read += 1;
                    self.next_read
                }
            };
            let read = match files::import_printed(&file) {
                Ok((pixels, fingerprint)) => {
                    let film_size = crate::session::film_size(pixels.width, pixels.height);
                    let film = files::resample(pixels.width, pixels.height, &pixels.data, film_size.0, film_size.1);
                    Read { stamp, size: (pixels.width, pixels.height), fingerprint, film, film_size, thumb: None, number, failed: false }
                }
                Err(_) => Read { stamp, size: (0, 0), fingerprint: String::new(), film: Vec::new(), film_size: (0, 0), thumb: None, number, failed: true },
            };
            self.reads.insert(path.clone(), read);
            self.previews.remove(&path);
            self.generation += 1;
        }
        for read in self.reads.values_mut() {
            if read.thumb.is_some() || read.failed {
                continue;
            }
            let file = cache.join(format!("{tab}-take-{}.rgba", read.number));
            if files::write_atomic(&file, &read.film).is_ok() {
                *revision += 1;
                read.thumb = Some(Thumbnail { path: file.display().to_string(), width: read.film_size.0, height: read.film_size.1, revision: *revision });
            }
        }
    }

    /// The host now looks for the images somewhere else: the thumbnails
    /// and the round view are written again.
    pub fn written_elsewhere(&mut self) {
        for read in self.reads.values_mut() {
            read.thumb = None;
        }
        self.drawn = None;
    }

    /// The round a tile is of.
    pub fn round_of(&self, tile: &Tile) -> Option<&Round> {
        let (_, rounds) = self.by_cell.iter().find(|(cell, _)| *cell == tile.cell)?;
        rounds.iter().find(|round| round.request == tile.request)
    }

    /// The tile of the take at `path`.
    pub fn tile_of(&self, path: &str) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.path() == Some(path))
    }

    /// Whether that cell has made a round: its draft is no longer only
    /// editing state.
    pub fn has_rounds(&self, cell: &str) -> bool {
        self.by_cell.iter().any(|(id, rounds)| id == cell && !rounds.is_empty())
    }

    /// The strip's runs: each round's label, "Round 14 · GPT Image 2.5",
    /// and how many tiles it has.
    pub fn groups(&self) -> Vec<(String, usize)> {
        let mut groups: Vec<(String, usize)> = Vec::new();
        let mut last: Option<(&str, &str)> = None;
        for tile in &self.tiles {
            let key = (tile.cell.as_str(), tile.request.as_str());
            if last == Some(key)
                && let Some(group) = groups.last_mut()
            {
                group.1 += 1;
                continue;
            }
            last = Some(key);
            let model = self.round_of(tile).map_or("", |round| round.model.as_str());
            groups.push((format!("Round {} · {model}", tile.number), 1));
        }
        groups
    }

    /// A tile's thumbnail: its take's, once read, or for a place still
    /// waiting, a shape like the takes' with no file.
    pub fn thumbnail(&self, tile: &Tile) -> Thumbnail {
        if let Some(thumb) = tile.path().and_then(|path| self.reads.get(path)).and_then(|read| read.thumb.clone()) {
            return thumb;
        }
        let (width, height) = self.shape().map_or((78, 44), |(width, height)| crate::session::film_size(width, height));
        Thumbnail { path: String::new(), width, height, revision: 0 }
    }

    /// The takes' size: the first read's.
    pub fn shape(&self) -> Option<(u32, u32)> {
        self.tiles.iter().filter_map(Tile::path).filter_map(|path| self.reads.get(path)).find(|read| !read.failed).map(|read| read.size)
    }

    fn aspect(&self) -> f32 {
        self.shape().map_or(16.0 / 9.0, |(width, height)| width as f32 / height.max(1) as f32)
    }

    /// Each take as an item of the tab's file, with its file's fingerprint.
    pub fn items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        for tile in &self.tiles {
            let Some(file) = &tile.file else {
                continue;
            };
            let Some(read) = self.reads.get(&file.path).filter(|read| !read.failed) else {
                continue;
            };
            let id = take_item(&file.path);
            if items.iter().any(|item| item.id == id) {
                continue;
            }
            items.push(Item {
                id,
                name: crate::session::file_title(Path::new(&file.path)),
                media_types: vec![file.media_type.clone()],
                fingerprint: read.fingerprint.clone(),
                made: None,
            });
        }
        items
    }

    /// A take's file and fingerprint, by its item's id.
    pub fn take_by_item(&self, id: &str) -> Option<(TakeFile, String)> {
        let path = id.strip_prefix(TAKE_ITEM)?;
        let tile = self.tiles.iter().find(|tile| tile.path() == Some(path))?;
        let read = self.reads.get(path).filter(|read| !read.failed)?;
        Some((tile.file.clone()?, read.fingerprint.clone()))
    }

    /// Shows tile `index`, whose frame has been made. A take shown before
    /// it that holds nothing of the person's lets its frame go, and its
    /// number is returned, for its files to go too.
    pub fn show(&mut self, index: usize) -> Option<u64> {
        let before = self.shown.replace(index);
        if matches!(self.view, View::Pair(_)) {
            self.view = View::Single;
        }
        let before = before.filter(|before| *before != index)?;
        self.let_go(before)
    }

    /// The working area shows an image of the gallery again.
    pub fn leave(&mut self) -> Option<u64> {
        let before = self.shown.take()?;
        self.view = View::Single;
        self.let_go(before)
    }

    fn let_go(&mut self, index: usize) -> Option<u64> {
        let tile = self.tiles.get_mut(index)?;
        if tile.keeps() {
            return None;
        }
        tile.frame.take().map(|frame| frame.uid)
    }

    /// What a round view at this size would be drawn from.
    pub fn key(&self, width: u32, height: u32) -> Drawn {
        (self.view.clone(), self.shown, self.generation, width, height)
    }

    /// The round view, `width` × `height`, or none while no take is shown
    /// or the view is the take alone. References and takes are read from
    /// `root`, the project, unless whole.
    pub fn compose(&mut self, root: &Path, width: u32, height: u32) -> Option<Rgba> {
        let shown = self.shown?;
        let tile = self.tiles.get(shown)?;
        let round = self.round_of(tile)?.clone();
        let own = self.tiles_of(&tile.cell, &tile.request);
        let other = match &self.view {
            View::Single => return None,
            View::Pair(request) => {
                let first = self.tiles.iter().find(|tile| tile.request == *request)?;
                let round = self.round_of(first)?.clone();
                let tiles = self.tiles_of(&first.cell, &first.request);
                Some((round, tiles))
            }
            View::Grid | View::Sheet => None,
        };
        // Everything the view draws is read first.
        let mut used: Vec<String> = Vec::new();
        for reference in round.references.iter().chain(&round.left_out) {
            used.push(reference.path.clone());
        }
        for index in own.iter().chain(other.iter().flat_map(|(_, tiles)| tiles.iter())) {
            if let Some(path) = self.tiles[*index].path() {
                used.push(path.to_string());
            }
        }
        for path in &used {
            self.load(root, path);
        }
        let mut canvas = Rgba::new(width, height, PAPER);
        match (&self.view, other) {
            (View::Grid, _) => self.draw_grid(&mut canvas, &round, &own, shown),
            (View::Pair(_), Some(other)) => {
                // The older round above.
                let mut pair = [(round, own), other];
                pair.sort_by_key(|(_, tiles)| tiles.first().copied().unwrap_or(usize::MAX));
                self.draw_pair(&mut canvas, &pair, shown);
            }
            (View::Sheet, _) => self.draw_sheet(&mut canvas, &round, shown),
            _ => return None,
        }
        self.previews.retain(|path, _| used.contains(path));
        Some(canvas)
    }

    /// The tiles of one round, in order.
    fn tiles_of(&self, cell: &str, request: &str) -> Vec<usize> {
        (0..self.tiles.len()).filter(|index| self.tiles[*index].cell == cell && self.tiles[*index].request == request).collect()
    }

    /// Reads a take or a reference small, once.
    fn load(&mut self, root: &Path, path: &str) {
        if self.previews.contains_key(path) {
            return;
        }
        let image = files::import(&files::resolve(root, path)).ok().map(|pixels| {
            let (width, height, data) = files::shrink(pixels.width, pixels.height, &pixels.data, PREVIEW_MOST);
            Rgba { width, height, data }
        });
        self.previews.insert(path.to_string(), image);
    }

    fn preview(&self, path: &str) -> Option<&Rgba> {
        self.previews.get(path).and_then(Option::as_ref)
    }

    fn tile_preview(&self, index: usize) -> Option<&Rgba> {
        self.tiles.get(index)?.path().and_then(|path| self.preview(path))
    }

    /// A round whole, as ig-round-grid shows it: its label, its references
    /// in a column at the left, each with its role, and its takes as a
    /// grid of numbered takes, the one shown outlined with a star.
    fn draw_grid(&self, canvas: &mut Rgba, round: &Round, tiles: &[usize], shown: usize) {
        let (width, height) = (canvas.width, canvas.height);
        let label = format!("Round {} · {} · {}", round.number, round.model, takes_word(tiles.len()));
        let mut top = MARGIN as f32;
        top += line(canvas, &label, [MARGIN as f32, top], TITLE, MUTED, false) + 12.0;
        let column = if round.references.is_empty() { 0 } else { (width as f32 * 0.16).clamp(110.0, 220.0) as u32 };
        if column > 0 {
            let x = MARGIN as f32;
            let mut y = top;
            y += line(canvas, "References", [x, y], LABEL, MUTED, false) + 8.0;
            for reference in &round.references {
                let image = self.preview(&reference.path);
                let (w, h) = layout::fit(column, column * 3 / 4, aspect_of(image));
                draw_image(canvas, image, Rect { x: x as i32, y: y as i32, w, h });
                y += h as f32 + 6.0;
                y += words(canvas, &reference_name(reference), [x, y], LABEL, INK, false, Some(column as f32)) + 14.0;
            }
        }
        let left = MARGIN + if column > 0 { column + 2 * GAP } else { 0 };
        let area = Rect {
            x: left as i32,
            y: top as i32,
            w: width.saturating_sub(left + MARGIN),
            h: (height as f32 - top - MARGIN as f32).max(1.0) as u32,
        };
        let rects = layout::grid(area, tiles.len() as u32, GAP, self.aspect());
        for (place, (index, rect)) in tiles.iter().zip(&rects).enumerate() {
            draw_take(canvas, self.tile_preview(*index), *rect, place + 1, *index == shown);
        }
    }

    /// Two rounds one above the other, as ig-rounds-side-by-side shows
    /// them: each labelled with its model and its prompt's first words,
    /// its takes in a row.
    fn draw_pair(&self, canvas: &mut Rgba, pair: &[(Round, Vec<usize>); 2], shown: usize) {
        let (width, height) = (canvas.width, canvas.height);
        let section = height.saturating_sub(2 * MARGIN + GAP) / 2;
        let wide = width.saturating_sub(2 * MARGIN);
        for (place, (round, tiles)) in pair.iter().enumerate() {
            let top = MARGIN + place as u32 * (section + GAP);
            let mut y = top as f32;
            y += line(canvas, &format!("Round {} · {}", round.number, round.model), [MARGIN as f32, y], TITLE, INK, true) + 2.0;
            y += line(canvas, &first_words(&round.prompt), [MARGIN as f32, y], LABEL, MUTED, false) + 10.0;
            let area = Rect { x: MARGIN as i32, y: y as i32, w: wide, h: ((top + section) as f32 - y).max(1.0) as u32 };
            let rects = layout::cells(area, 1, tiles.len().max(1) as u32, 8, self.aspect());
            for (number, (index, rect)) in tiles.iter().zip(&rects).enumerate() {
                draw_take(canvas, self.tile_preview(*index), *rect, number + 1, *index == shown);
            }
        }
        let between = (MARGIN + section + GAP / 2) as f32;
        notes::fill(canvas, [MARGIN as f32, between, wide as f32, 1.0], LINE);
    }

    /// The take beside the references its round sent, as ig-references-sheet
    /// shows it: each labelled with its order and role, then those the
    /// round's report says were not sent.
    fn draw_sheet(&self, canvas: &mut Rgba, round: &Round, shown: usize) {
        let (width, height) = (canvas.width, canvas.height);
        let half = (width as f32 * 0.45) as u32;
        let x = MARGIN as f32;
        let mut y = MARGIN as f32;
        y += line(canvas, &format!("Round {} · the references sent", round.number), [x, y], LABEL, MUTED, false) + 14.0;
        let thumb = ((half.saturating_sub(2 * MARGIN)) as f32 * 0.42).max(40.0) as u32;
        let text_x = x + thumb as f32 + 14.0;
        let text_w = (half as f32 - MARGIN as f32 - text_x).max(40.0);
        let row = |canvas: &mut Rgba, y: &mut f32, path: &str, label: &str, colour: [u8; 3]| {
            let image = self.preview(path);
            let (w, h) = layout::fit(thumb, thumb * 9 / 16, aspect_of(image));
            draw_image(canvas, image, Rect { x: x as i32, y: *y as i32, w, h });
            let tall = words(canvas, label, [text_x, *y + 4.0], LABEL, colour, false, Some(text_w));
            *y += (h as f32).max(tall + 4.0) + 14.0;
        };
        for (index, reference) in round.references.iter().enumerate() {
            row(&mut *canvas, &mut y, reference.path.as_str(), sheet_label(index, reference).as_str(), INK);
        }
        if !round.left_out.is_empty() {
            notes::fill(canvas, [x, y, half.saturating_sub(2 * MARGIN) as f32, 1.0], LINE);
            y += 12.0;
            y += line(canvas, "Not sent", [x, y], LABEL, MUTED, false) + 8.0;
            for reference in &round.left_out {
                row(&mut *canvas, &mut y, reference.path.as_str(), format!("{}, left out", file_name(&reference.path)).as_str(), MUTED);
            }
        }
        notes::fill(canvas, [half as f32, 0.0, 1.0, height as f32], LINE);
        let right = half + MARGIN;
        let area = Rect {
            x: right as i32,
            y: (MARGIN + 40) as i32,
            w: width.saturating_sub(right + MARGIN),
            h: height.saturating_sub(2 * MARGIN + 40),
        };
        let image = self.tile_preview(shown);
        let (w, h) = layout::fit(area.w, area.h, image.map_or(self.aspect(), |image| aspect_of(Some(image))));
        let rect = Rect { x: area.x + ((area.w - w) / 2) as i32, y: area.y + ((area.h - h) / 2) as i32, w, h };
        let number = self.tiles.get(shown).map_or(1, |tile| tile.take + 1);
        line(canvas, &format!("Take {number}"), [rect.x as f32, rect.y as f32 - 24.0], LABEL, MUTED, false);
        draw_image(canvas, image, rect);
    }
}

/// How an item of the tab's file names the take at `path`.
pub fn take_item(path: &str) -> String {
    format!("{TAKE_ITEM}{path}")
}

/// "1 take", "4 takes".
fn takes_word(count: usize) -> String {
    if count == 1 { String::from("1 take") } else { format!("{count} takes") }
}

/// A file's name, without its folders.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A reference's words under it in the grid: its role, or its file's name.
fn reference_name(reference: &SentReference) -> String {
    reference.role.clone().filter(|role| !role.trim().is_empty()).unwrap_or_else(|| file_name(&reference.path).to_string())
}

/// A reference's words on the references sheet, as ig-references-sheet
/// shows them: its order and role, "1st, control: the room's layout.", or
/// its order and file.
pub fn sheet_label(index: usize, reference: &SentReference) -> String {
    let what = match reference.role.as_deref().map(str::trim).filter(|role| !role.is_empty()) {
        Some(role) => match role.split_once(", ") {
            Some((word, words)) => format!("{}: {words}", lower_role(word)),
            None => lower_role(role),
        },
        None => file_name(&reference.path).to_string(),
    };
    format!("{}, {what}.", ordinal(index + 1))
}

/// A role's menu word in the middle of a sentence: one of the menu's own
/// roles lower case, and any other word as it was written.
fn lower_role(word: &str) -> String {
    if ROLES.contains(&word) { word.to_lowercase() } else { word.to_string() }
}

/// "1st", "2nd", "3rd", "4th", … "11th", "21st".
fn ordinal(number: usize) -> String {
    let suffix = match (number % 10, number % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

/// A prompt's first words, with "…" when there are more.
pub fn first_words(prompt: &str) -> String {
    let all: Vec<&str> = prompt.split_whitespace().collect();
    if all.len() <= FIRST_WORDS {
        return all.join(" ");
    }
    format!("{}…", all[..FIRST_WORDS].join(" ").trim_end_matches([',', '.', ';', ':']))
}

fn aspect_of(image: Option<&Rgba>) -> f32 {
    image.map_or(16.0 / 9.0, |image| image.width as f32 / image.height.max(1) as f32)
}

/// One line of words with its top left at `at`, drawn by `layers::label`,
/// as both workspaces draw their views' labels. Returns how far the next
/// line goes below it.
fn line(canvas: &mut Rgba, words: &str, at: [f32; 2], size: f32, colour: [u8; 3], bold: bool) -> f32 {
    if words.trim().is_empty() {
        return 0.0;
    }
    let look = label::Look { size, colour: [colour[0], colour[1], colour[2], 255], bold };
    label::draw(canvas, words, at, look, label::Anchor::Left);
    label::height(size) * 1.2
}

/// Words drawn with their box's corner at `at`, wrapped to `width` when
/// given, with the image's own text layout. Returns how tall they are.
fn words(canvas: &mut Rgba, words: &str, at: [f32; 2], size: f32, colour: [u8; 3], bold: bool, width: Option<f32>) -> f32 {
    if words.trim().is_empty() {
        return 0.0;
    }
    let text = Text { words: words.to_string(), style: Style { size, color: colour, bold, ..Style::default() }, width };
    let layout = text::lay(&text);
    notes::paint(canvas, &text::draw(&text), at);
    layout.height.max(layout.advance)
}

/// An image scaled into `rect`, or a grey place where none could be read.
fn draw_image(canvas: &mut Rgba, image: Option<&Rgba>, rect: Rect) {
    match image {
        Some(image) => layout::place(canvas, image, rect),
        None => notes::fill(canvas, [rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32], WAITING),
    }
}

/// A take in a round view: its image, its number on a dark tile at its
/// corner, and, when it is the one shown, a blue outline and a star.
fn draw_take(canvas: &mut Rgba, image: Option<&Rgba>, rect: Rect, number: usize, chosen: bool) {
    draw_image(canvas, image, rect);
    let side = (rect.h as f32 * 0.14).clamp(16.0, 28.0).round();
    let corner = [rect.x as f32 + 8.0, rect.y as f32 + 8.0];
    notes::fill(canvas, [corner[0], corner[1], side, side], BADGE);
    let size = side * 0.62;
    let look = label::Look { size, colour: [255, 255, 255, 255], bold: true };
    let at = [corner[0] + side / 2.0, corner[1] + (side - label::height(size)) / 2.0];
    label::draw(canvas, &number.to_string(), at, look, label::Anchor::Centre);
    if chosen {
        let [x, y, w, h] = [rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32];
        let edge = 3.0;
        for bar in [[x - edge, y - edge, w + 2.0 * edge, edge], [x - edge, y + h, w + 2.0 * edge, edge], [x - edge, y, edge, h], [x + w, y, edge, h]] {
            notes::fill(canvas, bar, BLUE);
        }
        let radius = (side * 0.5).max(9.0);
        let centre = [x + w - radius - 8.0, y + radius + 8.0];
        layers::notes::disc(canvas, centre[0], centre[1], radius, BLUE);
        star(canvas, centre, radius * 0.6, [255, 255, 255, 255]);
    }
}

/// A five-pointed star, filled, its points `radius` from `centre`.
fn star(canvas: &mut Rgba, centre: [f32; 2], radius: f32, colour: [u8; 4]) {
    let points: Vec<[f32; 2]> = (0..10)
        .map(|index| {
            let reach = if index % 2 == 0 { radius } else { radius * 0.45 };
            let angle = std::f32::consts::PI * (index as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
            [centre[0] + reach * angle.cos(), centre[1] + reach * angle.sin()]
        })
        .collect();
    let (x0, x1) = ((centre[0] - radius).floor().max(0.0) as u32, ((centre[0] + radius).ceil().max(0.0) as u32).min(canvas.width));
    let (y0, y1) = ((centre[1] - radius).floor().max(0.0) as u32, ((centre[1] + radius).ceil().max(0.0) as u32).min(canvas.height));
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut inside = false;
            for (index, a) in points.iter().enumerate() {
                let b = points[(index + 1) % points.len()];
                if (a[1] > py) != (b[1] > py) && px < a[0] + (py - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                    inside = !inside;
                }
            }
            if inside {
                canvas.set(x, y, colour);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(request: &str, number: u32, takes: &[&str], state: RoundState, count: Option<u32>) -> Round {
        Round {
            request: request.into(),
            number,
            model: "Model".into(),
            prompt: String::new(),
            references: Vec::new(),
            left_out: Vec::new(),
            state,
            started: Some(1_793_146_000),
            count,
            takes: takes.iter().map(|path| TakeFile { path: path.to_string(), media_type: "image/png".into(), canon: false }).collect(),
            error: None,
            report: None,
            artifact: None,
        }
    }

    /// Catches a strip that loses a round's runs or its places: a done
    /// round has a tile for each image, one being made a place for each
    /// take still to come, and one that failed with nothing a failed
    /// place; a take of another kind is left out.
    #[test]
    fn rounds_lay_out_the_strip() {
        let mut rounds = Rounds::default();
        let mut first = round("r1", 1, &["library/a.png", "library/b.png"], RoundState::Done, None);
        first.takes.push(TakeFile { path: "library/c.mp4".into(), media_type: "video/mp4".into(), canon: false });
        let sent = vec![first, round("r2", 2, &["library/d.png"], RoundState::Running, Some(3)), round("r3", 3, &[], RoundState::Failed, None)];
        rounds.set("c1", sent, &["c1".to_string()]);
        let groups: Vec<(String, usize)> = rounds.groups();
        assert_eq!(groups, [("Round 1 · Model".to_string(), 2), ("Round 2 · Model".to_string(), 3), ("Round 3 · Model".to_string(), 1)]);
        assert!(matches!(rounds.tiles[3].pending, Some(FilmPending::Running { since: Some(_) })));
        assert_eq!(rounds.tiles[5].pending, Some(FilmPending::Failed));
        assert_eq!(rounds.tile_of("library/b.png"), Some(1));
        // A round sent again without a take: its tile goes.
        rounds.set("c1", vec![round("r1", 1, &["library/b.png"], RoundState::Done, None)], &["c1".to_string()]);
        assert_eq!(rounds.groups(), [("Round 1 · Model".to_string(), 1)]);
        assert_eq!(rounds.tile_of("library/a.png"), None);
    }

    /// Catches a references sheet that reads other than ig-references-sheet:
    /// order and role, the menu word lower case, a role of the person's
    /// own words as written, and a file's name with no role.
    #[test]
    fn the_sheet_labels_each_reference_by_order_and_role() {
        let reference = |path: &str, role: Option<&str>| SentReference { path: path.into(), role: role.map(str::to_string) };
        assert_eq!(sheet_label(0, &reference("a.png", Some("Control, the room's layout"))), "1st, control: the room's layout.");
        assert_eq!(sheet_label(1, &reference("b.png", Some("Image 1, the keeper"))), "2nd, Image 1: the keeper.");
        assert_eq!(sheet_label(2, &reference("refs/c.png", None)), "3rd, c.png.");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(first_words("The keeper turns from the window toward the radio as it crackles."), "The keeper turns from the window toward the radio…");
    }
}
