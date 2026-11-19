//! The take's own sound and the tab's sound layers, as vg-layers shows
//! them: the Layers page lists the take shown as its linked layer, its eye
//! muting its own sound, then each sound layer, a link to a sound file it
//! never writes, with an eye that mutes it, its gain, and its waveform; the
//! Properties page holds the chosen one's Gain, and a sound layer's Starts
//! at. Each change is one step of the gallery's history. Each heard sound
//! layer is a waveform row under the scrubber, and the take plays with its
//! own sound and every heard layer mixed through `media`'s Player. The
//! values are kept plain, so the mix's script takes them as they are
//! (`mix_args`).

use super::*;
use crate::gallery::{GAIN_DB, SoundLayer, TakeSound};
use protocol::{Action, LayerRow, MenuEntry, PropValue, Row, RowKind, Section, TransportRow};
use serde_json::json;

/// The Layers page's + and the filmstrip's + tile's row, pending the
/// owner's word.
pub const OPEN_SOUND: &str = "Open sound as layer…";
/// The Properties page's rows: Gain, pending the owner's word, and Starts
/// at, as the Storyboard Generator's Sound page says it.
pub const GAIN: &str = "Gain";
pub const STARTS_AT: &str = "Starts at";
/// The sound files a layer can be.
pub const SOUND_EXTENSIONS: [&str; 3] = ["wav", "mp3", "flac"];
/// A sound layer's thumbnail on its row, in pixels.
const THUMB: (u32, u32) = (48, 24);

/// The panel's pages: Layers, Properties, and Prompt. Adjustments waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    Layers,
    Properties,
    #[default]
    Prompt,
}

/// The key a sound layer's sound is decoded and kept by.
pub(super) fn layer_key(layer: &SoundLayer) -> String {
    if layer.link.sha256.is_empty() { format!("layer {}", layer.file.display()) } else { format!("layer {}", layer.link.sha256) }
}

/// A gain in decibels as the Player's volume.
pub fn volume(gain_db: f64) -> f32 {
    10f64.powf(gain_db / 20.0) as f32
}

/// A gain as a layer's row shows it at its right, as vg-layers does: "0 dB",
/// "−6 dB", "+1.5 dB".
fn gain_words(gain_db: f64) -> String {
    let tenths = (gain_db * 10.0).round() / 10.0;
    if tenths == 0.0 {
        return String::from("0 dB");
    }
    let sign = if tenths < 0.0 { "−" } else { "+" };
    format!("{sign}{} dB", tenths.abs())
}

/// One sound as the mix's script takes it: its file in the project, its
/// gain in decibels, its start in seconds, and whether it is muted.
#[derive(Debug, Clone, PartialEq)]
pub struct MixSound {
    pub path: String,
    pub gain_db: f64,
    pub start: f64,
    pub muted: bool,
}

/// The mix's arguments, as the build-animatic skill's `mix.py` takes them:
/// `--take <take>`; `--take-sound <gain_db>` when the take has its own
/// sound, `take_sound`, and it is not muted, since `mix.py` refuses it for a
/// take with none; and for each sound layer `--sound <file> <gain_db>
/// <start>`, then `muted` when its eye is off. Gains in decibels, starts in
/// seconds.
pub fn mix_args(take: &str, take_sound: Option<TakeSound>, sounds: &[MixSound]) -> Vec<String> {
    let mut args = vec![String::from("--take"), take.to_string()];
    if let Some(own) = take_sound.filter(|own| !own.muted) {
        args.push(String::from("--take-sound"));
        args.push(own.gain_db.to_string());
    }
    for sound in sounds {
        args.extend([String::from("--sound"), sound.path.clone(), sound.gain_db.to_string(), sound.start.to_string()]);
        if sound.muted {
            args.push(String::from("muted"));
        }
    }
    args
}

fn row(label: &str, action: Action) -> MenuEntry {
    MenuEntry { label: label.into(), icon: None, enabled: true, divider: false, action: Some(action), submenu: Vec::new(), rename: false, tip: None }
}

impl Session {
    /// What the take shown is, alone or first of two.
    fn shown_info(&self) -> Option<media::Info> {
        let tab = self.active_tab()?;
        let shown = tab.round_view();
        shown.as_ref().unwrap_or(&tab.gallery).take()?.info
    }

    /// Whether the take shown has a sound of its own.
    pub fn take_has_sound(&self) -> bool {
        self.shown_info().is_some_and(|info| info.sound)
    }

