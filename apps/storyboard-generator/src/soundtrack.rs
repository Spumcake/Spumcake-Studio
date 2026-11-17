//! The film's Sound tracks: sounds placed on the film, trimmed, at a volume.
//! Like the Shots track's clips they are part of the film, so every change
//! is one undo step with the film before and after. The sound itself is a
//! file in the project's `sounds/` folder, which `crate::sounds` decodes;
//! the film keeps only where and how it plays.
//!
//! Every place, trim, and length is in film frames, as `media::Track` takes
//! them. Sound clips on one track never overlap.

use crate::film::Film;
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct SoundTrack {
    pub id: u64,
    pub name: String,
    pub locked: bool,
    pub muted: bool,
    /// In the order they start.
    pub clips: Vec<SoundClip>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundClip {
    pub id: u64,
    /// Its file, relative to the project.
    pub file: String,
    pub name: String,
    /// Where it starts on the film.
    pub at: u32,
    /// Where it starts inside its sound.
    pub from: u32,
    pub length: u32,
    /// The whole sound's length, which a trim never passes.
    pub source: u32,
    /// 0 to 1.
    pub volume: f64,
}

impl SoundClip {
    pub fn end(&self) -> u32 {
        self.at + self.length
    }
}

/// Which track a flag is set on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackRef {
    Shots,
    Sound(u64),
}

/// A track's switch the headers' toggles and the tools set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackFlag {
    Locked,
    /// The Shots track's eye.
    Visible,
    /// A Sound track's speaker.
    Muted,
}

impl TrackRef {
    /// The row id the timeline gives the track.
    pub fn row(self) -> String {
        match self {
            TrackRef::Shots => "shots".to_string(),
            TrackRef::Sound(id) => format!("sound-{id}"),
        }
    }

    pub fn from_row(row: &str) -> Option<Self> {
        if row == "shots" {
            return Some(TrackRef::Shots);
        }
        Some(TrackRef::Sound(row.strip_prefix("sound-")?.parse().ok()?))
    }
}

impl SoundTrack {
    fn sort(&mut self) {
        self.clips.sort_by_key(|clip| (clip.at, clip.id));
    }

    /// Whether `at .. at + length` is free.
    fn fits(&self, at: u32, length: u32) -> bool {
        self.clips.iter().all(|clip| clip.end() <= at || clip.at >= at + length)
    }

    /// Puts `clip` on the track. A sound it lands inside is cut where it
    /// starts; the sounds from there on move later to make room.
    fn insert_pushing(&mut self, clip: SoundClip) {
        let end = clip.end();
        for other in &mut self.clips {
            if other.at < clip.at && other.end() > clip.at {
                other.length = clip.at - other.at;
            }
        }
        let first_after = self.clips.iter().filter(|other| other.at >= clip.at).map(|other| other.at).min();
        if let Some(first_after) = first_after
            && first_after < end
        {
            let push = end - first_after;
            for other in self.clips.iter_mut().filter(|other| other.at >= clip.at) {
                other.at += push;
            }
        }
        self.clips.push(clip);
        self.sort();
    }

    /// Puts `clip` on the track over what is there: sounds it covers whole
    /// go, and those it covers in part are cut short.
    fn insert_replacing(&mut self, clip: SoundClip) {
        let (start, end) = (clip.at, clip.end());
        let mut kept = Vec::new();
        for mut other in std::mem::take(&mut self.clips) {
            let other_end = other.end();
            if other_end <= start || other.at >= end {
                kept.push(other);
            } else if other.at >= start && other_end <= end {
                // Covered whole: it goes.
            } else if other.at < start {
                // Its head stays; a sound covered in its middle keeps only that.
                other.length = start - other.at;
                kept.push(other);
            } else {
                let cut = end - other.at;
                other.from += cut;
                other.length -= cut;
                other.at = end;
                kept.push(other);
            }
        }
        self.clips = kept;
        self.clips.push(clip);
        self.sort();
    }
}

impl Film {
    pub fn sound_track(&self, id: u64) -> Option<&SoundTrack> {
        self.sound_tracks.iter().find(|track| track.id == id)
    }

