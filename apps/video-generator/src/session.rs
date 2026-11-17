//! The Video Generator's tabs, each a gallery of takes, and everything the
//! host sends them: commands, the filmstrip's clicks, the Move tool's drag,
//! the keys that step frames, files, and the project.
//!
//! Inside a project, which the host names with `project`, every tab is
//! written into the project's folder as a `.vgx` file by itself, a second
//! after it changes, and before its tab closes, a `keep` is answered, or
//! the host quits, as the Image Generator's images are; in a Skills
//! project, into its `work/` folder. Save and Save As are offered only
//! outside a project.
//!
//! Each tab has cells, the Prompt page's, and the rounds the host says
//! they made, with `takes`, shown on the filmstrip and never kept.

use crate::decode::{Decoders, Key, even};
use crate::declare;
use crate::files::{self, ViewFile};
use crate::gallery::{Gallery, Mode};
use crate::rounds::{Rounds, With};
use crate::view::{self, Look, Scene};
use layers::link::Link;
use protocol::{
    AppMessage, Ask, AskButton, ContentsKind, DocumentCamera, FileFilter, FileKind, HostMessage, PanelSide, RasterCamera,
    RasterGesture, RasterTool, RasterView, Thumbnail,
};
pub use deliver::DELIVER;
pub use mix::{OPEN_SOUND, Page};
pub use notes::{Tool, Tools};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod animatic;
mod deliver;
mod mix;
mod notes;
mod play;
mod requests;

/// How long a change waits before it is written, so a burst of changes is
/// written once.
const WRITE_AFTER: Duration = Duration::from_secs(1);
/// How long a failed write waits before it is tried again by itself.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// The conventions guide's word for what this workspace makes: a new one
/// is "Untitled Video", numbered when that is taken.
pub const ITEM: &str = "Video";
/// The largest a filmstrip thumbnail is, as the Image Generator's: 44 tall,
/// and no wider than four times that.
const FILM_SIZE: [u32; 2] = [176, 44];
/// The image's size before the host says how large its area is.
const FIRST_VIEWPORT: [f64; 2] = [1200.0, 800.0];
/// This workspace's id, as its renders are filed under it.
const OWNER: &str = "video-generator";
/// How an item of a tab's file names a marked frame a next round sends:
/// this, then its fingerprint.
const MARKED_ITEM: &str = "marked ";
/// The words a next round's prompt gives a take's notes, and the role of
/// the marked frame it sends, as the Image Generator's.
const NOTES_WORDS: &str = "Notes on the last round";
/// How an item of a tab's file names a Start frame taken from the
/// animatic: this, then its fingerprint.
const ANIMATIC_ITEM: &str = "animatic ";
/// The tip of the reference menu's row taking the animatic's frame while
/// the cell has no Motion reference, pending the owner's word.
pub const NO_ANIMATIC: &str = "Add the shot's animatic as a Motion reference first.";
/// The video files a take can be.
const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mov", "m4v", "mkv", "webm", "avi"];

/// One tab: a gallery of takes, kept in one file.
pub struct Tab {
    pub id: String,
    pub label: String,
    pub path: Option<PathBuf>,
    pub gallery: Gallery,
    /// The frame shown, from 0; a shorter take shows its last.
    pub frame: u32,
    /// None fits the takes in the area; otherwise 1 is 100%.
    pub zoom: Option<f64>,
    /// The point of the shown take kept in the middle while zoomed, from 0
    /// to 1 across and down.
    pub focus: [f64; 2],
    pub dirty: bool,
    /// The host's camera is put on the image, origin [0, 0] and zoom 1,
    /// with the next view: the first, and after the area changes. With
    /// the camera the workspace's, nothing else moves it, and a view with
    /// no camera would leave the host fitting the image with its margin.
    pub frame_camera: bool,
    /// Inside a project: when its change was first seen unwritten, and its
    /// last failed write, why and when.
    unwritten_since: Option<Instant>,
    write_failed: Option<(String, Instant)>,
    /// The image last written for the host.
    drawn: Option<Drawn>,
    /// The focus as the Move tool's drag began.
    drag: Option<[f64; 2]>,
    /// The rounds its cells made, as the host last sent them, and the take
    /// shown, if one is. Never kept in its file.
    pub rounds: Rounds,
    /// What the host was last told of its cells and of its file's items.
    cells_sent: Option<(Vec<protocol::Cell>, Option<String>)>,
    items_sent: Option<(String, Vec<protocol::Item>)>,
    /// The artifact each cell's step publishes, by the cell's id, as the
    /// host says it, while the program runs; never in the file.
    pub artifacts: HashMap<String, String>,
    /// The marked frames its next rounds sent, by item id, with their
    /// fingerprints, while the program runs.
    marked: Vec<(String, String)>,
    /// The Start frames taken from the animatic, by item id, with their
    /// fingerprints, while the program runs.
    frames_taken: Vec<(String, String)>,
    /// The pre-vis check: a round's take shown over its animatic. For the
    /// tab while the program runs, never kept.
    pub over_animatic: bool,
    /// The Layers page's row chosen, the take's, 0, or a sound layer's id,
    /// while the program runs.
    pub sound: Option<u64>,
    /// The take's sound and the sound layers as a held slider found them,
    /// until it is let go and the drag becomes one step.
    mix_before: Option<(crate::gallery::TakeSound, Vec<crate::gallery::SoundLayer>)>,
}

/// An image written for the host: what it shows, how many of its frames
/// were ready, and its revision.
struct Drawn {
    scene: Scene,
    held: usize,
    revision: u64,
}

impl Tab {
    pub fn look(&self) -> Look {
        Look { frame: self.frame, zoom: self.zoom, focus: self.focus }
    }

    /// The frame shown, within the shown take.
    pub fn frame_shown(&self) -> u32 {
        let shown = self.round_view();
        self.frame.min(shown.as_ref().unwrap_or(&self.gallery).take().map_or(0, |take| take.last_frame()))
    }

    /// A take of a round shown in the working area, as a gallery for the
    /// view to draw as any: that one take; or it beside another take, of a
    /// round or of the gallery, side by side; or its round's takes as frame
    /// strips. None while the gallery's own takes are shown. It is never
    /// the tab's gallery, so nothing of it is a step or kept.
    pub fn round_view(&self) -> Option<Gallery> {
        let rounds = &self.rounds;
        let tile = rounds.tiles.get(rounds.shown?)?;
        let shown = self.round_take(tile)?;
        let own = |uid: u64| self.gallery.takes.iter().find(|take| take.uid == uid).map(|take| (take.name.clone(), take.link.clone(), take.file.clone(), take.info));
        let pair = match &rounds.with {
            With::Alone => return Some(Gallery::new(vec![shown], 0)),
            With::Strips => {
                let tiles = rounds.round_tiles(tile);
                let at = tiles.iter().position(|index| Some(*index) == rounds.shown).unwrap_or(0);
                let mut gallery = Gallery::new(tiles.iter().filter_map(|index| self.round_take(&rounds.tiles[*index])).collect(), at);
                gallery.mode = Mode::Strips;
                return Some(gallery);
            }
            With::Tile(path) => [Some(shown), rounds.tile_of(path).and_then(|other| self.round_take(&rounds.tiles[other]))],
            With::Own { uid, first: false } => [Some(shown), own(*uid)],
            With::Own { uid, first: true } => [own(*uid), Some(shown)],
        };
        let takes: Vec<_> = pair.into_iter().flatten().collect();
        let mut gallery = Gallery::new(takes, 0);
        if let Some(other) = gallery.uid_at(1) {
            gallery.mode = Mode::Side { other };
        }
        Some(gallery)
    }

    /// A round's take as the round view holds it: its caption, its link,
    /// known by its sha256 as a gallery take's is, so its sound and notes
    /// are found, its file, and what it is.
    fn round_take(&self, tile: &crate::rounds::Tile) -> Option<(String, Link, PathBuf, Option<media::Info>)> {
        let file = tile.file.as_ref()?;
        let read = self.rounds.read(&file.path)?;
        let link = Link { path: file.path.clone(), sha256: read.sha256.clone() };
        Some((self.take_caption(tile), link, read.file.clone(), read.info))
    }

    /// A round's take's caption: a finishing round's label, "Take 1 · 60
    /// fps · 4K, from Take 7", or else "Take N · <model name>".
    pub fn take_caption(&self, tile: &crate::rounds::Tile) -> String {
        if let Some(label) = self.finishing_label(tile) {
            return label;
        }
        let model = self.rounds.round_of(tile).map_or("", crate::rounds::model_name);
        if model.is_empty() { format!("Take {}", tile.take + 1) } else { format!("Take {} · {model}", tile.take + 1) }
    }

