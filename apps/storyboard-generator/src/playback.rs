//! Playing the film with its sound, and scrubbing it. The sound plays in
//! `media`'s Player, and the image always follows the Player's clock: each
//! tick sends the frame `frame_to_show` names, never a frame counted here.
//! The frames are the export's own images, rendered ahead on a thread of
//! their own, and reach the host as one stage image whose revision rises.
//! When the host says it drew a revision, the delay since it was sent keeps
//! the Player's lead true.

use crate::export::{Image, Shown};
use crate::look::Looker;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// How many images ahead of the one showing the worker keeps ready.
const AHEAD: usize = 48;
/// The most memory the rendered images may hold.
const MOST_BYTES: usize = 1 << 30;
/// How many of the host's reports the lead is the mean of.
const LEAD_REPORTS: usize = 30;

/// Rendered images, shared with the thread that renders them.
struct Renders {
    images: Vec<Image>,
    size: (u32, u32),
    done: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
    /// The image showing now, which the worker renders ahead of.
    want: AtomicUsize,
    stop: AtomicBool,
}

impl Renders {
    fn get(&self, index: usize) -> Option<Arc<Vec<u8>>> {
        self.done.lock().ok()?.get(&index).cloned()
    }

    /// Renders on this thread, for a frame the worker has not reached.
    fn now(&self, looker: &mut Looker, index: usize) -> Option<Arc<Vec<u8>>> {
        if let Some(found) = self.get(index) {
            return Some(found);
        }
        let rendered = Arc::new(crate::export::render_image(looker, self.images.get(index)?, self.size));
        self.keep(index, rendered.clone());
        Some(rendered)
    }

    fn keep(&self, index: usize, rendered: Arc<Vec<u8>>) {
        let Ok(mut done) = self.done.lock() else {
            return;
        };
        done.insert(index, rendered);
        // Over the limit, the images furthest behind go first.
        let each = (self.size.0 * self.size.1 * 4) as usize;
        let want = self.want.load(Ordering::Relaxed);
        while done.len() * each > MOST_BYTES {
            let Some(far) = done.keys().copied().filter(|at| *at != index).max_by_key(|at| if *at < want { usize::MAX - at } else { at - want }) else {
                break;
            };
            done.remove(&far);
        }
    }

    fn work(self: Arc<Self>) {
        let mut looker = Looker::new();
        while !self.stop.load(Ordering::Relaxed) {
            let want = self.want.load(Ordering::Relaxed);
            let next = (want..(want + AHEAD).min(self.images.len())).find(|index| self.get(*index).is_none());
            match next {
                Some(index) => {
                    let rendered = Arc::new(crate::export::render_image(&mut looker, &self.images[index], self.size));
                    self.keep(index, rendered);
                }
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
    }
}

/// What plays: the film's frames from `from` to `to`, looping or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: u32,
    pub to: u32,
    pub looping: bool,
}

/// The film's images while it plays or is scrubbed.
pub struct Running {
    frames: Vec<Shown>,
    renders: Arc<Renders>,
    /// Playing, rather than only scrubbed.
    pub playing: bool,
    pub span: Span,
    /// The film's frame on the stage now.
    pub shown: Option<u32>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.renders.stop.store(true, Ordering::Relaxed);
    }
}

/// What the stage shows for a frame while playing: an image, or a plain
/// colour for a gap or a hidden Shots track.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Image(protocol::Thumbnail),
    Fill([u8; 3]),
}

/// One storyboard's playback: its Player, opened on first use, and the
/// images while it plays.
pub struct Playback {
    player: Option<media::Player>,
    pub running: Option<Running>,
    /// The film's frame the clock last named, to tell a new frame.
    last: Option<u32>,
    pub frame: Option<Frame>,
    revision: u64,
    /// Each revision sent and when, in milliseconds since the Unix epoch.
    sent: VecDeque<(u64, f64)>,
    delays: VecDeque<f64>,
    folder: PathBuf,
    name: String,
    looker: Looker,
}

/// Milliseconds since the Unix epoch, as `shown`'s `at` counts.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

impl Playback {
    /// Frames are written into `folder`, one file named `name`.
    pub fn new(folder: PathBuf, name: String) -> Self {
        Self {
            player: None,
            running: None,
            last: None,
            frame: None,
            revision: 0,
            sent: VecDeque::new(),
            delays: VecDeque::new(),
            folder,
            name,
            looker: Looker::new(),
        }
    }

    /// Uses `player` instead of the computer's sound output, as tests do.
    pub fn with_player(mut self, player: media::Player) -> Self {
        self.player = Some(player);
        self
    }

    pub fn player(&mut self) -> &mut media::Player {
        self.player.get_or_insert_with(|| media::Player::open(crate::sounds::RATE, crate::sounds::fps()))
    }

    pub fn playing(&self) -> bool {
        self.running.as_ref().is_some_and(|running| running.playing)
    }

