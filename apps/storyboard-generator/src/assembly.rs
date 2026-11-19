//! `build-animatic`'s assembly file, which Skills' `assemble.py` makes the
//! animatic from: each frame an image held from its in to its out, a bed of
//! sound from the film's start, and clips of sound laid at their cues, in
//! seconds. A storyboard opens from one, and saves its timing as a new one.
//!
//! Studio counts whole frames at 24 fps, and an assembly's times need not
//! fall on frames. So the storyboard keeps every time as it read it, and
//! writes back unchanged whatever the person did not change; only what they
//! retimed is written as frames ÷ 24.

use crate::film::{FPS, Film, NewDrawing};
use crate::soundtrack::SoundTrack;
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub length: f64,
    pub fps: f64,
    pub size: [u32; 2],
    pub frames: Vec<Frame>,
    pub bed: Bed,
    pub clips: Vec<Voice>,
}

/// One image, held from `start` to `end`, in seconds: the file's `in` and `out`.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub image: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bed {
    pub audio: String,
    pub gain_db: f64,
}

/// A clip of sound: `from` to `to` of its file, laid at `at`.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub audio: String,
    pub from: f64,
    pub to: f64,
    pub at: f64,
    pub gain_db: f64,
}

/// The nearest film frame to a time.
pub fn frame_of(seconds: f64) -> u32 {
    (seconds * f64::from(FPS)).round().max(0.0) as u32
}

/// A frame as seconds, to four places.
pub fn seconds_of(frame: u32) -> f64 {
    (f64::from(frame) / f64::from(FPS) * 10_000.0).round() / 10_000.0
}

/// A gain in decibels as a volume from 0, where 1 is the sound as it is.
pub fn volume_of(gain_db: f64) -> f64 {
    10f64.powf(gain_db / 20.0)
}

/// A volume as a gain in decibels, to two places.
pub fn gain_of(volume: f64) -> f64 {
    (20.0 * volume.log10() * 100.0).round() / 100.0
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value.get(key).and_then(Value::as_f64).filter(|value| value.is_finite()).ok_or_else(|| format!("The animatic's assembly has no {key}."))
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    value.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("The animatic's assembly has no {key}."))
}

impl Assembly {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let size = value.get("size").and_then(Value::as_array).ok_or("The animatic's assembly has no size.")?;
        let side = |index: usize| size.get(index).and_then(Value::as_u64).map(|side| side.max(2) as u32).ok_or("The animatic's assembly has no size.");
        let mut frames = Vec::new();
        for frame in value.get("frames").and_then(Value::as_array).ok_or("The animatic's assembly has no frames.")? {
            frames.push(Frame { image: text(frame, "image")?, start: number(frame, "in")?, end: number(frame, "out")? });
        }
        let bed = value.get("bed").ok_or("The animatic's assembly has no bed.")?;
        let mut clips = Vec::new();
        for clip in value.get("clips").and_then(Value::as_array).cloned().unwrap_or_default() {
            clips.push(Voice {
                audio: text(&clip, "audio")?,
                from: number(&clip, "from")?,
                to: number(&clip, "to")?,
                at: number(&clip, "at")?,
                gain_db: clip.get("gain_db").and_then(Value::as_f64).unwrap_or(0.0),
            });
        }
        Ok(Assembly {
            length: number(value, "length")?,
            fps: value.get("fps").and_then(Value::as_f64).unwrap_or(f64::from(FPS)),
            size: [side(0)?, side(1)?],
            frames,
            bed: Bed { audio: text(bed, "audio")?, gain_db: bed.get("gain_db").and_then(Value::as_f64).unwrap_or(0.0) },
            clips,
        })
    }

    /// As `assemble.py`'s usage writes it.
    pub fn to_value(&self) -> Value {
        json!({
            "length": self.length,
            "fps": self.fps,
            "size": self.size,
            "frames": self.frames.iter().map(|frame| json!({ "image": frame.image, "in": frame.start, "out": frame.end })).collect::<Vec<_>>(),
            "bed": { "audio": self.bed.audio, "gain_db": self.bed.gain_db },
            "clips": self.clips.iter().map(|clip| json!({
                "audio": clip.audio, "from": clip.from, "to": clip.to, "at": clip.at, "gain_db": clip.gain_db,
            })).collect::<Vec<_>>(),
        })
    }
}

/// What a storyboard read from an assembly: each item's times as read, and
/// the film's frames they became, so what the person leaves alone is
/// written back as it was.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    /// The file, relative to the project, and its bytes' fingerprint.
    pub path: String,
    pub fingerprint: String,
    pub assembly: Assembly,
    /// For each frame, the clip it became.
    pub clips: Vec<u64>,
    /// The bed's sound and each voice's sound.
    pub bed: u64,
    pub voices: Vec<u64>,
    /// Drawn holds filed as plain files, kept under "boards" as before.
    pub boards: Vec<FiledHold>,
    /// The project's animatic step it came from, by chain and name.
    pub step: Option<(String, String)>,
}

