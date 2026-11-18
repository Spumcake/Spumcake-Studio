//! The window's side of the Prompt page's runner duties: a cell's card, its
//! runs through the plan card, its rounds sent to its workspace, a step's
//! edits saved to the step, and a workspace's files filed into the
//! project. Every generation runs through Spumcake-Skills' request.py; when
//! Studio cannot find Spumcake-Skills, a cell says so and nothing is sent.

use super::HostWindow;
use crate::prompt_page::CardView;
use crate::runner::{Begun, ScriptEnv};
use protocol::{Cell, CellPart, HostMessage, ItemRef, ToolResult};
use std::path::PathBuf;

/// The runner's call for a cell's run, so its answer goes to the cell.
fn cell_call(id: &str) -> String {
    format!("cell:{id}")
}

impl HostWindow {
    /// The open project's folder, and what Spumcake-Skills' scripts run
    /// with, when Studio finds the scripts.
    pub(super) fn project_scripts(&mut self) -> Option<(PathBuf, ScriptEnv)> {
        let folder = self.project.as_ref()?.folder.clone();
        let skills = crate::skills_setup::listed_skills(&crate::skills_checkout::skill_folders(&self.settings.skill_folders));
        let (settings, keys, data) = (&self.settings, &mut self.keys, self.data.clone());
        let env = crate::runner::script_env(settings, &mut |id| keys.get(id), data.as_deref(), crate::runner::scripts_folder(&skills));
        env.scripts.is_some().then_some((folder, env))
    }

    /// A cell as an item: its tab's file, relative to the project, and its id.
    fn cell_item(&self, app: usize, tab: &str, id: &str) -> ItemRef {
        let folder = self.project.as_ref().map(|project| project.folder.clone());
        let path = self
            .apps
            .get(app)
            .and_then(|started| started.running.tabs().0.into_iter().find(|entry| entry.id == tab))
            .and_then(|entry| entry.path)
            .map(|path| match &folder {
                Some(folder) => std::path::Path::new(&path).strip_prefix(folder).map(|inside| inside.display().to_string()).unwrap_or(path),
                None => path,
            })
            .unwrap_or_default();
        ItemRef { path, item: Some(id.to_string()) }
    }

    /// What the card shows of a cell, and its rounds
    /// sent to its workspace when they changed.
    pub(super) fn prompt_card_view(&mut self, app: usize, tab: &str, cell: &Cell) -> Option<CardView> {
        let (folder, env) = self.project_scripts()?;
        let scripts = env.scripts.clone()?;
        let at = self.cell_item(app, tab, &cell.id);
        let view = self.prompt_page.card(&folder, &scripts, &env.vars, &self.settings, cell, &at);
        if let Some(rounds) = self.prompt_page.rounds_due(&folder, &scripts, &env.vars, cell, false) {
            let rounds = self.named(rounds);
            self.send(app, HostMessage::Takes { cell: at, rounds });
        }
        Some(view)
    }

    /// Generate, or Try again, on a cell: its spec goes to the plan card,
    /// and runs only as approved. With no project open it is not the
    /// runner's, and false; with no Spumcake-Skills found, the cell says so.
    pub(super) fn run_prompt_cell(&mut self, id: &str) -> bool {
        // Only a Prompt page's cell is the runner's. The conversation's and
        // a recipe's cells run straight on their endpoint, as the owner set
        // on 2026-11-17: they are the user's own, not the project's record.
        if self.project.is_none() {
            return false;
        }
        let Some((app, tab)) = self.cell_home(id) else { return false };
        let Some(cell) = self.panel_cell(app, &tab, id) else { return false };
        let on_the_prompt_page = cell.step.is_some()
            || self.apps.get(app).and_then(|started| started.running.declared.properties.as_ref()).and_then(|properties| properties.cell.as_ref()).is_some_and(|declared| declared.cell == id);
        if !on_the_prompt_page {
            return false;
        }
        let Some((folder, env)) = self.project_scripts() else {
            self.board.failed.insert(id.to_string(), String::from(crate::skills_checkout::MISSING));
            return true;
        };
        let Some(scripts) = env.scripts.clone() else { return false };
        // The card's unsaved prompt is the one sent.
        let mut cell = cell;
        if cell.step.is_none()
            && let Some(prompt) = self.board.drafts.get(id).cloned()
        {
            let mut input: Vec<CellPart> = cell.input.iter().filter(|part| !matches!(part, CellPart::Text { .. })).cloned().collect();
            input.insert(0, CellPart::Text { text: prompt });
            cell.input = input;
        }
        let at = self.cell_item(app, &tab, id);
        let spec = self
            .prompt_page
            .spec(&folder, &scripts, &env.vars, &self.settings, &cell, &at)
            .and_then(|spec| crate::prompt_page::write_spec(&folder, id, &spec));
        let file = match spec {
            Ok(file) => file,
            Err(reason) => {
                self.board.failed.insert(id.to_string(), reason);
                return true;
            }
        };
        self.board.failed.remove(id);
        if let Begun::Answered(result) = self.runner.ask_request(&folder, env, &cell_call(id), &file) {
            let reason = result.output.get("error").and_then(serde_json::Value::as_str).unwrap_or("It could not run.").to_string();
            self.board.failed.insert(id.to_string(), reason);
        }
        true
    }

