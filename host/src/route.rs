//! Routing between the assistant and the running sub-applications.
//!
//! The host offers the assistant every running sub-application, with its
//! name, its open tabs, and the skills it declared. When the assistant calls
//! a tool, the host sends `invoke` to the process of that tool's application
//! and passes back the `result` it answers. A process that has exited, or
//! stops answering, gets the call answered for it with `ok: false`. The host
//! never knows a skill or a tool by name.

use crate::runner::{self, Begun, Runner, ScriptEnv};
use crate::skills_setup;
use crate::window::Started;
use crate::workspace::{DISCUSSION, Makes, Settings, SkillSetting};
use assistant::{Command, TabInfo, Target};
use protocol::{HostMessage, ToolResult};
use serde_json::json;
use std::path::Path;
use std::time::{Duration, Instant};

/// A call with no answer after this long is answered as failed.
const NO_RESULT: Duration = Duration::from_secs(120);

/// One call sent to a sub-application and not yet answered.
struct Pending {
    call: String,
    /// The index of the sub-application in the window's list.
    app: usize,
    sent: Instant,
}

#[derive(Default)]
pub(crate) struct Routing {
    pending: Vec<Pending>,
    /// The targets the assistant was last told about.
    offered: Option<Vec<Target>>,
}

/// The host's own id, for the tool it offers. The assistant is not told to
/// treat this id differently from any other.
pub(crate) const HOST_ID: &str = "inpainter";

/// The host's tool: a prompt that runs as a cell. Learned the same way as
/// any application's skill.
pub(crate) fn host_tools() -> protocol::ToolGroup {
    protocol::ToolGroup {
        id: PROMPT_TOOLS.into(),
        name: "Prompt".into(),
        description: "Write a prompt and run it as a cell in the conversation, and find the project's work that is out of date.".into(),
        instructions: "When the person asks for a piece of writing, or to write or run a prompt, call run_text_prompt with the prompt. Do not write the piece in the reply. When they ask to edit an image an Image cell made, call edit_image with the cell's id, or with none for the latest. When they ask what is out of date, or to render again what changed, call list_out_of_date: it lists each out-of-date item with its workspace, those made from others after the others; render each again with its workspace's own tools, in that order. After the tool returns, reply briefly.".into(),
        tools: vec![
            protocol::Tool {
                name: RUN_PROMPT.into(),
                description: "Show a text prompt as a cell and run it.".into(),
                input: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "prompt": {"type": "string", "description": "The prompt to run."}
                    },
                    "required": ["prompt"],
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
            protocol::Tool {
                name: LIST_OUT_OF_DATE.into(),
                description: "List every item of the project that is out of date, because something it was made from changed: each with its file, its id, its name, its workspace, and what changed, those made from others after the others.".into(),
                input: serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false }),
                implements: Vec::new(),
            },
            protocol::Tool {
                name: EDIT_IMAGE.into(),
                description: "Open the image an Image cell in the conversation shows as new work, in its own tab, ready to paint. The cell is unchanged.".into(),
                input: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "cell": {"type": "string", "description": "The Image cell's id. Leave it out for the latest Image cell."}
                    },
                    "additionalProperties": false
                }),
                implements: Vec::new(),
            },
        ],
    }
}

/// The host's own tool group, which runs prompts and opens cells' images.
pub(crate) const PROMPT_TOOLS: &str = "prompt";

/// The tool that lists the project's out-of-date items, upstream first,
/// as the owner named it on 2026-11-06.
pub(crate) const LIST_OUT_OF_DATE: &str = "list_out_of_date";

/// The tool that opens a cell's image as new work. A skill of the
/// person's never takes this name.
pub(crate) const EDIT_IMAGE: &str = "edit_image";

/// `EDIT_IMAGE`'s name before 0.14, the spelling earlier conversations
/// stored. It is read as `EDIT_IMAGE`, so a later turn can call it again.
const FORMER_EDIT_IMAGE: &str = "edit_picture"; // stored spelling