    fn sound_track_mut(&mut self, id: u64) -> Option<&mut SoundTrack> {
        self.sound_tracks.iter_mut().find(|track| track.id == id)
    }

    /// A sound clip and the track it is on.
    pub fn sound(&self, id: u64) -> Option<(&SoundTrack, &SoundClip)> {
        self.sound_tracks
            .iter()
            .find_map(|track| Some((track, track.clips.iter().find(|clip| clip.id == id)?)))
    }

    fn sound_mut(&mut self, id: u64) -> Option<&mut SoundClip> {
        self.sound_tracks.iter_mut().find_map(|track| track.clips.iter_mut().find(|clip| clip.id == id))
    }

    /// Every sound clip, track by track.
    pub fn sounds(&self) -> impl Iterator<Item = (&SoundTrack, &SoundClip)> {
        self.sound_tracks.iter().flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
    }

    /// Whether a track is locked against changes.
    pub fn track_locked(&self, track: TrackRef) -> bool {
        match track {
            TrackRef::Shots => self.shots_locked,
            TrackRef::Sound(id) => self.sound_track(id).is_some_and(|track| track.locked),
        }
    }

    /// The first free name of "Sound", "Sound 2", and on.
    pub fn free_track_name(&self) -> String {
        let mut n = 1;
        loop {
            let name = if n == 1 { "Sound".to_string() } else { format!("Sound {n}") };
            if !self.sound_tracks.iter().any(|track| track.name == name) {
                return name;
            }
            n += 1;
        }
    }

    /// A new, empty Sound track under the others. Returns its id.
    pub fn add_sound_track(&mut self) -> u64 {
        let id = self.id();
        let name = self.free_track_name();
        self.sound_tracks.push(SoundTrack { id, name, locked: false, muted: false, clips: Vec::new() });
        id
    }

    pub fn rename_sound_track(&mut self, id: u64, name: &str) -> bool {
        let name = name.trim();
        let Some(track) = self.sound_track_mut(id) else {
            return false;
        };
        if name.is_empty() || track.name == name {
            return false;
        }
        track.name = name.to_string();
        true
    }

    /// Deletes a Sound track and the sounds on it.
    pub fn delete_sound_track(&mut self, id: u64) -> bool {
        let before = self.sound_tracks.len();
        self.sound_tracks.retain(|track| track.id != id);
        self.sound_tracks.len() != before
    }

    pub fn set_track_flag(&mut self, track: TrackRef, flag: TrackFlag, on: bool) -> bool {
        let field = match (track, flag) {
            (TrackRef::Shots, TrackFlag::Locked) => &mut self.shots_locked,
            (TrackRef::Shots, TrackFlag::Visible) => &mut self.shots_visible,
            (TrackRef::Sound(id), TrackFlag::Locked) => match self.sound_track_mut(id) {
                Some(track) => &mut track.locked,
                None => return false,
            },
            (TrackRef::Sound(id), TrackFlag::Muted) => match self.sound_track_mut(id) {
                Some(track) => &mut track.muted,
                None => return false,
            },
            _ => return false,
        };
        if *field == on {
            return false;
        }
        *field = on;
        true
    }

    /// Places a whole sound of `source` frames at `at`: on `track` when it
    /// fits there, or on the first unlocked Sound track it fits on, or on a
    /// new one. Returns the track's id and the sound clip's.
    pub fn place_sound(&mut self, track: Option<u64>, at: u32, file: &str, name: &str, source: u32) -> Option<(u64, u64)> {
        self.lay_sound(track, at, 0, source, source, file, name)
    }