    /// A finishing round's take's label, when its round is a finishing
    /// endpoint's and its one reference a take of this tab's rounds: from
    /// its cell's settings while the cell still has them, or else the
    /// request's, and from its source, whose own finishing part it keeps.
    pub fn finishing_label(&self, tile: &crate::rounds::Tile) -> Option<String> {
        let rounds = &self.rounds;
        let round = rounds.round_of(tile)?;
        let source = crate::finishing::source(round)?;
        let source_tile = &rounds.tiles[rounds.tile_of(source)?];
        let settings_of = |cell: &str| self.gallery.cells.iter().find(|own| own.id == cell).and_then(|own| own.settings.as_ref());
        let inherited = rounds
            .round_of(source_tile)
            .filter(|before| crate::finishing::source(before).is_some())
            .map(|before| crate::finishing::own_part(before, None, settings_of(&source_tile.cell)))
            .unwrap_or_default();
        let width = rounds.read(source).and_then(|read| read.info).map(|info| info.width);
        Some(crate::finishing::label(round, tile.take, source_tile.take, &inherited, width, settings_of(&tile.cell)))
    }

    /// The words over a round shown whole as frame strips: its line,
    /// "Round N · <model name> · N takes", and each reference by its order
    /// and role, as the Image Generator's round grid and references sheet
    /// write them.
    pub fn round_sheet(&self) -> Option<view::Sheet> {
        if self.rounds.with != With::Strips {
            return None;
        }
        let tile = self.rounds.tiles.get(self.rounds.shown?)?;
        let round = self.rounds.round_of(tile)?;
        let count = self.rounds.round_tiles(tile).len();
        let title = format!("Round {} · {} · {}", round.number, crate::rounds::model_name(round), layers::words::takes_word(count));
        let references =
            round.references.iter().enumerate().map(|(index, reference)| layers::words::sheet_label(index, reference, &crate::rounds::ROLES)).collect();
        Some(view::Sheet { title, references })
    }
}

/// What a file dialog's answer is for.
#[derive(Debug, Clone)]
enum Purpose {
    Open,
    /// The filmstrip's open row: takes added to this tab.
    AddTake { tab: String },
    /// Open sound as layer…: a sound layer added to this tab.
    AddSound { tab: String },
    Save { tab: String, then_quit: bool },
}

pub struct Session {
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_tab: u64,
    pub project: Option<PathBuf>,
    /// The host says the project is a Skills project: videos are kept in
    /// its `work/` folder.
    pub skills: bool,
    /// Whether the properties panel, with the Prompt page, is open.
    pub panel_open: bool,
    /// Where images and thumbnails for the host are written, and whether it
    /// is a folder of this session's own in memory, which stays when a
    /// project is named.
    cache: PathBuf,
    in_memory: bool,
    viewport: [f64; 2],
    file: Option<(String, Purpose)>,
    next_file: u64,
    /// Every image and thumbnail written gets the next revision, starting
    /// from the time the program started, so one of an earlier run is never
    /// taken for a current one.
    next_revision: u64,
    decoders: Decoders,
    /// Each file's filmstrip thumbnail, once written, and the files whose
    /// thumbnail could not be made.
    pub films: HashMap<PathBuf, Thumbnail>,
    films_failed: HashSet<PathBuf>,
    next_film: u64,
    /// Playing the take shown with its sound: the Player, the sounds
    /// decoded, and the waveforms drawn.
    playback: play::Playback,
    /// The toolbar's tool and the notes tool's settings, a pin's words
    /// being typed, and a mark being drawn.
    pub tools: Tools,
    typing: Option<notes::Typing>,
    marking: Option<notes::Marking>,
    /// What each animatic's file is, read once.
    animatics: HashMap<PathBuf, Option<media::Info>>,
    /// The panel's page shown.
    pub page: Page,
    /// A Deliver under way: its sound layers being filed, its mix being
    /// made, or the host's card asked.
    delivery: Option<deliver::Delivery>,
    /// Tab ids still to ask about while quitting, after the one asked.
    quit_rest: Vec<String>,
    quitting: bool,
    /// A close of one tab is asking, rather than the window's quit.
    closing: bool,
}

impl Session {
    /// Launched: one new, empty video, already declared.
    pub fn launch() -> Self {
        let mut session = Self::empty();
        let label = session.new_label();
        session.push_tab(Gallery::default(), None, Some(label));
        session
    }

    /// Started with `--restore`: no tab until the host sends `open`.
    pub fn restored() -> Self {
        Self::empty()
    }

