//! The project panel, on the conversation column's Project tab: a project's
//! five folders as Spumcake-Skills lays them out, Brief, Pipeline, Library,
//! Requests, and Output, each with its count, opening to its files, as the
//! approved views pp-anchor, pp-requests, pp-folders, and pp-file-menu show
//! them. Everything is read off the window's thread: the folders and each
//! report's record when the panel is shown or the window comes back into
//! focus, the recipes with `pipeline.py sql --read-only`, and each tile's
//! image when it is first drawn.

use crate::report::Report;
use crate::typescale as ts;
use crate::widgets::{self, MUTED, TEXT};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// What workspaces made of videos and sounds, by path.
type Previews = HashMap<String, crate::panel::ReferencePreview>;

/// A project's folders, in the order the panel lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Folder {
    Brief,
    Pipeline,
    Library,
    Requests,
    Output,
}

impl Folder {
    const ALL: [Folder; 5] = [Folder::Brief, Folder::Pipeline, Folder::Library, Folder::Requests, Folder::Output];

    /// Its name as Skills names the folder, capitalised: "Library".
    fn name(self) -> &'static str {
        match self {
            Folder::Brief => "Brief",
            Folder::Pipeline => "Pipeline",
            Folder::Library => "Library",
            Folder::Requests => "Requests",
            Folder::Output => "Output",
        }
    }
}

/// What the user asked of the panel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PanelEvent {
    /// Open a file of the project, by its path in it, as its kind asks.
    Open(String),
    /// Show a request's report card, by the request's id.
    OpenReport(String),
    /// Open the folder that holds a file, by the file's path.
    ShowInFolder(String),
}

/// What kind of media a file or a request is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Video,
    Sound,
}

impl Kind {
    pub(crate) fn of(path: &str) -> Option<Kind> {
        let lower = path.to_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        match ext {
            "png" | "jpg" | "jpeg" | "webp" | "gif" => Some(Kind::Image),
            "mp4" | "mov" | "webm" => Some(Kind::Video),
            "wav" | "mp3" | "flac" | "ogg" | "m4a" => Some(Kind::Sound),
            _ => None,
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Kind::Image => "media-image",
            Kind::Video => "media-video",
            Kind::Sound => "sound-high",
        }
    }
}

/// One request's row: its name, and on a grey line its model, day, and cost.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RequestRow {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The model as the record names it, shown by its name.
    pub model: Option<String>,
    /// "14 Nov".
    pub day: String,
    /// "$0.24", or "cost unknown".
    pub cost: String,
    /// The request's first output, for its thumbnail.
    pub output: Option<String>,
    /// Closed as lost: likely spent, never filed.
    pub lost: bool,
}

impl RequestRow {
    fn of(report: &Report) -> RequestRow {
        let output = report.outputs.first().cloned();
        let model = report.model.clone();
        let named = model.as_deref().unwrap_or_default().to_lowercase();
        let kind = output.as_deref().and_then(Kind::of).unwrap_or(if named.contains("video") {
            Kind::Video
        } else if named.contains("audio") || named.contains("sound") {
            Kind::Sound
        } else {
            Kind::Image
        });
        let made = report.made();
        RequestRow {
            id: report.id.clone(),
            name: report.name.clone(),
            kind,
            model,
            day: made.split(',').next().unwrap_or_default().to_string(),
            cost: report.cost_short(),
            output,
            lost: report.lost.is_some(),
        }
    }
}

/// One of Library's tiles: a reference asset at its latest iteration.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tile {
    /// Its latest iteration, the file shown.
    pub file: String,
    /// How many iterations it has, the first take counted.
    pub iterations: usize,
    /// Its cell, "the keeper · model sheet", when it is in a recipe.
    pub cell: Option<String>,
    /// Out of date, as `pipeline.py touches` says.
    pub outlined: bool,
}

