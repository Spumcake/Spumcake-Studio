//! Routing between the assistant and the running sub-applications.
//!
//! The host offers the assistant every running sub-application, with its
//! name, its open tabs, and the skills it declared. When the assistant calls
//! a tool, the host sends `invoke` to the process of that tool's application
//! and passes back the `result` it answers. A process that has exited, or
//! stops answering, gets the call answered for it with `ok: false`. The host
//! never knows a skill or a tool by name.

use crate::window::Started;
use crate::workspace::{DISCUSSION, Makes, Settings, SkillSetting};
use assistant::{Command, TabInfo, Target};
use protocol::{HostMessage, ToolResult};
use serde_json::json;
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
        description: "Write a prompt and run it as a cell in the conversation.".into(),
        instructions: "When the person asks for a piece of writing, or to write or run a prompt, call run_text_prompt with the prompt. Do not write the piece in the reply. When they ask to edit an image an Image cell made, call edit_image with the cell's id, or with none for the latest. After the tool returns, reply briefly.".into(),
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
/// but Discussion whose endpoint answers prompts. Each with its tool's name:
/// a slug of the skill's name, made unique.
pub(crate) fn offered_skills(settings: &Settings) -> Vec<(String, SkillSetting)> {
    let mut offered: Vec<(String, SkillSetting)> = Vec::new();
    for skill in &settings.skills {
        let chats = settings.skill_runs(skill);
        // A skill with no model yet, as New Skill makes it, is not offered.
        let model = skill.model.as_deref().is_some_and(|model| !model.trim().is_empty());
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
/// as a cell on its own endpoint and model.
fn your_skills(settings: &Settings) -> Option<protocol::ToolGroup> {
    let offered = offered_skills(settings);
    if offered.is_empty() {
        return None;
    }
    let tools = offered
        .into_iter()
        .map(|(name, skill)| {
            let described = if skill.description.trim().is_empty() { skill.name.clone() } else { format!("{}: {}", skill.name, skill.description) };
            // An image skill makes one image from a prompt, shown as an
            // Image cell; the assistant writes the whole description.
            let (description, input) = match skill.makes {
                Makes::Images => (
                    format!("{described}. Makes one image from the prompt and shows it as an Image cell in the conversation."),
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "prompt": {"type": "string", "description": "The image, described in full."}
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
                            "input": {"type": "string", "description": "What to ask the skill for."}
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
        description: "Skills the person made, each running its own prompt on its own endpoint.".into(),
        instructions: "When the person asks for something one of these skills is described for, call it with what to ask. When they ask for an image, call a skill that makes images with the image described in full. Its result shows as a cell in the conversation. After the tool returns, reply briefly.".into(),
        tools,
    })
}

fn host_target(settings: &Settings) -> Target {
    let mut tool_groups = vec![host_tools()];
    tool_groups.extend(your_skills(settings));
    Target {
        app: HOST_ID.into(),
        name: "Spumcake Studio".into(),
        tabs: Vec::new(),
        tool_groups,
        tabless: true,
    }
}

/// Every running sub-application, and the host's own tools, as a request offers them.
pub(crate) fn targets(apps: &[Started], settings: &Settings) -> Vec<Target> {
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
    offered.push(host_target(settings));
    offered
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
    pub(crate) fn changed_targets(&mut self, apps: &[Started], settings: &Settings) -> Option<Vec<Target>> {
        let now = targets(apps, settings);
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
        let skill = |name: &str, model: Option<&str>| SkillSetting {
            id: name.to_string(),
            name: name.to_string(),
            description: String::new(),
            endpoint: Some(endpoint.clone()),
            model: model.map(String::from),
            prompt: String::new(),
            makes: crate::workspace::Makes::Text,
        };
        settings.skills.push(skill("New Skill", None));
        settings.skills.push(skill("Story notes", Some("m")));
        settings.skills.push(skill("Story  Notes!", Some("m")));
        let names: Vec<String> = offered_skills(&settings).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["story_notes", "story_notes_2"]);
        assert!(skill_for_tool(&settings, "inpainter__skills__story_notes").is_some());
        assert!(skill_for_tool(&settings, "inpainter__skills__new_skill").is_none());
    }
}