/// A stored tool key as the host's tools name it now: the host's own tool
/// under its former name becomes `EDIT_IMAGE`; any other key is unchanged.
pub(crate) fn current_host_tool_key(key: &str) -> String {
    match protocol::split_tool_key(key) {
        Some((HOST_ID, PROMPT_TOOLS, FORMER_EDIT_IMAGE)) => protocol::tool_key(HOST_ID, PROMPT_TOOLS, EDIT_IMAGE),
        _ => key.to_string(),
    }
}

/// The tool the host's prompt skill offers. A skill of the person's never
/// takes this name.
const RUN_PROMPT: &str = "run_text_prompt";

/// The skills the person made that the assistant may call: every skill
/// but Discussion whose chosen endpoint answers prompts. Each with its
/// tool's name: a slug of the skill's name, made unique.
pub(crate) fn offered_skills(settings: &Settings) -> Vec<(String, SkillSetting)> {
    let mut offered: Vec<(String, SkillSetting)> = Vec::new();
    for skill in &settings.skills {
        let chats = settings.skill_runs(skill);
        // A skill with no model chosen yet, and none to fall back on, is not
        // offered.
        let model = settings.skill_route(skill).is_some_and(|(_, model)| !model.trim().is_empty());
        if skill.id == DISCUSSION || !chats || !model {
            continue;
        }
        let mut slug: String = skill
            .name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        slug = slug.split('_').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("_");
        slug.truncate(48);
        if slug.is_empty() || slug == RUN_PROMPT || slug == EDIT_IMAGE {
            slug = format!("skill_{slug}").trim_end_matches('_').to_string();
        }
        let mut name = slug.clone();
        let mut n = 2;
        while offered.iter().any(|(taken, _)| *taken == name) {
            name = format!("{slug}_{n}");
            n += 1;
        }
        offered.push((name, skill.clone()));
    }
    offered
}

/// The person's skill a call to the host runs, if it is one. `key` is the
/// tool's full key, as the assistant names it.
pub(crate) fn skill_for_tool(settings: &Settings, key: &str) -> Option<SkillSetting> {
    let (_, skill, tool) = protocol::split_tool_key(key)?;
    if skill != SKILLS {
        return None;
    }
    offered_skills(settings).into_iter().find(|(name, _)| name == tool).map(|(_, skill)| skill)
}

/// The id of the host's skill that holds the person's skills.
const SKILLS: &str = "skills";

/// "Your skills": one tool for each skill the person made, as they named
/// and described it. Calling one runs its prompt, with the input after it,
/// as a cell on the endpoint and model chosen for it, or on the endpoint
/// and model the assistant names from those it is told of.
fn your_skills(settings: &Settings) -> Option<protocol::ToolGroup> {
    let offered = offered_skills(settings);
    if offered.is_empty() {
        return None;
    }
    let tools = offered
        .into_iter()
        .map(|(name, skill)| {
            let described = if skill.description.trim().is_empty() { skill.name.clone() } else { format!("{}: {}", skill.name, skill.description) };
            // Where it runs now, so the assistant can keep it there or name
            // another endpoint and model.
            let described = match settings.skill_route(&skill) {
                Some((endpoint, model)) => format!("{described}. Runs on {} with {model} unless you name another endpoint and model", endpoint.name),
                None => described,
            };
            // An image skill makes one image from a prompt, shown as an
            // Image cell; the assistant writes the whole description.
            let (description, input) = match skill.makes {
                Makes::Images => (
                    format!("{described}. Makes one image from the prompt and shows it as an Image cell in the conversation."),
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string", "description": "The image, described in full."},
                            "endpoint": {"type": "string", "description": ON_ENDPOINT},
                            "model": {"type": "string", "description": ON_MODEL}
                        },
                        "required": ["prompt"],
                        "additionalProperties": false
                    }),
                ),
                Makes::Text => (
                    described,
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "input": {"type": "string", "description": "What to ask the skill for."},
                            "endpoint": {"type": "string", "description": ON_ENDPOINT},
                            "model": {"type": "string", "description": ON_MODEL}
                        },
                        "required": ["input"],
                        "additionalProperties": false
                    }),
                ),
            };
            protocol::Tool { name, description, input, implements: Vec::new() }
        })
        .collect();
    Some(protocol::ToolGroup {
        id: SKILLS.into(),
        name: "Your skills".into(),
        description: "Skills the person made, each running its own prompt on the endpoint chosen for it.".into(),
        instructions: "When the person asks for something one of these skills is described for, call it with what to ask. When they ask for an image, call a skill that makes images with the image described in full. Its result shows as a cell in the conversation. After the tool returns, reply briefly.".into(),
        tools,
    })
}

