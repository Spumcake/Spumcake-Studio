//! The window's side of a workspace's own reads and edits of the chain, in
//! a project: canon read with `pipeline.py sql --read-only`, a step
//! edited with `pipeline.py step` on the user's yes on the runner's card,
//! and a skill's own script run, such as `build-animatic`'s writer, so a
//! workspace never writes a Skills format itself. A step run again as a
//! take waits in `step_runs` for the runner's rerun. Reads and scripts run
//! on threads of their own, so a workspace that polls canon never stalls
//! drawing or playback; their answers are sent on the frame they arrive.

use super::HostWindow;
use protocol::{HostMessage, StepRef, ToolResult};
use serde_json::Value;
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

/// The runner's call for a workspace's element or chain, so its answer goes
/// back.
fn canon_call(app: usize, id: &str) -> String {
    format!("canon:{app}:{id}")
}

/// A step's script names Skills' scripts as `${SPUMCAKE_AGENT}`, as a
/// workspace that does not know where the checkout is writes it; recorded,
/// it falls back to the checkout Studio found, as Skills' own form does.
pub(crate) fn with_checkout(script: &str, checkout: &std::path::Path) -> String {
    script.replace("${SPUMCAKE_AGENT}", &format!("${{SPUMCAKE_AGENT:-{}}}", checkout.display()))
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

/// What a workspace's `edit_step` sets on a step, each only when given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StepFields {
    pub script: Option<String>,
    pub refs: Option<String>,
    pub order: Option<u32>,
    pub skill: Option<String>,
    pub artifact: Option<String>,
    pub endpoint: Option<String>,
}

/// The `pipeline.py step` command that sets only what is given, so it
/// edits a step or, given its order, skill, and artifact, records a new one.
pub(crate) fn step_command(pipeline: &str, step: &StepRef, fields: &StepFields) -> Vec<String> {
    let mut argv = vec![String::from("pipeline.py"), String::from("step"), pipeline.to_string(), step.chain.clone(), step.step.clone()];
    if let Some(order) = fields.order {
        argv.extend([String::from("--order"), order.to_string()]);
    }
    if let Some(skill) = &fields.skill {
        argv.extend([String::from("--skill"), skill.clone()]);
    }
    if let Some(artifact) = &fields.artifact {
        argv.extend([String::from("--artifact"), artifact.clone()]);
    }
    if let Some(endpoint) = &fields.endpoint {
        argv.extend([String::from("--endpoint"), endpoint.clone()]);
    }
    if let Some(script) = &fields.script {
        argv.extend([String::from("--script"), script.clone()]);
    }
    if let Some(refs) = &fields.refs {
        argv.extend([String::from("--refs"), refs.clone()]);
    }
    argv
}