    fn empty() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64 * 1000)
            .unwrap_or(0);
        let memory = memory_folder();
        Self {
            tabs: Vec::new(),
            active: 0,
            next_tab: 1,
            project: None,
            skills: false,
            panel_open: true,
            in_memory: memory.is_some(),
            cache: memory.unwrap_or_else(scratch_folder),
            viewport: FIRST_VIEWPORT,
            file: None,
            next_file: 1,
            next_revision: seed,
            decoders: Decoders::new(),
            films: HashMap::new(),
            films_failed: HashSet::new(),
            next_film: 1,
            playback: play::Playback::default(),
            tools: Tools::default(),
            typing: None,
            animatics: HashMap::new(),
            page: Page::default(),
            delivery: None,
            marking: None,
            quit_rest: Vec::new(),
            quitting: false,
            closing: false,
        }
    }

    pub fn greeting(&mut self) -> Vec<AppMessage> {
        let mut messages = vec![AppMessage::Hello { protocol: protocol::PROTOCOL_VERSION }];
        if let Some(tab) = self.tabs.first() {
            messages.push(AppMessage::Tab { id: tab.id.clone(), label: tab.label.clone() });
            messages.push(AppMessage::Contents { tab: tab.id.clone(), kind: ContentsKind::Raster });
            messages.push(AppMessage::Panel { tab: tab.id.clone(), side: PanelSide::Right });
        }
        messages.push(AppMessage::Tools { groups: crate::assist::tool_groups() });
        // The host asks for a video reference's preview here.
        messages.push(AppMessage::Edits { media_types: crate::preview::VIDEO_TYPES.iter().map(|kind| kind.to_string()).collect() });
        // It previews sounds too, which it never edits.
        messages.push(AppMessage::Previews { media_types: crate::preview::previewed_types() });
        messages.push(AppMessage::End);
        if !self.tabs.is_empty() {
            messages.extend(self.emit());
        }
        messages
    }

    /// One message from the host, and every reply.
    pub fn handle(&mut self, message: HostMessage) -> Vec<AppMessage> {
        let mut out = Vec::new();
        // A pin's words being typed stay open through the keyboard, the
        // notes tool's own clicks and settings, and the host's own
        // messages; anything else ends the typing first, which makes its
        // step.
        let typing = match &message {
            HostMessage::RasterText { .. }
            | HostMessage::Camera { .. }
            | HostMessage::Viewport { .. }
            | HostMessage::Ping
            | HostMessage::Keep { .. }
            | HostMessage::Claim { .. }
            | HostMessage::Render { .. }
            | HostMessage::Preview { .. }
            | HostMessage::Shown { .. }
            | HostMessage::Answer { .. } => true,
            HostMessage::RasterGesture { gesture: RasterGesture::Click { .. } | RasterGesture::Rect { .. } | RasterGesture::TextBox { .. }, .. } => {
                self.tools.tool == Tool::Notes
            }
            HostMessage::Command { id, .. } => matches!(id.as_str(), "notes_colour" | "toggle_panel" | "panel_page"),
            _ => false,
        };
        if !typing && self.end_typing() && !matches!(message, HostMessage::Shutdown) {
            out.extend(self.raster());
        }
        match message {
            HostMessage::Shutdown => {}
            HostMessage::Ping => out.push(AppMessage::Pong),
            HostMessage::Quit => {
                if self.project.is_some() {
                    // Everything is written first, so the quit asks nothing.
                    for index in 0..self.tabs.len() {
                        let _ = self.write_into_project(index);
                    }
                }
                self.note_quit(&mut out);
            }
            HostMessage::Project { folder, skills } => {
                self.skills = skills;
                let folder = PathBuf::from(folder);
                let folder = folder.canonicalize().unwrap_or(folder);
                if !self.in_memory {
                    if self.project.is_none() {
                        let _ = std::fs::remove_dir_all(&self.cache);
                    }
                    self.cache = folder.join(protocol::PROJECT_DIR).join("cache").join("video-generator");
                    // Everything is written again where the host now looks.
                    for tab in &mut self.tabs {
                        tab.drawn = None;
                    }
                    self.films.clear();
                }
                self.project = Some(folder);
                out.extend(self.emit());
                for index in 0..self.tabs.len() {
                    self.restore_items(index);
                }
                out.extend(self.items_due());
            }
            // A video's file from outside the project opens nothing.
            HostMessage::Open { path: Some(path) } if files::is_vgx(Path::new(&path)) && !self.in_project(Path::new(&path)) => {}
            HostMessage::Open { path } => self.open_path(path.map(PathBuf::from), &mut out),
            // As `edits` declared: a new video called `name`, one take
            // linking the file, never the file itself.
            HostMessage::OpenAsNew { path, name, .. } => self.open_as_new(Path::new(&path), &name, &mut out),
            HostMessage::Command { id, tab, arg, viewport, .. } => {
                self.set_viewport(viewport);
                if let Some(tab) = &tab {
                    self.activate(tab);
                }
                self.command(&id, tab.as_deref(), arg.unwrap_or(Value::Null), &mut out);
            }
            // The Move tool's drag pans a zoomed view: the focus moves with
            // the pointer from where the drag began.
            HostMessage::RasterGesture { tab, gesture: RasterGesture::DragPart { from, to } } => {
                self.activate(&tab);
                if self.pan(from, to) {
                    out.extend(self.raster());
                }
            }
            HostMessage::RasterGesture { tab, gesture: RasterGesture::Drag { from, to } } => {
                self.activate(&tab);
                let moved = self.pan(from, to);
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.drag = None;
                }
                if moved {
                    out.extend(self.raster());
                }
            }
            HostMessage::RasterGesture { tab, gesture: RasterGesture::StrokeCancel } => {
                self.activate(&tab);
                let mut redraw = self.mark_cancel();
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && let Some(start) = tab.drag.take()
                {
                    tab.focus = start;
                    redraw = true;
                }
                if redraw {
                    out.extend(self.raster());
                }
            }
            // The notes tool: Pin's click and drag, its words typed, and
            // Draw's mark, part by part.
            HostMessage::RasterGesture { tab, gesture: RasterGesture::Click { at } } => {
                self.activate(&tab);
                if self.tools.tool == Tool::Notes && !self.tools.draw {
                    self.note_click(at);
                    out.extend(self.emit());
                }
            }
            HostMessage::RasterGesture { tab, gesture: RasterGesture::Rect { from, to } } => {
                self.activate(&tab);
                if self.tools.tool == Tool::Notes && !self.tools.draw {
                    self.note_drag(from, to);
                    out.extend(self.emit());
                }
            }
            HostMessage::RasterText { tab, input } => {
                self.activate(&tab);
                self.note_text(&input);
                out.extend(self.emit());
            }
            HostMessage::RasterGesture { tab, gesture: RasterGesture::StrokePart { from, points } } => {
                self.activate(&tab);
                if self.tools.tool == Tool::Notes && self.tools.draw {
                    self.mark_part(from, &points);
                    out.extend(self.raster());
                }
            }
            HostMessage::RasterGesture { tab, gesture: RasterGesture::Stroke { points } } => {
                self.activate(&tab);
                if self.tools.tool == Tool::Notes && self.tools.draw {
                    self.mark_done(&points);
                    out.extend(self.emit());
                }
            }
            // The view is this workspace's own: the wheel, a pan, and the
            // magnifier come as `view` gestures, and the zoom menu and the
            // drag choose what it shows, decoded at that size. The host's
            // camera, which it never moves, is not this workspace's.
            HostMessage::RasterGesture { tab, gesture: RasterGesture::View { zoom_by, pan_by, at } } => {
                self.activate(&tab);
                if self.view_by(zoom_by, pan_by, at) {
                    out.extend(self.emit());
                }
            }
            HostMessage::Camera { .. } => {}
            HostMessage::Viewport { viewport } => {
                if self.set_viewport(viewport) {
                    out.extend(self.raster());
                }
            }
            // A frame asked for by the host.
            HostMessage::Seek { tab, frame, .. } => {
                self.activate(&tab);
                self.set_frame(|_, _| i64::from(frame));
                out.extend(self.emit());
            }
            // The transport row's scrub: a time, played as that frame's
            // slice of sound.
            HostMessage::TransportTime { tab, time } => {
                self.activate(&tab);
                self.transport_time(time);
                out.extend(self.emit());
            }
            // The host drew a revision: while playing, it keeps the
            // Player's lead and paces the frames.
            HostMessage::Shown { revision, at, .. } => self.frame_shown(revision, at),
            HostMessage::Answer { id, button } => {
                if id == "keep-video" {
                    self.answer(&button, &mut out);
                }
            }
            HostMessage::FileResult { id, path } => self.file_result(&id, path, &mut out),
            HostMessage::Invoke { call, tab, tool, input, .. } => out.extend(self.invoke(call, &tab, &tool, input)),
            HostMessage::Keep { tab } => {
                let kept = match self.tabs.iter().position(|own| own.id == tab) {
                    Some(index) if self.project.is_some() => self.write_into_project(index),
                    Some(index) => Ok(self.tabs[index].path.clone()),
                    None => Ok(None),
                };
                out.extend(self.declared());
                out.push(match kept {
                    Ok(path) => AppMessage::Kept { tab, path: path.map(|path| path.display().to_string()), error: None },
                    Err(error) => AppMessage::Kept { tab, path: None, error: Some(error) },
                });
                // A file written for the first time has items to declare.
                out.extend(self.items_due());
            }
            HostMessage::Claim { id, path } => {
                let path = Path::new(&path);
                out.push(AppMessage::Claimed { id, yours: files::is_vgx(path) && self.in_project(path) });
            }
            HostMessage::Render { id, item, media_type, .. } => out.push(self.render(id, &item, &media_type)),
            HostMessage::Takes { cell, rounds } => {
                if let Some(id) = cell.item {
                    self.takes(&id, rounds, &mut out);
                }
            }
            HostMessage::EditCell { cell, input, settings, endpoint, model, .. } => {
                self.edit_cell(&cell, input, settings, endpoint, model, &mut out);
            }
            HostMessage::InsertCell { tab, index, cell } => self.insert_cell(&tab, index, cell, &mut out),
            HostMessage::SelectCell { tab, cell } => self.select_cell(&tab, cell, &mut out),
            HostMessage::CellBecameStep { tab, cell, step, artifact } => self.cell_became_step(&tab, &cell, &step, artifact, &mut out),
            HostMessage::CellResult { cell, output, .. } => self.cell_result(&cell, &output, &mut out),
            HostMessage::Preview { id, path, media_type } => {
                let root = self.project.clone().unwrap_or_default();
                out.push(crate::preview::preview(id, &path, &media_type, &root, &self.cache, &mut self.next_revision));
            }
            // The Properties page's Gain and Starts at.
            HostMessage::Property { prop, value, preview } => {
                self.mix_property(&prop, &value, preview);
                if !preview {
                    out.extend(self.emit());
                }
            }
            // Deliver: the sound layers filed, the mix made, and the host's
            // card answered.
            HostMessage::Filed { id, file, error } => self.filed(&id, &file, error, &mut out),
            HostMessage::SkillScriptRan { id, output, error } => self.mix_ran(&id, &output, error, &mut out),
            HostMessage::Delivered { tab, file, error } => self.delivered(&tab, &file, error, &mut out),
            _ => {}
        }
        out
    }

    /// Takes in the frames decoded since the last call, writes the
    /// thumbnails that came, and sends the view when anything came.
    pub fn poll(&mut self) -> Vec<AppMessage> {
        // While playing, the Player's clock names the frame: a new one is
        // asked of the seekers, and drawn when it comes.
        let heard = self.poll_sounds();
        let ticked = self.play_tick();
        if !self.decoders.poll() && !ticked && !heard {
            return Vec::new();
        }
        for (key, result) in std::mem::take(&mut self.decoders.thumbs) {
            match result {
                Ok(frame) => {
                    let name = format!("film-{}.rgba", self.next_film);
                    self.next_film += 1;
                    match media::write_frame(&self.cache, &name, &frame.data, (frame.width, frame.height)) {
                        Ok(path) => {
                            self.next_revision += 1;
                            let thumb = Thumbnail { path: path.display().to_string(), width: frame.width, height: frame.height, revision: self.next_revision };
                            self.films.insert(key.file, thumb);
                        }
                        Err(_) => {
                            self.films_failed.insert(key.file);
                        }
                    }
                }
                Err(_) => {
                    self.films_failed.insert(key.file);
                }
            }
        }
        let mut out = self.raster();
        // A sound layer's waveform, once decoded, is its row's thumbnail.
        if heard {
            out.extend(self.declared());
        }
        out
    }

    /// Whether frames asked for are still to come, so the loop wakes soon.
    pub fn waiting(&self) -> bool {
        self.decoders.waiting() || self.is_playing() || self.playback.sounds.waiting()
    }

    /// The loop's wake with no message: writes what is due inside a project,
    /// and sends what that changed, such as a tab's new name.
    pub fn tick(&mut self) -> Vec<AppMessage> {
        if self.project.is_none() {
            return Vec::new();
        }
        let now = Instant::now();
        let mut wrote = false;
        for index in 0..self.tabs.len() {
            let tab = &mut self.tabs[index];
            if !tab.dirty {
                tab.unwritten_since = None;
                continue;
            }
            let since = *tab.unwritten_since.get_or_insert(now);
            if now.duration_since(since) < WRITE_AFTER
                || tab.write_failed.as_ref().is_some_and(|(_, failed)| now.duration_since(*failed) < RETRY_AFTER)
            {
                continue;
            }
            let _ = self.write_into_project(index);
            wrote = true;
        }
        if !wrote {
            return Vec::new();
        }
        // A file written for the first time has items to declare.
        let mut out = self.declared();
        out.extend(self.items_due());
        out
    }

    /// Inside a project, writes every tab not yet written, as when the host
    /// has gone.
    pub fn write_all(&mut self) {
        if self.project.is_some() {
            for index in 0..self.tabs.len() {
                let _ = self.write_into_project(index);
            }
        }
    }

    fn in_project(&self, path: &Path) -> bool {
        let Some(project) = &self.project else {
            return true;
        };
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        path.starts_with(project)
    }

    pub fn tab_index(&self, tab: &str) -> Option<usize> {
        self.tabs.iter().position(|own| own.id == tab)
    }

    fn activate(&mut self, tab: &str) {
        if let Some(index) = self.tab_index(tab) {
            if index != self.active {
                self.stop_playing();
            }
            self.active = index;
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// The area's size changed: every tab's image is made again at it.
    /// Returns whether it changed.
    fn set_viewport(&mut self, viewport: [f64; 2]) -> bool {
        if viewport[0] <= 0.0 || viewport[1] <= 0.0 || viewport == self.viewport {
            return false;
        }
        self.viewport = viewport;
        for tab in &mut self.tabs {
            tab.frame_camera = true;
        }
        true
    }

    /// The image's size: the working area's, even.
    fn image_size(&self) -> (u32, u32) {
        even(((self.viewport[0].round() as u32).max(64), (self.viewport[1].round() as u32).max(64)))
    }

    /// A new tab's name: "Untitled Video", or the first free name after it
    /// among the open tabs and the `.vgx` files in the project's folder.
    fn new_label(&self) -> String {
        protocol::free_name(&protocol::untitled(ITEM), |name| {
            self.tabs.iter().any(|tab| tab.label == name)
                || self.project.as_ref().is_some_and(|folder| folder.join(format!("{name}.vgx")).exists())
                || self.work_folder().is_some_and(|folder| folder.join(format!("{name}.vgx")).exists())
        })
    }

    /// Where a tab never written goes: the project's folder, or in a
    /// Skills project its `work/` folder, as the contract with
    /// Spumcake-Skills keeps workspace files.
    fn work_folder(&self) -> Option<PathBuf> {
        let project = self.project.as_ref()?;
        Some(if self.skills { project.join("work") } else { project.clone() })
    }

    fn push_tab(&mut self, gallery: Gallery, path: Option<PathBuf>, label: Option<String>) -> usize {
        let id = self.next_tab.to_string();
        self.next_tab += 1;
        let label = label.or_else(|| path.as_deref().map(file_title)).unwrap_or_else(|| self.new_label());
        self.tabs.push(Tab {
            id,
            label,
            path,
            gallery,
            frame: 0,
            zoom: None,
            focus: [0.5, 0.5],
            dirty: false,
            frame_camera: true,
            unwritten_since: None,
            write_failed: None,
            drawn: None,
            drag: None,
            rounds: Rounds::default(),
            cells_sent: None,
            items_sent: None,
            artifacts: HashMap::new(),
            marked: Vec::new(),
            frames_taken: Vec::new(),
            over_animatic: false,
            sound: None,
            mix_before: None,
        });
        self.active = self.tabs.len() - 1;
        self.active
    }

    /// A tab made after the greeting declares its contents, then
    /// everything else.
    fn declare_new(&mut self, out: &mut Vec<AppMessage>) {
        let tab = self.tabs[self.active].id.clone();
        out.push(AppMessage::Contents { tab: tab.clone(), kind: ContentsKind::Raster });
        out.push(AppMessage::Panel { tab, side: PanelSide::Right });
        out.extend(self.emit());
    }

    /// Runs `change` on the active tab's gallery, and marks the tab changed
    /// when it was: by a step, or, inside a project, by the take shown.
    fn change<T>(&mut self, change: impl FnOnce(&mut Gallery) -> T) -> Option<T> {
        // Any change to the gallery pauses first.
        self.stop_playing();
        let in_project = self.project.is_some();
        let tab = self.tabs.get_mut(self.active)?;
        // Whatever the gallery does shows a take of its own.
        tab.rounds.show(None);
        let (edits, shown) = (tab.gallery.edits, tab.gallery.take().map(|take| take.uid));
        let result = change(&mut tab.gallery);
        if tab.gallery.edits != edits || (in_project && tab.gallery.take().map(|take| take.uid) != shown) {
            tab.dirty = true;
        }
        Some(result)
    }

    /// A Ctrl- or Shift-click on the strip's tile `index`, counting the
    /// rounds' tiles first, when a round's take is the one shown or the one
    /// clicked: the take clicked beside the one shown, as vg-takes-compare
    /// shows, a round's or the gallery's, from one time. Two of the
    /// gallery's own takes are its own view, a step, as before. Returns
    /// whether it was taken.
    fn beside(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let tiles = tab.rounds.tiles.len();
        let with = match (tab.rounds.shown, index < tiles) {
            (Some(shown), true) if shown == index => return true,
            (Some(_), true) => tab.rounds.tiles[index].path().map(|path| (None, With::Tile(path.to_string()))),
            (Some(_), false) => tab.gallery.uid_at(index - tiles).map(|uid| (None, With::Own { uid, first: false })),
            // A gallery take shown, and a round's take clicked: the
            // gallery's on the left, as the one shown.
            (None, true) => tab
                .gallery
                .take()
                .filter(|_| tab.rounds.tiles[index].file.is_some())
                .map(|take| (Some(index), With::Own { uid: take.uid, first: true })),
            (None, false) => return false,
        };
        let Some((show, with)) = with else {
            return true;
        };
        self.stop_playing();
        let tab = &mut self.tabs[self.active];
        if let Some(show) = show {
            tab.rounds.shown = Some(show);
        }
        tab.rounds.with = with;
        true
    }

    /// Undo, or redo with `redo`: a step of a take's notes keeps the take
    /// shown, a round's take among them; any other shows a take of the
    /// gallery's own, as `change` does.
    fn step_history(&mut self, redo: bool) {
        let notes = self.active_tab().is_some_and(|tab| tab.gallery.steps_notes(redo));
        if !notes {
            if redo {
                self.change(Gallery::redo);
            } else {
                self.change(Gallery::undo);
            }
            return;
        }
        self.stop_playing();
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let stepped = if redo { tab.gallery.redo() } else { tab.gallery.undo() };
            tab.dirty |= stepped;
        }
    }

    /// Sets the frame from the frame shown and the shown take's last frame
    /// and frames per second, within the take. Inside a project, the frame
    /// is kept in the file.
    fn set_frame(&mut self, to: impl FnOnce(u32, (u32, f64)) -> i64) {
        // The keys, `seek`, a step, and a scrub pause first.
        self.stop_playing();
        let in_project = self.project.is_some();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let shown = tab.round_view();
        let take = shown.as_ref().unwrap_or(&tab.gallery).take();
        let last = take.map_or(0, |take| take.last_frame());
        let fps = take.and_then(|take| take.info).map_or(24.0, |info| info.fps());
        let frame = to(tab.frame_shown(), (last, fps)).clamp(0, i64::from(last)) as u32;
        if frame != tab.frame {
            tab.frame = frame;
            tab.dirty |= in_project;
        }
    }

    /// Moves a zoomed view's focus with the Move tool's drag. Returns
    /// whether it moved.
    fn pan(&mut self, from: [f64; 2], to: [f64; 2]) -> bool {
        let size = self.image_size();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        if tab.zoom.is_none() {
            return false;
        }
        let shown = tab.round_view();
        let scene = view::scene(shown.as_ref().unwrap_or(&tab.gallery), size, tab.look());
        let Some(place) = scene.places.first() else {
            return false;
        };
        let start = *tab.drag.get_or_insert(tab.focus);
        let focus = [
            (start[0] - (to[0] - from[0]) / f64::from(place.rect.w.max(1))).clamp(0.0, 1.0),
            (start[1] - (to[1] - from[1]) / f64::from(place.rect.h.max(1))).clamp(0.0, 1.0),
        ];
        let moved = focus != tab.focus;
        tab.focus = focus;
        moved
    }

    /// A `view` gesture: the shown take zoomed by `zoom_by` about `at`, a
    /// point of the image, so the take's point there stays there, and moved
    /// by `pan_by`, image pixels right and down. A fitted take starts from
    /// the zoom it is fitted at; frame strips, and a pan of a fitted take,
    /// change nothing. Each frame is then decoded at the size it is drawn,
    /// as with the zoom menu. Returns whether the view changed.
    fn view_by(&mut self, zoom_by: f64, pan_by: [f64; 2], at: [f64; 2]) -> bool {
        let size = self.image_size();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        if !zoom_by.is_finite() || zoom_by <= 0.0 || !at.iter().chain(&pan_by).all(|value| value.is_finite()) {
            return false;
        }
        let shown = tab.round_view();
        let gallery = shown.as_ref().unwrap_or(&tab.gallery);
        if gallery.view() == Mode::Strips || (tab.zoom.is_none() && zoom_by == 1.0) {
            return false;
        }
        let scene = view::scene(gallery, size, tab.look());
        let (Some(place), Some(info)) = (scene.places.first(), gallery.take().and_then(|take| take.info)) else {
            return false;
        };
        let (rect, cell) = (place.rect, place.clip);
        let drawn = [f64::from(rect.w.max(1)), f64::from(rect.h.max(1))];
        // The zoom a fitted take is drawn at, in the zoom menu's terms: side
        // by side, the wider take's scale.
        let widest = match gallery.view() {
            Mode::Side { .. } => [gallery.take(), gallery.other().map(|other| &gallery.takes[other])]
                .into_iter()
                .flatten()
                .filter_map(|take| take.info.map(|info| info.width))
                .max()
                .unwrap_or(info.width),
            _ => info.width,
        };
        let now = tab.zoom.unwrap_or(drawn[0] / f64::from(widest.max(1)));
        let zoom = (now * zoom_by).clamp(0.05, 8.0);
        let factor = zoom / now;
        // The take's point under `at`, from 0 to 1, is kept under it: the
        // point at the cell's middle is worked out from it, then moved by
        // the pan.
        let middle = [f64::from(cell.x) + f64::from(cell.w) / 2.0, f64::from(cell.y) + f64::from(cell.h) / 2.0];
        let corner = [f64::from(rect.x), f64::from(rect.y)];
        let mut focus = [0.0; 2];
        for axis in 0..2 {
            let under = (at[axis] - corner[axis]) / drawn[axis];
            let after = drawn[axis] * factor;
            focus[axis] = (under + (middle[axis] - at[axis] - pan_by[axis]) / after).clamp(0.0, 1.0);
        }
        let changed = tab.zoom != Some(zoom) || tab.focus != focus;
        tab.zoom = Some(zoom);
        tab.focus = focus;
        changed
    }

    fn command(&mut self, id: &str, tab: Option<&str>, arg: Value, out: &mut Vec<AppMessage>) {
        let number = |key: &str| arg.get(key).and_then(Value::as_f64);
        let position = |key: &str| arg.get(key).and_then(Value::as_f64).map(|value| value.max(0.0) as usize);
        match id {
            "new_video" => {
                self.push_tab(Gallery::default(), None, None);
                self.declare_new(out);
                return;
            }
            "open" => {
                self.ask_file(FileKind::Open, None, Purpose::Open, open_filter(), out);
                return;
            }
            "save" | "save_as" if self.project.is_some() => {
                let _ = self.write_into_project(self.active);
            }
            "save" | "save_as" => {
                let Some(tab) = self.active_tab() else {
                    return;
                };
                match (&tab.path, id) {
                    (Some(path), "save") => {
                        let path = path.clone();
                        self.write_at(self.active, &path, false, out);
                    }
                    _ => {
                        let purpose = Purpose::Save { tab: tab.id.clone(), then_quit: false };
                        let name = format!("{}.vgx", tab.label);
                        self.ask_file(FileKind::Save, Some(name), purpose, vgx_filter(), out);
                    }
                }
                return;
            }
            "undo" => self.step_history(false),
            "redo" => self.step_history(true),
            // The toolbar's tools, and the notes tool's settings row: Pin
            // or Draw, and the marks' colour.
            "tool" => {
                if let Some(tool) = arg.get("tool").and_then(Value::as_str).and_then(Tool::named) {
                    self.choose_tool(tool);
                }
            }
            // The Start frame row's menu: the animatic's frame at the
            // playhead, whichever reference's menu sent it.
            "frame_from_animatic" => {
                self.frame_from_animatic(out);
                return;
            }
            // Open in the Image Generator, sent with the reference's place.
            "frame_open_image" => {
                if let Some(reference) = number("value") {
                    self.frame_open_image(reference as usize, out);
                }
                return;
            }
            // The pre-vis check: the take shown over its animatic.
            "previs_toggle" => self.previs_toggle(),
            "notes_mode" => {
                if let Some(index) = number("value") {
                    self.tools.draw = index >= 1.0;
                    self.marking = None;
                }
            }
            "notes_colour" => {
                if let Some(rgb) = arg.get("value").and_then(rgb_of) {
                    self.tools.rgb = rgb;
                }
            }
            // A press on the strip; the gallery has one history, so the
            // keyboard's Undo is the same either way.
            "film_focus" => return,
            // The strip holds the rounds' tiles first, then the gallery's
            // takes. A click shows that take alone; with `extend`, as a
            // Ctrl- or Shift-click, a take of the gallery beside the take
            // shown.
            "film_select" => {
                let Some(index) = position("value") else {
                    return;
                };
                let tiles = self.active_tab().map_or(0, |tab| tab.rounds.tiles.len());
                let extend = arg.get("extend").and_then(Value::as_bool).unwrap_or(false);
                // With `extend`, a take beside the one shown, when a round's
                // take is either: a view only, never a step or kept.
                if extend && self.beside(index) {
                    out.extend(self.emit());
                    return;
                }
                if index < tiles {
                    self.select_take(index);
                    out.extend(self.emit());
                    return;
                }
                let index = index - tiles;
                self.change(|gallery| match gallery.uid_at(index) {
                    Some(other) if extend && index != gallery.shown => gallery.set_view(Mode::Side { other }, gallery.shown),
                    Some(_) if extend => Ok(()),
                    Some(_) => gallery.set_view(Mode::Single, index),
                    None => Ok(()),
                });
            }
            // A round's take keeps its round's place; a take moves among the
            // gallery's.
            "film_reorder" => {
                let tiles = self.active_tab().map_or(0, |tab| tab.rounds.tiles.len());
                if let (Some(from), Some(to)) = (position("from"), position("to"))
                    && from >= tiles
                {
                    self.change(|gallery| gallery.move_to(from - tiles, to.saturating_sub(tiles)));
                }
            }
            "toggle_panel" => self.panel_open = !self.panel_open,
            // Layers, Properties, and Prompt; Adjustments waits.
            "panel_page" => {
                self.page = match number("value").or_else(|| number("page")).map(|page| page as i64) {
                    Some(0) => Page::Layers,
                    Some(1) => Page::Properties,
                    Some(3) => Page::Prompt,
                    _ => self.page,
                };
            }
            // A held slider let go.
            "finish_edit" => self.finish_mix(),
            // Open sound as layer…, from the Layers page's + or the + tile.
            "open_sound" => {
                self.open_sound(out);
                return;
            }
            "sound_click" | "sound_mute" | "sound_rename" | "sound_delete" | "sound_reorder" | "sound_lock" => {
                self.mix_command(id, &arg);
            }
            // A finishing request, or a finished take beside its source.
            "take_retime" | "take_upscale" | "take_compare" | "take_deliver" => {
                let Some(path) = arg.get("take").and_then(Value::as_str).map(str::to_string) else {
                    return;
                };
                match id {
                    "take_retime" => self.finish_take(&path, crate::finishing::Finish::Retime, out),
                    "take_upscale" => self.finish_take(&path, crate::finishing::Finish::Upscale, out),
                    "take_compare" => {
                        self.compare_take(&path);
                        out.extend(self.emit());
                    }
                    _ => self.deliver(&path, out),
                }
                return;
            }
            // A notice's close.
            "notice_close" => {
                if let Some(tab) = self.active_tab() {
                    out.push(deliver::notices(&tab.id, None));
                }
                return;
            }
            // A take's menu.
            "take_next_round" | "take_ask" | "take_edit" | "take_report" => {
                let Some(path) = arg.get("take").and_then(Value::as_str).map(str::to_string) else {
                    return;
                };
                match id {
                    "take_next_round" => {
                        self.next_round(&path, out);
                        return;
                    }
                    "take_ask" => {
                        self.ask_about(&path, out);
                        return;
                    }
                    "take_report" => {
                        self.open_report(&path, out);
                        return;
                    }
                    _ => self.edit_take(&path, out),
                }
            }
            "film_open" => {
                let Some(tab) = self.active_tab() else {
                    return;
                };
                let purpose = Purpose::AddTake { tab: tab.id.clone() };
                self.ask_file(FileKind::Open, None, purpose, video_filter(), out);
                return;
            }
            "film_duplicate" => {
                if let Some(index) = position("take") {
                    self.change(|gallery| gallery.duplicate(index));
                }
            }
            "film_delete" => {
                if let Some(index) = position("take") {
                    self.change(|gallery| gallery.remove(index));
                }
            }
            // The thumbnail's Rename row: the name typed in place comes as
            // `value`. A host that cannot take a name sends none; the person
            // is told how a take is renamed instead.
            "film_rename" => {
                let Some(index) = position("take") else {
                    return;
                };
                match arg.get("value").and_then(Value::as_str) {
                    Some(name) => {
                        let name = name.to_string();
                        if let Some(Err(reason)) = self.change(|gallery| gallery.rename(index, &name)) {
                            out.push(notice("The take could not be renamed", &reason));
                        }
                    }
                    None => {
                        out.push(notice("Rename a take", "Ask the assistant to rename this take, and give it the new name."));
                        return;
                    }
                }
            }
            // The grid button: every take as frame strips, or the shown
            // take alone again.
            "view_strips" => {
                // A round's take shown: its round whole, or the take again,
                // a view only, never a step.
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && tab.rounds.shown.is_some()
                {
                    tab.rounds.with = if tab.rounds.with == With::Strips { With::Alone } else { With::Strips };
                    self.stop_playing();
                    out.extend(self.emit());
                    return;
                }
                self.change(|gallery| {
                    let mode = if gallery.view() == Mode::Strips { Mode::Single } else { Mode::Strips };
                    gallery.set_view(mode, gallery.shown)
                });
            }
            // The transport row: play or pause, with `value` saying which,
            // and a step of a frame back or forward. Neither changes the
            // gallery.
            "transport_play" => {
                let playing = arg.get("value").and_then(Value::as_bool).unwrap_or(!self.is_playing());
                self.transport_play(playing);
            }
            "transport_step" => {
                if let Some(by) = number("value") {
                    self.transport_step(by.round() as i64);
                }
            }
            "view_zoom" => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.zoom = number("zoom").filter(|zoom| zoom.is_finite() && *zoom > 0.0).map(|zoom| zoom.clamp(0.05, 8.0));
                }
            }
            // The keys: a step of frames or seconds, or the first or last.
            "frame" => {
                let step = number("step");
                let seconds = number("seconds");
                let to = arg.get("to").and_then(Value::as_str).map(str::to_string);
                self.set_frame(|frame, (last, fps)| match (to.as_deref(), step, seconds) {
                    (Some("first"), _, _) => 0,
                    (Some("last"), _, _) => i64::from(last),
                    (_, Some(step), _) => i64::from(frame) + step.round() as i64,
                    (_, _, Some(seconds)) => i64::from(frame) + (seconds * fps).round() as i64,
                    _ => i64::from(frame),
                });
            }
            "close_tab" => {
                self.close_tab(tab, out);
                return;
            }
            _ => {}
        }
        out.extend(self.emit());
    }

    fn ask_file(&mut self, kind: FileKind, name: Option<String>, purpose: Purpose, filter: FileFilter, out: &mut Vec<AppMessage>) {
        let id = format!("file-{}", self.next_file);
        self.next_file += 1;
        self.file = Some((id.clone(), purpose));
        out.push(AppMessage::File {
            id,
            kind,
            name,
            folder: self.project.as_ref().map(|folder| folder.display().to_string()),
            filter: Some(filter),
        });
    }

    fn file_result(&mut self, id: &str, path: Option<String>, out: &mut Vec<AppMessage>) {
        let Some((asked, purpose)) = self.file.take() else {
            return;
        };
        if asked != id {
            self.file = Some((asked, purpose));
            return;
        }
        let Some(path) = path.map(PathBuf::from) else {
            if matches!(purpose, Purpose::Save { then_quit: true, .. }) || self.quitting {
                self.cancel_quit(out);
            }
            return;
        };
        match purpose {
            Purpose::Open => {
                if files::is_vgx(&path) && !self.in_project(&path) {
                    out.push(notice("This video is not in the project", "Only videos inside the project's folder open here."));
                    return;
                }
                self.open_path(Some(path), out);
            }
            Purpose::AddTake { tab } => {
                let Some(index) = self.tab_index(&tab) else {
                    return;
                };
                self.active = index;
                match self.add_take(&path, None) {
                    Ok(_) => out.extend(self.emit()),
                    Err(reason) => out.push(notice("This video could not be opened", &reason)),
                }
            }
            Purpose::AddSound { tab } => {
                if let Some(index) = self.tab_index(&tab) {
                    self.active = index;
                }
                match self.add_sound(&tab, &path) {
                    Ok(()) => out.extend(self.emit()),
                    Err(reason) => out.push(notice("This sound could not be opened", &reason)),
                }
            }
            Purpose::Save { tab, then_quit } => {
                if let Some(index) = self.tab_index(&tab) {
                    self.write_at(index, &path, then_quit, out);
                }
            }
        }
    }

    /// Adds a take of the video file at `path` to the active tab, at the
    /// end, named `name` or after the file, and shows it. Returns its
    /// position.
    pub fn add_take(&mut self, path: &Path, name: Option<&str>) -> Result<usize, String> {
        let (link, file, info) = link_file(path, self.project.as_deref())?;
        self.decoders.forget(&file);
        self.films_failed.remove(&file);
        let title = name.map(str::trim).filter(|name| !name.is_empty()).map_or_else(|| file_title(&file), str::to_string);
        self.change(|gallery| gallery.add(&title, link, file, Some(info), None)).ok_or_else(|| String::from("There is no tab."))
    }

    /// Opens a file in a new tab: a `.vgx` file as itself, or a video file
    /// as a new video of one take. No path makes a new video.
    fn open_path(&mut self, path: Option<PathBuf>, out: &mut Vec<AppMessage>) {
        let Some(path) = path else {
            self.push_tab(Gallery::default(), None, None);
            self.declare_new(out);
            return;
        };
        if let Some(index) = self.tabs.iter().position(|tab| tab.path.as_deref() == Some(path.as_path())) {
            self.active = index;
            out.extend(self.emit());
            return;
        }
        if files::is_vgx(&path) {
            match files::read_vgx(&path) {
                Ok(read) => {
                    let beside = path.parent().map(Path::to_path_buf);
                    let takes = read
                        .takes
                        .into_iter()
                        .map(|(name, link)| {
                            let file = files::resolve(&link.path, self.project.as_deref(), beside.as_deref());
                            // A file gone or changed is shown empty, and its
                            // link kept as it was.
                            let info = media::Video::open(&file).ok().map(|video| video.info());
                            (name, link, file, info)
                        })
                        .collect();
                    let mut gallery = Gallery::new(takes, read.shown);
                    gallery.mode = match read.view {
                        ViewFile::Single => Mode::Single,
                        ViewFile::Strips => Mode::Strips,
                        ViewFile::Side { with } => gallery.uid_at(with).map_or(Mode::Single, |other| Mode::Side { other }),
                    };
                    gallery.cells = read.cells;
                    gallery.cell = read.cell;
                    gallery.notes = read.take_notes;
                    gallery.take_sound = read.take_sound;
                    gallery.sounds = read
                        .sounds
                        .into_iter()
                        .enumerate()
                        .map(|(index, sound)| crate::gallery::SoundLayer {
                            id: index as u64 + 1,
                            name: sound.name,
                            file: files::resolve(&sound.link.path, self.project.as_deref(), beside.as_deref()),
                            link: sound.link,
                            gain_db: sound.gain_db,
                            start: sound.start,
                            muted: sound.muted,
                        })
                        .collect();
                    let index = self.push_tab(gallery, Some(path), None);
                    self.tabs[index].frame = read.frame;
                    self.declare_new(out);
                    // What its drafts name of its own items, declared again.
                    self.restore_items(index);
                    out.extend(self.items_due());
                }
                Err(reason) => out.push(notice("This video could not be opened", &reason)),
            }
            return;
        }
        let title = file_title(&path);
        self.push_tab(Gallery::default(), None, Some(title));
        match self.add_take(&path, None) {
            Ok(_) => {
                // It is new work, so it is kept in the project as a video.
                self.tabs[self.active].dirty = true;
                self.declare_new(out);
            }
            Err(reason) => {
                self.tabs.pop();
                self.active = self.tabs.len().saturating_sub(1);
                out.push(notice("This video could not be opened", &reason));
            }
        }
    }

    /// Opens a video file as new work, as `open_as_new` asks: a new tab
    /// called `name`, made safe as a file's, or after the file, whose
    /// gallery is one take linking the file. Inside a project it is kept as
    /// a new `.vgx`. A relative path is read from the project.
    fn open_as_new(&mut self, path: &Path, name: &str, out: &mut Vec<AppMessage>) {
        let path = match (&self.project, path.is_relative()) {
            (Some(project), true) => project.join(path),
            _ => path.to_path_buf(),
        };
        let name = safe_name(name);
        let label = if name.is_empty() { file_title(&path) } else { name };
        self.push_tab(Gallery::default(), None, Some(label));
        match self.add_take(&path, None) {
            Ok(_) => {
                self.tabs[self.active].dirty = true;
                self.declare_new(out);
            }
            Err(reason) => {
                self.tabs.pop();
                self.active = self.tabs.len().saturating_sub(1);
                out.push(notice("This video could not be opened", &reason));
            }
        }
    }

    /// Inside a project: writes a tab into the project's folder, to its file
    /// or, for one never written, to a new file named after it that
    /// replaces nothing. Returns the file, or none for an untouched tab.
    fn write_into_project(&mut self, index: usize) -> Result<Option<PathBuf>, String> {
        let (Some(project), Some(folder)) = (self.project.clone(), self.work_folder()) else {
            return Err("No project folder.".into());
        };
        let Some(tab) = self.tabs.get(index) else {
            return Ok(None);
        };
        if !tab.dirty {
            return Ok(tab.path.clone());
        }
        let path = match &tab.path {
            Some(path) => path.clone(),
            None => match new_file(&self.tabs, index, &folder) {
                Some(path) => path,
                None => return Err("No free file name.".into()),
            },
        };
        let tab = &mut self.tabs[index];
        match files::write_vgx(&path, &tab.gallery, tab.frame_shown(), Some(&project)) {
            Ok(()) => {
                tab.label = file_title(&path);
                tab.path = Some(path.clone());
                tab.dirty = false;
                tab.unwritten_since = None;
                tab.write_failed = None;
                Ok(Some(path))
            }
            Err(reason) => {
                tab.write_failed = Some((reason.clone(), Instant::now()));
                Err(reason)
            }
        }
    }

    /// Outside a project: Save, or Save As, to `path`.
    fn write_at(&mut self, index: usize, path: &Path, then_quit: bool, out: &mut Vec<AppMessage>) {
        let project = self.project.clone();
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if let Err(reason) = files::write_vgx(path, &tab.gallery, tab.frame_shown(), project.as_deref()) {
            out.push(notice("This video could not be saved", &reason));
            if then_quit || self.quitting {
                self.cancel_quit(out);
            }
            return;
        }
        tab.path = Some(path.to_path_buf());
        tab.label = file_title(path);
        tab.dirty = false;
        out.extend(self.declared());
        if self.closing {
            self.closing = false;
            self.remove_tab(index, out);
        } else if then_quit {
            self.advance_quit(out);
        }
    }

    fn close_tab(&mut self, tab: Option<&str>, out: &mut Vec<AppMessage>) {
        let Some(index) = tab.and_then(|tab| self.tab_index(tab)).or((!self.tabs.is_empty()).then_some(self.active)) else {
            return;
        };
        if self.project.is_some() {
            // Written first, so closing asks nothing. A tab that could not
            // be written still asks.
            let _ = self.write_into_project(index);
        }
        if self.tabs[index].dirty {
            self.active = index;
            self.closing = true;
            self.quitting = false;
            self.quit_rest.clear();
            out.push(AppMessage::Ask(keep_ask()));
            return;
        }
        self.remove_tab(index, out);
    }

    /// Removes a tab and its images for the host, then sends what the host
    /// now shows. The last tab closed leaves a new video.
    fn remove_tab(&mut self, index: usize, out: &mut Vec<AppMessage>) {
        let tab = self.tabs.remove(index);
        let _ = std::fs::remove_file(self.cache.join(format!("{}-view.rgba", tab.id)));
        if self.tabs.is_empty() {
            self.push_tab(Gallery::default(), None, None);
            self.declare_new(out);
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        out.extend(self.emit());
    }

    fn note_quit(&mut self, out: &mut Vec<AppMessage>) {
        if self.quitting {
            return;
        }
        let dirty: Vec<String> = self.tabs.iter().filter(|tab| tab.dirty).map(|tab| tab.id.clone()).collect();
        let Some(first) = dirty.first() else {
            out.push(AppMessage::QuitReady);
            return;
        };
        self.quitting = true;
        self.activate(&first.clone());
        self.quit_rest = dirty[1..].to_vec();
        out.push(AppMessage::Ask(keep_ask()));
    }

    fn answer(&mut self, button: &str, out: &mut Vec<AppMessage>) {
        match button {
            "save" => {
                let Some(tab) = self.active_tab() else {
                    return;
                };
                match tab.path.clone() {
                    Some(path) => self.write_at(self.active, &path, self.quitting, out),
                    None => {
                        let purpose = Purpose::Save { tab: tab.id.clone(), then_quit: self.quitting };
                        let name = format!("{}.vgx", tab.label);
                        self.ask_file(FileKind::Save, Some(name), purpose, vgx_filter(), out);
                    }
                }
            }
            "discard" => {
                if self.closing {
                    self.closing = false;
                    self.remove_tab(self.active, out);
                } else if self.quitting {
                    self.advance_quit(out);
                }
            }
            _ => self.cancel_quit(out),
        }
    }

    fn advance_quit(&mut self, out: &mut Vec<AppMessage>) {
        if !self.quitting {
            return;
        }
        if self.quit_rest.is_empty() {
            self.quitting = false;
            out.push(AppMessage::QuitReady);
            return;
        }
        let next = self.quit_rest.remove(0);
        self.activate(&next);
        out.push(AppMessage::Ask(keep_ask()));
    }

    fn cancel_quit(&mut self, out: &mut Vec<AppMessage>) {
        self.quitting = false;
        self.closing = false;
        self.quit_rest.clear();
        out.extend(self.declared());
    }

    // What the host draws.

    /// The active tab's view and every declaration.
    pub fn emit(&mut self) -> Vec<AppMessage> {
        let mut out = self.raster();
        // The cells come before the panel that names one.
        out.extend(self.cells_due());
        out.extend(self.declared());
        out
    }

    /// The active tab's view. The frames it shows are asked for; the image
    /// is drawn again once they have all come, so stepping never shows a
    /// half-made image. Frame strips are drawn as their frames come.
    fn raster(&mut self) -> Vec<AppMessage> {
        // The sound of the take that plays, decoded once off the loop, and
        // its waveform for the transport's row.
        self.want_sound();
        let width = self.wave_width();
        self.prepare_wave(width);
        let index = self.active;
        let size = self.image_size();
        let Some(tab) = self.tabs.get(index) else {
            return Vec::new();
        };
        let shown = tab.round_view();
        // A round shown whole has its line and references over its strips.
        let sheet = tab.round_sheet();
        let mut scene = view::scene_with(shown.as_ref().unwrap_or(&tab.gallery), size, tab.look(), sheet.as_ref());
        // The notes at the time shown, drawn over the take's frame.
        scene.notes = self.notes_over();
        // The pre-vis check's animatic frame at the same time, asked for
        // with the take's.
        scene.animatic = self.animatic_over(&scene);
        if let Some(caption) = self.animatic_caption(&scene) {
            scene.captions.push(caption);
        }
        let keys = scene.keys();
        if scene.live {
            self.decoders.want(&keys, &[]);
        } else {
            self.decoders.want(&[], &keys);
        }
        self.want_thumbnails();
        let held = keys.iter().filter(|key| self.decoders.frames.contains_key(key)).count();
        let settled = keys.iter().all(|key| self.decoders.settled(key));
        let tab = &mut self.tabs[index];
        let redraw = match &tab.drawn {
            None => true,
            Some(drawn) if drawn.scene != scene => settled || (!scene.live && held > 0),
            Some(drawn) => held > drawn.held,
        };
        if redraw {
            let image = view::compose(&scene, &self.decoders.frames);
            let name = format!("{}-view.rgba", tab.id);
            let _ = std::fs::create_dir_all(&self.cache);
            self.next_revision += 1;
            if media::write_frame(&self.cache, &name, &image.data, (image.width, image.height)).is_ok() {
                if let Some(focus) = scene.focus.filter(|_| tab.drag.is_none()) {
                    tab.focus = focus;
                }
                tab.drawn = Some(Drawn { scene, held, revision: self.next_revision });
                self.playback.sent(self.next_revision);
                if settled {
                    self.decoders.keep_only(&keys);
                }
            }
        }
        let tab = &mut self.tabs[index];
        let Some((width, height, revision)) = tab.drawn.as_ref().map(|drawn| (drawn.scene.width, drawn.scene.height, drawn.revision)) else {
            return Vec::new();
        };
        let id = tab.id.clone();
        let camera = std::mem::take(&mut tab.frame_camera).then_some(DocumentCamera { origin: [0.0, 0.0], zoom: 1.0, rotation: 0.0, flip: false });
        let view = RasterView {
            image: self.cache.join(format!("{id}-view.rgba")).display().to_string(),
            tab: id,
            width,
            height,
            revision,
            patch: None,
            tool: self.raster_tool(),
            selection: Vec::new(),
            crop: None,
            bounds: None,
            camera,
            camera_owner: RasterCamera::Workspace,
            film: Some(declare::film(self, &self.tabs[index])),
            transport: declare::transport(self, &self.tabs[index]),
            text: self.text_editing(),
        };
        vec![AppMessage::Raster(view)]
    }

    /// Asks for each take's thumbnail, its first frame, once for each file.
    fn want_thumbnails(&mut self) {
        let mut wanted = Vec::new();
        for tab in &self.tabs {
            let own = tab.gallery.takes.iter().map(|take| (&take.file, take.info));
            let rounds = tab.rounds.reads().map(|read| (&read.file, read.info));
            for (file, info) in own.chain(rounds) {
                if let Some(info) = info
                    && !self.films.contains_key(file)
                    && !self.films_failed.contains(file)
                {
                    wanted.push(Key::new(file.clone(), 0, film_size(info.width, info.height)));
                }
            }
        }
        for key in wanted {
            self.decoders.want_thumbnail(key);
        }
    }

    /// The toolbar, menu, keys, panel, and tab list.
    pub fn declared(&self) -> Vec<AppMessage> {
        let mut messages = vec![
            AppMessage::Toolbar(declare::toolbar(self)),
            AppMessage::Menu { rows: declare::menu(self) },
            AppMessage::Keys(declare::keys()),
        ];
        if let Some(properties) = declare::properties(self) {
            messages.push(AppMessage::Properties(properties));
        }
        if let Some(tabs) = declare::tabs(self) {
            messages.push(AppMessage::Tabs(tabs));
        }
        messages
    }

    /// An assistant's call, carried out on its tab, then its result.
    fn invoke(&mut self, call: String, tab: &str, tool: &str, input: Value) -> Vec<AppMessage> {
        let Some(index) = self.tab_index(tab) else {
            return vec![crate::assist::result(call, Err(format!("There is no video in tab {tab}.")))];
        };
        let name = protocol::split_tool_key(tool).map_or(tool, |(_, _, name)| name);
        let active = self.active;
        self.active = index;
        let before = {
            let tab = &self.tabs[index];
            (tab.gallery.edits, tab.gallery.take().map(|take| take.uid), tab.frame)
        };
        let outcome = crate::assist::run(self, name, &input);
        let tab = &self.tabs[index];
        let after = (tab.gallery.edits, tab.gallery.take().map(|take| take.uid), tab.frame);
        let mut out = Vec::new();
        if after != before {
            if self.project.is_some() || after.0 != before.0 {
                self.tabs[index].dirty = true;
            }
            // The tab the call changed shows it, whichever tab is selected;
            // the declarations stay the selected tab's.
            out.extend(self.raster());
            self.active = active;
            out.extend(self.declared());
        }
        self.active = active;
        out.push(crate::assist::result(call, outcome));
        out
    }
}

