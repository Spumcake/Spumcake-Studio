//! The assistant's "Video Generator" tool group: it sees a tab's takes,
//! opens a video file into the gallery, shows a take at a frame, and
//! switches the view. Each call goes through the same changes the
//! filmstrip and the toolbar make; opening a take and changing the view are
//! steps of the gallery's history, and choosing a take or a frame is not.

use crate::gallery::Mode;
use crate::session::Session;
use protocol::{AppMessage, Tool, ToolGroup, ToolResult};
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn tool_groups() -> Vec<ToolGroup> {
    let take = json!({"type": "integer", "minimum": 0, "description": "The take's position on the filmstrip, from list_takes, 0 being the first."});
    let object = |properties: Value, required: &[&str]| {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    };
    vec![ToolGroup {
        id: "takes".into(),
        name: "Video Generator".into(),
        description: "See the takes in a Video Generator tab's gallery, open a video file into it as a take, show a take at a frame, and show the takes alone, as frame strips, or two side by side.".into(),
        instructions: "A Video Generator tab is a gallery: a filmstrip of takes, each a link to a video file that is never changed. Call list_takes to see them in order, with each one's length, which is shown, the view, and the frame shown. Takes are named by their position, 0 being the first, and positions change when takes are added, moved, or removed. The view is single (the shown take alone), strips (every take as a row of eight frames at equal times), or side_by_side (the shown take beside another, at the same frame). Frames count from 0.".into(),
        tools: vec![
            Tool {
                name: "list_takes".into(),
                description: "List the gallery's takes in filmstrip order, with each one's position, name, file, size, frames, frames per second, and length in seconds, and which is shown, the view, and the frame shown.".into(),
                input: object(json!({}), &[]),
                implements: Vec::new(),
            },
            Tool {
                name: "open_video".into(),
                description: "Add a video file to the end of the filmstrip as a take, linked and never copied, and show it. Returns its position, name, and length.".into(),
                input: object(
                    json!({
                        "path": {"type": "string", "description": "The video file: absolute, or relative to the project's folder."},
                        "name": {"type": "string", "description": "The take's name. Leave out for the file's name; a name already used gets a number."},
                    }),
                    &["path"],
                ),
                implements: Vec::new(),
            },
            Tool {
                name: "show_take".into(),
                description: "Show a take, as a click on its thumbnail does, at a frame.".into(),
                input: object(
                    json!({
                        "take": take,
                        "frame": {"type": "integer", "minimum": 0, "description": "The frame to show, from 0. Leave out to keep the frame shown; past the end shows the last."},
                    }),
                    &["take"],
                ),
                implements: Vec::new(),
            },
            Tool {
                name: "set_view".into(),
                description: "Show the shown take alone, every take as frame strips, or the shown take side by side with another at the same frame.".into(),
                input: object(
                    json!({
                        "view": {"type": "string", "enum": ["single", "strips", "side_by_side"]},
                        "with": {"type": "integer", "minimum": 0, "description": "For side_by_side: the other take's position, from list_takes."},
                    }),
                    &["view"],
                ),
                implements: Vec::new(),
            },
        ],
    }]
}

/// Runs one tool on the session's active tab.
pub fn run(session: &mut Session, tool: &str, input: &Value) -> Result<Value, String> {
    let position = |key: &str| input.get(key).and_then(Value::as_u64).map(|value| value as usize);
    match tool {
        "list_takes" => {
            let project = session.project.clone();
            let tab = session.active_tab().ok_or("There is no tab.")?;
            let gallery = &tab.gallery;
            let view = match gallery.view() {
                Mode::Single => "single",
                Mode::Strips => "strips",
                Mode::Side { .. } => "side_by_side",
            };
            Ok(json!({
                "shown": gallery.take().map(|_| gallery.shown),
                "view": view,
                "with": gallery.other(),
                "frame": tab.frame_shown(),
                "takes": gallery
                    .takes
                    .iter()
                    .enumerate()
                    .map(|(index, take)| json!({
                        "take": index,
                        "name": take.name,
                        "file": crate::files::link_path(&take.file, project.as_deref()),
                        "width": take.info.map(|info| info.width),
                        "height": take.info.map(|info| info.height),
                        "frames": take.info.map(|info| info.frames),
                        "fps": take.info.map(|info| info.fps()),
                        "seconds": take.seconds(),
                        "shown": index == gallery.shown,
                    }))
                    .collect::<Vec<_>>(),
            }))
        }
        "open_video" => {
            let path = input.get("path").and_then(Value::as_str).ok_or("Give the video file as `path`.")?;
            let path = PathBuf::from(path);
            let path = match &session.project {
                Some(project) if path.is_relative() => project.join(path),
                _ => path,
            };
            let name = input.get("name").and_then(Value::as_str);
            let index = session.add_take(&path, name)?;
            let tab = session.active_tab().ok_or("There is no tab.")?;
            let take = &tab.gallery.takes[index];
            Ok(json!({"take": index, "name": take.name, "frames": take.info.map(|info| info.frames), "seconds": take.seconds()}))
        }
        "show_take" => {
            let index = position("take").ok_or("Give the take's position as `take`.")?;
            let frame = input.get("frame").and_then(Value::as_u64);
            let tab = session.tabs.get_mut(session.active).ok_or("There is no tab.")?;
            tab.gallery.select(index)?;
            if let Some(frame) = frame {
                tab.frame = u32::try_from(frame).unwrap_or(u32::MAX).min(tab.gallery.takes[index].last_frame());
            }
            Ok(json!({"take": index, "name": tab.gallery.takes[index].name, "frame": tab.frame_shown()}))
        }
        "set_view" => {
            let tab = session.tabs.get_mut(session.active).ok_or("There is no tab.")?;
            let gallery = &mut tab.gallery;
            let shown = gallery.shown;
            let mode = match input.get("view").and_then(Value::as_str) {
                Some("single") => Mode::Single,
                Some("strips") => Mode::Strips,
                Some("side_by_side") => {
                    let with = position("with").ok_or("Give the other take's position as `with`.")?;
                    let other = gallery.uid_at(with).ok_or_else(|| format!("There is no take {with}; the gallery holds {}.", gallery.takes.len()))?;
                    Mode::Side { other }
                }
                _ => return Err("Give the view as `single`, `strips`, or `side_by_side`.".into()),
            };
            if gallery.takes.is_empty() {
                return Err("The gallery has no takes yet.".into());
            }
            gallery.set_view(mode, shown)?;
            Ok(json!({"view": input.get("view"), "with": gallery.other()}))
        }
        _ => Err(format!("There is no tool {tool}.")),
    }
}

pub fn result(call: String, outcome: Result<Value, String>) -> AppMessage {
    let (ok, output) = match outcome {
        Ok(output) => (true, output),
        Err(error) => (false, json!({"error": error})),
    };
    AppMessage::ToolResult(ToolResult { call, ok, output, images: Vec::new() })
}
