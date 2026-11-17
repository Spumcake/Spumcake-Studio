//! The window's side of a workspace's own reads and edits of the chain, in
//! a Skills project: canon read with `pipeline.py sql --read-only`, a step
//! edited with `pipeline.py step` on the user's yes on the runner's card,
//! and a skill's own script run, such as `build-animatic`'s writer, so a
//! workspace never writes a Skills format itself. A step run again as a
//! take waits in `step_runs` for the runner's rerun. Reads and scripts run
//! on threads of their own, so a workspace that polls canon never stalls
//! drawing or playback; their answers are sent on the frame they arrive.

use super::HostWindow;
use protocol::{HostMessage, StepRef, ToolResult};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Answers made off the window's thread, each with the workspace it goes to.
pub(crate) struct Answers {
    sender: Sender<(usize, HostMessage)>,
    receiver: Receiver<(usize, HostMessage)>,
}

impl Default for Answers {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self { sender, receiver }
    }
}

/// Runs one of Skills' scripts on its own thread, and sends what `answer`
/// makes of it to the workspace `app`.
fn run_off_thread(
    answers: &Answers,
    app: usize,
    (scripts, folder, vars): (PathBuf, PathBuf, Vec<(String, String)>),
    name: String,
    args: Vec<String>,
    answer: impl FnOnce(Result<String, String>) -> HostMessage + Send + 'static,
) {
    let sender = answers.sender.clone();
    std::thread::spawn(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = crate::prompt_page::script(&scripts, &folder, &vars, &name, &args);
        let _ = sender.send((app, answer(result)));
    });
}

/// The runner's call for a workspace's step edit, so its answer goes back.
fn edit_call(app: usize, id: &str) -> String {
    format!("step:{app}:{id}")
}

/// One printed line of `pipeline.py sql`: JSON when it holds an object or
/// a list, otherwise the whole line, never split on " | ".
pub(crate) fn canon_row(line: &str) -> Value {
    match serde_json::from_str::<Value>(line) {
        Ok(value @ (Value::Object(_) | Value::Array(_))) => value,
        _ => Value::String(line.to_string()),
    }
}

/// A skill's or a script's name, which names one file and no folder.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

/// The `pipeline.py step` command that sets only what is given.
pub(crate) fn step_command(pipeline: &str, step: &StepRef, script: Option<&str>, refs: Option<&str>) -> Vec<String> {
    let mut argv = vec![String::from("pipeline.py"), String::from("step"), pipeline.to_string(), step.chain.clone(), step.step.clone()];
    if let Some(script) = script {
        argv.extend([String::from("--script"), script.to_string()]);
    }
    if let Some(refs) = refs {
        argv.extend([String::from("--refs"), refs.to_string()]);
    }
    argv
}