    /// Places `length` frames of a sound of `source` frames, from its frame
    /// `from`, at `at`, by the same rule as `place_sound`: two sounds that
    /// overlap on the film go on two tracks.
    #[allow(clippy::too_many_arguments)]
    pub fn lay_sound(&mut self, track: Option<u64>, at: u32, from: u32, length: u32, source: u32, file: &str, name: &str) -> Option<(u64, u64)> {
        let source = source.max(1);
        let from = from.min(source - 1);
        let length = length.clamp(1, source - from);
        let fits = |track: &SoundTrack| !track.locked && track.fits(at, length);
        let chosen = track
            .and_then(|id| self.sound_track(id))
            .filter(|track| fits(track))
            .or_else(|| self.sound_tracks.iter().find(|track| fits(track)))
            .map(|track| track.id);
        let track = match chosen {
            Some(id) => id,
            None => self.add_sound_track(),
        };
        let id = self.id();
        let clip = SoundClip {
            id,
            file: file.to_string(),
            name: name.to_string(),
            at,
            from,
            length,
            source,
            volume: 1.0,
        };
        let track_ref = self.sound_track_mut(track)?;
        track_ref.clips.push(clip);
        track_ref.sort();
        Some((track, id))
    }

    /// Moves sounds by `frames`, and onto `to` when it is given. With `push`
    /// the sounds they land on move later; without, they are covered. A
    /// locked track neither gives nor takes a sound.
    pub fn move_sounds(&mut self, ids: &[u64], to: Option<u64>, frames: i64, push: bool) -> bool {
        if let Some(to) = to
            && self.sound_track(to).is_none_or(|track| track.locked)
        {
            return false;
        }
        let mut moving = Vec::new();
        for track in &self.sound_tracks {
            for clip in &track.clips {
                if ids.contains(&clip.id) && !track.locked {
                    moving.push((to.unwrap_or(track.id), clip.clone()));
                }
            }
        }
        let Some(earliest) = moving.iter().map(|(_, clip)| clip.at).min() else {
            return false;
        };
        let frames = frames.max(-i64::from(earliest));
        let changes_track = moving.iter().any(|(track, clip)| self.sound(clip.id).is_some_and(|(from, _)| from.id != *track));
        if frames == 0 && !changes_track {
            return false;
        }
        let moved: Vec<u64> = moving.iter().map(|(_, clip)| clip.id).collect();
        for track in &mut self.sound_tracks {
            track.clips.retain(|clip| !moved.contains(&clip.id));
        }
        moving.sort_by_key(|(_, clip)| clip.at);
        for (track, mut clip) in moving {
            clip.at = (i64::from(clip.at) + frames) as u32;
            let Some(track) = self.sound_track_mut(track) else {
                continue;
            };
            if push {
                track.insert_pushing(clip);
            } else {
                track.insert_replacing(clip);
            }
        }
        true
    }

    /// Moves one edge of a sound by `frames`. Its start moves where it
    /// starts on the film and inside its sound together; neither edge passes
    /// its sound's ends or the sounds beside it on its track.
    pub fn trim_sound(&mut self, id: u64, end_edge: bool, frames: i64) -> bool {
        let Some((track, clip)) = self.sound(id) else {
            return false;
        };
        if track.locked {
            return false;
        }
        let before = track.clips.iter().filter(|other| other.id != id && other.at < clip.at).map(SoundClip::end).max().unwrap_or(0);
        let after = track.clips.iter().filter(|other| other.id != id && other.at >= clip.end()).map(|other| other.at).min();
        let clip = clip.clone();
        let (at, from, length) = if end_edge {
            let mut most = i64::from(clip.source - clip.from);
            if let Some(after) = after {
                most = most.min(i64::from(after - clip.at));
            }
            (clip.at, clip.from, (i64::from(clip.length) + frames).clamp(1, most) as u32)
        } else {
            // How far the start may move: back to the sound's start or the
            // sound before, forward to one frame before the end.
            let least = -(i64::from(clip.from).min(i64::from(clip.at) - i64::from(before)));
            let shift = frames.clamp(least, i64::from(clip.length) - 1);
            (
                (i64::from(clip.at) + shift) as u32,
                (i64::from(clip.from) + shift) as u32,
                (i64::from(clip.length) - shift) as u32,
            )
        };
        if (at, from, length) == (clip.at, clip.from, clip.length) {
            return false;
        }
        let Some(target) = self.sound_mut(id) else {
            return false;
        };
        target.at = at;
        target.from = from;
        target.length = length;
        true
    }

