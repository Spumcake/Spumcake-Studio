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

/// A stand-in for Spumcake-Skills' request.py: `plan` prints the plan of a
/// spec with a hash of the spec, and `run` refuses unless `--expect` is
/// that hash, then files an output and prints what to tell the person.
const STAND_IN_REQUEST: &str = r#"import hashlib, json, os, sys
cmd, spec = sys.argv[1], sys.argv[2]
text = open(spec).read()
h = hashlib.sha256(text.encode()).hexdigest()
def plan():
    s = json.loads(text)
    print(json.dumps({"plan": 1, "endpoint": {"kind": "fal", "model": "stand-in/model", "settings": {"seed": 7}},
                      "prompt": s["prompt"], "references": [], "requests": 1, "estimate": "unknown: no price", "hash": h}))
if cmd == "plan":
    plan()
elif cmd == "run":
    expect = sys.argv[sys.argv.index("--expect") + 1]
    if expect != h:
        sys.exit("refused: what this would send is not the plan approved")
    open("library/2026-11-16_12-00-take.png", "wb").write(b"png")
    print("--- tell the user, word for word ---")
    print("made library/2026-11-16_12-00-take.png")
"#;

/// Sends a message without waiting for its reply, which waits here for
/// the person's yes to a plan.
fn send_message(window: &mut super::Window, text: &str) {
    window.app.chat.input = text.to_string();
    let ctx = window.ctx.clone();
    let super::Window { app, frame, .. } = window;
    app.chat_event(crate::chat::ChatEvent::Send, &ctx, frame);
}

/// Steps the window until the runner asks the person, saying, if it never
/// does, how long the slowest frame took and what the conversation holds.
fn wait_for_card(window: &mut super::Window) {
    let start = std::time::Instant::now();
    let mut slowest = std::time::Duration::ZERO;
    while start.elapsed() < std::time::Duration::from_secs(20) {
        let frame = std::time::Instant::now();
        window.step();
        slowest = slowest.max(frame.elapsed());
        if window.app.runner.asking().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let entries: Vec<String> = window.app.chat.conversation.iter().map(|entry| format!("{:?} ok={:?} {:?}", entry.role, entry.ok, entry.text)).collect();
    panic!("no plan card; slowest frame {slowest:?}; {entries:?}");
}

fn stand_in_skills(at: &Path) {
    skill_repo(at);
    std::fs::create_dir_all(at.join("scripts")).unwrap();
    std::fs::write(at.join("scripts/request.py"), STAND_IN_REQUEST).unwrap();
}

/// Catches a request sent before the person approves its plan, or one sent
/// after its spec changed: the plan card waits with the prompt as sent,
/// nothing runs until its yes, the run carries the approved plan's hash,
/// and a spec changed after the yes is refused with nothing made.
#[test]
fn a_request_runs_only_as_its_approved_plan() {
    let dir = scratch("skills-plan-card");
    let send = protocol::tool_key(crate::route::HOST_ID, "project", "send_request");
    let stand_in = StandIn::start(vec![
        call_reply(&send, json!({"spec": "agent/spec.json"})),
        text_reply("It is made."),
        call_reply(&send, json!({"spec": "agent/spec.json"})),
        text_reply("It was refused."),
    ]);
    let mut window = project_window(Vec::new(), &stand_in, &dir);
    let folder = window.app.project.as_ref().unwrap().folder.clone();
    for sub in ["library", "requests", "agent"] {
        std::fs::create_dir_all(folder.join(sub)).unwrap();
    }
    std::fs::write(folder.join("agent/spec.json"), r#"{"name": "take", "prompt": "the cook lifts the towel"}"#).unwrap();
    let repo = dir.join("spumcake-skills");
    stand_in_skills(&repo);
    window.app.settings.skill_folders = vec![repo.display().to_string()];
    window.app.runner.assume_prerequisites(&folder);
    let take = folder.join("library/2026-11-16_12-00-take.png");

    send_message(&mut window, "Make the take.");
    wait_for_card(&mut window);
    match window.app.runner.asking() {
        Some(crate::runner::Asking::Plans { plans, runs, .. }) => {
            assert_eq!(plans[0]["prompt"], "the cook lifts the towel");
            let hash = plans[0]["hash"].as_str().unwrap();
            assert!(runs[0].windows(2).any(|pair| pair[0] == "--expect" && pair[1] == hash), "{runs:?}");
        }
        other => panic!("no plan card: {other:?}"),
    }
    assert!(!take.exists(), "it ran before the yes");
    let answer = window.app.runner.answer(true);
    assert!(answer.is_none());
    window.until(|app| !app.chat.working && stand_in.requests().len() >= 2);
    assert!(take.exists(), "the approved request did not run");
    assert!(stand_in.requests()[1].to_string().contains("tell the user, word for word"));

    std::fs::remove_file(&take).unwrap();
    send_message(&mut window, "Make it again.");
    wait_for_card(&mut window);
    std::fs::write(folder.join("agent/spec.json"), r#"{"name": "take", "prompt": "a changed prompt"}"#).unwrap();
    assert!(window.app.runner.answer(true).is_none());
    window.until(|app| !app.chat.working && stand_in.requests().len() >= 4);
    assert!(!take.exists(), "a spec changed after its approval was run");
    assert!(stand_in.requests()[3].to_string().contains("refused"));
    let _ = std::fs::remove_dir_all(dir);
}