impl HostWindow {
    /// Why the project's scripts cannot run: no project is open, or
    /// Spumcake-Skills is not found.
    pub(super) fn scripts_missing(&self) -> &'static str {
        if self.project.is_none() { crate::skills_checkout::NO_PROJECT } else { crate::skills_checkout::MISSING }
    }

    /// Takes the canon reads, step edits, and skill scripts workspaces
    /// asked for.
    pub(super) fn take_canon_steps(&mut self) {
        let mut reads = Vec::new();
        let mut canon = Vec::new();
        let mut edits = Vec::new();
        let mut scripts = Vec::new();
        for (app, started) in self.apps.iter_mut().enumerate() {
            let declared = &mut started.running.declared;
            reads.extend(declared.canon_reads.drain(..).map(|read| (app, read)));
            canon.extend(declared.canon_edits.drain(..).map(|edit| (app, edit)));
            edits.extend(declared.step_edits.drain(..).map(|edit| (app, edit)));
            scripts.extend(declared.skill_scripts.drain(..).map(|script| (app, script)));
        }
        if reads.is_empty() && edits.is_empty() && scripts.is_empty() && canon.is_empty() {
            return;
        }
        let project = self.project_scripts();
        let pipeline = project.as_ref().and_then(|(folder, _)| crate::prompt_page::living_pipeline(folder));
        let outside = String::from(self.scripts_missing());
        // Skills found, but the project has no pipeline yet to read.
        let no_pipeline = String::from("This project has no pipeline yet.");
        let unread = if project.is_some() { no_pipeline.clone() } else { outside.clone() };
        for (app, (id, query)) in reads {
            let (Some((folder, env)), Some(pipeline)) = (&project, &pipeline) else {
                self.send(app, HostMessage::CanonRows { id, rows: Vec::new(), error: Some(unread.clone()) });
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
        // A workspace records canon on the user's own Save, which spends
        // nothing, so it runs at once, as filing a file does.
        for (app, (id, args)) in canon {
            let (Some((folder, env)), Some(pipeline)) = (project.clone(), pipeline.clone()) else {
                self.send(app, HostMessage::CanonEdited { id, error: Some(unread.clone()) });
                continue;
            };
            let mut command = vec![String::from("pipeline.py"), args[0].clone(), pipeline.clone()];
            command.extend(args[1..].iter().cloned());
            if let Err(reason) = self.runner.run_now(&folder, env, &canon_call(app, &id), &command) {
                self.send(app, HostMessage::CanonEdited { id, error: Some(reason) });
            }
        }
        for (app, (id, _tab, step, mut fields)) in edits {
            let (Some((folder, env)), Some(pipeline)) = (project.clone(), pipeline.clone()) else {
                self.send(app, HostMessage::StepEdited { id, error: Some(unread.clone()) });
                continue;
            };
            if let (Some(script), Some(checkout)) = (&fields.script, env.scripts.as_ref().and_then(|scripts| scripts.parent())) {
                fields.script = Some(with_checkout(script, checkout));
            }
            let command = step_command(&pipeline, &step, &fields);
            if let Err(reason) = self.runner.run_now(&folder, env, &edit_call(app, &id), &command) {
                self.send(app, HostMessage::StepEdited { id, error: Some(reason) });
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
        if let Some(rest) = answer.call.strip_prefix("canon:") {
            let Some((app, id)) = rest.split_once(':') else { return false };
            let Ok(app) = app.parse::<usize>() else { return false };
            let error = (!answer.ok).then(|| answer.output.get("error").or_else(|| answer.output.get("output")).and_then(Value::as_str).unwrap_or("It was not recorded.").to_string());
            self.send(app, HostMessage::CanonEdited { id: id.to_string(), error });
            return true;
        }
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
    use serde_json::json;

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
        let script = StepFields { script: Some("python3 assemble.py a.json".into()), ..Default::default() };
        assert_eq!(step_command("002", &step, &script), ["pipeline.py", "step", "002", "the animatic", "animatic", "--script", "python3 assemble.py a.json"]);
        let refs = StepFields { refs: Some("[]".into()), ..Default::default() };
        assert_eq!(step_command("002", &step, &refs), ["pipeline.py", "step", "002", "the animatic", "animatic", "--refs", "[]"]);
        let new = StepFields { order: Some(1), skill: Some("build-animatic".into()), artifact: Some("The Last Signal: animatic".into()), ..script.clone() };
        assert_eq!(step_command("002", &step, &new)[5..9], ["--order", "1", "--skill", "build-animatic"]);
    }

    /// Catches a recorded script that names Skills' scripts by a variable
    /// Claude Code may not set, with no fallback to the checkout.
    #[test]
    fn a_steps_script_falls_back_to_the_checkout() {
        let script = r#"python3 "${SPUMCAKE_AGENT}/skills/build-animatic/scripts/assemble.py" a.json .build/animatic.mp4"#;
        assert_eq!(
            with_checkout(script, std::path::Path::new("/src/spumcake-skills")),
            r#"python3 "${SPUMCAKE_AGENT:-/src/spumcake-skills}/skills/build-animatic/scripts/assemble.py" a.json .build/animatic.mp4"#
        );
    }

    /// Catches a skill or script name that would reach outside its folder.
    #[test]
    fn a_skill_script_names_one_file() {
        assert!(plain_name("write-assembly.py"));
        assert!(!plain_name("../request.py") && !plain_name("..") && !plain_name(""));
    }
}