    /// Sets where a sound starts on the film, where it starts in its sound,
    /// and its length at once, as the panel's rows and the trim tool do. The
    /// sound's ends hold, and it may not land on another sound.
    pub fn set_sound_span(&mut self, id: u64, at: Option<u32>, from: Option<u32>, length: Option<u32>) -> bool {
        let Some((track, clip)) = self.sound(id) else {
            return false;
        };
        if track.locked {
            return false;
        }
        let from = from.unwrap_or(clip.from).min(clip.source - 1);
        let length = length.unwrap_or(clip.length).clamp(1, clip.source - from);
        let at = at.unwrap_or(clip.at);
        let others_fit = track.clips.iter().filter(|other| other.id != id).all(|other| other.end() <= at || other.at >= at + length);
        if !others_fit || (at, from, length) == (clip.at, clip.from, clip.length) {
            return false;
        }
        let Some(target) = self.sound_mut(id) else {
            return false;
        };
        target.at = at;
        target.from = from;
        target.length = length;
        if let Some(track) = self.sound_tracks.iter_mut().find(|track| track.clips.iter().any(|clip| clip.id == id)) {
            track.sort();
        }
        true
    }

    pub fn set_sound_volume(&mut self, id: u64, volume: f64) -> bool {
        if !volume.is_finite() || self.sound(id).is_none_or(|(track, _)| track.locked) {
            return false;
        }
        let volume = volume.clamp(0.0, 1.0);
        match self.sound_mut(id) {
            Some(clip) if clip.volume != volume => {
                clip.volume = volume;
                true
            }
            _ => false,
        }
    }

    pub fn delete_sound(&mut self, id: u64) -> bool {
        if self.sound(id).is_none_or(|(track, _)| track.locked) {
            return false;
        }
        for track in &mut self.sound_tracks {
            track.clips.retain(|clip| clip.id != id);
        }
        true
    }

    /// The highest id the Sound tracks hold.
    pub(crate) fn highest_sound_id(&self) -> u64 {
        self.sound_tracks
            .iter()
            .flat_map(|track| std::iter::once(track.id).chain(track.clips.iter().map(|clip| clip.id)))
            .max()
            .unwrap_or(0)
    }
}

/// A Sound track as `.smx` version 3 writes it.
pub fn track_value(track: &SoundTrack) -> Value {
    json!({
        "kind": "sound",
        "id": track.id,
        "name": track.name,
        "locked": track.locked,
        "muted": track.muted,
        "clips": track.clips.iter().map(|clip| json!({
            "id": clip.id,
            "file": clip.file,
            "name": clip.name,
            "at": clip.at,
            "from": clip.from,
            "length": clip.length,
            "source": clip.source,
            "volume": clip.volume,
        })).collect::<Vec<_>>(),
    })
}