/// The project's reference assets, grouped as the cells' rounds group them
/// (`crate::assets`), with the canon they were read with; none where the
/// project has no pipeline or Skills is not found.
pub(crate) fn grouped(project: &Path, scripts: Option<&Scripts>) -> Option<(Vec<crate::assets::Asset>, crate::prompt_page::Canon)> {
    let (scripts, pipeline) = (scripts?, crate::prompt_page::living_pipeline(project)?);
    let canon = crate::prompt_page::read_canon(&scripts.scripts, project, &scripts.vars, &pipeline).ok()?;
    let printed = crate::prompt_page::script(&scripts.scripts, project, &scripts.vars, "pipeline.py", &["touches", &pipeline, "--json"]).ok()?;
    let touches = serde_json::from_str(&printed).ok()?;
    Some((crate::assets::assets(project, &canon, &touches), canon))
}

/// An asset's cell as its tile and its report name it: the recipe and its
/// step, "the keeper · model sheet".
pub(crate) fn cell_words(asset: &crate::assets::Asset, canon: &crate::prompt_page::Canon) -> Option<String> {
    let (chain, order) = asset.cell.as_ref()?;
    let mut steps: Vec<&(String, String)> = canon.step_orders.iter().filter(|((c, _), o)| c == chain && **o == *order).map(|(step, _)| step).collect();
    steps.sort();
    let own = steps.iter().find(|step| asset.artifact.is_some() && canon.step_artifacts.get(**step) == asset.artifact.as_ref()).or(steps.first())?;
    Some(format!("{chain} · {}", own.1))
}

/// Everything the panel lists, as read at once.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Listing {
    pub brief: Vec<String>,
    /// Each recipe, the chain it is in Skills, with its count of cells.
    pub recipes: Vec<(String, usize)>,
    /// The library's reference assets, newest first.
    pub library: Vec<Tile>,
    /// Newest first.
    pub requests: Vec<RequestRow>,
    /// The delivered films, each with its length in seconds when known.
    pub output: Vec<(String, Option<f64>)>,
}

impl Listing {
    fn count(&self, folder: Folder) -> String {
        match folder {
            Folder::Brief => self.brief.len().to_string(),
            Folder::Pipeline => format!("{} recipes", self.recipes.len()),
            Folder::Library => format!("{} assets", self.library.len()),
            Folder::Requests => self.requests.len().to_string(),
            Folder::Output => self.output.len().to_string(),
        }
    }
}

/// Where Skills' scripts run for this project, to read its recipes.
#[derive(Clone)]
pub(crate) struct Scripts {
    pub scripts: PathBuf,
    pub vars: Vec<(String, String)>,
}

/// Reads a project's folders. Reports are read again only when changed.
pub(crate) fn read(project: &Path, scripts: Option<&Scripts>, known: &Mutex<HashMap<String, (SystemTime, RequestRow)>>) -> Listing {
    let names = |sub: &str| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(project.join(sub))
            .map(|entries| entries.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        names.retain(|name| !name.starts_with('.'));
        names.sort();
        names
    };
    let brief = names("brief").into_iter().map(|name| format!("brief/{name}")).collect();
    // Library's assets, each at its latest iteration; where Skills cannot
    // read the canon, each file is a take of its own.
    let mut library: Vec<Tile> = match grouped(project, scripts) {
        Some((assets, canon)) => assets
            .iter()
            .map(|asset| Tile { file: asset.canonical().to_string(), iterations: asset.files().count(), cell: cell_words(asset, &canon), outlined: asset.outlined })
            .collect(),
        None => names("library")
            .into_iter()
            .filter(|name| Kind::of(name).is_some())
            .map(|name| Tile { file: format!("library/{name}"), iterations: 1, cell: None, outlined: false })
            .collect(),
    };
    let named = |tile: &Tile| tile.file.rsplit('/').next().unwrap_or(&tile.file).to_string();
    library.sort_by_key(|tile| std::cmp::Reverse(named(tile)));
    let mut requests = Vec::new();
    {
        let mut known = known.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        for name in names("requests").into_iter().filter(|name| name.ends_with(".html")) {
            let path = format!("requests/{name}");
            let changed = std::fs::metadata(project.join(&path)).and_then(|meta| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            match known.get(&path) {
                Some((at, row)) if *at == changed => requests.push(row.clone()),
                _ => {
                    if let Ok(report) = Report::read(project, &path) {
                        let row = RequestRow::of(&report);
                        known.insert(path, (changed, row.clone()));
                        requests.push(row);
                    }
                }
            }
        }
    }
    requests.reverse();
    let output = names("output").into_iter().map(|name| {
        let path = format!("output/{name}");
        let length = if Kind::of(&name) == Some(Kind::Video) { mp4_seconds(&project.join(&path)) } else { None };
        (path, length)
    });
    Listing { brief, recipes: recipes(project, scripts), library, requests, output: output.collect() }
}

/// The living pipeline's recipes, the chains in Skills, in their order,
/// each with its count of cells: the places its steps share.
fn recipes(project: &Path, scripts: Option<&Scripts>) -> Vec<(String, usize)> {
    let (Some(scripts), Some(pipeline)) = (scripts, crate::prompt_page::living_pipeline(project)) else { return Vec::new() };
    let query = "SELECT json_array(name, (SELECT count(DISTINCT ord) FROM step WHERE step.chain = chain.name)) FROM chain ORDER BY ord";
    let printed = crate::prompt_page::script(&scripts.scripts, project, &scripts.vars, "pipeline.py", &["sql", &pipeline, query, "--read-only"]).unwrap_or_default();
    printed
        .lines()
        .filter_map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).ok()?;
            Some((row.get(0)?.as_str()?.to_string(), row.get(1)?.as_u64()? as usize))
        })
        .collect()
}