    /// The latest a sound layer can start: the take's last frame, since the
    /// mix refuses a sound starting at or after the take's end.
    fn latest_start(&self) -> Option<f64> {
        let info = self.shown_info()?;
        Some(f64::from(info.frames.saturating_sub(1)) / info.fps().max(f64::EPSILON))
    }

    /// Open sound as layer…: the host's file dialog, for a WAV, MP3, or
    /// FLAC.
    pub(super) fn open_sound(&mut self, out: &mut Vec<AppMessage>) {
        let Some(tab) = self.active_tab().map(|tab| tab.id.clone()) else {
            return;
        };
        let filter = FileFilter { name: "Sounds".into(), extensions: SOUND_EXTENSIONS.iter().map(|extension| extension.to_string()).collect() };
        self.ask_file(FileKind::Open, None, Purpose::AddSound { tab }, filter, out);
    }

    /// A sound layer linked to `path`, named after it, free among the
    /// tab's layers, added at the top: one step. The file is read for its
    /// sha256, never written.
    pub(super) fn add_sound(&mut self, tab: &str, path: &Path) -> Result<(), String> {
        let extension = path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_lowercase();
        if !SOUND_EXTENSIONS.contains(&extension.as_str()) {
            return Err("Only WAV, MP3, and FLAC sounds are layers.".into());
        }
        let file = path.canonicalize().map_err(|error| error.to_string())?;
        let sha256 = layers::link::sha256_file(&file).map_err(|error| error.to_string())?;
        let link = Link { path: files::link_path(&file, self.project.as_deref()), sha256 };
        let Some(index) = self.tab_index(tab) else {
            return Ok(());
        };
        self.stop_playing();
        let tab = &mut self.tabs[index];
        let name = protocol::free_name(&file_title(&file), |name| tab.gallery.sounds.iter().any(|sound| sound.name == name));
        let id = tab.gallery.next_sound();
        let mut sounds = tab.gallery.sounds.clone();
        sounds.insert(0, SoundLayer { id, name, link, file, gain_db: 0.0, start: 0.0, muted: false });
        if tab.gallery.set_sounds(sounds) {
            tab.dirty = true;
        }
        tab.sound = Some(id);
        Ok(())
    }

    /// The Layers page's actions on the take's row, id 0, and the sound
    /// layers' rows: choose, rename, mute, delete, and reorder. Each change
    /// but choosing is one step. Returns whether `id` was one of them.
    pub(super) fn mix_command(&mut self, id: &str, arg: &Value) -> bool {
        let sound = arg.get("sound").or_else(|| arg.get("node")).and_then(Value::as_u64);
        let Some(index) = self.tabs.get(self.active).map(|_| self.active) else {
            return false;
        };
        let (take_sound, mut sounds) = (self.tabs[index].gallery.take_sound, self.tabs[index].gallery.sounds.clone());
        let mut take = take_sound;
        let has_sound = self.take_has_sound();
        match id {
            "sound_click" => {
                self.tabs[index].sound = sound;
                return true;
            }
            "sound_mute" => match sound {
                // A take with no sound of its own has nothing to mute.
                Some(0) if !has_sound => return true,
                Some(0) => take.muted = !take.muted,
                Some(own) => {
                    if let Some(layer) = sounds.iter_mut().find(|layer| layer.id == own) {
                        layer.muted = !layer.muted;
                    }
                }
                None => return true,
            },
            "sound_rename" => {
                let name = arg.get("value").and_then(Value::as_str).map(str::trim).unwrap_or("");
                if let (Some(own), false) = (sound.filter(|own| *own != 0), name.is_empty())
                    && let Some(layer) = sounds.iter_mut().find(|layer| layer.id == own)
                {
                    layer.name = name.to_string();
                }
            }
            "sound_delete" => {
                let own = sound.or(self.tabs[index].sound).filter(|own| *own != 0);
                sounds.retain(|layer| Some(layer.id) != own);
            }
            "sound_reorder" => {
                // The place among the other rows, the take's above them all.
                let to = arg.get("value").and_then(Value::as_f64).map(|value| value.max(0.0) as usize);
                let above = usize::from(self.tabs[index].round_view().as_ref().unwrap_or(&self.tabs[index].gallery).take().is_some());
                if let (Some(own), Some(to)) = (sound.filter(|own| *own != 0), to)
                    && let Some(from) = sounds.iter().position(|layer| layer.id == own)
                {
                    let layer = sounds.remove(from);
                    sounds.insert(to.saturating_sub(above).min(sounds.len()), layer);
                }
            }
            "sound_lock" => return true,
            _ => return false,
        }
        self.stop_playing();
        let tab = &mut self.tabs[index];
        if tab.gallery.set_mix(take, sounds) {
            tab.dirty = true;
        }
        if tab.sound.is_some_and(|own| own != 0 && !tab.gallery.sounds.iter().any(|layer| layer.id == own)) {
            tab.sound = None;
        }
        true
    }