impl HostWindow {
    /// Takes the canon reads, step edits, and skill scripts workspaces
    /// asked for.
    pub(super) fn take_canon_steps(&mut self) {
        let mut reads = Vec::new();
        let mut edits = Vec::new();
        let mut scripts = Vec::new();
        for (app, started) in self.apps.iter_mut().enumerate() {
            let declared = &mut started.running.declared;
            reads.extend(declared.canon_reads.drain(..).map(|read| (app, read)));
            edits.extend(declared.step_edits.drain(..).map(|edit| (app, edit)));
            scripts.extend(declared.skill_scripts.drain(..).map(|script| (app, script)));
        }
        if reads.is_empty() && edits.is_empty() && scripts.is_empty() {
            return;
        }
        let project = self.skills_project();
        let pipeline = project.as_ref().and_then(|(folder, _)| crate::prompt_page::living_pipeline(folder));
        let outside = String::from("Only a Skills project has canon.");
        for (app, (id, query)) in reads {
            let (Some((folder, env)), Some(pipeline)) = (&project, &pipeline) else {
                self.send(app, HostMessage::CanonRows { id, rows: Vec::new(), error: Some(outside.clone()) });
                continue;
            };
            let place = (env.scripts.clone().unwrap_or_default(), folder.clone(), env.vars.clone());
            let args = vec![String::from("sql"), pipeline.clone(), query, String::from("--read-only")];
            run_off_thread(&self.canon_answers, app, place, String::from("pipeline.py"), args, move |result| match result {
                Ok(printed) => HostMessage::CanonRows { id, rows: printed.lines().filter(|line| !line.is_empty()).map(canon_row).collect(), error: None },
                Err(error) => HostMessage::CanonRows { id, rows: Vec::new(), error: Some(error) },
            });
        }
        for (app, (id, skill, script, args)) in scripts {
            let found = match &project {
                Some(_) if !plain_name(&skill) || !plain_name(&script) => Err(format!("“{skill}/{script}” names no skill's script.")),
                Some((folder, env)) => {
                    let folder_of = env.scripts.as_ref().and_then(|scripts| scripts.parent()).map(|checkout| checkout.join("skills").join(&skill).join("scripts"));
                    match folder_of {
                        Some(found) if found.join(&script).is_file() => Ok((found, folder.clone(), env.vars.clone())),
                        _ => Err(format!("The skill {skill} has no script {script}.")),
                    }
                }
                None => Err(outside.clone()),
            };
            match found {
                Ok(place) => run_off_thread(&self.canon_answers, app, place, script, args, move |result| match result {
                    Ok(output) => HostMessage::SkillScriptRan { id, output, error: None },
                    Err(error) => HostMessage::SkillScriptRan { id, output: String::new(), error: Some(error) },
                }),
                Err(error) => self.send(app, HostMessage::SkillScriptRan { id, output: String::new(), error: Some(error) }),
            }
        }
        for (app, (id, _tab, step, script, refs)) in edits {
            let (Some((folder, env)), Some(pipeline)) = (project.clone(), pipeline.clone()) else {
                self.send(app, HostMessage::StepEdited { id, error: Some(outside.clone()) });
                continue;
            };
            let command = step_command(&pipeline, &step, script.as_deref(), refs.as_deref());
            let why = format!("Edit the step {}: {}.", step.chain, step.step);
            let call = edit_call(app, &id);
            if let crate::runner::Begun::Answered(answer) = self.runner.begin(&folder, env, &call, "", crate::runner::RUN_COMMAND, &json!({ "command": command, "why": why })) {
                self.answer_step_edit(answer);
            }
        }
    }

    /// Sends the canon reads and skill scripts that finished off the
    /// window's thread.
    pub(super) fn send_canon_answers(&mut self) {
        let finished: Vec<(usize, HostMessage)> = self.canon_answers.receiver.try_iter().collect();
        for (app, message) in finished {
            self.send(app, message);
        }
    }

    /// The runner's answer to a step edit, sent to the workspace that asked.
    /// Returns false when the answer is not a step edit's.
    pub(super) fn answer_step_edit(&mut self, answer: ToolResult) -> bool {
        let Some((app, id)) = answer.call.strip_prefix("step:").and_then(|rest| rest.split_once(':')) else {
            return false;
        };
        let Ok(app) = app.parse::<usize>() else { return false };
        let error = (!answer.ok).then(|| answer.output.get("error").or_else(|| answer.output.get("output")).and_then(Value::as_str).unwrap_or("The step was not edited.").to_string());
        self.send(app, HostMessage::StepEdited { id: id.to_string(), error });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a row split where a script holds " | ", or JSON left as text.
    #[test]
    fn a_canon_row_is_its_json_or_its_whole_line() {
        assert_eq!(canon_row(r#"{"script": "a | b"}"#), json!({"script": "a | b"}));
        assert_eq!(canon_row("python3 assemble.py x.json | the animatic"), json!("python3 assemble.py x.json | the animatic"));
        assert_eq!(canon_row("8.0"), json!("8.0"));
    }

    /// Catches a step edit that would clear what it was not given.
    #[test]
    fn a_step_edit_names_only_what_it_changes() {
        let step = StepRef { chain: "the animatic".into(), step: "animatic".into() };
        assert_eq!(step_command("002", &step, Some("python3 assemble.py a.json"), None), ["pipeline.py", "step", "002", "the animatic", "animatic", "--script", "python3 assemble.py a.json"]);
        assert_eq!(step_command("002", &step, None, Some("[]")), ["pipeline.py", "step", "002", "the animatic", "animatic", "--refs", "[]"]);
    }

    /// Catches a skill or script name that would reach outside its folder.
    #[test]
    fn a_skill_script_names_one_file() {
        assert!(plain_name("write-assembly.py"));
        assert!(!plain_name("../request.py") && !plain_name("..") && !plain_name(""));
    }
}
