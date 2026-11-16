//! Window checks for Studio as a setup for Spumcake-Skills: in a Skills
//! project the assistant reads the project and the skills, looks at what a
//! request made, and writes nowhere a skill's script owns, and no tool that
//! generates around `request.py` is offered. They drive the real window and
//! a scripted stand-in, so they spend nothing.

use super::{StandIn, call_reply, offered, project_window, scratch, text_reply};
use serde_json::{Value, json};
use std::path::Path;

/// A request's report, as `request.py` writes it, with its record.
const REPORT: &str = "<!doctype html><html><body><h1>2026-11-14_13-30-keyframe-the-line</h1>\
<script type=\"application/json\" id=\"request\">{\"record\": 2, \"id\": \"2026-11-14_13-30-keyframe-the-line\", \
\"skill\": \"generate-keyframe\", \"prompt\": \"the cook in image 1 lifts the towel\", \
\"outputs\": [\"library/2026-11-14_13-30-keyframe-the-line.png\"]}</script></body></html>";

fn skill_repo(at: &Path) {
    std::fs::create_dir_all(at.join(".git")).unwrap();
    for (name, description) in [("generate-keyframe", "Make the on-model keyframes."), ("run-prompt", "Use whenever a written prompt is to be rendered.")] {
        let folder = at.join("skills").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("SKILL.md"), format!("---\nname: {name}\ndescription: {description}\n---\n\n# The {name} method\n")).unwrap();
    }
}

fn tool_text(request: &Value) -> String {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .and_then(|message| message["content"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Catches the assistant blind to a Skills project: with the stand-in model
/// it reads a request's record, looks at its output, and reads a skill;
/// writing into `library/` is refused; the host offers no tool that
/// generates around `request.py`; and every skill is listed as rendering
/// through `request.py`.
#[test]
fn the_assistant_reads_a_skills_project_and_its_skills() {
    let dir = scratch("skills-project");
    let host_tool = |name: &str| protocol::tool_key(crate::route::HOST_ID, "project", name);
    let stand_in = StandIn::start(vec![
        call_reply(&host_tool("read_file"), json!({"path": "requests/2026-11-14_13-30-keyframe-the-line.html"})),
        call_reply(&host_tool("look_at_file"), json!({"path": "library/2026-11-14_13-30-keyframe-the-line.png"})),
        call_reply(&host_tool("read_skill"), json!({"name": "generate-keyframe"})),
        call_reply(&host_tool("write_file"), json!({"folder": "library", "name": "x.png", "text": "x"})),
        text_reply("generate-keyframe made it, from the prompt in its report."),
    ]);
    let mut window = project_window(Vec::new(), &stand_in, &dir);
    let folder = window.app.project.as_ref().unwrap().folder.clone();
    for sub in ["library", "requests", "agent"] {
        std::fs::create_dir_all(folder.join(sub)).unwrap();
    }
    std::fs::write(folder.join("requests/2026-11-14_13-30-keyframe-the-line.html"), REPORT).unwrap();
    std::fs::write(folder.join("library/2026-11-14_13-30-keyframe-the-line.png"), b"\x89PNG\r\n\x1a\nkeyframe").unwrap();
    let repo = dir.join("spumcake-skills");
    skill_repo(&repo);
    window.app.settings.skill_folders = vec![repo.display().to_string()];
    window.ask("What made this keyframe?");
    window.until(|app| !app.chat.working && stand_in.requests().len() >= 5);
    let requests = stand_in.requests();

    let tools = offered(&requests[0]);
    for generating in ["run_text_prompt", "edit_image", "list_out_of_date"] {
        assert!(!tools.iter().any(|tool| tool.ends_with(generating)), "{generating} is offered in a Skills project: {tools:?}");
    }
    for name in ["list_files", "read_file", "look_at_file", "write_file", "read_skill"] {
        assert!(tools.contains(&host_tool(name)), "{name} is not offered: {tools:?}");
    }
    let system = requests[0]["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("request.py is the one renderer"), "{system}");
    for name in ["generate-keyframe", "run-prompt"] {
        assert!(system.lines().any(|line| line.starts_with(&format!("- {name}:")) && line.contains("renders through request.py here")), "{name}: {system}");
    }

    assert!(tool_text(&requests[1]).contains("the cook in image 1 lifts the towel"), "the record was not read: {}", tool_text(&requests[1]));
    assert!(requests[2].to_string().contains("data:image/png;base64,"), "the keyframe was not shown to the model");
    let shown: Vec<&String> = window.app.chat.conversation.iter().flat_map(|entry| &entry.attachments).collect();
    assert!(
        shown.iter().any(|path| Path::new(path).is_absolute() && path.ends_with("library/2026-11-14_13-30-keyframe-the-line.png")),
        "the person is not shown the keyframe's full path: {shown:?}"
    );
    let skill = tool_text(&requests[3]);
    assert!(skill.contains("# The generate-keyframe method") && skill.contains("renders through request.py here"), "{skill}");
    assert!(tool_text(&requests[4]).contains("error"), "writing into library/ was not refused: {}", tool_text(&requests[4]));
    assert!(!folder.join("library/x.png").exists());
    let _ = std::fs::remove_dir_all(dir);
}