    /// A Gain or Starts at edit on the Properties page: while a slider is
    /// held it is heard at the next play, with no step; when it is let go,
    /// the whole drag is one step.
    pub(super) fn mix_property(&mut self, prop: &Value, value: &Value, preview: bool) {
        let (Some(own), Some(field), Some(number)) = (prop.get("mix").and_then(Value::as_u64), prop.get("field").and_then(Value::as_str), value.as_f64())
        else {
            return;
        };
        let (latest, has_sound) = (self.latest_start(), self.take_has_sound());
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let before = tab.mix_before.clone().unwrap_or((tab.gallery.take_sound, tab.gallery.sounds.clone()));
        let (mut take, mut sounds) = (tab.gallery.take_sound, tab.gallery.sounds.clone());
        match (own, field) {
            (0, "gain_db") if has_sound => take.gain_db = number.clamp(GAIN_DB.0, GAIN_DB.1),
            (0, _) => return,
            (_, "gain_db") | (_, "start") => {
                let Some(layer) = sounds.iter_mut().find(|layer| layer.id == own) else {
                    return;
                };
                if field == "gain_db" {
                    layer.gain_db = number.clamp(GAIN_DB.0, GAIN_DB.1);
                } else {
                    layer.start = number.clamp(0.0, latest.unwrap_or(f64::MAX).max(0.0));
                }
            }
            _ => return,
        }
        if preview {
            tab.mix_before = Some(before);
            tab.gallery.take_sound = take;
            tab.gallery.sounds = sounds;
            return;
        }
        tab.mix_before = None;
        (tab.gallery.take_sound, tab.gallery.sounds) = before;
        if tab.gallery.set_mix(take, sounds) {
            tab.dirty = true;
        }
    }

    /// A slider let go with no last value: what it was dragged to becomes
    /// one step.
    pub(super) fn finish_mix(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(before) = tab.mix_before.take() else {
            return;
        };
        let now = (tab.gallery.take_sound, tab.gallery.sounds.clone());
        (tab.gallery.take_sound, tab.gallery.sounds) = before;
        if tab.gallery.set_mix(now.0, now.1) {
            tab.dirty = true;
        }
    }