impl Drop for Session {
    /// The scratch images, and the folder in memory, go with the session; a
    /// project's cache stays.
    fn drop(&mut self) {
        if self.project.is_none() || self.in_memory {
            let _ = std::fs::remove_dir_all(&self.cache);
        }
    }
}

/// Links a video file: where the `.vgx` names it, the file itself, its
/// sha256, and what it is. A file that is not a video is refused.
pub fn link_file(path: &Path, project: Option<&Path>) -> Result<(Link, PathBuf, media::Info), String> {
    let file = path.canonicalize().map_err(|_| format!("{} could not be found.", path.display()))?;
    let info = media::Video::open(&file).map_err(|error| error.0)?.info();
    let named = files::link_path(&file, project);
    let sha256 = layers::link::sha256_file(&file).map_err(|error| error.to_string())?;
    Ok((Link { path: named, sha256 }, file, info))
}

/// A folder of this session's own in memory for the host's images,
/// `$XDG_RUNTIME_DIR/inpainter/video-generator/<process>-<number>`, when the
/// system has one: a frame is written for every step, read by the host, and
/// never kept. Folders left by programs no longer running are removed
/// first.
fn memory_folder() -> Option<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if !runtime.is_absolute() || !runtime.is_dir() {
        return None;
    }
    let parent = runtime.join("inpainter").join("video-generator");
    let processes = Path::new("/proc/self").exists();
    for entry in std::fs::read_dir(&parent).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let process = name.to_str().and_then(|name| name.split('-').next()).and_then(|id| id.parse::<u32>().ok());
        if processes && let Some(process) = process && !Path::new("/proc").join(process.to_string()).exists() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let folder = parent.join(format!("{}-{number}", std::process::id()));
    std::fs::create_dir_all(&folder).ok()?;
    Some(folder)
}