/// What a skill tool's `endpoint` input says.
const ON_ENDPOINT: &str = "An endpoint's name, from those listed, to run on instead of the one chosen for the skill. Leave it out to use that one.";
/// What a skill tool's `model` input says.
const ON_MODEL: &str = "With endpoint, the model to run on: one its list gives.";

/// Every endpoint as the assistant is told of it, so it can pair a skill
/// with one: its name, its kind, the user's description, and the models its
/// list gives when they are known. Never a key, and never an address.
fn endpoints_seen(settings: &Settings, listed: &crate::endpoints::Listed) -> String {
    if settings.endpoints.is_empty() {
        return String::new();
    }
    let mut seen = String::from("\n\nThe endpoints the user has set up:");
    for endpoint in &settings.endpoints {
        seen.push_str(&format!("\n- {} ({})", endpoint.name, endpoint.kind.label()));
        if !endpoint.description.trim().is_empty() {
            seen.push_str(&format!(": {}", endpoint.description.trim()));
        }
        if let Some((chat, images)) = listed.get(&endpoint.id) {
            const MOST: usize = 40;
            for (what, models) in [("Chat models", chat), ("Image models", images)] {
                if models.is_empty() {
                    continue;
                }
                let shown: Vec<&str> = models.iter().take(MOST).map(String::as_str).collect();
                let more = if models.len() > MOST { format!(", and {} more", models.len() - MOST) } else { String::new() };
                seen.push_str(&format!(". {what}: {}{more}", shown.join(", ")));
            }
        }
    }
    seen
}

/// The host's tools: in a project, the project tools and Spumcake-Skills'
/// skills, through which the project's work runs on `request.py`; and in
/// every project, the conversation's own prompt tools and the user's
/// skills, which make cells in the conversation, with the endpoints the
/// user has set up.
fn host_target(settings: &Settings, project: Option<&Path>, listed: &crate::endpoints::Listed) -> Target {
    // The conversation's own prompt tools and the user's skills are offered
    // in every project, as the owner set on 2026-11-17, beside the project's
    // tools.
    let mut tool_groups = Vec::new();
    if let Some(folder) = project {
        tool_groups.push(skills_setup::tool_group(folder, &skills_setup::listed_skills(&crate::skills_checkout::skill_folders(&settings.skill_folders))));
    }
    let mut prompts = host_tools();
    prompts.instructions.push_str(&endpoints_seen(settings, listed));
    tool_groups.push(prompts);
    tool_groups.extend(your_skills(settings));
    Target {
        app: HOST_ID.into(),
        name: "Spumcake Studio".into(),
        tabs: Vec::new(),
        tool_groups,
        tabless: true,
    }
}