    /// Takes the film's images for playing or scrubbing.
    pub fn prepare(&mut self, plan: crate::export::Plan) {
        if self.running.is_some() {
            return;
        }
        let images = plan.images.into_iter().map(|image| image.expect("a fresh plan")).collect();
        let renders = Arc::new(Renders { images, size: plan.size, done: Mutex::new(HashMap::new()), want: AtomicUsize::new(0), stop: AtomicBool::new(false) });
        let worker = renders.clone();
        std::thread::spawn(move || worker.work());
        self.player().set_tracks(plan.tracks);
        self.running = Some(Running { frames: plan.frames, renders, playing: false, span: Span { from: 0, to: 0, looping: false }, shown: None });
    }

    /// Plays `span` from `at`.
    pub fn play(&mut self, span: Span, at: u32) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        running.playing = true;
        running.span = span;
        self.last = None;
        let player = self.player();
        if span.looping {
            player.play_range(span.from, span.to, true);
        } else {
            player.play_range(at.clamp(span.from, span.to.saturating_sub(1).max(span.from)), span.to, false);
        }
    }

    /// Stops playing and scrubbing; the stage shows the drawing again.
    pub fn stop(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.pause();
        }
        self.running = None;
        self.frame = None;
        self.last = None;
    }

    /// The frame the clock names now, within what plays. `None` once a span
    /// that does not loop has ended.
    pub fn clock_frame(&mut self) -> Option<u32> {
        let span = self.running.as_ref()?.span;
        let frame = self.player().frame_to_show()?;
        if !span.looping && frame >= span.to {
            return None;
        }
        Some(frame.clamp(span.from, span.to.saturating_sub(1).max(span.from)))
    }

    /// Shows the film's frame `frame` on the stage. Returns whether the stage
    /// changed. A image not yet rendered keeps the last one showing, unless
    /// `wait`, which renders it here, as scrubbing does.
    pub fn show(&mut self, frame: u32, wait: bool) -> bool {
        let Some(running) = self.running.as_mut() else {
            return false;
        };
        if running.shown == Some(frame) && self.frame.is_some() {
            return false;
        }
        let shown = running.frames.get(frame as usize).copied().unwrap_or(Shown::Gap);
        let next = match shown {
            Shown::Gap => Frame::Fill([0, 0, 0]),
            Shown::White => Frame::Fill([255, 255, 255]),
            Shown::Image(index) => {
                running.renders.want.store(index, Ordering::Relaxed);
                let rendered = if wait { running.renders.now(&mut self.looker, index) } else { running.renders.get(index) };
                let Some(rendered) = rendered else {
                    return false;
                };
                let (width, height) = running.renders.size;
                let _ = std::fs::create_dir_all(&self.folder);
                let Ok(path) = media::write_frame(&self.folder, &self.name, &rendered, (width, height)) else {
                    return false;
                };
                self.revision += 1;
                self.sent.push_back((self.revision, now_ms()));
                if self.sent.len() > LEAD_REPORTS * 4 {
                    self.sent.pop_front();
                }
                Frame::Image(protocol::Thumbnail { path: path.display().to_string(), width, height, revision: self.revision })
            }
        };
        running.shown = Some(frame);
        self.frame = Some(next);
        true
    }

    /// A tick while playing: the frame the clock names, if it is new.
    /// Returns whether the stage changed, and the frame when playing ended.
    pub fn tick(&mut self) -> (bool, Option<u32>) {
        if !self.playing() {
            return (false, None);
        }
        let Some(frame) = self.clock_frame() else {
            let end = self.running.as_ref().map(|running| running.span.to.saturating_sub(1));
            self.stop();
            return (true, end);
        };
        if self.last == Some(frame) {
            return (false, None);
        }
        // A image not rendered yet is tried again on the next tick.
        let changed = self.show(frame, false);
        if changed {
            self.last = Some(frame);
        }
        (changed, None)
    }

    /// The host drew `revision` at `at`: the delay since it was sent keeps
    /// the Player's lead, as the mean of the last reports.
    pub fn shown(&mut self, revision: u64, at: f64) {
        let Some(position) = self.sent.iter().position(|(sent, _)| *sent == revision) else {
            return;
        };
        let (_, sent) = self.sent[position];
        self.sent.drain(..=position);
        self.delays.push_back((at - sent).max(0.0));
        if self.delays.len() > LEAD_REPORTS {
            self.delays.pop_front();
        }
        let mean = self.delays.iter().sum::<f64>() / self.delays.len() as f64;
        self.player().set_lead(mean);
    }

    /// Scrubs the film's frame: its slice of sound, and its image.
    pub fn scrub(&mut self, frame: u32) -> bool {
        self.player().scrub(frame);
        self.show(frame, true)
    }

    /// Why the Player has no sound, as a sentence, when it has none.
    pub fn problem(&self) -> Option<String> {
        self.player.as_ref()?.problem().map(|problem| problem.to_string())
    }

    /// The lead the Player keeps now, in milliseconds.
    pub fn lead(&mut self) -> f64 {
        self.player().lead()
    }
}