/// A drawn hold filed into the project as a plain file, never published:
/// by its clip and its first frame in the clip, with its file in the
/// project, and a stamp of what the hold showed, so drawing on it again
/// files it again at the next save.
#[derive(Clone, Debug, PartialEq)]
pub struct FiledHold {
    pub clip: u64,
    pub frame: u32,
    pub file: String,
    pub stamp: String,
}

impl FiledHold {
    pub fn to_value(&self) -> Value {
        json!({ "clip": self.clip, "frame": self.frame, "file": self.file, "stamp": self.stamp })
    }

    pub fn from_value(board: &Value) -> Option<Self> {
        Some(FiledHold {
            clip: board.get("clip")?.as_u64()?,
            frame: board.get("frame")?.as_u64()? as u32,
            file: board.get("file")?.as_str()?.to_string(),
            // A hold kept with no stamp matches no hold, so it is filed
            // again before it is used.
            stamp: board.get("stamp").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
    }
}

impl Read {
    /// `other`'s file and timing, keeping this one's step and filed holds,
    /// as undo and redo of a Follow do.
    pub fn take_timing(&mut self, other: &Read) {
        self.path = other.path.clone();
        self.fingerprint = other.fingerprint.clone();
        self.assembly = other.assembly.clone();
        self.clips = other.clips.clone();
        self.bed = other.bed;
        self.voices = other.voices.clone();
    }

    /// The file a drawn hold was filed as, while it shows what it showed
    /// then.
    pub fn board(&self, clip: u64, frame: u32, stamp: &str) -> Option<&FiledHold> {
        self.boards.iter().find(|board| board.clip == clip && board.frame == frame && board.stamp == stamp)
    }

    pub fn to_value(&self) -> Value {
        json!({
            "path": self.path,
            "fingerprint": self.fingerprint,
            "assembly": self.assembly.to_value(),
            "clips": self.clips,
            "bed": self.bed,
            "voices": self.voices,
            "boards": self.boards.iter().map(FiledHold::to_value).collect::<Vec<_>>(),
            "step": self.step.as_ref().map(|(chain, name)| json!({ "chain": chain, "name": name })),
        })
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        let ids = |key: &str| -> Option<Vec<u64>> { value.get(key)?.as_array()?.iter().map(Value::as_u64).collect() };
        let mut boards = Vec::new();
        for board in value.get("boards").and_then(Value::as_array).cloned().unwrap_or_default() {
            boards.push(FiledHold::from_value(&board)?);
        }
        Some(Read {
            path: value.get("path")?.as_str()?.to_string(),
            fingerprint: value.get("fingerprint")?.as_str()?.to_string(),
            assembly: Assembly::from_value(value.get("assembly")?).ok()?,
            clips: ids("clips")?,
            bed: value.get("bed")?.as_u64()?,
            voices: ids("voices")?,
            boards,
            step: value.get("step").and_then(|step| Some((step.get("chain")?.as_str()?.to_string(), step.get("name")?.as_str()?.to_string()))),
        })
    }
}

/// A film opened from an assembly, the blank drawings its keys need, and
/// what it read. `length_of` gives each sound file's length in frames.
pub fn open(assembly: &Assembly, title: &str, path: &str, fingerprint: &str, mut length_of: impl FnMut(&str) -> Option<u32>) -> (Film, Vec<NewDrawing>, Read) {
    let mut film = Film::new(title);
    film.size = assembly.size;
    let mut drawings = Vec::new();
    let mut clips = Vec::new();
    for frame in &assembly.frames {
        let start = frame_of(frame.start);
        let length = frame_of(frame.end).saturating_sub(start).max(1);
        let (clip, made) = film.add_clip(Some(start), length);
        drawings.extend(made);
        if let Some(layer) = film.clip_mut(clip).and_then(|clip| clip.layers.first_mut())
            && let Some(key) = layer.keys.first()
        {
            layer.images.insert(key.id, frame.image.clone());
        }
        clips.push(clip);
    }
    let length = film.length();
    let name = |file: &str| file.rsplit('/').next().unwrap_or(file).to_string();
    // The bed: one sound from the film's start, cut at its end.
    let bed_source = length_of(&assembly.bed.audio).unwrap_or(length).max(1);
    let (bed_track, bed) = film.place_sound(None, 0, &assembly.bed.audio, &name(&assembly.bed.audio), bed_source).expect("a new track has room");
    film.set_sound_span(bed, Some(0), Some(0), Some(length.min(bed_source).max(1)));
    film.set_sound_volume(bed, volume_of(assembly.bed.gain_db).min(1.0));
    // The voices, on tracks after the bed's, never on it, even where the
    // bed ends early; two that overlap once rounded to frames go on two
    // tracks, so neither time is changed.
    let mut voices = Vec::new();
    for voice in &assembly.clips {
        let source = length_of(&voice.audio).unwrap_or_else(|| frame_of(voice.to)).max(1);
        let from = frame_of(voice.from).min(source - 1);
        let span = frame_of(voice.to).saturating_sub(frame_of(voice.from)).clamp(1, source - from);
        let at = frame_of(voice.at);
        let track = match film.sound_tracks.iter().skip(1).find(|track| track.fits(at, span)) {
            Some(track) => track.id,
            None => film.add_sound_track(),
        };
        let Some((_, id)) = film.lay_sound(Some(track), at, from, span, source, &voice.audio, &name(&voice.audio)) else {
            continue;
        };
        film.set_sound_volume(id, volume_of(voice.gain_db).min(1.0));
        voices.push(id);
    }
    let _ = bed_track;
    let read = Read {
        path: path.to_string(),
        fingerprint: fingerprint.to_string(),
        assembly: assembly.clone(),
        clips,
        bed,
        voices,
        boards: Vec::new(),
        step: None,
    };
    (film, drawings, read)
}

/// Follows a retime made in another tool: `film`, which read `old`, takes
/// the timing of `assembly`, which `fresh` and `fresh_read` were opened
/// from. Each of the assembly's frames goes to the clip that held the same
/// frame before, by its place when the count is the same and else by its
/// image, so a clip saved as several holds keeps its drawing on each; a
/// frame no clip held becomes a new clip. The sound is the assembly's, with
/// ids of the film's own. Returns the drawings new clips need and what the
/// storyboard now read.
pub fn follow(film: &mut Film, old: &Read, assembly: &Assembly, fresh: &Film, fresh_read: &Read) -> (Vec<NewDrawing>, Read) {
    // Which old frame each new frame was, if any.
    let mut was: Vec<Option<usize>> = Vec::new();
    if assembly.frames.len() == old.assembly.frames.len() {
        was.extend((0..assembly.frames.len()).map(Some));
    } else {
        let mut used = vec![false; old.assembly.frames.len()];
        for frame in &assembly.frames {
            let found = old.assembly.frames.iter().enumerate().position(|(index, before)| !used[index] && before.image == frame.image);
            if let Some(index) = found {
                used[index] = true;
            }
            was.push(found);
        }
    }
    // Runs of new frames that one clip of the film held.
    let place = |index: usize| fresh_read.clips.get(index).and_then(|id| fresh.clip(*id)).map_or((0, 1), |clip| (clip.start, clip.length));
    let mut runs: Vec<(Option<u64>, Vec<usize>)> = Vec::new();
    for index in 0..assembly.frames.len() {
        let owner = was[index].and_then(|before| old.clips.get(before).copied()).filter(|id| film.clip(*id).is_some());
        match runs.last_mut() {
            Some((last, frames)) if owner.is_some() && *last == owner => frames.push(index),
            _ => {
                // A clip met again after another is a new clip.
                let owner = owner.filter(|id| !runs.iter().any(|(other, _)| *other == Some(*id)));
                runs.push((owner, vec![index]));
            }
        }
    }
    let kept: Vec<u64> = runs.iter().filter_map(|(owner, _)| *owner).collect();
    film.clips.retain(|clip| kept.contains(&clip.id));
    let mut made = Vec::new();
    let mut clips = vec![0; assembly.frames.len()];
    let mut moved: Vec<(u64, std::collections::HashMap<u32, u32>)> = Vec::new();
    for (owner, frames) in &runs {
        let (start, _) = place(frames[0]);
        let (last_start, last_length) = place(*frames.last().unwrap_or(&frames[0]));
        let length = (last_start + last_length).saturating_sub(start).max(1);
        let Some(id) = owner else {
            continue;
        };
        let Some(clip) = film.clip_mut(*id) else {
            continue;
        };
        // Its holds, in order, move to the new frames' places in it, as
        // `holds_of` counts them: from 0, and the keys inside the clip.
        let was_length = clip.length;
        let mut holds: Vec<u32> = vec![0];
        holds.extend(clip.layers.iter().filter(|layer| !layer.folder).flat_map(|layer| layer.keys.iter().map(|key| key.frame)).filter(|frame| *frame < was_length));
        holds.sort_unstable();
        holds.dedup();
        clip.start = start;
        clip.length = length;
        let mut map = std::collections::HashMap::new();
        if holds.len() == frames.len() {
            for (hold, index) in holds.iter().zip(frames) {
                map.insert(*hold, place(*index).0.saturating_sub(start));
            }
            // A key or a mark moves with the hold it is in, and stays inside
            // the clip; keys past the clip's end stay where they were.
            let moved = |frame: u32| {
                let hold = holds.iter().rev().find(|hold| **hold <= frame).copied().unwrap_or(0);
                (map[&hold] + (frame - hold)).min(length.saturating_sub(1))
            };
            for layer in &mut clip.layers {
                for key in layer.keys.iter_mut().filter(|key| key.frame < was_length) {
                    key.frame = moved(key.frame);
                }
                for mark in layer.marks.iter_mut().filter(|mark| mark.frame < was_length) {
                    mark.frame = moved(mark.frame);
                }
            }
        }
        // A frame whose image changed shows the new image under its drawing.
        for index in frames {
            let image = &assembly.frames[*index].image;
            let before = was[*index].and_then(|before| old.assembly.frames.get(before)).map(|frame| &frame.image);
            if before != Some(image)
                && let Some(layer) = clip.layers.iter_mut().rev().find(|layer| !layer.folder)
                && let Some(key) = layer.key_at(place(*index).0.saturating_sub(start)).map(|key| key.id)
            {
                layer.images.insert(key, image.clone());
            }
            clips[*index] = *id;
        }
        moved.push((*id, map));
    }
    for (owner, frames) in &runs {
        if owner.is_some() {
            continue;
        }
        let (start, _) = place(frames[0]);
        let (last_start, last_length) = place(*frames.last().unwrap_or(&frames[0]));
        let (id, drawings) = film.add_clip(Some(start), (last_start + last_length).saturating_sub(start).max(1));
        made.extend(drawings);
        if let Some(layer) = film.clip_mut(id).and_then(|clip| clip.layers.first_mut())
            && let Some(key) = layer.keys.first().map(|key| key.id)
        {
            layer.images.insert(key, assembly.frames[frames[0]].image.clone());
        }
        for index in frames {
            clips[*index] = id;
        }
    }
    film.clips.sort_by_key(|clip| clip.start);
    // The sound as the assembly has it, with ids the film has not used.
    let mut ids = std::collections::HashMap::new();
    let mut tracks = fresh.sound_tracks.clone();
    for track in &mut tracks {
        track.id = film.id();
        for sound in &mut track.clips {
            let id = film.id();
            ids.insert(sound.id, id);
            sound.id = id;
        }
    }
    film.sound_tracks = tracks;
    let mut boards = old.boards.clone();
    boards.retain(|board| kept.contains(&board.clip));
    for board in &mut boards {
        if let Some(to) = moved.iter().find(|(id, _)| *id == board.clip).and_then(|(_, map)| map.get(&board.frame)) {
            board.frame = *to;
        }
    }
    let read = Read {
        path: fresh_read.path.clone(),
        fingerprint: fresh_read.fingerprint.clone(),
        assembly: assembly.clone(),
        clips,
        bed: ids.get(&fresh_read.bed).copied().unwrap_or(fresh_read.bed),
        voices: fresh_read.voices.iter().map(|id| ids.get(id).copied().unwrap_or(*id)).collect(),
        boards,
        step: old.step.clone(),
    };
    (made, read)
}

/// Why a film cannot be saved as an assembly, as a sentence.
pub type Refusal = String;

/// What saving left out, for the card, and which clip and sound each
/// entry of the saved assembly came from, for the storyboard to read it as.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notes {
    pub muted_left_out: bool,
    pub clips: Vec<u64>,
    pub bed: u64,
    pub voices: Vec<u64>,
}

impl Notes {
    /// The storyboard's memory of an assembly it has just saved as `path`.
    pub fn read(&self, assembly: Assembly, path: &str, fingerprint: &str, boards: Vec<FiledHold>) -> Read {
        Read {
            path: path.to_string(),
            fingerprint: fingerprint.to_string(),
            assembly,
            clips: self.clips.clone(),
            bed: self.bed,
            voices: self.voices.clone(),
            boards,
            step: None,
        }
    }
}

/// One hold of a clip: from `start` for `length` frames, inside the clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Hold {
    pub start: u32,
    pub length: u32,
    /// The image it shows: an image key's alone, or the file its drawing
    /// was filed as; none while it has drawing not filed yet.
    pub image: Option<String>,
    /// It shows nothing: no drawing and no image key.
    pub blank: bool,
}