pub fn track_from(value: &Value) -> Option<SoundTrack> {
    let object = value.as_object()?;
    let number = |object: &Map<String, Value>, key: &str| object.get(key).and_then(Value::as_u64);
    let mut clips = Vec::new();
    for clip in object.get("clips").and_then(Value::as_array).cloned().unwrap_or_default() {
        let clip = clip.as_object()?;
        let source = (number(clip, "source")? as u32).max(1);
        let from = (number(clip, "from")? as u32).min(source - 1);
        clips.push(SoundClip {
            id: number(clip, "id")?,
            file: clip.get("file")?.as_str()?.to_string(),
            name: clip.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            at: number(clip, "at")? as u32,
            from,
            length: (number(clip, "length")? as u32).clamp(1, source - from),
            source,
            volume: clip.get("volume").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 1.0),
        });
    }
    let mut track = SoundTrack {
        id: number(object, "id")?,
        name: object.get("name").and_then(Value::as_str).unwrap_or("Sound").to_string(),
        locked: object.get("locked").and_then(Value::as_bool).unwrap_or(false),
        muted: object.get("muted").and_then(Value::as_bool).unwrap_or(false),
        clips,
    };
    track.sort();
    Some(track)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn film_with_sound(source: u32) -> (Film, u64, u64) {
        let mut film = Film::new("Test");
        let (track, clip) = film.place_sound(None, 10, "sounds/a.wav", "a.wav", source).unwrap();
        (film, track, clip)
    }

    #[test]
    fn a_sound_lands_on_a_track_with_room() {
        let (mut film, track, _) = film_with_sound(100);
        assert_eq!(film.sound_tracks.len(), 1);
        assert_eq!(film.sound_track(track).unwrap().name, "Sound");
        // Overlapping the first: a second track, named on.
        let (second, _) = film.place_sound(Some(track), 50, "sounds/b.wav", "b.wav", 20).unwrap();
        assert_ne!(second, track);
        assert_eq!(film.sound_track(second).unwrap().name, "Sound 2");
        // After it, on the first again.
        let (third, _) = film.place_sound(None, 200, "sounds/c.wav", "c.wav", 20).unwrap();
        assert_eq!(third, track);
    }

    #[test]
    fn trims_stop_at_the_sound_and_its_neighbours() {
        let (mut film, track, clip) = film_with_sound(100);
        assert!(film.trim_sound(clip, true, -40));
        assert_eq!(film.sound(clip).unwrap().1.length, 60);
        // Not past the sound's end.
        assert!(film.trim_sound(clip, true, 500));
        assert_eq!(film.sound(clip).unwrap().1.length, 100);
        // The start moves on the film and in the sound together.
        assert!(film.trim_sound(clip, false, 25));
        let trimmed = film.sound(clip).unwrap().1.clone();
        assert_eq!((trimmed.at, trimmed.from, trimmed.length), (35, 25, 75));
        // Back no further than the sound's start.
        assert!(film.trim_sound(clip, false, -100));
        let back = film.sound(clip).unwrap().1.clone();
        assert_eq!((back.at, back.from, back.length), (10, 0, 100));
        // A neighbour stops the end.
        film.place_sound(Some(track), 150, "sounds/b.wav", "b.wav", 30).unwrap();
        assert!(film.trim_sound(clip, true, -50));
        assert!(film.trim_sound(clip, true, 500));
        assert_eq!(film.sound(clip).unwrap().1.end(), 110);
    }

    #[test]
    fn moving_pushes_or_covers_on_the_track_it_lands_on() {
        // The first sound plays 10..50, the second 60..100.
        let (mut film, track, first) = film_with_sound(40);
        let (_, second) = film.place_sound(Some(track), 60, "sounds/b.wav", "b.wav", 40).unwrap();
        // Pushed: the first lands at 50..90, and the second moves to 90.
        assert!(film.move_sounds(&[first], None, 40, true));
        assert_eq!(film.sound(first).unwrap().1.at, 50);
        assert_eq!(film.sound(second).unwrap().1.at, 90);
        // Covering: the first lands at 70..110 over the second's head, which
        // then starts 20 frames into its sound.
        assert!(film.move_sounds(&[first], None, 20, false));
        let cut = film.sound(second).unwrap().1.clone();
        assert_eq!((cut.at, cut.from, cut.length), (110, 20, 20));
        // Onto another track.
        let other = film.add_sound_track();
        assert!(film.move_sounds(&[first], Some(other), 0, true));
        assert_eq!(film.sound(first).unwrap().0.id, other);
    }

    #[test]
    fn a_locked_track_keeps_its_sounds() {
        let (mut film, track, clip) = film_with_sound(40);
        assert!(film.set_track_flag(TrackRef::Sound(track), TrackFlag::Locked, true));
        assert!(!film.move_sounds(&[clip], None, 10, true));
        assert!(!film.trim_sound(clip, true, -5));
        assert!(!film.set_sound_volume(clip, 0.5));
        assert!(!film.delete_sound(clip));
        // The Shots track has no speaker, and a Sound track no eye.
        assert!(!film.set_track_flag(TrackRef::Shots, TrackFlag::Muted, true));
        assert!(!film.set_track_flag(TrackRef::Sound(track), TrackFlag::Visible, false));
    }

    #[test]
    fn rows_name_tracks_both_ways() {
        assert_eq!(TrackRef::from_row(&TrackRef::Sound(12).row()), Some(TrackRef::Sound(12)));
        assert_eq!(TrackRef::from_row("shots"), Some(TrackRef::Shots));
        assert_eq!(TrackRef::from_row("7"), None);
    }
}