    /// The Layers page's rows, as vg-layers shows them: the take shown, its
    /// linked layer, at the top, with its filmstrip thumbnail, its file's
    /// name under its name, and its eye muting its own sound, off with no
    /// gain for a take with none; then each sound layer with its eye, its
    /// waveform, and its gain.
    pub fn layer_rows(&self) -> Vec<LayerRow> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let shown = tab.round_view();
        let take_sound = tab.gallery.take_sound;
        let target = |own: u64| json!({ "sound": own });
        let layer_row = |id: u64, icon: &str, name: &str, visible: bool| LayerRow {
            dim: false,
            outlined: false,
            id,
            parent: None,
            depth: 0,
            icon: icon.into(),
            name: name.to_string(),
            group: false,
            selected: tab.sound == Some(id),
            visible,
            locked: false,
            renamable: id != 0,
            click: Action::with_arg("sound_click", target(id)),
            rename: Action::with_arg("sound_rename", target(id)),
            toggle_visible: Action::with_arg("sound_mute", target(id)),
            toggle_locked: Action::with_arg("sound_lock", target(id)),
            menu: Vec::new(),
            colors: Vec::new(),
            thumbnail: None,
            mark: None,
            detail: None,
            toggle_icon: None,
            // The eye mutes: its tip says what a click does now.
            toggle_tip: Some(String::from(if visible { "Mute" } else { "Unmute" })),
            value: None,
        };
        if let Some(take) = shown.as_ref().unwrap_or(&tab.gallery).take() {
            let sounding = take.info.is_some_and(|info| info.sound);
            let mut take_row = layer_row(0, "media-video", &take.name, sounding && !take_sound.muted);
            take_row.mark = Some(String::from("link"));
            take_row.thumbnail = self.films.get(&take.file).cloned();
            take_row.detail = take.file.file_name().map(|name| name.to_string_lossy().into_owned());
            take_row.value = sounding.then(|| gain_words(take_sound.gain_db));
            rows.push(take_row);
        }
        for layer in &tab.gallery.sounds {
            let mut sound_row = layer_row(layer.id, "sound-high", &layer.name, !layer.muted);
            sound_row.thumbnail = self.playback.waves.get(&(format!("{} thumb", layer_key(layer)), THUMB.0)).cloned();
            sound_row.value = Some(gain_words(layer.gain_db));
            sound_row.menu = vec![row("Delete", Action::with_arg("sound_delete", target(layer.id)))];
            rows.push(sound_row);
        }
        rows
    }

    /// The Properties page for the row chosen: the take's Gain, when it has
    /// a sound of its own, or a sound layer's Gain and Starts at, up to the
    /// take's last frame.
    pub fn mix_section(&self) -> Option<Section> {
        let tab = self.active_tab()?;
        let own = tab.sound?;
        let prop = |field: &str| json!({ "mix": own, "field": field });
        let gain = |gain_db: f64| Row {
            prop: prop("gain_db"),
            label: GAIN.into(),
            kind: RowKind::Number { min: GAIN_DB.0, max: GAIN_DB.1, percent: false, places: 1, centered: true },
            value: PropValue::Number(gain_db),
        };
        if own == 0 {
            if !self.take_has_sound() {
                return None;
            }
            return Some(Section { title: String::from("Sound"), rows: vec![gain(tab.gallery.take_sound.gain_db)], icon: None });
        }
        let layer = tab.gallery.sounds.iter().find(|layer| layer.id == own)?;
        let start = Row {
            prop: prop("start"),
            label: STARTS_AT.into(),
            kind: RowKind::Field { min: 0.0, max: self.latest_start().unwrap_or(3600.0).max(0.0), places: 2 },
            value: PropValue::Number(layer.start),
        };
        Some(Section { title: layer.name.clone(), rows: vec![gain(layer.gain_db), start], icon: None })
    }

    /// Asks for the sound of each of the tab's sound layers, once for each
    /// file.
    pub(super) fn want_layer_sounds(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let wanted: Vec<(String, PathBuf)> = tab.gallery.sounds.iter().map(|layer| (layer_key(layer), layer.file.clone())).collect();
        for (key, file) in wanted {
            self.playback.sounds.want(&key, &file);
        }
    }

    /// The sound layers' tracks, as the Player mixes them with the take's
    /// own: each heard layer from its start, at its gain, for its whole
    /// length, in the take's frames.
    pub(super) fn layer_tracks(&self, fps: f64) -> Vec<media::Track> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        tab.gallery
            .sounds
            .iter()
            .filter(|layer| !layer.muted)
            .filter_map(|layer| {
                let pcm = self.playback.sounds.pcm(&layer_key(layer))?;
                let seconds = pcm.len() as f64 / f64::from(pcm.rate.max(1));
                Some(media::Track {
                    at: (layer.start.max(0.0) * fps).round() as u32,
                    from: 0,
                    length: (seconds * fps).ceil() as u32,
                    volume: volume(layer.gain_db),
                    pcm,
                })
            })
            .collect()
    }

    /// Writes each sound layer's waveform once decoded: its row's
    /// thumbnail, and, while heard, its row under the scrubber at `width`,
    /// drawn from its start on the take for its length.
    pub(super) fn prepare_layer_waves(&mut self, width: u32, length: Option<f64>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let mut wanted = Vec::new();
        for layer in &tab.gallery.sounds {
            let key = layer_key(layer);
            let Some(crate::waveform::Heard::Sound { peaks, pcm }) = self.playback.sounds.ready.get(&key) else {
                continue;
            };
            let thumb = format!("{key} thumb");
            if !self.playback.waves.contains_key(&(thumb.clone(), THUMB.0)) {
                wanted.push((thumb, THUMB.0, crate::waveform::draw_sized(peaks, THUMB.0, THUMB.1)));
            }
            if let Some(length) = length.filter(|length| *length > 0.0) {
                let row = row_key(&key, layer.start, length, layer.muted);
                if !self.playback.waves.contains_key(&(row.clone(), width)) {
                    let seconds = pcm.len() as f64 / f64::from(pcm.rate.max(1));
                    let mut image = crate::waveform::draw_span(peaks, width, crate::waveform::HEIGHT, layer.start / length, (layer.start + seconds) / length);
                    // A muted layer keeps its row, fainter.
                    if layer.muted {
                        for pixel in image.data.chunks_exact_mut(4) {
                            pixel[3] /= 2;
                        }
                    }
                    wanted.push((row, width, image));
                }
            }
        }
        let _ = std::fs::create_dir_all(&self.cache);
        for (key, width, image) in wanted {
            let name = format!("wave-{}.rgba", self.playback.next_wave);
            self.playback.next_wave += 1;
            if let Ok(path) = media::write_frame(&self.cache, &name, &image.data, (image.width, image.height)) {
                self.next_revision += 1;
                let thumb = Thumbnail { path: path.display().to_string(), width: image.width, height: image.height, revision: self.next_revision };
                self.playback.waves.insert((key, width), thumb);
            }
        }
    }

    /// The transport's rows after the take's own "Sound": each sound
    /// layer's, labelled with its name, once written, a muted one at half
    /// its alpha, so every sound layer is a row.
    pub fn layer_rows_under(&self, width: u32, length: f64) -> Vec<TransportRow> {
        let Some(tab) = self.active_tab() else {
            return Vec::new();
        };
        tab.gallery
            .sounds
            .iter()
            .filter_map(|layer| {
                let image = self.playback.waves.get(&(row_key(&layer_key(layer), layer.start, length, layer.muted), width))?.clone();
                Some(TransportRow { label: layer.name.clone(), image })
            })
            .collect()
    }
}