/// Why a blank shot cannot be saved. Proposed to the owner on 2026-11-17,
/// not yet approved.
pub fn blank_refusal(shot: &str) -> String {
    format!("{shot} is blank. Draw on it or insert an image key first.")
}

/// The film's timing as an assembly. `holds` gives each clip's holds;
/// `canon_length` is the film's canon length in seconds, when the project
/// has one.
pub fn save(film: &Film, read: Option<&Read>, holds: impl Fn(u64) -> Vec<Hold>, canon_length: Option<f64>) -> Result<(Assembly, Notes), Refusal> {
    let length = film.length();
    // No gaps: each clip starts where the one before ends, from 0.
    let mut at = 0;
    for clip in &film.clips {
        if clip.start != at {
            return Err(format!("The film has a gap at {:.1} s, which the project's animatic cannot hold.", f64::from(at) / f64::from(FPS)));
        }
        at = clip.start + clip.length;
    }
    if let Some(canon) = canon_length
        && frame_of(canon) != length
    {
        return Err(format!(
            "The film is {:.1} s, and {}'s length is {:.1} s. Change the length in the shot list first.",
            f64::from(length) / f64::from(FPS),
            film.title,
            canon
        ));
    }
    // A time as read when its frame is unchanged, else the frame's own.
    let time = |read_seconds: Option<f64>, frame: u32| match read_seconds {
        Some(seconds) if frame_of(seconds) == frame => seconds,
        _ => seconds_of(frame),
    };
    // The frames a clip was read as, in order.
    let read_frames = |clip: u64| -> Vec<&Frame> {
        read.map(|read| read.clips.iter().zip(&read.assembly.frames).filter(|(id, _)| **id == clip).map(|(_, frame)| frame).collect()).unwrap_or_default()
    };
    let mut frames = Vec::new();
    let mut frame_clips = Vec::new();
    for clip in &film.clips {
        let was = read_frames(clip.id);
        let clip_holds = holds(clip.id);
        let same = was.len() == clip_holds.len();
        for (index, hold) in clip_holds.into_iter().enumerate() {
            let Some(image) = hold.image else {
                if hold.blank {
                    return Err(blank_refusal(&clip.name));
                }
                return Err(format!("{} has drawing that is not filed yet.", clip.name));
            };
            let start = clip.start + hold.start;
            let end = start + hold.length;
            // A clip read as as many frames as it has holds keeps their
            // times as read; a clip split into holds since is new timing.
            let (start_seconds, end_seconds) = match was.get(index) {
                Some(was) if same => (time(Some(was.start), start), time(Some(was.end), end)),
                _ => (seconds_of(start), seconds_of(end)),
            };
            frames.push(Frame { image, start: start_seconds, end: end_seconds });
            frame_clips.push(clip.id);
        }
    }
    // Each frame starts where the one before ends, so a held time read as
    // written and a time from frames never leave a sliver between them.
    for index in 1..frames.len() {
        frames[index].start = frames[index - 1].end;
    }
    let length_seconds = match read {
        Some(read) if frame_of(read.assembly.length) == length => read.assembly.length,
        _ => seconds_of(length),
    };
    // The bed: the first Sound track's one sound, from the start, heard.
    let refused = || "The bed must be one sound from the film's start, and heard.".to_string();
    let first: &SoundTrack = film.sound_tracks.first().ok_or_else(refused)?;
    let [bed_clip] = first.clips.as_slice() else {
        return Err(refused());
    };
    if first.muted || bed_clip.volume <= 0.0 || bed_clip.at != 0 || bed_clip.from != 0 {
        return Err(refused());
    }
    let bed_was = read.filter(|read| read.bed == bed_clip.id).map(|read| &read.assembly.bed);
    let bed = Bed {
        audio: bed_clip.file.clone(),
        gain_db: match bed_was {
            Some(was) if volume_of(was.gain_db).min(1.0) == bed_clip.volume => was.gain_db,
            _ => gain_of(bed_clip.volume),
        },
    };
    let mut notes = Notes::default();
    let mut clips = Vec::new();
    for track in &film.sound_tracks[1..] {
        for sound in &track.clips {
            if track.muted || sound.volume <= 0.0 {
                notes.muted_left_out = true;
                continue;
            }
            let was = read.and_then(|read| {
                let index = read.voices.iter().position(|id| *id == sound.id)?;
                read.assembly.clips.get(index)
            });
            let from = time(was.map(|was| was.from), sound.from);
            let at = time(was.map(|was| was.at), sound.at);
            let to = match was {
                Some(was) if frame_of(was.to).saturating_sub(frame_of(was.from)) == sound.length && frame_of(was.from) == sound.from => was.to,
                _ => seconds_of(sound.from + sound.length),
            };
            let gain_db = match was {
                Some(was) if volume_of(was.gain_db).min(1.0) == sound.volume => was.gain_db,
                _ => gain_of(sound.volume),
            };
            // In the order the file had them; new ones after, by their cue.
            let order = read.and_then(|read| read.voices.iter().position(|id| *id == sound.id)).unwrap_or(usize::MAX);
            clips.push((order, sound.at, sound.id, Voice { audio: sound.file.clone(), from, to, at, gain_db }));
        }
    }
    clips.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    notes.voices = clips.iter().map(|(_, _, id, _)| *id).collect();
    notes.clips = frame_clips;
    notes.bed = bed_clip.id;
    let clips: Vec<Voice> = clips.into_iter().map(|(_, _, _, voice)| voice).collect();
    Ok((
        Assembly {
            length: length_seconds,
            fps: read.map_or(f64::from(FPS), |read| read.assembly.fps),
            size: film.size,
            frames,
            bed,
            clips,
        },
        notes,
    ))
}

