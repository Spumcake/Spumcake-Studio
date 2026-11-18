//! The storyboard and the project's animatic step, through the host:
//! finding the step whose `build-animatic` script runs `assemble.py`,
//! opening the assembly it names, saving the film's timing as a new
//! assembly that the step is pointed at, and following a retime made in
//! another tool. The host runs Skills' scripts; this asks and answers.

use super::Session;
use crate::assembly::{Assembly, Notes};
use draw::document::DrawingId;
use protocol::{Action, AppMessage, Notice, NoticeAction, NoticeState};
use serde_json::Value;
use std::time::{Duration, Instant};

/// How often an open animatic asks whether it was retimed elsewhere.
pub const CHECK_EVERY: Duration = Duration::from_secs(5);

/// How long a Save timing to the project waits for each answer, which
/// includes the person's yes on the step's edit, before it gives up.
pub const SAVE_WAIT: Duration = Duration::from_secs(600);

/// The columns of one answer row, joined by the unit separator so a script
/// or a reference never splits a row.
const SEPARATOR: char = '\u{1f}';

/// The project's animatics: each `build-animatic` step whose script runs
/// `assemble.py`, by its chain and name, with the artifact it makes and its
/// script.
const STEPS: &str = "SELECT chain || char(31) || name || char(31) || coalesce(artifact, '') || char(31) || script FROM step WHERE skill = 'build-animatic' AND script LIKE '%assemble.py%' ORDER BY chain, ord";
/// Every published file and the artifact it is published as.
const ARTIFACTS: &str = "SELECT file || char(31) || id FROM artifact WHERE file IS NOT NULL";
/// The project's film: its canon element of kind film.
const FILM: &str = "SELECT name FROM element WHERE kind = 'film' ORDER BY name";
/// The chain an animatic is recorded in, as `build-animatic` names it.
pub const ANIMATIC_CHAIN: &str = "the animatic";
/// The step that makes the animatic, in that chain.
pub const ANIMATIC_STEP: &str = "animatic";
/// Where the animatic's chain stands: whether it is there, its last step's
/// order, and the last chain's order.
const ORDERS: &str = "SELECT (SELECT count(*) FROM chain WHERE name = 'the animatic') || char(31) || (SELECT coalesce(max(ord), 0) FROM step WHERE chain = 'the animatic') || char(31) || (SELECT coalesce(max(ord), 0) FROM chain)";

/// Where the project's chains stand, for a new animatic step: it goes last
/// in its chain, and a missing chain goes after the last one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainOrders {
    pub chain_there: bool,
    pub last_step: u32,
    pub last_chain: u32,
}

/// One animatic step.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimaticStep {
    pub chain: String,
    pub name: String,
    /// The canon artifact it makes, such as "Last order: animatic".
    pub artifact: String,
    pub script: String,
}

impl AnimaticStep {
    /// The film's name, the element of the artifact the step makes, such as
    /// "Last order" for "Last order: animatic"; the project folder's when
    /// the step names none.
    pub fn film(&self, project: &std::path::Path) -> String {
        match self.artifact.split_once(':') {
            Some((element, _)) if !element.trim().is_empty() => element.trim().to_string(),
            _ => project.file_name().map_or_else(|| "Animatic".to_string(), |name| name.to_string_lossy().into_owned()),
        }
    }

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
    /// Showing the project's animatic, when the project opens.
    Open,
    /// The project's film.
    Film,
    /// Where the project's chains stand.
    Orders,
    /// Whether the animatic a storyboard read was retimed elsewhere.
    Check(DrawingId),
    Artifacts,
}

/// A Save timing to the project under way, for the storyboard that asked.
#[derive(Clone, Debug)]
pub struct Saving {
    pub storyboard: DrawingId,
    /// When the last ask went out.
    pub since: Instant,
    pub assembly: Assembly,
    pub notes: Notes,
    pub refs: Vec<Value>,
    pub step: AnimaticStep,
    /// Waiting for Skills' writer to file it, then for the step's edit.
    pub file: Option<String>,
    pub waiting: String,
    /// Export animatic video asked for the save: the step runs once it is
    /// done, made from these items.
    pub then_export: Option<Vec<(String, String)>>,
    /// The project had no animatic: the save records its step.
    pub new_step: Option<NewStep>,
    /// What is still to be recorded once the host answers the last ask:
    /// the chain and the step, after the film's element.
    pub then: Vec<AppMessage>,
}

