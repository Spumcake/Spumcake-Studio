//! Frames decoded off the message loop. The frame a person steps through,
//! alone or side by side, comes from a `media::Seeker` for its file, so the
//! newest request wins while keys are held. Frame strips and the
//! filmstrip's thumbnails come from one thread of this program's own, which
//! decodes a list in order: thumbnails first, then the strips last asked
//! for, a newer list replacing an older one.

use layers::Rgba;
use media::{Seeker, Video};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// One decoded frame: the file, the frame's index, and the size it was
/// decoded at, which is always even, as `media` decodes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub file: PathBuf,
    pub index: u32,
    pub width: u32,
    pub height: u32,
}

impl Key {
    pub fn new(file: PathBuf, index: u32, size: (u32, u32)) -> Key {
        let (width, height) = even(size);
        Key { file, index, width, height }
    }
}

/// A size as `media` decodes it: even, and at least 2.
pub fn even(size: (u32, u32)) -> (u32, u32) {
    (size.0.max(2) & !1, size.1.max(2) & !1)
}

/// Why a frame or a file could not be decoded, kept so it is not asked for
/// again and its place is drawn empty.
type Failed = String;

/// A file's seeker and the frame last asked of it, and whether it came.
struct Live {
    seeker: Seeker,
    asked: Option<(Key, bool)>,
}

enum Job {
    Thumbnail(Key),
    Strips(Vec<Key>),
}

pub struct Decoders {
    live: HashMap<PathBuf, Live>,
    jobs: Sender<Job>,
    done: Receiver<(Key, bool, media::Result<media::Frame>)>,
    /// The strips' frames asked for and not yet back.
    strips: Vec<Key>,
    /// Thumbnails asked for and not yet back.
    thumbnails: HashSet<Key>,
    /// Frames ready to draw.
    pub frames: HashMap<Key, Rgba>,
    /// Thumbnails come back here, to be written once and let go.
    pub thumbs: Vec<(Key, Result<Rgba, Failed>)>,
    failed: HashMap<Key, Failed>,
    /// Files that would not open for a seeker.
    closed: HashMap<PathBuf, Failed>,
}

impl Default for Decoders {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoders {
    pub fn new() -> Self {
        let (jobs, incoming) = std::sync::mpsc::channel();
        let (out, done) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name(String::from("video-generator-frames"))
            .spawn(move || serve(incoming, out))
            .expect("a thread for decoding");
        Self {
            live: HashMap::new(),
            jobs,
            done,
            strips: Vec::new(),
            thumbnails: HashSet::new(),
            frames: HashMap::new(),
            thumbs: Vec::new(),
            failed: HashMap::new(),
            closed: HashMap::new(),
        }
    }

    /// Whether the frame is ready, or will never be: its place can be drawn.
    pub fn settled(&self, key: &Key) -> bool {
        self.frames.contains_key(key) || self.failed.contains_key(key) || self.closed.contains_key(&key.file)
    }

    /// Asks for the frames a view shows: `live` ones from their file's
    /// seeker, the newest for each file winning, and the rest, in order, from
    /// the strips' thread, in place of the strips asked for before.
    pub fn want(&mut self, live: &[Key], rest: &[Key]) {
        let mut queued = Vec::new();
        let mut asked: HashSet<PathBuf> = HashSet::new();
        for key in live {
            if self.settled(key) {
                continue;
            }
            // A second frame of the same file goes to the strips' thread, so
            // neither request takes the other's place.
            if !asked.insert(key.file.clone()) {
                queued.push(key.clone());
                continue;
            }
            if !self.live.contains_key(&key.file) {
                match Video::open(&key.file) {
                    Ok(video) => {
                        self.live.insert(key.file.clone(), Live { seeker: Seeker::new(video), asked: None });
                    }
                    Err(error) => {
                        self.closed.insert(key.file.clone(), error.0);
                        continue;
                    }
                }
            }
            let Some(live) = self.live.get_mut(&key.file) else {
                continue;
            };
            let waiting = matches!(&live.asked, Some((asked, false)) if asked == key);
            if !waiting {
                live.seeker.frame(key.index, (key.width, key.height));
                live.asked = Some((key.clone(), false));
            }
        }
        queued.extend(rest.iter().filter(|key| !self.settled(key)).cloned());
        if queued != self.strips {
            self.strips = queued.clone();
            let _ = self.jobs.send(Job::Strips(queued));
        }
    }