/// A time as the step's role words write it: "3.0", "4.2", "0.05".
pub fn said(seconds: f64) -> String {
    if seconds.fract() == 0.0 {
        return format!("{seconds:.1}");
    }
    let text = format!("{seconds:.4}");
    text.trim_end_matches('0').to_string()
}

/// A gain as the role words write it: "0 dB", "-6 dB", "-4.5 dB".
pub fn said_gain(gain_db: f64) -> String {
    if gain_db.fract() == 0.0 {
        format!("{} dB", gain_db as i64)
    } else {
        format!("{} dB", format!("{gain_db:.2}").trim_end_matches('0').trim_end_matches('.'))
    }
}

/// The step's references for a saved assembly: each frame's artifact, the
/// bed's, and each clip's, by role words as `build-animatic` writes them.
/// `artifact_of` names the canon artifact a file is published as.
pub fn refs(assembly: &Assembly, artifact_of: impl Fn(&str) -> Option<String>) -> Vec<Value> {
    // A canon file by its artifact; any other, such as a drawn hold filed
    // as a plain file or a sound not yet used by a recipe, by its file, as
    // Skills' step references take either.
    let named = |file: &str, role: String| match artifact_of(file) {
        Some(artifact) => json!({ "artifact": artifact, "role": role }),
        None => json!({ "file": file, "role": role }),
    };
    let mut out = Vec::new();
    for frame in &assembly.frames {
        out.push(named(&frame.image, format!("frame, {} to {} s", said(frame.start), said(frame.end))));
    }
    out.push(named(&assembly.bed.audio, format!("sound bed, at {}", said_gain(assembly.bed.gain_db))));
    for clip in &assembly.clips {
        out.push(named(&clip.audio, format!("clip {} to {} s, laid at {} s, at {}", said(clip.from), said(clip.to), said(clip.at), said_gain(clip.gain_db))));
    }
    out
}

