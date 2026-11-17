//! The storyboard and a Skills project's animatic step, through the host:
//! finding the step whose `build-animatic` script runs `assemble.py`,
//! opening the assembly it names, saving the film's timing as a new
//! assembly that the step is pointed at, and following a retime made in
//! another tool. The host runs Skills' scripts; this asks and answers.

use super::Session;
use crate::assembly::{Assembly, Notes};
use protocol::{Action, AppMessage, Notice, NoticeAction, NoticeState};
use serde_json::Value;
use std::time::{Duration, Instant};

/// How often an open animatic asks whether it was retimed elsewhere.
pub const CHECK_EVERY: Duration = Duration::from_secs(5);

/// The columns of one answer row, joined by the unit separator so a script
/// or a reference never splits a row.
const SEPARATOR: char = '\u{1f}';

/// The project's animatics: each `build-animatic` step whose script runs
/// `assemble.py`, by its chain and name, with its script.
const STEPS: &str = "SELECT chain || char(31) || name || char(31) || script FROM step WHERE skill = 'build-animatic' AND script LIKE '%assemble.py%' ORDER BY chain, ord";
/// Every published file and the artifact it is published as.
const ARTIFACTS: &str = "SELECT file || char(31) || id FROM artifact WHERE file IS NOT NULL";

/// One animatic step.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimaticStep {
    pub chain: String,
    pub name: String,
    pub script: String,
}

impl AnimaticStep {
    /// How this session keys it: "chain: name".
    pub fn key(&self) -> String {
        format!("{}: {}", self.chain, self.name)
    }

    /// How the host's step messages name it.
    pub fn step_ref(&self) -> protocol::StepRef {
        protocol::StepRef { chain: self.chain.clone(), step: self.name.clone() }
    }
}

/// `write-assembly.py`'s arguments for an assembly: every time as it reads,
/// so a time written back unchanged is the same number in the file.
pub fn writer_args(assembly: &Assembly) -> Vec<String> {
    let mut args = vec![
        "--length".to_string(),
        assembly.length.to_string(),
        "--fps".to_string(),
        assembly.fps.to_string(),
        "--size".to_string(),
        format!("{}x{}", assembly.size[0], assembly.size[1]),
    ];
    for frame in &assembly.frames {
        args.extend(["--frame".to_string(), frame.image.clone(), frame.start.to_string(), frame.end.to_string()]);
    }
    args.extend(["--bed".to_string(), assembly.bed.audio.clone(), assembly.bed.gain_db.to_string()]);
    for clip in &assembly.clips {
        args.extend(["--clip".to_string(), clip.audio.clone(), clip.from.to_string(), clip.to.to_string(), clip.at.to_string(), clip.gain_db.to_string()]);
    }
    args.extend(["--name".to_string(), "animatic-assembly".to_string(), "--json".to_string()]);
    args
}

/// What a canon read was for.
#[derive(Clone, Debug)]
pub enum Asked {
    /// Open animatic, of the step named, or the first.
    Open(Option<String>),
    /// Whether the open animatic was retimed elsewhere.
    Check,
    Artifacts,
}

/// A Save timing to the project under way.
#[derive(Clone, Debug)]
pub struct Saving {
    pub assembly: Assembly,
    pub notes: Notes,
    pub refs: Vec<Value>,
    pub step: AnimaticStep,
    /// Waiting for Skills' writer to file it, then for the step's edit.
    pub file: Option<String>,
    pub waiting: String,
}