    /// Stop on a cell's run: an approved request runs on, paid for, and its
    /// result still reaches the cell's rounds. False when it is not the
    /// runner's.
    pub(super) fn stop_prompt_cell(&mut self, id: &str) -> bool {
        let call = cell_call(id);
        if !self.runner.is_running(&call) {
            return false;
        }
        for answer in self.runner.stop_call(&call) {
            if let Some(reason) = answer.output.get("error").and_then(serde_json::Value::as_str) {
                self.tell(reason.to_string());
            }
        }
        true
    }

    /// Whether a cell's run is the runner's and still running.
    pub(super) fn prompt_cell_running(&self, id: &str) -> bool {
        self.runner.is_running(&cell_call(id))
    }

    /// An edit to a step's cell, saved to the step in `canon.sql`. False for
    /// a draft, which the workspace keeps.
    pub(super) fn edit_prompt_step(&mut self, id: &str, input: &[CellPart], settings: Option<&serde_json::Value>) -> bool {
        let Some((app, tab)) = self.cell_home(id) else { return false };
        let Some(step) = self.panel_cell(app, &tab, id).and_then(|cell| cell.step) else { return false };
        let Some((folder, env)) = self.project_scripts() else { return false };
        let Some(scripts) = env.scripts.clone() else { return false };
        if let Err(reason) = self.prompt_page.save_step(&folder, &scripts, &env.vars, &step, input, settings) {
            self.tell(format!("The step could not be saved: {reason}"));
        }
        true
    }

    /// Open report, on a cell's failed round: its newest round's report.
    pub(super) fn open_cell_report(&mut self, id: &str) {
        let Some((app, tab)) = self.cell_home(id) else { return };
        let Some(cell) = self.panel_cell(app, &tab, id) else { return };
        let Some((folder, env)) = self.project_scripts() else {
            self.tell(String::from("This cell's report is not known yet."));
            return;
        };
        let Some(scripts) = env.scripts.clone() else { return };
        let rounds = self.prompt_page.rounds_due(&folder, &scripts, &env.vars, &cell, true).unwrap_or_default();
        match rounds.iter().rev().find(|round| round.report.is_some()) {
            Some(round) => {
                let request = round.request.clone();
                self.open_report(&request);
            }
            None => self.tell(String::from("This cell has no report yet.")),
        }
    }

    /// A workspace's file, filed into `prompts/resources/` by Skills'
    /// `request.py resource`: the user's own action, so it runs at once.
    pub(super) fn file_resources(&mut self) {
        let mut asked = Vec::new();
        for (app, started) in self.apps.iter_mut().enumerate() {
            asked.extend(started.running.declared.resources.drain(..).map(|resource| (app, resource)));
        }
        if asked.is_empty() {
            return;
        }
        let Some((folder, env)) = self.project_scripts() else {
            let why = String::from(if self.project.is_some() { crate::skills_checkout::MISSING } else { crate::skills_checkout::NO_PROJECT });
            for (app, (_, _, _, id)) in asked {
                match id {
                    Some(id) => self.send(app, HostMessage::Filed { id, file: String::new(), error: Some(why.clone()) }),
                    None => self.tell(why.clone()),
                }
            }
            return;
        };
        for (app, (_, path, from, id)) in asked {
            let command = crate::prompt_page::resource_command(&path, &from);
            // A filing with an id is answered to its workspace by `filed`.
            let call = match &id {
                Some(id) => format!("filed:{app}:{id}"),
                None => format!("file:{path}"),
            };
            if let Err(reason) = self.runner.run_now(&folder, env.clone(), &call, &command) {
                match id {
                    Some(id) => self.send(app, HostMessage::Filed { id, file: String::new(), error: Some(reason) }),
                    None => self.tell(reason),
                }
            }
        }
    }