/// What a new film's first save records, as `build-animatic` records it.
#[derive(Clone, Debug)]
pub struct NewStep {
    /// The step's order, last in its chain.
    pub order: u32,
    /// The chain's order, when the chain is not there yet.
    pub chain_order: Option<u32>,
    /// The film's element, when the project has none yet.
    pub film: Option<String>,
}

/// The step's endpoint, as `build-animatic` records it.
pub const ANIMATIC_ENDPOINT: &str = r#"{"kind": "script", "outputs": [".build/animatic.mp4"]}"#;

/// The animatic step's script: `assemble.py` from the agent's Skills
/// checkout, on the assembly, into `.build/`. The host records it in
/// Skills' own form, with the checkout it found as `SPUMCAKE_AGENT`'s
/// default.
pub fn animatic_script(assembly: &str) -> String {
    format!("python3 \"${{SPUMCAKE_AGENT}}/skills/build-animatic/scripts/assemble.py\" {assembly} .build/animatic.mp4")
}

/// Export animatic video: the animatic's step running again through the
/// host, for the storyboard that asked.
#[derive(Clone, Debug)]
pub struct Stepping {
    pub id: String,
    pub storyboard: DrawingId,
    pub step: AnimaticStep,
    /// The items the animatic is made from, as the export began.
    pub sources: Vec<(String, String)>,
    pub since: Instant,
}