/// An MP4's length, from its `mvhd` box, without decoding it: the host
/// never decodes video.
pub(crate) fn mp4_seconds(path: &Path) -> Option<f64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let end = file.metadata().ok()?.len();
    let header = |file: &mut std::fs::File, at: u64| -> Option<(u64, [u8; 4], u64)> {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut head = [0u8; 8];
        file.read_exact(&mut head).ok()?;
        let mut size = u64::from(u32::from_be_bytes(head[..4].try_into().ok()?));
        let mut body = at + 8;
        if size == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large).ok()?;
            size = u64::from_be_bytes(large);
            body += 8;
        } else if size == 0 {
            size = end - at;
        }
        Some((size, head[4..].try_into().ok()?, body))
    };
    // The top-level boxes, to `moov`, then within it, to `mvhd`.
    let (mut at, mut stop) = (0, end);
    let mut inside = false;
    while at + 8 <= stop {
        let (size, kind, body) = header(&mut file, at)?;
        if size < 8 {
            return None;
        }
        match (&kind, inside) {
            (b"moov", false) => {
                inside = true;
                stop = at + size;
                at = body;
                continue;
            }
            (b"mvhd", true) => {
                file.seek(SeekFrom::Start(body)).ok()?;
                let mut version = [0u8; 4];
                file.read_exact(&mut version).ok()?;
                let (scale, duration) = if version[0] == 1 {
                    let mut fields = [0u8; 28];
                    file.read_exact(&mut fields).ok()?;
                    (u32::from_be_bytes(fields[16..20].try_into().ok()?), u64::from_be_bytes(fields[20..28].try_into().ok()?))
                } else {
                    let mut fields = [0u8; 16];
                    file.read_exact(&mut fields).ok()?;
                    (u32::from_be_bytes(fields[8..12].try_into().ok()?), u64::from(u32::from_be_bytes(fields[12..16].try_into().ok()?)))
                };
                return (scale > 0).then(|| duration as f64 / f64::from(scale));
            }
            _ => at += size,
        }
    }
    None
}