fn row_text(row: &Value) -> Option<String> {
    match row {
        Value::String(text) => Some(text.clone()),
        Value::Array(cells) => cells.first().and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

fn steps_of(rows: &[Value]) -> Vec<AnimaticStep> {
    rows.iter()
        .filter_map(row_text)
        .filter_map(|text| {
            let mut parts = text.splitn(3, SEPARATOR);
            Some(AnimaticStep { chain: parts.next()?.to_string(), name: parts.next()?.to_string(), script: parts.next()?.to_string() })
        })
        .collect()
}

impl Session {
    fn next_request(&mut self) -> String {
        let id = format!("canon-{}", self.next_notice);
        self.next_notice += 1;
        id
    }

    fn tab(&self) -> String {
        self.active_id().get().to_string()
    }

    fn ask_canon(&mut self, query: &str, asked: Asked) {
        let id = self.next_request();
        self.canon_asked.insert(id.clone(), asked);
        self.outgoing.push(AppMessage::ReadCanon { id, query: query.to_string() });
    }

    pub fn has_outgoing(&self) -> bool {
        !self.outgoing.is_empty()
    }

    /// What to send the host since the last call.
    pub fn take_outgoing(&mut self) -> Vec<AppMessage> {
        std::mem::take(&mut self.outgoing)
    }

    /// Open animatic: asks the project for its animatic steps and the files
    /// it has published; the assembly opens when the host answers.
    pub fn open_animatic(&mut self, step: Option<&str>) -> Result<(), String> {
        if !self.skills || self.project.is_none() {
            return Err("Only a Skills project has an animatic to open.".to_string());
        }
        self.ask_canon(ARTIFACTS, Asked::Artifacts);
        self.ask_canon(STEPS, Asked::Open(step.map(str::to_string)));
        Ok(())
    }

    /// Asks, now and then, whether the open animatic's step names another
    /// assembly than the one the storyboard read.
    pub fn check_retimed(&mut self, now: Instant) {
        let has_step = self.active_storyboard().assembly().is_some_and(|read| read.step.is_some());
        if !self.skills || !has_step || self.last_check.is_some_and(|at| now.duration_since(at) < CHECK_EVERY) {
            return;
        }
        self.last_check = Some(now);
        self.ask_canon(STEPS, Asked::Check);
    }

    /// The host's answer to a canon read.
    pub fn canon_rows(&mut self, id: &str, rows: &[Value], error: Option<&str>) {
        let Some(asked) = self.canon_asked.remove(id) else {
            return;
        };
        if let Some(error) = error {
            if let Asked::Open(_) = asked {
                let id = self.next_notice;
                self.next_notice += 1;
                self.keep_failed(id, "Animatic not opened", error);
            }
            return;
        }
        match asked {
            Asked::Artifacts => {
                for text in rows.iter().filter_map(row_text) {
                    if let Some((file, artifact)) = text.split_once(SEPARATOR) {
                        self.canon_files.insert(file.to_string(), artifact.to_string());
                    }
                }
            }
            Asked::Open(want) => {
                let steps = steps_of(rows);
                let chosen = match &want {
                    Some(want) => steps.iter().find(|step| &step.name == want || &step.key() == want),
                    None => steps.first(),
                };
                let Some(step) = chosen.cloned() else {
                    let id = self.next_notice;
                    self.next_notice += 1;
                    self.keep_failed(id, "Animatic not opened", "This project has no animatic yet.");
                    return;
                };
                if let Err(reason) = self.open_step(&step) {
                    let id = self.next_notice;
                    self.next_notice += 1;
                    self.keep_failed(id, "Animatic not opened", &reason);
                }
            }
            Asked::Check => {
                let Some(read) = self.active_storyboard().assembly().cloned() else {
                    return;
                };
                let Some((chain, name)) = read.step.clone() else {
                    return;
                };
                let Some(step) = steps_of(rows).into_iter().find(|step| step.chain == chain && step.name == name) else {
                    return;
                };
                self.steps.insert(step.key(), step.clone());
                let current = crate::assembly::named_in(&step.script);
                // Retimed elsewhere: the step names another file than the
                // one read, which Studio did not save.
                let ours = self.saving.as_ref().and_then(|saving| saving.file.clone());
                self.retimed = current.filter(|file| *file != read.path && Some(file) != ours.as_ref());
            }
        }
    }

    /// Opens the assembly a step names, and remembers the step.
    fn open_step(&mut self, step: &AnimaticStep) -> Result<(), String> {
        let file = crate::assembly::named_in(&step.script).ok_or("The animatic's step names no assembly.")?;
        let project = self.project.clone().ok_or("No project folder.")?;
        self.open_assembly(&project.join(&file))?;
        self.steps.insert(step.key(), step.clone());
        let record = self.active_storyboard_mut();
        if let Some(mut read) = record.assembly().cloned() {
            read.step = Some((step.chain.clone(), step.name.clone()));
            record.set_assembly(Some(read));
        }
        Ok(())
    }

    /// Save timing to the project: writes the film's timing as an assembly
    /// and asks the host to file it; once filed, the step is pointed at it.
    /// The error says why it cannot be saved.
    pub fn save_timing(&mut self) -> Result<(), String> {
        let project = self.project.clone().filter(|_| self.skills).ok_or("Only a Skills project has an animatic to save to.")?;
        let read = self.active_storyboard().assembly().cloned().ok_or("This storyboard was not opened from the project's animatic.")?;
        let (chain, name) = read.step.clone().ok_or("This storyboard was not opened from the project's animatic.")?;
        let step = self.steps.get(&format!("{chain}: {name}")).cloned().ok_or("The project's animatic step is not known yet.")?;
        if self.saving.is_some() {
            return Err("The timing is already being saved.".to_string());
        }
        // The canon length when the project has one; else the length the
        // animatic had, which is canon too.
        let (assembly, notes) = self.timing(Some(read.assembly.length))?;
        let mut files = self.canon_files.clone();
        for (_, _, file) in &read.boards {
            if let Some(artifact) = self.published_files.get(file) {
                files.insert(file.clone(), artifact.clone());
            }
        }
        let refs = crate::assembly::refs(&assembly, |file| files.get(file).cloned())?;
        let _ = project;
        // Skills' writer writes and files it, so its format stays Skills'.
        let id = self.next_request();
        self.outgoing.push(AppMessage::RunSkillScript {
            id: id.clone(),
            skill: "build-animatic".to_string(),
            script: "write-assembly.py".to_string(),
            args: writer_args(&assembly),
        });
        self.saving = Some(Saving { assembly, notes, refs, step, file: None, waiting: id });
        Ok(())
    }

    /// Skills' writer ran: its output is the filed file's `{path, sha256}`.
    pub fn skill_script_ran(&mut self, id: &str, output: &str, error: Option<&str>) {
        if !self.saving.as_ref().is_some_and(|saving| saving.waiting == id) {
            return;
        }
        let filed = output.lines().rev().find_map(|line| serde_json::from_str::<Value>(line.trim()).ok()).and_then(|value| Some(value.get("path")?.as_str()?.to_string()));
        match (error, filed) {
            (None, Some(path)) => self.filed(id, &path, None),
            (Some(error), _) => self.filed(id, "", Some(error)),
            (None, None) => self.filed(id, "", Some("Skills' writer did not say where it filed the timing.")),
        }
    }

    /// Save timing to the project refused, by hand: a card says why.
    pub fn timing_refused(&mut self, reason: &str) {
        let id = self.next_notice;
        self.next_notice += 1;
        self.keep_failed(id, "Timing not saved", reason);
    }

    /// The host filed a file: the saved timing, now pointed at by the step.
    pub fn filed(&mut self, id: &str, file: &str, error: Option<&str>) {
        let Some(saving) = self.saving.as_mut().filter(|saving| saving.waiting == id) else {
            return;
        };
        if let Some(error) = error {
            let error = error.to_string();
            self.saving = None;
            let id = self.next_notice;
            self.next_notice += 1;
            self.keep_failed(id, "Timing not saved", &error);
            return;
        }
        let Some(script) = crate::assembly::repoint(&saving.step.script, file) else {
            self.saving = None;
            return;
        };
        saving.file = Some(file.to_string());
        let refs = Value::Array(saving.refs.clone()).to_string();
        let step = saving.step.step_ref();
        let id = self.next_request();
        if let Some(saving) = self.saving.as_mut() {
            saving.waiting = id.clone();
            saving.step.script = script.clone();
        }
        let tab = self.tab();
        self.outgoing.push(AppMessage::EditStep { id, tab, step, script: Some(script), refs: Some(refs) });
    }

    /// The host edited the step: the storyboard reads the saved timing as
    /// its own, and a card says where it went.
    pub fn step_edited(&mut self, id: &str, error: Option<&str>) {
        let Some(saving) = self.saving.take().filter(|saving| saving.waiting == id) else {
            return;
        };
        if let Some(error) = error {
            let id = self.next_notice;
            self.next_notice += 1;
            self.keep_failed(id, "Timing not saved", error);
            return;
        }
        let file = saving.file.clone().unwrap_or_default();
        let bytes = self.project.as_ref().and_then(|project| std::fs::read(project.join(&file)).ok()).unwrap_or_default();
        self.timing_saved(saving.assembly, &saving.notes, &file, &protocol::fingerprint(&bytes));
        self.steps.insert(saving.step.key(), saving.step.clone());
        let record = self.active_storyboard_mut();
        if let Some(mut read) = record.assembly().cloned() {
            read.step = Some((saving.step.chain.clone(), saving.step.name.clone()));
            record.set_assembly(Some(read));
        }
        self.retimed = None;
        let id = self.next_notice;
        self.next_notice += 1;
        self.keep_notice(Notice {
            id: format!("notice-{id}"),
            text: "Timing saved to the project".to_string(),
            detail: if saving.notes.muted_left_out { "Muted sounds were left out of the project's animatic.".to_string() } else { file },
            progress: None,
            state: NoticeState::Done,
            actions: Vec::new(),
            close: Some(Action::with_arg("close_notice", serde_json::json!(format!("notice-{id}")))),
        });
    }

    /// The card while the open animatic was retimed in another tool.
    pub(crate) fn retimed_notice(&self) -> Option<Notice> {
        self.retimed.as_ref()?;
        Some(Notice {
            id: "retimed".to_string(),
            text: "Retimed in another tool".to_string(),
            detail: String::new(),
            progress: None,
            state: NoticeState::Working,
            actions: vec![NoticeAction { label: "Follow".to_string(), action: Action::new("follow_retime") }],
            close: None,
        })
    }

    pub fn retimed(&self) -> Option<&str> {
        self.retimed.as_deref()
    }

    /// Follow: the open animatic takes the timing the step now names, as one
    /// undo step, keeping each clip's drawing by its place in the film.
    pub fn follow_retime(&mut self) -> Result<(), String> {
        let file = self.retimed.clone().ok_or("The animatic was not retimed elsewhere.")?;
        let project = self.project.clone().ok_or("No project folder.")?;
        let bytes = std::fs::read(project.join(&file)).map_err(|error| crate::store::reason(&error))?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "It is not an animatic's assembly, or it is damaged.".to_string())?;
        let assembly = Assembly::from_value(&value)?;
        let old = self.active_storyboard().assembly().cloned().ok_or("This storyboard was not opened from the project's animatic.")?;
        let title = self.film().title.clone();
        let sounds = &mut self.sounds;
        let project_dir = Some(project.clone());
        let (fresh, _, mut read) = crate::assembly::open(&assembly, &title, &file, &protocol::fingerprint(&bytes), |audio| {
            sounds.loaded(project_dir.as_deref(), audio).map(|loaded| loaded.frames)
        });
        let mut ids = Vec::new();
        let changed = self.change_film(|film| {
            let mut made = Vec::new();
            // Clips by their place: retimed and given the new image; any
            // more are added, any fewer deleted.
            for (index, frame) in fresh.clips.iter().enumerate() {
                let image = frame.layers.first().and_then(|layer| layer.images.values().next().cloned());
                let id = match film.clips.get(index).map(|clip| clip.id) {
                    Some(id) => id,
                    None => {
                        let (id, drawings) = film.add_clip(Some(frame.start), frame.length);
                        made.extend(drawings);
                        id
                    }
                };
                let clip = film.clip_mut(id)?;
                clip.start = frame.start;
                clip.length = frame.length;
                if let (Some(image), Some(layer)) = (image, clip.layers.iter_mut().rev().find(|layer| !layer.folder))
                    && let Some(key) = layer.keys.first().map(|key| key.id)
                {
                    layer.images.insert(key, image);
                }
                ids.push(id);
            }
            film.clips.retain(|clip| ids.contains(&clip.id));
            film.clips.sort_by_key(|clip| clip.start);
            // The sound as the new assembly has it.
            let next = film.next_id.max(fresh.next_id);
            film.sound_tracks = fresh.sound_tracks.clone();
            film.next_id = next;
            Some(made)
        });
        if !changed {
            return Err("The animatic already has that timing.".to_string());
        }
        read.clips = ids;
        read.step = old.step;
        read.boards = old.boards;
        self.active_storyboard_mut().set_assembly(Some(read));
        self.retimed = None;
        Ok(())
    }

    /// A published file and the artifact it is published as, from the
    /// host's answer to a publish.
    pub(crate) fn note_published(&mut self, artifact: &str, file: &str) {
        self.published_files.insert(file.to_string(), artifact.to_string());
        self.canon_files.insert(file.to_string(), artifact.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_writer_is_given_each_time_as_read() {
        let assembly = Assembly::from_value(&serde_json::json!({
            "length": 8.0, "fps": 24, "size": [1536, 864],
            "frames": [{ "image": "library/a.png", "in": 0.0, "out": 4.2 }, { "image": "library/b.png", "in": 4.2, "out": 8.0 }],
            "bed": { "audio": "library/bed.wav", "gain_db": 0 },
            "clips": [{ "audio": "library/take.wav", "from": 0.05, "to": 0.65, "at": 4.2, "gain_db": -6 }]
        }))
        .unwrap();
        assert_eq!(writer_args(&assembly).join(" "), "--length 8 --fps 24 --size 1536x864 --frame library/a.png 0 4.2 --frame library/b.png 4.2 8 --bed library/bed.wav 0 --clip library/take.wav 0.05 0.65 4.2 -6 --name animatic-assembly --json");
    }

    #[test]
    fn rows_name_the_animatic_steps_whole() {
        let script = "python3 \"/skills/build-animatic/scripts/assemble.py\" prompts/resources/a.json .build/animatic.mp4";
        let rows = vec![Value::String(format!("the animatic{SEPARATOR}animatic{SEPARATOR}{script}")), serde_json::json!([format!("x{SEPARATOR}y{SEPARATOR}z | w")])];
        let steps = steps_of(&rows);
        assert_eq!(steps[0], AnimaticStep { chain: "the animatic".into(), name: "animatic".into(), script: script.into() });
        assert_eq!(steps[1].script, "z | w");
        assert_eq!(steps[0].key(), "the animatic: animatic");
    }
}