    /// The runner's answers: a cell's run refreshes its rounds, a filing
    /// that failed is said, and a step recorded from a draft's request turns
    /// that draft into the step. What is the assistant's is returned.
    pub(super) fn note_runner_answers(&mut self, answers: Vec<ToolResult>, finished: Vec<Vec<String>>) -> Vec<ToolResult> {
        let mut theirs = Vec::new();
        for answer in answers {
            if let Some(id) = answer.call.strip_prefix("cell:") {
                let id = id.to_string();
                // A plan card the user cancelled is not a failed round: the
                // cell simply stays as it was.
                let declined = answer.output.get("declined").and_then(serde_json::Value::as_bool).unwrap_or(false);
                if !answer.ok && !declined {
                    let reason = answer.output.get("error").and_then(serde_json::Value::as_str).map(String::from).unwrap_or_else(|| String::from("The request failed. Open its report."));
                    self.board.failed.insert(id.clone(), reason);
                }
                self.send_rounds(&id);
            } else if answer.call.starts_with("file:") {
                if !answer.ok {
                    self.tell(format!("The file could not be filed: {}", answer.output.get("output").and_then(serde_json::Value::as_str).unwrap_or("")));
                }
            } else if let Some((app, id)) = answer.call.strip_prefix("filed:").and_then(|rest| rest.split_once(':')) {
                // `request.py resource` prints each path it filed, the last last.
                let printed = answer.output.get("output").and_then(serde_json::Value::as_str).unwrap_or("");
                let (file, error) = if answer.ok {
                    (printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or("").to_string(), None)
                } else {
                    let error = answer.output.get("error").and_then(serde_json::Value::as_str).unwrap_or(printed);
                    (String::new(), Some(format!("The file could not be filed: {error}")))
                };
                if let Ok(app) = app.parse::<usize>() {
                    self.send(app, HostMessage::Filed { id: id.to_string(), file, error });
                }
            } else if answer.call.starts_with("steprun:") {
                self.answer_step_run(answer);
            } else if answer.call.starts_with("step:") || answer.call.starts_with("canon:") {
                self.answer_step_edit(answer);
            } else {
                theirs.push(answer);
            }
        }
        for argv in finished {
            self.note_recorded_step(&argv);
        }
        theirs
    }

    fn send_rounds(&mut self, id: &str) {
        let Some((folder, env)) = self.project_scripts() else { return };
        let Some(scripts) = env.scripts.clone() else { return };
        let Some((app, tab)) = self.cell_home(id) else { return };
        let Some(cell) = self.panel_cell(app, &tab, id) else { return };
        if let Some(rounds) = self.prompt_page.rounds_due(&folder, &scripts, &env.vars, &cell, true) {
            let at = self.cell_item(app, &tab, id);
            let rounds = self.named(rounds);
            self.send(app, HostMessage::Takes { cell: at, rounds });
        }
    }

    /// Each round with its model's name, the one the card shows.
    fn named(&mut self, rounds: Vec<protocol::Round>) -> Vec<protocol::Round> {
        rounds
            .into_iter()
            .map(|mut round| {
                if !round.model.is_empty() {
                    round.model_name = self.model_name(&round.model);
                }
                round
            })
            .collect()
    }

    /// A model's name as the card shows it, from an endpoint's own model
    /// list, or else a skill's `models.json`; empty when neither names it,
    /// so the workspace shows the model's id as it is.
    fn model_name(&mut self, model: &str) -> String {
        let shown = crate::cells::model_name(&self.ctx, model);
        if shown != model.rsplit('/').next().unwrap_or(model) {
            return shown;
        }
        let skills = crate::skills_setup::listed_skills(&crate::skills_checkout::skill_folders(&self.settings.skill_folders));
        crate::prices::local_name(&skills, model).unwrap_or_default()
    }

    /// When the agent records a step with the request a draft cell made,
    /// that cell becomes the step.
    fn note_recorded_step(&mut self, argv: &[String]) {
        let Some((step, request)) = crate::prompt_page::recorded_step(argv) else { return };
        let Some(folder) = self.project.as_ref().map(|project| project.folder.clone()) else { return };
        let Some(id) = crate::prompt_page::cell_of_request(&folder, &request).and_then(|item| item.item) else { return };
        let Some((app, tab)) = self.cell_home(&id) else { return };
        self.prompt_page.forget();
        let artifact = self.project_scripts().and_then(|(folder, env)| {
            let scripts = env.scripts.clone()?;
            self.prompt_page.canon(&folder, &scripts, &env.vars).step_artifacts.get(&(step.chain.clone(), step.step.clone())).cloned()
        });
        self.send(app, HostMessage::CellBecameStep { tab, cell: id, step, artifact });
    }
}