/// A scratch folder of this session's own for the host's images while
/// there is no project, so two sessions never write one file.
fn scratch_folder() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("inpainter-video-generator-{}-{number}", std::process::id()))
}

/// A filmstrip thumbnail's size: 44 tall at the take's shape, unless it is
/// more than four times as wide as tall, as the Image Generator's are.
pub fn film_size(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = (f64::from(FILM_SIZE[0]) / w).min(f64::from(FILM_SIZE[1]) / h);
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

fn video_filter() -> FileFilter {
    FileFilter { name: "Videos".into(), extensions: VIDEO_EXTENSIONS.iter().map(|extension| extension.to_string()).collect() }
}

fn open_filter() -> FileFilter {
    let mut filter = video_filter();
    filter.extensions.push("vgx".into());
    filter
}

fn vgx_filter() -> FileFilter {
    FileFilter { name: "Video".into(), extensions: vec!["vgx".into()] }
}

/// A colour as the settings row's swatches send it, `[r, g, b]`.
fn rgb_of(value: &Value) -> Option<[u8; 3]> {
    let list = value.as_array()?;
    if list.len() != 3 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (slot, item) in rgb.iter_mut().zip(list) {
        *slot = u8::try_from(item.as_u64()?).ok()?;
    }
    Some(rgb)
}

fn notice(title: &str, line: &str) -> AppMessage {
    AppMessage::Ask(Ask {
        id: "video-notice".into(),
        title: title.into(),
        lines: vec![line.to_string()],
        buttons: vec![AskButton { id: "ok".into(), label: "OK".into() }],
        dismiss: Some("ok".into()),
    })
}

fn keep_ask() -> Ask {
    Ask {
        id: "keep-video".into(),
        title: "Keep this video?".into(),
        lines: vec!["The video has changes that are not saved.".into()],
        buttons: vec![
            AskButton { id: "save".into(), label: "Save".into() },
            AskButton { id: "discard".into(), label: "Discard".into() },
            AskButton { id: "cancel".into(), label: "Cancel".into() },
        ],
        dismiss: Some("cancel".into()),
    }
}

pub fn file_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| protocol::untitled(ITEM))
}