/// Every running sub-application, and the host's own tools, as a request
/// offers them, for the project open in `project`.
pub(crate) fn targets(apps: &[Started], settings: &Settings, project: Option<&Path>, listed: &crate::endpoints::Listed) -> Vec<Target> {
    let mut offered: Vec<Target> = apps
        .iter()
        .filter(|started| !started.running.exited() && !started.running.pending())
        .map(|started| Target {
            app: started.id.clone(),
            name: started.name.clone(),
            tabs: started
                .running
                .tabs()
                .0
                .into_iter()
                .map(|tab| TabInfo {
                    id: tab.id,
                    label: tab.label,
                })
                .collect(),
            tool_groups: started.running.declared.tool_groups.clone(),
            tabless: false,
        })
        .collect();
    offered.push(host_target(settings, project, listed));
    offered
}

/// What a call to the host's project tools comes to.
pub(crate) enum ProjectAnswer {
    /// Answered at once.
    Now(ToolResult),
    /// The runner answers it, after the person and the command.
    Later,
    /// The call is for another of the host's tools.
    NotMine,
}

/// A call to the host's project tools. A runner tool waits for the
/// person's yes; `env` gives what its scripts run with.
pub(crate) fn project_call(
    project: Option<&Path>,
    settings: &Settings,
    runner: &mut Runner,
    env: impl FnOnce(&[skills_setup::Skill]) -> ScriptEnv,
    call: &str,
    tool: &str,
    input: &serde_json::Value,
) -> ProjectAnswer {
    if !skills_setup::is_project_tool(tool) {
        return ProjectAnswer::NotMine;
    }
    let Some((_, _, name)) = protocol::split_tool_key(tool) else { return ProjectAnswer::NotMine };
    let Some(folder) = project else {
        return ProjectAnswer::Now(failed(call.to_string(), crate::skills_checkout::NO_PROJECT));
    };
    let skills = skills_setup::listed_skills(&crate::skills_checkout::skill_folders(&settings.skill_folders));
    if runner::is_runner_tool(name) {
        return match runner.begin(folder, env(&skills), call, tool, name, input) {
            Begun::Answered(result) => ProjectAnswer::Now(result),
            Begun::Waiting => ProjectAnswer::Later,
        };
    }
    ProjectAnswer::Now(skills_setup::call(folder, &skills, call.to_string(), name, input))
}

fn failed(call: String, reason: &str) -> ToolResult {
    ToolResult {
        call,
        ok: false,
        output: json!({ "error": reason }),
        images: Vec::new(),
    }
}

impl Routing {
    /// The assistant asks for a tool. Returns an answer at once when the
    /// call cannot be sent.
    pub(crate) fn call(
        &mut self,
        apps: &mut [Started],
        call: String,
        app: &str,
        message: HostMessage,
    ) -> Option<Command> {
        let Some(index) = apps.iter().position(|started| started.id == app) else {
            return Some(Command::Result(failed(
                call,
                "No running application has that id.",
            )));
        };
        let started = &mut apps[index];
        if started.running.exited() {
            return Some(Command::Result(failed(
                call,
                &format!("{} has exited.", started.name),
            )));
        }
        started.running.send(&message);
        self.pending.push(Pending {
            call,
            app: index,
            sent: Instant::now(),
        });
        None
    }

    /// The answers that arrived this frame, and failures for calls whose
    /// process exited, stopped responding, or never answered.
    pub(crate) fn poll(&mut self, apps: &mut [Started]) -> Vec<Command> {
        let mut out = Vec::new();
        for (index, started) in apps.iter_mut().enumerate() {
            while let Some(result) = started.running.declared.results.pop_front() {
                let waiting = self
                    .pending
                    .iter()
                    .position(|pending| pending.app == index && pending.call == result.call);
                // A late answer, after its call was given up, is dropped.
                if let Some(position) = waiting {
                    self.pending.remove(position);
                    out.push(Command::Result(result));
                }
            }
        }
        let mut kept = Vec::new();
        for pending in std::mem::take(&mut self.pending) {
            let reason = match apps.get(pending.app) {
                None => Some("The application is no longer running.".to_string()),
                Some(started) if started.running.exited() => {
                    Some(format!("{} has exited.", started.name))
                }
                Some(started) if started.running.not_responding() => {
                    Some(format!("{} is not responding.", started.name))
                }
                Some(started) if pending.sent.elapsed() >= NO_RESULT => {
                    Some(format!("{} did not answer.", started.name))
                }
                Some(_) => None,
            };
            match reason {
                Some(reason) => out.push(Command::Result(failed(pending.call, &reason))),
                None => kept.push(pending),
            }
        }
        self.pending = kept;
        out
    }