/// A length as the panel shows it: "1:42".
pub(crate) fn clock(seconds: f64) -> String {
    let whole = seconds.round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// The request a library file was filed by, from its name,
/// `<request id>[-n].<ext>`, as Skills' layout names outputs.
pub(crate) fn request_of(path: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if exists(stem) {
        return Some(stem.to_string());
    }
    let (base, n) = stem.rsplit_once('-')?;
    (n.parse::<u32>().is_ok() && exists(base)).then(|| base.to_string())
}

/// The panel between frames.
pub(crate) struct ProjectPanel {
    project: Option<PathBuf>,
    pub(crate) open: BTreeSet<Folder>,
    pub(crate) listing: Listing,
    reading: Option<Receiver<Listing>>,
    known: Arc<Mutex<HashMap<String, (SystemTime, RequestRow)>>>,
    /// The file or request chosen, by its path.
    selected: Option<String>,
    thumbs: Thumbs,
    focused: bool,
    /// Videos and sounds drawn this frame, whose previews the window asks
    /// of the workspaces that make them.
    pub(crate) wanted: Vec<String>,
}

impl Default for ProjectPanel {
    fn default() -> Self {
        Self {
            project: None,
            open: BTreeSet::from([Folder::Library]),
            listing: Listing::default(),
            reading: None,
            known: Arc::default(),
            selected: None,
            thumbs: Thumbs::default(),
            focused: true,
            wanted: Vec::new(),
        }
    }
}

/// A tile's or a row's small image, decoded and shrunk off the window's
/// thread, once.
#[derive(Default)]
struct Thumbs {
    shown: HashMap<String, egui::TextureHandle>,
    asked: HashSet<String>,
    sender: Option<Sender<PathBuf>>,
    results: Option<Receiver<(String, egui::ColorImage)>>,
}

/// The widest a thumbnail is kept, in pixels.
const THUMB_WIDTH: usize = 240;

impl Thumbs {
    fn get(&mut self, ctx: &egui::Context, project: &Path, path: &str) -> Option<egui::TextureId> {
        if let Some(results) = &self.results {
            for (key, image) in results.try_iter() {
                let texture = ctx.load_texture(format!("project-panel:{key}"), image, egui::TextureOptions::LINEAR);
                self.shown.insert(key, texture);
            }
        }
        if let Some(texture) = self.shown.get(path) {
            return Some(texture.id());
        }
        if Kind::of(path) != Some(Kind::Image) || !self.asked.insert(path.to_string()) {
            return None;
        }
        if self.sender.is_none() {
            let (asks, asked) = channel::<PathBuf>();
            let (send, results) = channel();
            let ctx = ctx.clone();
            let root = project.to_path_buf();
            std::thread::spawn(move || {
                for path in asked {
                    if let Some(image) = shrunk(&root.join(&path)) {
                        if send.send((path.display().to_string(), image)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                }
            });
            self.sender = Some(asks);
            self.results = Some(results);
        }
        let _ = self.sender.as_ref()?.send(PathBuf::from(path));
        None
    }
}

/// An image file, shrunk to at most `THUMB_WIDTH` wide by averaging.
fn shrunk(file: &Path) -> Option<egui::ColorImage> {
    let icon = eframe::icon_data::from_png_bytes(&std::fs::read(file).ok()?).ok()?;
    let (w, h) = (icon.width as usize, icon.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let step = w.div_ceil(THUMB_WIDTH).max(1);
    let (tw, th) = (w / step, h / step);
    let mut pixels = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        for x in 0..tw {
            let mut sum = [0u32; 4];
            for dy in 0..step {
                for dx in 0..step {
                    let at = ((y * step + dy) * w + x * step + dx) * 4;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(icon.rgba[at + c]);
                    }
                }
            }
            let n = (step * step) as u32;
            pixels.extend(sum.iter().map(|total| (total / n) as u8));
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([tw, th], &pixels))
}

const ROW_H: f32 = 43.0;
const REQUEST_H: f32 = 62.0;
const FILE_H: f32 = 40.0;
const LEFT: f32 = 18.0;
const ICON: f32 = 20.0;
/// A chosen row, pale blue as the outliner's.
const PICKED: Color32 = Color32::from_rgb(226, 236, 252);
const PICKED_EDGE: Color32 = Color32::from_rgb(37, 99, 235);
/// The red of what failed, was lost, or is out of date.
const RED: Color32 = Color32::from_rgb(220, 38, 38);

impl ProjectPanel {
    /// Reads the project's folders again, off the window's thread.
    pub(crate) fn refresh(&mut self, ctx: &egui::Context, project: &Path, scripts: Option<Scripts>) {
        if self.project.as_deref() != Some(project) {
            *self = ProjectPanel { project: Some(project.to_path_buf()), ..ProjectPanel::default() };
        }
        let (send, reading) = channel();
        let (root, known, ctx) = (project.to_path_buf(), self.known.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = send.send(read(&root, scripts.as_ref(), &known));
            ctx.request_repaint();
        });
        self.reading = Some(reading);
    }

    /// Whether the panel should read its folders again: it shows another
    /// project, or the window has just come back into focus.
    pub(crate) fn stale(&mut self, ctx: &egui::Context, project: &Path) -> bool {
        let focused = ctx.input(|input| input.focused);
        let regained = focused && !self.focused;
        self.focused = focused;
        self.project.as_deref() != Some(project) || regained
    }

    /// Draws the panel in `rect`. `model_name` names a model as the user
    /// knows it.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, rect: Rect, model_name: &dyn Fn(&str) -> String, previews: &Previews) -> Vec<PanelEvent> {
        if let Some(listing) = self.reading.as_ref().and_then(|reading| reading.try_recv().ok()) {
            self.listing = listing;
            self.reading = None;
        }
        let mut events = Vec::new();
        let Some(project) = self.project.clone() else { return events };
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical().id_salt("project-panel").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let width = rect.width();
                for folder in Folder::ALL {
                    self.folder_row(ui, width, folder);
                    if self.open.contains(&folder) {
                        match folder {
                            Folder::Library => self.library(ui, width, &project, previews, &mut events),
                            Folder::Requests => self.requests(ui, width, &project, model_name, previews, &mut events),
                            Folder::Brief => {
                                for path in self.listing.brief.clone() {
                                    self.file_row(ui, width, "page", &path, None, &mut events);
                                }
                            }
                            Folder::Pipeline => {
                                // A recipe opens in its workspace once
                                // milestone 8 builds it.
                                for (name, cells) in self.listing.recipes.clone() {
                                    let (row, _) = ui.allocate_exact_size(vec2(width, FILE_H), Sense::hover());
                                    let painter = ui.painter().with_clip_rect(row);
                                    crate::toolbar::paint_icon(&painter, pos2(row.left() + LEFT + 46.0, row.center().y), "table-2-columns", ICON, MUTED);
                                    painter.text(pos2(row.left() + LEFT + 70.0, row.center().y), Align2::LEFT_CENTER, &name, ts::body(), TEXT);
                                    painter.text(pos2(row.right() - LEFT, row.center().y), Align2::RIGHT_CENTER, cells.to_string(), ts::body(), MUTED);
                                }
                            }
                            Folder::Output => {
                                for (path, length) in self.listing.output.clone() {
                                    self.file_row(ui, width, "media-video", &path, length.map(clock), &mut events);
                                }
                            }
                        }
                    }
                }
            });
        });
        events
    }

    fn folder_row(&mut self, ui: &mut egui::Ui, width: f32, folder: Folder) {
        let (row, response) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
        let painter = ui.painter().with_clip_rect(row);
        let open = self.open.contains(&folder);
        let y = row.center().y;
        if open {
            widgets::chevron(&painter, pos2(row.left() + LEFT + 6.0, y), TEXT, widgets::Scale(1.0));
        } else {
            widgets::chevron_side(&painter, pos2(row.left() + LEFT + 6.0, y), true, TEXT);
        }
        crate::toolbar::paint_icon(&painter, pos2(row.left() + LEFT + 38.0, y), "folder", ICON, TEXT);
        painter.text(pos2(row.left() + LEFT + 64.0, y), Align2::LEFT_CENTER, folder.name(), ts::body(), TEXT);
        painter.text(pos2(row.right() - LEFT, y), Align2::RIGHT_CENTER, self.listing.count(folder), ts::body(), MUTED);
        if response.clicked() && !self.open.remove(&folder) {
            self.open.insert(folder);
        }
    }

    /// One file of Brief or Output: its icon, its name, and what is at its
    /// right, such as a film's length. A click opens it.
    fn file_row(&mut self, ui: &mut egui::Ui, width: f32, icon: &str, path: &str, right: Option<String>, events: &mut Vec<PanelEvent>) {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let room = width - 2.0 * LEFT - 56.0 - 40.0;
        let galley = ui.painter().layout(name, ts::body(), TEXT, room);
        let (row, response) = ui.allocate_exact_size(vec2(width, (galley.size().y + 16.0).max(FILE_H)), Sense::click());
        let selected = self.selected.as_deref() == Some(path);
        let painter = ui.painter().with_clip_rect(row);
        if selected {
            painter.rect_filled(row.shrink2(vec2(8.0, 2.0)), 8.0, PICKED);
        }
        crate::toolbar::paint_icon(&painter, pos2(row.left() + LEFT + 28.0, row.center().y), icon, ICON, TEXT);
        painter.galley(pos2(row.left() + LEFT + 52.0, row.center().y - galley.size().y / 2.0), galley, TEXT);
        if let Some(right) = right {
            painter.text(pos2(row.right() - LEFT, row.center().y), Align2::RIGHT_CENTER, right, ts::body(), MUTED);
        }
        if response.clicked() {
            self.selected = Some(path.to_string());
            events.push(PanelEvent::Open(path.to_string()));
        }
        file_menu(&response, path, events);
    }

    /// Library's tiles, three across, newest first, each with its name
    /// under it; a click chooses one, a double click opens it.
    fn library(&mut self, ui: &mut egui::Ui, width: f32, project: &Path, previews: &Previews, events: &mut Vec<PanelEvent>) {
        const GAP: f32 = 8.0;
        let inner = width - 2.0 * LEFT;
        let tile_w = (inner - 2.0 * GAP) / 3.0;
        let tile_h = tile_w * 9.0 / 16.0;
        let caption_h = 34.0;
        let library = self.listing.library.clone();
        for chunk in library.chunks(3) {
            let (band, _) = ui.allocate_exact_size(vec2(width, tile_h + caption_h + GAP), Sense::hover());
            if !ui.is_rect_visible(band) {
                continue;
            }
            for (i, shown) in chunk.iter().enumerate() {
                let path = &shown.file;
                let tile = Rect::from_min_size(pos2(band.left() + LEFT + i as f32 * (tile_w + GAP), band.top() + 4.0), vec2(tile_w, tile_h));
                let response = ui.interact(tile, ui.id().with(("library-tile", path)), Sense::click());
                if Kind::of(path) == Some(Kind::Image) {
                    match self.thumbs.get(ui.ctx(), project, path) {
                        Some(texture) => {
                            ui.painter().image(texture, tile, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            ui.painter().rect_filled(tile, 4.0, Color32::from_rgb(243, 244, 246));
                        }
                    }
                } else {
                    // A video's frame or a sound's waveform, as its
                    // workspace previews it.
                    self.wanted.push(path.clone());
                    ui.painter().rect_filled(tile, 4.0, Color32::from_rgb(243, 244, 246));
                    crate::report::shown_file(ui, project, path, previews, tile, false);
                }
                let painter = ui.painter();
                if self.selected.as_deref() == Some(path.as_str()) {
                    painter.rect_stroke(tile.expand(1.0), 4.0, Stroke::new(2.0, PICKED_EDGE), egui::StrokeKind::Outside);
                }
                // Out of date: the red outline.
                if shown.outlined {
                    painter.rect_stroke(tile, 2.0, Stroke::new(2.0, RED), egui::StrokeKind::Inside);
                }
                // Its count of iterations, in a small grey badge at its top
                // right, when it has more than one.
                if shown.iterations > 1 {
                    let galley = painter.layout_no_wrap(shown.iterations.to_string(), ts::small(), Color32::WHITE);
                    let size = vec2((galley.size().x + 10.0).max(galley.size().y + 4.0), galley.size().y + 4.0);
                    let badge = Rect::from_min_size(pos2(tile.right() - size.x - 4.0, tile.top() + 4.0), size);
                    painter.rect_filled(badge, size.y / 2.0, crate::plan_card::BADGE_FILL);
                    painter.galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
                }
                // Its cell, or, for a take in no recipe, its file's name.
                let name = path.rsplit('/').next().unwrap_or(path);
                let caption = shown.cell.clone().unwrap_or_else(|| name.get(17..).unwrap_or(name).to_string());
                let galley = painter.layout(caption, ts::small(), MUTED, tile_w);
                painter.with_clip_rect(Rect::from_min_size(pos2(tile.left(), tile.bottom() + 4.0), vec2(tile_w, caption_h - 4.0))).galley(
                    pos2(tile.left(), tile.bottom() + 4.0),
                    galley,
                    MUTED,
                );
                if response.clicked() {
                    self.selected = Some(path.clone());
                }
                if response.double_clicked() {
                    events.push(PanelEvent::Open(path.clone()));
                }
                if response.secondary_clicked() {
                    self.selected = Some(path.clone());
                }
                file_menu(&response, path, events);
            }
        }
    }

    /// Requests' rows, newest first: the kind's icon, the first output's
    /// image, the request's name, and its model, day, and cost; a lost one
    /// with the red mark. A click opens its report card.
    fn requests(&mut self, ui: &mut egui::Ui, width: f32, project: &Path, model_name: &dyn Fn(&str) -> String, previews: &Previews, events: &mut Vec<PanelEvent>) {
        let rows = self.listing.requests.clone();
        for request in rows {
            let (row, response) = ui.allocate_exact_size(vec2(width, REQUEST_H), Sense::click());
            if !ui.is_rect_visible(row) {
                continue;
            }
            let painter = ui.painter().with_clip_rect(row);
            if self.selected.as_deref() == Some(request.id.as_str()) {
                painter.rect_filled(row.shrink2(vec2(8.0, 2.0)), 8.0, PICKED);
            }
            let y = row.center().y;
            crate::toolbar::paint_icon(&painter, pos2(row.left() + LEFT + 6.0, y), request.kind.icon(), ICON, TEXT);
            let thumb = Rect::from_min_size(pos2(row.left() + LEFT + 32.0, row.top() + 9.0), vec2(56.0, 44.0));
            let other = request.output.clone().filter(|output| Kind::of(output).is_some_and(|kind| kind != Kind::Image));
            if let Some(output) = other {
                // A video's frame or a sound's waveform, as its workspace
                // previews it.
                self.wanted.push(output.clone());
                painter.rect_filled(thumb, 4.0, Color32::from_rgb(243, 244, 246));
                crate::report::shown_file(ui, project, &output, previews, thumb, false);
            } else { match request.output.as_deref().and_then(|output| self.thumbs.get(ui.ctx(), project, output)) {
                Some(texture) => {
                    let size = ui.ctx().tex_manager().read().meta(texture).map(|meta| vec2(meta.size[0] as f32, meta.size[1] as f32)).unwrap_or(thumb.size());
                    let scale = (thumb.width() / size.x).max(thumb.height() / size.y);
                    let uv = vec2(thumb.width() / (size.x * scale), thumb.height() / (size.y * scale));
                    let uv = Rect::from_center_size(pos2(0.5, 0.5), uv);
                    painter.image(texture, thumb, uv, Color32::WHITE);
                }
                None => {
                    painter.rect_filled(thumb, 4.0, Color32::from_rgb(243, 244, 246));
                    if request.kind != Kind::Image {
                        crate::toolbar::paint_icon(&painter, thumb.center(), request.kind.icon(), 16.0, MUTED);
                    }
                }
            } }
            let left = thumb.right() + 14.0;
            let room = row.right() - LEFT - left;
            let named = crate::report::cut(ui, &request.name, ts::body(), TEXT, room - 16.0);
            let name = Rect::from_min_size(pos2(left, y - 10.0 - named.size().y / 2.0), named.size());
            painter.galley(name.min, named, TEXT);
            let second = if request.lost {
                widgets::disc(&painter, pos2(name.right() + 10.0, name.center().y), 4.0, RED);
                String::from("Likely spent, never filed")
            } else {
                let model = request.model.as_deref().map(model_name).unwrap_or_default();
                [model, request.day.clone(), request.cost.clone()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" · ")
            };
            let second = crate::report::cut(ui, &second, ts::small(), MUTED, room);
            painter.galley(pos2(left, y + 11.0 - second.size().y / 2.0), second, MUTED);
            if response.clicked() {
                self.selected = Some(request.id.clone());
                events.push(PanelEvent::OpenReport(request.id.clone()));
            }
        }
    }
}