/// The file a tab never written goes to inside the project: its name, or
/// the first free name after it among the `.vgx` files in the folder and
/// the other open tabs. None when no name is free.
fn new_file(tabs: &[Tab], index: usize, folder: &Path) -> Option<PathBuf> {
    let label = &tabs.get(index)?.label;
    let taken = |name: &str| {
        let path = folder.join(format!("{name}.vgx"));
        path.exists()
            || tabs.iter().enumerate().any(|(other, tab)| {
                other != index && ((tab.label != *label && tab.label == name) || tab.path.as_ref() == Some(&path))
            })
    };
    let name = protocol::free_name(label, taken);
    (!taken(&name)).then(|| folder.join(format!("{name}.vgx")))
}

/// One line of the protocol, and the lines to write back.
/// A name that is safe as a file's, as the Image Generator makes one: path
/// separators, control characters, and line breaks become spaces, runs of
/// spaces one, and no leading dot.
fn safe_name(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { ' ' } else { c }).collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.trim_start_matches('.').trim().to_string()
}

pub fn respond(session: &mut Session, line: &str) -> Vec<String> {
    let Ok(message) = protocol::parse_host_message(line) else {
        return Vec::new();
    };
    session.handle(message).iter().map(protocol::encode_app_message).collect()
}