/// The step's script with its assembly file replaced by `file`: the one
/// argument ending in `.json` after `assemble.py`.
pub fn repoint(script: &str, file: &str) -> Option<String> {
    let after = script.find("assemble.py")?;
    let rest = &script[after..];
    let old = rest.split_whitespace().find(|word| word.trim_matches(['"', '\'']).ends_with(".json"))?;
    Some(format!("{}{}", &script[..after], rest.replacen(old, file, 1)))
}

/// The assembly file a step's script names.
pub fn named_in(script: &str) -> Option<String> {
    let after = script.find("assemble.py")?;
    script[after..].split_whitespace().map(|word| word.trim_matches(['"', '\''])).find(|word| word.ends_with(".json")).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Last order's assembly, as filed on 2026-11-14.
    fn last_order() -> Assembly {
        Assembly::from_value(&json!({
            "length": 8.0, "fps": 24, "size": [1536, 864],
            "frames": [
                { "image": "library/2026-11-12_20-48-keyframe-wiping-2.png", "in": 0.0, "out": 3.0 },
                { "image": "library/2026-11-14_16-04-keyframe-the-bell.png", "in": 3.0, "out": 4.2 },
                { "image": "library/2026-11-14_15-37-keyframe-the-line.png", "in": 4.2, "out": 6.3 },
                { "image": "library/2026-11-14_15-40-keyframe-waiting.png", "in": 6.3, "out": 8.0 }
            ],
            "bed": { "audio": "library/2026-11-12_20-27-scene-3.wav", "gain_db": 0 },
            "clips": [
                { "audio": "library/2026-11-12_20-03-otis-line-1.wav", "from": 0.05, "to": 0.65, "at": 4.2, "gain_db": 0 },
                { "audio": "library/2026-11-12_20-03-otis-line-1.wav", "from": 2.05, "to": 3.35, "at": 4.8, "gain_db": 0 }
            ]
        }))
        .unwrap()
    }

    /// The sounds' lengths: the bed 8 s, Otis's take 4.08 s.
    fn lengths(file: &str) -> Option<u32> {
        Some(if file.contains("scene-3") { 192 } else { 98 })
    }

    fn opened() -> (Film, Read) {
        let (film, _, read) = open(&last_order(), "Last order", "prompts/resources/a.json", "f", lengths);
        (film, read)
    }

    /// Each clip held whole, its image key's image alone.
    fn plain_holds(film: &Film) -> impl Fn(u64) -> Vec<Hold> + '_ {
        move |clip| {
            let clip = film.clip(clip).unwrap();
            vec![Hold { start: 0, length: clip.length, image: clip.layers[0].images.values().next().cloned(), blank: false }]
        }
    }

    #[test]
    fn last_orders_assembly_opens_as_four_clips_over_its_sound() {
        let (film, _) = opened();
        let spans: Vec<(u32, u32)> = film.clips.iter().map(|clip| (clip.start, clip.start + clip.length)).collect();
        assert_eq!(spans, [(0, 72), (72, 101), (101, 151), (151, 192)]);
        assert_eq!(film.clips[1].layers[0].image_at(0), Some("library/2026-11-14_16-04-keyframe-the-bell.png"));
        let bed = &film.sound_tracks[0].clips[0];
        assert_eq!((bed.at, bed.from, bed.length), (0, 0, 192));
        let mut voices: Vec<(u32, u32, u32)> = film.sound_tracks[1..].iter().flat_map(|track| track.clips.iter().map(|clip| (clip.at, clip.from, clip.length))).collect();
        voices.sort_unstable();
        // 4.2 s and 4.8 s; 0.05–0.65 s and 2.05–3.35 s of the take. Rounded,
        // the first ends a frame after the second starts, so they lie on two
        // tracks rather than either moving.
        assert_eq!(voices, [(101, 1, 15), (115, 49, 31)]);
        assert_eq!(film.sound_tracks.len(), 3);
    }

    #[test]
    fn a_short_bed_keeps_its_track_to_itself() {
        // A bed of 2 s under an 8 s film: the voices at 4.2 and 4.8 s fit
        // after it, and still go on tracks of their own.
        let short = |file: &str| Some(if file.contains("scene-3") { 48 } else { 98 });
        let (film, _, read) = open(&last_order(), "Last order", "prompts/resources/a.json", "f", short);
        assert_eq!(film.sound_tracks[0].clips.len(), 1);
        assert_eq!(film.sound_tracks[0].clips[0].id, read.bed);
        let saved = save(&film, Some(&read), plain_holds(&film), Some(8.0)).unwrap().0;
        assert_eq!(saved.clips, last_order().clips);
    }

    #[test]
    fn an_untouched_storyboard_writes_back_the_same_assembly() {
        let (film, read) = opened();
        let (saved, notes) = save(&film, Some(&read), plain_holds(&film), Some(8.0)).unwrap();
        assert_eq!(saved.to_value(), last_order().to_value());
        assert!(!notes.muted_left_out);
        assert_eq!(notes.clips, film.clips.iter().map(|clip| clip.id).collect::<Vec<_>>());
    }

    #[test]
    fn a_retime_moves_only_what_was_retimed() {
        let (mut film, read) = opened();
        let (second, last) = (film.clips[1].id, film.clips[3].id);
        assert!(film.resize_clip(second, true, 2));
        // The film is two frames longer: refused until balanced.
        let refused = save(&film, Some(&read), plain_holds(&film), Some(8.0)).unwrap_err();
        assert_eq!(refused, "The film is 8.1 s, and Last order's length is 8.0 s. Change the length in the shot list first.");
        assert!(film.resize_clip(last, true, -2));
        let (saved, _) = save(&film, Some(&read), plain_holds(&film), Some(8.0)).unwrap();
        let times: Vec<(f64, f64)> = saved.frames.iter().map(|frame| (frame.start, frame.end)).collect();
        assert_eq!(times, [(0.0, 3.0), (3.0, 4.2917), (4.2917, 6.375), (6.375, 8.0)]);
        // The voices are where they were.
        assert_eq!(saved.clips, last_order().clips);
    }

    #[test]
    fn refusals_say_why() {
        let (mut film, read) = opened();
        // A gap.
        let mut gapped = film.clone();
        let third = gapped.clips[2].id;
        gapped.delete_clip(third);
        assert_eq!(save(&gapped, Some(&read), plain_holds(&gapped), None).unwrap_err(), "The film has a gap at 4.2 s, which the project's animatic cannot hold.");
        // Drawing not published.
        let unpublished = |_| vec![Hold { start: 0, length: 72, image: None, blank: false }];
        assert_eq!(save(&film, Some(&read), unpublished, None).unwrap_err().split(" has").next(), Some("Shot 1"));
        // A blank shot says it is blank.
        let blank = |_| vec![Hold { start: 0, length: 72, image: None, blank: true }];
        assert_eq!(save(&film, Some(&read), blank, None).unwrap_err(), blank_refusal("Shot 1"));
        // A muted bed.
        let bed_track = film.sound_tracks[0].id;
        assert!(film.set_track_flag(crate::soundtrack::TrackRef::Sound(bed_track), crate::soundtrack::TrackFlag::Muted, true));
        assert_eq!(save(&film, Some(&read), plain_holds(&film), None).unwrap_err(), "The bed must be one sound from the film's start, and heard.");
    }

    #[test]
    fn muted_voices_are_left_out_and_said_so() {
        let (mut film, read) = opened();
        for track in film.sound_tracks[1..].iter().map(|track| track.id).collect::<Vec<_>>() {
            film.set_track_flag(crate::soundtrack::TrackRef::Sound(track), crate::soundtrack::TrackFlag::Muted, true);
        }
        let (saved, notes) = save(&film, Some(&read), plain_holds(&film), None).unwrap();
        assert!(saved.clips.is_empty() && notes.muted_left_out);
    }

    /// Last order's step "animatic", as canon.sql holds it.
    const SCRIPT: &str = "python3 \"${SPUMCAKE_AGENT:-/skills}/skills/build-animatic/scripts/assemble.py\" prompts/resources/2026-11-14_16-07-animatic-assembly.json .build/animatic.mp4";

    #[test]
    fn the_steps_words_and_script_follow_the_saved_timing() {
        let artifact_of = |file: &str| {
            Some(
                match file {
                    "library/2026-11-12_20-48-keyframe-wiping-2.png" => "Last order: keyframe, wiping",
                    "library/2026-11-14_16-04-keyframe-the-bell.png" => "Last order: keyframe, the bell",
                    "library/2026-11-14_15-37-keyframe-the-line.png" => "Last order: keyframe, the line",
                    "library/2026-11-14_15-40-keyframe-waiting.png" => "Last order: keyframe, waiting",
                    "library/2026-11-12_20-27-scene-3.wav" => "Last order: sound bed",
                    "library/2026-11-12_20-03-otis-line-1.wav" => "Otis: the line, voice take",
                    _ => return None,
                }
                .to_string(),
            )
        };
        // Untouched, the words are Last order's own.
        let words: Vec<String> = refs(&last_order(), artifact_of).iter().map(|r| r["role"].as_str().unwrap().to_string()).collect();
        assert_eq!(words, [
            "frame, 0.0 to 3.0 s", "frame, 3.0 to 4.2 s", "frame, 4.2 to 6.3 s", "frame, 6.3 to 8.0 s",
            "sound bed, at 0 dB", "clip 0.05 to 0.65 s, laid at 4.2 s, at 0 dB", "clip 2.05 to 3.35 s, laid at 4.8 s, at 0 dB",
        ]);
        // A file nobody published, such as a drawn hold filed as a plain
        // file, is named by its file.
        let mut unknown = last_order();
        unknown.frames[1].image = "prompts/resources/2026-11-18_10-00-shot-2-hold-1.png".into();
        assert_eq!(refs(&unknown, artifact_of)[1], json!({ "file": "prompts/resources/2026-11-18_10-00-shot-2-hold-1.png", "role": "frame, 3.0 to 4.2 s" }));
        // The script is pointed at the new file, and nothing else changes.
        assert_eq!(named_in(SCRIPT).as_deref(), Some("prompts/resources/2026-11-14_16-07-animatic-assembly.json"));
        let moved = repoint(SCRIPT, "prompts/resources/2026-11-17_10-00-animatic-assembly.json").unwrap();
        assert_eq!(moved, SCRIPT.replace("2026-11-14_16-07", "2026-11-17_10-00"));
        assert_eq!((said(4.2917), said_gain(-6.0), said_gain(-4.5)), ("4.2917".to_string(), "-6 dB".to_string(), "-4.5 dB".to_string()));
    }

    #[test]
    fn gains_round_trip_through_volumes() {
        let mut assembly = last_order();
        assembly.clips[0].gain_db = -6.0;
        let (film, _, read) = open(&assembly, "Last order", "a.json", "f", lengths);
        let volume = film.sound(read.voices[0]).unwrap().1.volume;
        assert!((volume - 0.501).abs() < 0.001);
        let (saved, _) = save(&film, Some(&read), plain_holds(&film), None).unwrap();
        assert_eq!(saved.clips[0].gain_db, -6.0);
        // Changed, it is written from the volume.
        assert_eq!(gain_of(0.25), -12.04);
    }
}