    /// Asks for a thumbnail, once.
    pub fn want_thumbnail(&mut self, key: Key) {
        if self.thumbnails.insert(key.clone()) {
            let _ = self.jobs.send(Job::Thumbnail(key));
        }
    }

    /// Takes in every frame decoded since the last call. Returns whether
    /// any came.
    pub fn poll(&mut self) -> bool {
        let mut came = false;
        for live in self.live.values_mut() {
            // Only the frame last asked for comes back; a seeker whose
            // thread has stopped answers with an error every time.
            if let Some(result) = live.seeker.poll()
                && let Some((key, received)) = &mut live.asked
            {
                *received = true;
                came = true;
                match result {
                    Ok(frame) => {
                        self.frames.insert(key.clone(), rgba(frame));
                    }
                    Err(error) => {
                        self.failed.insert(key.clone(), error.0);
                    }
                }
            }
        }
        while let Ok((key, thumbnail, result)) = self.done.try_recv() {
            came = true;
            if thumbnail {
                self.thumbnails.remove(&key);
                self.thumbs.push((key, result.map(rgba).map_err(|error| error.0)));
                continue;
            }
            self.strips.retain(|asked| *asked != key);
            match result {
                Ok(frame) => {
                    self.frames.insert(key, rgba(frame));
                }
                Err(error) => {
                    self.failed.insert(key, error.0);
                }
            }
        }
        came
    }

    /// Whether anything asked for has not come back yet.
    pub fn waiting(&self) -> bool {
        !self.strips.is_empty()
            || !self.thumbnails.is_empty()
            || self.live.values().any(|live| matches!(live.asked, Some((_, false))))
    }

    /// Lets go of every frame and seeker a view no longer shows. A file
    /// that failed is tried again when it is next wanted.
    pub fn keep_only(&mut self, keys: &[Key]) {
        let files: HashSet<&PathBuf> = keys.iter().map(|key| &key.file).collect();
        self.frames.retain(|key, _| keys.contains(key));
        self.failed.retain(|key, _| keys.contains(key));
        self.live.retain(|file, live| files.contains(file) || matches!(live.asked, Some((_, false))));
        self.closed.retain(|file, _| files.contains(file));
    }

    /// Forgets that a file would not open, as when it is linked again.
    pub fn forget(&mut self, file: &PathBuf) {
        self.closed.remove(file);
        self.live.remove(file);
        self.failed.retain(|key, _| key.file != *file);
        self.frames.retain(|key, _| key.file != *file);
    }
}

fn rgba(frame: media::Frame) -> Rgba {
    Rgba { width: frame.width, height: frame.height, data: frame.rgba }
}

/// The strips' and thumbnails' thread: thumbnails first, each asked once,
/// then the latest strips in order. It keeps a few files open.
fn serve(incoming: Receiver<Job>, out: Sender<(Key, bool, media::Result<media::Frame>)>) {
    const OPEN: usize = 6;
    let mut thumbnails: VecDeque<Key> = VecDeque::new();
    let mut strips: VecDeque<Key> = VecDeque::new();
    let mut videos: HashMap<PathBuf, Video> = HashMap::new();
    loop {
        let job = if thumbnails.is_empty() && strips.is_empty() {
            match incoming.recv() {
                Ok(job) => Some(job),
                Err(_) => return,
            }
        } else {
            match incoming.try_recv() {
                Ok(job) => Some(job),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        };
        match job {
            Some(Job::Thumbnail(key)) => {
                thumbnails.push_back(key);
                continue;
            }
            Some(Job::Strips(keys)) => {
                strips = keys.into();
                continue;
            }
            None => {}
        }
        let (key, thumbnail) = match thumbnails.pop_front() {
            Some(key) => (key, true),
            None => match strips.pop_front() {
                Some(key) => (key, false),
                None => continue,
            },
        };
        if !videos.contains_key(&key.file) {
            if videos.len() >= OPEN {
                videos.clear();
            }
            match Video::open(&key.file) {
                Ok(video) => {
                    videos.insert(key.file.clone(), video);
                }
                Err(error) => {
                    if out.send((key, thumbnail, Err(error))).is_err() {
                        return;
                    }
                    continue;
                }
            }
        }
        let Some(video) = videos.get_mut(&key.file) else {
            continue;
        };
        let frame = video.frame(key.index, (key.width, key.height));
        if out.send((key, thumbnail, frame)).is_err() {
            return;
        }
    }
}