/// The storyboard in `folder` whose timing was read from the step `key`,
/// by what its file keeps, whatever it is named.
fn kept_for(folder: &std::path::Path, key: &(String, String)) -> Option<std::path::PathBuf> {
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "smx"))
        .filter(|path| {
            let stored = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            stored.is_some_and(|stored| {
                let step = &stored["assembly"]["step"];
                step["chain"].as_str() == Some(key.0.as_str()) && step["name"].as_str() == Some(key.1.as_str())
            })
        })
        .collect();
    // The one kept last, when more than one was.
    found.sort_by_key(|path| std::cmp::Reverse(std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()));
    found.into_iter().next()
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
            let mut parts = text.splitn(4, SEPARATOR);
            Some(AnimaticStep {
                chain: parts.next()?.to_string(),
                name: parts.next()?.to_string(),
                artifact: parts.next()?.to_string(),
                script: parts.next()?.to_string(),
            })
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

    /// A project opened: asks for its animatic step and the files it has
    /// published. When the host answers, the project's animatic is shown as
    /// a storyboard; a project with no animatic yet keeps its blank, whose
    /// Save timing to the project makes one.
    pub fn show_animatic(&mut self) {
        if self.project.is_none() {
            return;
        }
        self.film_read = false;
        self.steps_read = false;
        self.chain_orders = None;
        self.ask_canon(ARTIFACTS, Asked::Artifacts);
        self.ask_canon(FILM, Asked::Film);
        self.ask_canon(ORDERS, Asked::Orders);
        self.ask_canon(STEPS, Asked::Open);
    }

    /// Asks, now and then, whether the shown animatic's step names another
    /// assembly than the one the storyboard read: never while it plays, nor
    /// while the last ask is unanswered. A save that waited too long for
    /// the host gives up with a card.
    pub fn check_retimed(&mut self, now: Instant) {
        if let Some(saving) = self.saving.take_if(|saving| now.duration_since(saving.since) >= SAVE_WAIT) {
            self.timing_refused("The project did not answer.");
            if saving.then_export.is_some() {
                self.step_export_failed("The project did not answer.");
            }
        }
        if self.stepping.take_if(|stepping| now.duration_since(stepping.since) >= SAVE_WAIT).is_some() {
            self.step_export_failed("The project did not answer.");
        }
        let has_step = self.active_storyboard().assembly().is_some_and(|read| read.step.is_some());
        let asking = self.canon_asked.values().any(|asked| matches!(asked, Asked::Check(_)));
        if self.project.is_none() || !has_step || asking || self.is_playing() || self.last_check.is_some_and(|at| now.duration_since(at) < CHECK_EVERY) {
            return;
        }
        self.last_check = Some(now);
        let storyboard = self.active_id();
        self.ask_canon(STEPS, Asked::Check(storyboard));
    }

    /// The host's answer to a canon read.
    pub fn canon_rows(&mut self, id: &str, rows: &[Value], error: Option<&str>) {
        let Some(asked) = self.canon_asked.remove(id) else {
            return;
        };
        // A read that failed is the host's to say, where it can say what to
        // do; the storyboard stays as it is.
        if error.is_some() {
            return;
        }
        match asked {
            Asked::Film => {
                // With several, the first by name; Skills asks which, and
                // Studio has one film to a project.
                self.film_element = rows.iter().filter_map(row_text).map(|name| name.trim().to_string()).find(|name| !name.is_empty());
                self.film_read = true;
            }
            Asked::Orders => {
                let text = rows.iter().filter_map(row_text).next().unwrap_or_default();
                let numbers: Vec<u32> = text.split(SEPARATOR).map(|part| part.trim().parse().unwrap_or(0)).collect();
                if let [there, step, chain] = numbers.as_slice() {
                    self.chain_orders = Some(ChainOrders { chain_there: *there > 0, last_step: *step, last_chain: *chain });
                }
            }
            Asked::Artifacts => {
                for text in rows.iter().filter_map(row_text) {
                    if let Some((file, artifact)) = text.split_once(SEPARATOR) {
                        self.canon_files.insert(file.to_string(), artifact.to_string());
                    }
                }
            }
            Asked::Open => {
                let steps = steps_of(rows);
                for step in &steps {
                    self.steps.insert(step.key(), step.clone());
                }
                self.steps_read = true;
                // No animatic yet: the blank storyboard is the one whose
                // save makes it.
                let Some(step) = steps.first().cloned() else {
                    return;
                };
                if let Err(reason) = self.show_step(&step) {
                    let id = self.next_notice;
                    self.next_notice += 1;
                    self.keep_failed(id, "Animatic not opened", &reason);
                }
            }
            Asked::Check(storyboard) => {
                let Some(read) = self.index_of(storyboard).and_then(|index| self.storyboards[index].assembly().cloned()) else {
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
                let ours = self.saving.as_ref().filter(|saving| saving.storyboard == storyboard).and_then(|saving| saving.file.clone());
                self.retimed = current.filter(|file| *file != read.path && Some(file) != ours.as_ref()).map(|file| (storyboard, file));
            }
        }
    }

    /// Shows the step's animatic as a storyboard, with the timing the step
    /// names now: the storyboard already open, or the one kept in `work/`
    /// under the film's name, with its drawing, or else one opened from the
    /// assembly alone.
    fn show_step(&mut self, step: &AnimaticStep) -> Result<(), String> {
        let file = crate::assembly::named_in(&step.script).ok_or("The animatic's step names no assembly.")?;
        let project = self.project.clone().ok_or("No project is open.")?;
        let key = (step.chain.clone(), step.name.clone());
        let ours = |record: &crate::record::Record| record.assembly().is_some_and(|read| read.step.as_ref() == Some(&key));
        if let Some(id) = self.storyboards.iter().find(|record| ours(record)).map(|record| record.id()) {
            self.activate(id);
            return self.take_timing(&file);
        }
        let film = self.film_element.clone().unwrap_or_else(|| step.film(&project));
        // The storyboard kept in `work/` whose timing came from this step,
        // under whatever name the person gave it.
        let blank = self.untouched_blank();
        if let Some(kept) = kept_for(&project.join(super::WORK), &key)
            && self.open_path(&kept)
        {
            self.drop_blank(blank);
            self.take_timing(&file)?;
            // Opening is not an edit: the timing it took is no undo step.
            self.active_storyboard_mut().clear_history();
            return Ok(());
        }
        // None kept, or one that would not open, which has said why in the
        // window's own card: the animatic opens from its assembly instead.
        self.file_error = None;
        self.open_assembly(&project.join(&file), &film)?;
        let record = self.active_storyboard_mut();
        if let Some(mut read) = record.assembly().cloned() {
            read.step = Some(key);
            record.set_assembly(Some(read));
        }
        Ok(())
    }

    /// The film's canon name, the project's element of kind film; else the
    /// element the animatic's step makes its artifact under; else the
    /// project's name; else the film's title.
    pub(crate) fn film_name(&self) -> String {
        if let Some(film) = &self.film_element {
            return film.clone();
        }
        let read = self.active_storyboard().assembly();
        let step = read.and_then(|read| read.step.as_ref()).and_then(|(chain, name)| self.steps.get(&format!("{chain}: {name}")));
        match (step, self.project.as_deref()) {
            (Some(step), Some(project)) => step.film(project),
            (None, Some(project)) => project.file_name().map_or_else(|| self.film().title.clone(), |name| name.to_string_lossy().into_owned()),
            _ => self.film().title.clone(),
        }
    }

    /// Opening the animatic takes the timing its step names now, made in
    /// Studio or in another tool, as Follow would, with no card.
    fn take_timing(&mut self, file: &str) -> Result<(), String> {
        match self.active_storyboard().assembly() {
            Some(read) if read.path != file => {
                self.retimed = Some((self.active_id(), file.to_string()));
                self.follow_retime()
            }
            _ => Ok(()),
        }
    }

    /// Save timing to the project: writes the film's timing as an assembly
    /// and asks the host to file it; once filed, the step is pointed at it.
    /// The error says why it cannot be saved.
    pub fn save_timing(&mut self) -> Result<(), String> {
        if self.project.is_none() {
            return Err("No project is open.".to_string());
        }
        // A project with no animatic yet: this storyboard's save makes it.
        if self.active_storyboard().assembly().is_none() && self.steps.is_empty() {
            return self.save_new_animatic();
        }
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
        for board in &read.boards {
            if let Some(artifact) = self.published_files.get(&board.file) {
                files.insert(board.file.clone(), artifact.clone());
            }
        }
        let refs = crate::assembly::refs(&assembly, |file| files.get(file).cloned())?;
        // Skills' writer writes and files it, so its format stays Skills'.
        let id = self.next_request();
        self.outgoing.push(AppMessage::RunSkillScript {
            id: id.clone(),
            skill: "build-animatic".to_string(),
            script: "write-assembly.py".to_string(),
            args: writer_args(&assembly),
        });
        let storyboard = self.active_id();
        self.saving = Some(Saving { storyboard, since: Instant::now(), assembly, notes, refs, step, file: None, waiting: id, then_export: None, new_step: None, then: Vec::new() });
        Ok(())
    }

    /// A new film's first Save timing to the project: the timing as an
    /// assembly, filed by Skills' writer, then the film's element and the
    /// animatic's chain when they are not there yet, and the step itself,
    /// as `build-animatic` records them. The film's name is the project's
    /// canon film, or the project's name.
    fn save_new_animatic(&mut self) -> Result<(), String> {
        if self.saving.is_some() {
            return Err("The timing is already being saved.".to_string());
        }
        // Only once the project's film, its chains, and its animatic steps
        // are all read, whatever order the host answered in, so a step or
        // an element already there is never recorded over.
        let orders = self.chain_orders.filter(|_| self.film_read && self.steps_read).ok_or("The project's animatic step is not known yet.")?;
        let (assembly, notes) = self.timing(None)?;
        let files = self.canon_files.clone();
        let refs = crate::assembly::refs(&assembly, |file| files.get(file).cloned())?;
        let film = self.film_name();
        let step = AnimaticStep {
            chain: ANIMATIC_CHAIN.to_string(),
            name: ANIMATIC_STEP.to_string(),
            artifact: format!("{film}: animatic"),
            script: animatic_script(""),
        };
        let new_step = NewStep {
            order: if orders.chain_there { orders.last_step + 1 } else { 1 },
            chain_order: (!orders.chain_there).then_some(orders.last_chain + 1),
            film: self.film_element.is_none().then_some(film),
        };
        let id = self.next_request();
        self.outgoing.push(AppMessage::RunSkillScript {
            id: id.clone(),
            skill: "build-animatic".to_string(),
            script: "write-assembly.py".to_string(),
            args: writer_args(&assembly),
        });
        let storyboard = self.active_id();
        self.saving = Some(Saving { storyboard, since: Instant::now(), assembly, notes, refs, step, file: None, waiting: id, then_export: None, new_step: Some(new_step), then: Vec::new() });
        Ok(())
    }

    /// Export animatic video: the animatic is made by running its step
    /// again, as `build-animatic` makes it, after saving
    /// the timing when it differs. The take is then offered for publishing
    /// on the publish card. Returns the step being run.
    pub(crate) fn export_by_step(&mut self) -> Result<String, String> {
        if self.active_storyboard().assembly().is_none() && self.steps.is_empty() {
            return Err("This project has no animatic yet.".to_string());
        }
        let read = self.active_storyboard().assembly().cloned().ok_or("This storyboard was not opened from the project's animatic.")?;
        let (chain, name) = read.step.clone().ok_or("This storyboard was not opened from the project's animatic.")?;
        let step = self.steps.get(&format!("{chain}: {name}")).cloned().ok_or("The project's animatic step is not known yet.")?;
        if self.saving.is_some() {
            return Err("The timing is already being saved.".to_string());
        }
        // A retime made elsewhere is not what the storyboard shows, so the
        // storyboard's timing is saved over it first.
        let differs = match self.timing(Some(read.assembly.length)) {
            Ok((assembly, _)) => assembly != read.assembly || self.retimed().is_some(),
            Err(reason) => return Err(reason),
        };
        let sources = self.export_sources();
        if differs {
            self.save_timing()?;
            if let Some(saving) = self.saving.as_mut() {
                saving.then_export = Some(sources);
            }
        } else {
            self.run_step(self.active_id(), step.clone(), sources);
        }
        Ok(step.key())
    }

    /// Whether Export animatic video is running the animatic's step, or
    /// saving the timing before it.
    pub(crate) fn exporting_by_step(&self) -> bool {
        self.stepping.is_some() || self.saving.as_ref().is_some_and(|saving| saving.then_export.is_some())
    }

    /// Asks the host to run the animatic's step again.
    fn run_step(&mut self, storyboard: DrawingId, step: AnimaticStep, sources: Vec<(String, String)>) {
        let id = self.next_request();
        self.outgoing.push(AppMessage::RunStep { id: id.clone(), tab: storyboard.get().to_string(), step: step.step_ref() });
        self.stepping = Some(Stepping { id, storyboard, step, sources, since: Instant::now() });
    }

    /// The host ran the animatic's step: its take is offered for publishing
    /// as the step's artifact, and the export's card says where it is.
    pub fn step_ran(&mut self, id: &str, take: Option<&str>, error: Option<&str>) {
        let Some(stepping) = self.stepping.take_if(|stepping| stepping.id == id) else {
            return;
        };
        let take = match (error, take) {
            (None, Some(take)) if !take.is_empty() => take.to_string(),
            (error, _) => {
                self.step_export_failed(error.unwrap_or_default());
                return;
            }
        };
        if !stepping.step.artifact.is_empty() {
            self.outgoing.push(AppMessage::Publish {
                tab: stepping.storyboard.get().to_string(),
                artifact: stepping.step.artifact.clone(),
                source: take.clone(),
                from: Vec::new(),
                with: Vec::new(),
            });
        }
        self.step_exported(stepping.storyboard, &take, stepping.sources);
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

    /// Follow or Publish hold failed, by hand: a card says
    /// why, under `text`, or alone when there is none.
    pub fn failed_by_hand(&mut self, text: Option<&str>, reason: &str) {
        let id = self.next_notice;
        self.next_notice += 1;
        match text {
            Some(text) => self.keep_failed(id, text, reason),
            None => self.keep_failed(id, reason, ""),
        }
    }

    /// The host filed a file: a sound brought in from outside, or the saved
    /// timing, now pointed at by the step.
    pub fn filed(&mut self, id: &str, file: &str, error: Option<&str>) {
        if self.filing.contains_key(id) {
            self.sound_filed(id, file, error);
            return;
        }
        let Some(saving) = self.saving.as_mut().filter(|saving| saving.waiting == id) else {
            return;
        };
        if let Some(error) = error {
            let error = error.to_string();
            let export = self.saving.take().is_some_and(|saving| saving.then_export.is_some());
            self.timing_refused(&error);
            if export {
                self.step_export_failed(&error);
            }
            return;
        }
        // A new step names the filed assembly in Last order's form; an
        // existing one has it swapped in.
        let script = match &saving.new_step {
            Some(_) => Some(animatic_script(file)),
            None => crate::assembly::repoint(&saving.step.script, file),
        };
        let Some(script) = script else {
            let export = self.saving.take().is_some_and(|saving| saving.then_export.is_some());
            self.timing_refused("The animatic's step names no assembly.");
            if export {
                self.step_export_failed("The animatic's step names no assembly.");
            }
            return;
        };
        let Some(saving) = self.saving.as_mut() else {
            return;
        };
        saving.file = Some(file.to_string());
        let refs = Value::Array(saving.refs.clone()).to_string();
        let step = saving.step.step_ref();
        let new_step = saving.new_step.clone();
        let artifact = saving.step.artifact.clone();
        let id = self.next_request();
        let mut tab = self.tab();
        if let Some(saving) = self.saving.as_mut() {
            saving.waiting = id.clone();
            saving.since = Instant::now();
            saving.step.script = script.clone();
            tab = saving.storyboard.get().to_string();
        }
        let (order, skill, artifact, endpoint) = match &new_step {
            Some(new_step) => (Some(new_step.order), Some("build-animatic".to_string()), Some(artifact), Some(ANIMATIC_ENDPOINT.to_string())),
            None => (None, None, None, None),
        };
        let edit = AppMessage::EditStep { id: id.clone(), tab: tab.clone(), step, script: Some(script), refs: Some(refs), order, skill, artifact, endpoint };
        // A new film's element and its animatic's chain are recorded first,
        // when the project has neither, each once the host answers the one
        // before; the step last.
        let mut asks = Vec::new();
        if let Some(new_step) = &new_step {
            if let Some(film) = &new_step.film {
                asks.push(AppMessage::EditElement { id: self.next_request(), tab: tab.clone(), name: film.clone(), kind: "film".to_string() });
            }
            if let Some(order) = new_step.chain_order {
                asks.push(AppMessage::EditChain { id: self.next_request(), tab: tab.clone(), chain: ANIMATIC_CHAIN.to_string(), order });
            }
        }
        asks.push(edit);
        self.send_next(asks);
    }

    /// Sends the first of `asks` and keeps the rest for when the host
    /// answers it; the save waits on its id.
    fn send_next(&mut self, mut asks: Vec<AppMessage>) {
        if asks.is_empty() {
            return;
        }
        let first = asks.remove(0);
        let id = match &first {
            AppMessage::EditElement { id, .. } | AppMessage::EditChain { id, .. } | AppMessage::EditStep { id, .. } => id.clone(),
            _ => String::new(),
        };
        if let Some(saving) = self.saving.as_mut() {
            saving.waiting = id;
            saving.since = Instant::now();
            saving.then = asks;
        }
        self.outgoing.push(first);
    }

    /// The host recorded the film's element or the animatic's chain: the
    /// next is asked for, or a card says why the timing was not saved.
    pub fn canon_edited(&mut self, id: &str, error: Option<&str>) {
        let Some(saving) = self.saving.as_mut().filter(|saving| saving.waiting == id) else {
            return;
        };
        if let Some(error) = error {
            let error = error.to_string();
            let export = self.saving.take().is_some_and(|saving| saving.then_export.is_some());
            self.timing_refused(&error);
            if export {
                self.step_export_failed(&error);
            }
            return;
        }
        let then = std::mem::take(&mut saving.then);
        self.send_next(then);
    }

    /// The host edited the step: the storyboard reads the saved timing as
    /// its own, and a card says where it went.
    pub fn step_edited(&mut self, id: &str, error: Option<&str>) {
        let Some(saving) = self.saving.take().filter(|saving| saving.waiting == id) else {
            return;
        };
        if let Some(error) = error {
            self.timing_refused(error);
            if saving.then_export.is_some() {
                self.step_export_failed(error);
            }
            return;
        }
        if let Some(sources) = saving.then_export.clone() {
            self.run_step(saving.storyboard, saving.step.clone(), sources);
        }
        let file = saving.file.clone().unwrap_or_default();
        let bytes = self.project.as_ref().and_then(|project| std::fs::read(project.join(&file)).ok()).unwrap_or_default();
        self.steps.insert(saving.step.key(), saving.step.clone());
        // A new film's element is the project's film from now on.
        if let Some(film) = saving.new_step.as_ref().and_then(|new_step| new_step.film.clone()) {
            self.film_element = Some(film);
        }
        // The storyboard that saved reads it, wherever the person is now.
        if self.timing_saved(saving.storyboard, saving.assembly, &saving.notes, &file, &protocol::fingerprint(&bytes))
            && let Some(index) = self.index_of(saving.storyboard)
        {
            let record = &mut self.storyboards[index];
            if let Some(mut read) = record.assembly().cloned() {
                read.step = Some((saving.step.chain.clone(), saving.step.name.clone()));
                record.set_assembly(Some(read));
            }
        }
        if self.retimed.as_ref().is_some_and(|(storyboard, _)| *storyboard == saving.storyboard) {
            self.retimed = None;
        }
        self.last_saved = Some((saving.storyboard, file.clone()));
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

    /// The card while the shown animatic was retimed in another tool.
    pub(crate) fn retimed_notice(&self) -> Option<Notice> {
        self.retimed()?;
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

    /// The assembly the shown animatic's step now names, when it was
    /// retimed in another tool.
    pub fn retimed(&self) -> Option<&str> {
        self.retimed.as_ref().filter(|(storyboard, _)| *storyboard == self.active_id()).map(|(_, file)| file.as_str())
    }

    /// Follow: the shown animatic takes the timing the step now names, as
    /// one undo step, which also takes back what it read. Each clip keeps
    /// its drawing on the frames it held.
    pub fn follow_retime(&mut self) -> Result<(), String> {
        let file = self.retimed().map(str::to_string).ok_or("The animatic was not retimed elsewhere.")?;
        let project = self.project.clone().ok_or("No project is open.")?;
        // A gesture still open is committed first, so Follow keeps it.
        self.settle();
        let bytes = std::fs::read(project.join(&file)).map_err(|error| crate::store::reason(&error))?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "It is not an animatic's assembly, or it is damaged.".to_string())?;
        let assembly = Assembly::from_value(&value)?;
        let old = self.active_storyboard().assembly().cloned().ok_or("This storyboard was not opened from the project's animatic.")?;
        let title = self.film().title.clone();
        let sounds = &mut self.sounds;
        let project_dir = Some(project.clone());
        let (fresh, _, fresh_read) = crate::assembly::open(&assembly, &title, &file, &protocol::fingerprint(&bytes), |audio| {
            sounds.loaded(project_dir.as_deref(), audio).map(|loaded| loaded.frames)
        });
        let mut film = self.film().clone();
        let (made, read) = crate::assembly::follow(&mut film, &old, &assembly, &fresh, &fresh_read);
        let changed = self.change_film_reading(|current| {
            *current = film;
            Some(made)
        }, read.clone());
        if !changed {
            // The same timing, filed again: only what was read changes.
            self.active_storyboard_mut().set_assembly(Some(read));
        }
        self.retimed = None;
        Ok(())
    }

    /// The shown storyboard and the project's animatic, for `read_film`:
    /// the assembly it read and its step, whether its timing differs from
    /// that assembly or why it cannot be saved, a save under way or
    /// finished, and a retime made in another tool. None for a storyboard
    /// not opened from the animatic.
    pub fn project_animatic(&self) -> Option<Value> {
        let read = self.active_storyboard().assembly()?;
        let storyboard = self.active_id();
        let (differs, refusal) = match self.timing(Some(read.assembly.length)) {
            Ok((assembly, _)) => (assembly != read.assembly, None),
            Err(reason) => (true, Some(reason)),
        };
        let saving = self.saving.as_ref().filter(|saving| saving.storyboard == storyboard);
        let saved = self.last_saved.as_ref().filter(|(id, _)| *id == storyboard).map(|(_, file)| file.clone());
        Some(serde_json::json!({
            "assembly": read.path,
            "step": read.step.as_ref().map(|(chain, name)| format!("{chain}: {name}")),
            "differs": differs,
            "cannot_save": refusal,
            "saving": saving.is_some(),
            "saved": saved.as_ref().map(|file| serde_json::json!({ "assembly": file, "step_uses_it": *file == read.path && self.retimed().is_none() })),
            "retimed_elsewhere": self.retimed(),
        }))
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
        let rows = vec![Value::String(format!("the animatic{SEPARATOR}animatic{SEPARATOR}Last order: animatic{SEPARATOR}{script}")), serde_json::json!([format!("x{SEPARATOR}y{SEPARATOR}{SEPARATOR}z | w")])];
        let steps = steps_of(&rows);
        assert_eq!(steps[0], AnimaticStep { chain: "the animatic".into(), name: "animatic".into(), artifact: "Last order: animatic".into(), script: script.into() });
        assert_eq!(steps[1].script, "z | w");
        assert_eq!(steps[0].key(), "the animatic: animatic");
    }
}