    /// The running sub-applications, when they changed since the assistant
    /// was last told.
    pub(crate) fn changed_targets(&mut self, apps: &[Started], settings: &Settings, project: Option<&Path>, listed: &crate::endpoints::Listed) -> Option<Vec<Target>> {
        let now = targets(apps, settings, project, listed);
        if self.offered.as_ref() == Some(&now) {
            return None;
        }
        self.offered = Some(now.clone());
        Some(now)
    }

    /// The request ended: no call is waited for any longer.
    pub(crate) fn end(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {

    /// Catches a session from before 0.14 whose earlier calls to the host's
    /// image tool no longer read: the former name is the current one.
    #[test]
    fn the_host_tools_former_name_is_its_current_one() {
        let former = protocol::tool_key(HOST_ID, PROMPT_TOOLS, FORMER_EDIT_IMAGE);
        assert_eq!(current_host_tool_key(&former), protocol::tool_key(HOST_ID, PROMPT_TOOLS, EDIT_IMAGE));
        assert_eq!(current_host_tool_key("app__skill__tool"), "app__skill__tool");
    }
    use super::*;

    /// Catches a skill offered before it can run: New Skill makes one with
    /// no model, which waits until it has one. Discussion is never a tool.
    #[test]
    fn only_skills_that_can_run_are_offered() {
        let mut settings = Settings::default();
        let endpoint = settings.first_start("https://stand-in/v1").unwrap();
        let skill = |name: &str| SkillSetting {
            id: name.to_string(),
            name: name.to_string(),
            description: String::new(),
            former_endpoint: None,
            former_model: None,
            prompt: String::new(),
            makes: crate::workspace::Makes::Text,
        };
        settings.skills.push(skill("Story notes"));
        settings.skills.push(skill("Story  Notes!"));
        settings.skill_routes.insert(String::from("Story notes"), crate::workspace::SkillRoute { endpoint: endpoint.clone(), model: String::from("m") });
        // A text skill with no choice of its own runs on the default level.
        let names: Vec<String> = offered_skills(&settings).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["story_notes", "story_notes_2"]);
        assert!(skill_for_tool(&settings, "inpainter__skills__story_notes").is_some());
        // One whose chosen endpoint is gone, and with no endpoint to fall
        // back on, is not offered.
        let mut lone = Settings::default();
        lone.skills.push(skill("Story notes"));
        assert!(offered_skills(&lone).is_empty(), "a skill with nowhere to run was offered");
    }

    /// Catches the assistant told an endpoint's key or address, or not told
    /// what the user wrote it is for.
    #[test]
    fn the_assistant_sees_each_endpoint_but_never_its_secrets() {
        let mut settings = Settings::default();
        let id = settings.first_start("https://stand-in/v1?key=secret").unwrap();
        settings.endpoints[0].description = String::from("Fast image drafts");
        let listed: crate::endpoints::Listed = [(id, (vec![String::from("anthropic/claude-haiku-5.5")], vec![String::from("openai/gpt-image-2.5-sunburst")]))].into_iter().collect();
        let seen = endpoints_seen(&settings, &listed);
        assert!(seen.contains("OpenRouter (OpenRouter): Fast image drafts. Chat models: anthropic/claude-haiku-5.5. Image models: openai/gpt-image-2.5-sunburst"), "{seen}");
        assert!(!seen.contains("stand-in") && !seen.contains("secret"), "an address reached the assistant: {seen}");
    }
}