/// A sound layer's row under the scrubber, known by its sound, its start,
/// and the take's length, which place it.
fn row_key(key: &str, start: f64, length: f64, muted: bool) -> String {
    format!("{key} at {start:.3} of {length:.3}{}", if muted { " muted" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-second take with a steady tone as its own sound.
    fn sounding_take(dir: &Path) -> PathBuf {
        let path = dir.join("Tone.mp4");
        let settings = media::Settings { width: 64, height: 36, rate: (24, 1), keyframes: 12, sound: Some((48_000, 2)) };
        let mut encoder = media::Encoder::create(&path, settings).unwrap();
        let tone: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let sample = 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin();
                [sample, sample]
            })
            .collect();
        for i in 0..24 {
            encoder.frame(&[90, 90, 90, 255].repeat(64 * 36)).unwrap();
            encoder.sound(&tone[i * 4000..(i + 1) * 4000]).unwrap();
        }
        encoder.finish().unwrap();
        path
    }

    /// A WAV of half a second at 8 000 samples a second, 16-bit mono, held
    /// at half of full scale.
    fn steady_wav(path: &Path) {
        let rate = 8000u32;
        let mut fmt = Vec::new();
        for value in [1u16, 1] {
            fmt.extend_from_slice(&value.to_le_bytes());
        }
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * 2).to_le_bytes());
        fmt.extend_from_slice(&2u16.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        let samples: Vec<u8> = (0..rate / 2).flat_map(|_| 16_384i16.to_le_bytes()).collect();
        let mut body = b"WAVE".to_vec();
        for (id, data) in [(b"fmt ", fmt), (b"data", samples)] {
            body.extend_from_slice(id);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend(data);
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        std::fs::write(path, out).unwrap();
    }

    /// The loudest sample from `from` to `to` seconds of a stereo mix at
    /// the session's rate.
    fn loudest(mix: &[f32], from: f64, to: f64) -> f32 {
        let at = |seconds: f64| ((seconds * f64::from(crate::waveform::RATE)) as usize * 2).min(mix.len());
        mix[at(from)..at(to)].iter().fold(0.0f32, |most, sample| most.max(sample.abs()))
    }

    /// Catches a sound layer or the take's own sound heard other than as
    /// set: a layer starting at half a second at -6 dB is heard only from
    /// then, at half its level; the take's own sound muted is not heard,
    /// and at -6 dB is heard at half its level; a mute undone is heard
    /// again. What the Player mixes is `media::mix` over these tracks.
    #[test]
    fn the_mix_is_heard_at_each_layers_start_gain_and_mute() {
        let dir = std::env::temp_dir().join(format!("video-generator-mix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("rain.wav");
        steady_wav(&wav);
        let mut session = Session::launch();
        session.add_take(&sounding_take(&dir), None).unwrap();
        let tab = session.tabs[0].id.clone();
        session.add_sound(&tab, &wav).unwrap();
        let layer = session.tabs[0].gallery.sounds[0].id;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            session.want_sound();
            session.poll_sounds();
            let take = session.playing_take().unwrap();
            if session.playback.sounds.pcm(&take.key).is_some() && session.playback.sounds.pcm(&layer_key(&session.tabs[0].gallery.sounds[0])).is_some() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the sounds were never decoded");
            std::thread::sleep(Duration::from_millis(20));
        }
        let mix = |session: &Session| {
            let take = session.playing_take().unwrap();
            media::mix(&session.tracks(&take), 0, 24, crate::waveform::RATE, (24, 1))
        };
        let gain = json!({ "mix": layer, "field": "gain_db" });
        session.mix_property(&json!({ "mix": layer, "field": "start" }), &json!(0.5), false);
        assert!(session.mix_command("sound_mute", &json!({ "sound": 0 })));
        // Its level as decoded first, since a mono file is spread to both
        // channels a little quieter: the gain is measured against it.
        let level_at_0 = loudest(&mix(&session), 0.6, 0.9);
        assert!(level_at_0 > 0.1, "the layer is not heard: {level_at_0}");
        session.mix_property(&gain, &json!(-6.0), false);
        let heard = mix(&session);
        assert!(loudest(&heard, 0.05, 0.45) < 0.01, "heard before its start, or the muted take was heard");
        let level = loudest(&heard, 0.6, 0.9);
        assert!((level / level_at_0 - 0.501).abs() < 0.02, "the layer at -6 dB is not at half its level: {level} of {level_at_0}");

        // The take's own sound alone, at 0 dB and then at -6 dB.
        assert!(session.mix_command("sound_mute", &json!({ "sound": layer })));
        assert!(session.mix_command("sound_mute", &json!({ "sound": 0 })));
        let full = loudest(&mix(&session), 0.1, 0.4);
        assert!(full > 0.3, "the take's own sound is not heard: {full}");
        session.mix_property(&json!({ "mix": 0, "field": "gain_db" }), &json!(-6.0), false);
        let half = loudest(&mix(&session), 0.1, 0.4);
        assert!((half / full - 0.501).abs() < 0.02, "the take's own sound at -6 dB is not at half: {half} of {full}");

        // Undone back to the take muted and the layer heard again, from its
        // start at -6 dB.
        session.step_history(false);
        session.step_history(false);
        session.step_history(false);
        assert!(!session.tabs[0].gallery.sounds[0].muted && session.tabs[0].gallery.take_sound.muted);
        let again = mix(&session);
        assert!(loudest(&again, 0.05, 0.45) < 0.01, "after the undo, heard before its start, or the muted take was heard");
        let level = loudest(&again, 0.6, 0.9);
        assert!((level / level_at_0 - 0.501).abs() < 0.02, "after the undo, the layer is not heard at -6 dB: {level} of {level_at_0}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Catches a mix's arguments other than the script takes them: the
    /// take, its own sound at -3 dB, each layer's file, gain, and start,
    /// and `muted` after a muted one; and no `--take-sound` when the take's
    /// own sound is muted, or the take has none.
    #[test]
    fn the_mix_takes_its_arguments_plainly() {
        let sounds = [
            MixSound { path: "sounds/voice.wav".into(), gain_db: 0.0, start: 0.0, muted: false },
            MixSound { path: "sounds/door.wav".into(), gain_db: -6.5, start: 3.0, muted: true },
        ];
        let args = mix_args("library/take-60fps-4k.mp4", Some(TakeSound { gain_db: -3.0, muted: false }), &sounds);
        assert_eq!(
            args,
            [
                "--take", "library/take-60fps-4k.mp4", "--take-sound", "-3", "--sound", "sounds/voice.wav", "0", "0", "--sound", "sounds/door.wav", "-6.5", "3",
                "muted",
            ]
        );
        let quiet = mix_args("a.mp4", Some(TakeSound { gain_db: 0.0, muted: true }), &[]);
        assert_eq!(quiet, ["--take", "a.mp4"]);
        // A take with no sound of its own never asks for it, muted or not.
        let silent = mix_args("a.mp4", None, &[]);
        assert_eq!(silent, ["--take", "a.mp4"]);
        assert_eq!((gain_words(-6.0), gain_words(1.5), gain_words(0.0)), ("−6 dB".to_string(), "+1.5 dB".to_string(), "0 dB".to_string()));
        assert!((volume(-6.0) - 0.501).abs() < 0.001);
    }
}