/// A file's right-click menu: Open, Open report, and Show in folder, as
/// pp-file-menu shows it. Open report is there for a file a request filed.
fn file_menu(response: &egui::Response, path: &str, events: &mut Vec<PanelEvent>) {
    let rows = [
        widgets::MenuItem { label: "Open", icon: Some("folder"), enabled: true, submenu: false, divider: false, tip: None },
        widgets::MenuItem { label: "Open report", icon: Some("page"), enabled: path.starts_with("library/"), submenu: false, divider: false, tip: None },
        widgets::MenuItem { label: "Show in folder", icon: Some("open-new-window"), enabled: true, submenu: false, divider: false, tip: None },
    ];
    egui::Popup::context_menu(response).show(|ui| match widgets::menu_rows(ui, &rows) {
        Some(0) => events.push(PanelEvent::Open(path.to_string())),
        Some(1) => events.push(PanelEvent::OpenReport(path.to_string())),
        Some(2) => events.push(PanelEvent::ShowInFolder(path.to_string())),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_file_names_the_request_that_filed_it() {
        let known = ["2026-11-14_13-30-keyframe-shot-12", "2026-11-10_19-17-diner-afternoon-1"];
        let exists = |id: &str| known.contains(&id);
        assert_eq!(request_of("library/2026-11-14_13-30-keyframe-shot-12-3.png", exists).as_deref(), Some("2026-11-14_13-30-keyframe-shot-12"));
        assert_eq!(request_of("library/2026-11-10_19-17-diner-afternoon-1.png", exists).as_deref(), Some("2026-11-10_19-17-diner-afternoon-1"));
        assert_eq!(request_of("library/2026-11-09_01-00-drawn-by-hand.png", exists), None);
        assert_eq!(clock(102.4), "1:42");
    }

    /// Every file in each of Last order's folders is listed, with its count,
    /// and its delivered film's length is read. Runs where `LAST_ORDER` and
    /// `SPUMCAKE_SKILLS_SCRIPTS` are set; reads Last order and writes nothing.
    #[test]
    fn the_panel_lists_every_file_of_last_order() {
        let (Some(project), Some(scripts)) = (std::env::var_os("LAST_ORDER"), std::env::var_os("SPUMCAKE_SKILLS_SCRIPTS")) else {
            eprintln!("LAST_ORDER or SPUMCAKE_SKILLS_SCRIPTS is not set, so the panel's listing is not checked");
            return;
        };
        let project = Path::new(&project);
        let scripts = Scripts { scripts: PathBuf::from(scripts), vars: Vec::new() };
        let listing = read(project, Some(&scripts), &Mutex::default());
        let walk = |sub: &str| std::fs::read_dir(project.join(sub)).unwrap().flatten().filter(|e| e.path().is_file() && !e.file_name().to_string_lossy().starts_with('.')).count();
        assert_eq!(listing.brief.len(), walk("brief"));
        let assets = crate::assets::read_assets(&scripts.scripts, project, &[], &crate::prompt_page::living_pipeline(project).unwrap()).unwrap();
        assert_eq!(listing.library.len(), assets.len(), "Library shows each asset once");
        assert!(listing.library.len() <= walk("library") + walk("prompts/resources"));
        assert!(listing.library.iter().any(|tile| tile.cell.is_some()), "an asset in a recipe names its cell");
        // The animatic, canon in prompts/resources/, is one of Library's
        // tiles, so it plays from there as the film plays from Output.
        let animatic = crate::prompt_page::script(&scripts.scripts, project, &[], "pipeline.py", &["sql", &crate::prompt_page::living_pipeline(project).unwrap(), "SELECT file FROM artifact WHERE role = 'animatic'", "--read-only"]).unwrap();
        assert!(listing.library.iter().any(|tile| tile.file == animatic.trim()), "the animatic {animatic} is not in Library");
        assert_eq!(listing.requests.len(), walk("requests"));
        assert_eq!(listing.output.len(), walk("output"));
        let name = |tile: &Tile| tile.file.rsplit('/').next().unwrap().to_string();
        assert!(listing.library.windows(2).all(|pair| name(&pair[0]) >= name(&pair[1])), "Library is newest first");
        let (_, length) = &listing.output[0];
        assert!(length.is_some_and(|seconds| seconds > 1.0), "the film's length is read: {length:?}");
        let chains = crate::prompt_page::script(&scripts.scripts, project, &[], "pipeline.py", &["sql", &crate::prompt_page::living_pipeline(project).unwrap(), "SELECT count(*) FROM chain", "--read-only"]).unwrap();
        assert_eq!(listing.recipes.len().to_string(), chains.trim(), "every chain is a recipe");
        assert!(listing.recipes.iter().all(|(_, cells)| *cells > 0));
        assert_eq!(listing.count(Folder::Pipeline), format!("{} recipes", listing.recipes.len()));
    }
}
