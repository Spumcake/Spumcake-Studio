//! M4.2's real-data checks: Last order's animatic, opened, retimed, and
//! followed on a whole copy of Last order, with Skills' own scripts run
//! where the host would run them. They run when `check.sh` finds Last order
//! and a Spumcake-Skills checkout (`SPUMCAKE_LAST_ORDER`,
//! `SPUMCAKE_SKILLS_SCRIPTS`); otherwise they say they were skipped, and
//! why. With both set, a copy that cannot be made fails. One copy serves
//! every check, put back as git holds it between them. Nothing here spends:
//! `assemble.py` and `write-assembly.py` cost nothing, and nothing runs with
//! `--stand-in`.

use crate::serve::Server;
use protocol::{AppMessage, HostMessage};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The pipeline Last order's canon is in.
const PIPELINE: &str = "002";

/// A whole copy of Last order, removed when dropped.
struct Copy {
    root: PathBuf,
    project: PathBuf,
    scripts: PathBuf,
    skills: PathBuf,
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A local clone of spumcake-benchmarks, sharing its objects, or why not.
fn copy(last_order: PathBuf, scripts: PathBuf) -> Result<Copy, String> {
    let skills = scripts.parent().ok_or("no Skills checkout")?.to_path_buf();
    let repo = Command::new("git").arg("-C").arg(&last_order).args(["rev-parse", "--show-toplevel"]).output().map_err(|error| error.to_string())?;
    let repo = PathBuf::from(String::from_utf8_lossy(&repo.stdout).trim());
    let inside = last_order.strip_prefix(&repo).map_err(|_| "Last order is not inside its repository")?.to_path_buf();
    let root = std::env::temp_dir().join(format!("storyboard-last-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let cloned = Command::new("git").args(["clone", "--quiet", "--local"]).arg(&repo).arg(&root).status().map_err(|error| error.to_string())?;
    if !cloned.success() {
        let _ = std::fs::remove_dir_all(&root);
        return Err("the clone failed".to_string());
    }
    for (key, value) in [("user.name", "Storyboard check"), ("user.email", "check@localhost")] {
        Command::new("git").arg("-C").arg(&root).args(["config", key, value]).status().map_err(|error| error.to_string())?;
    }
    Ok(Copy { project: root.join(inside), root, scripts, skills })
}

impl Copy {
    /// Puts the copy back as git holds it: Studio's folder, the storyboard's
    /// workspace file, `.build/`, and every file filed since, gone.
    fn reset(&self) {
        for args in [&["reset", "--hard", "--quiet"][..], &["clean", "-fdxq"][..]] {
            let done = Command::new("git").arg("-C").arg(&self.root).args(args).status().unwrap();
            assert!(done.success(), "the copy could not be put back");
        }
    }

    /// Runs a Skills script in the project, as the host does, giving its
    /// standard output, or its error.
    fn run(&self, program: &Path, args: &[String]) -> Result<String, String> {
        let output = Command::new("python3")
            .arg(program)
            .args(args)
            .current_dir(&self.project)
            .env("SPUMCAKE_AGENT", &self.skills)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
        }
    }

    fn pipeline(&self, args: &[&str]) -> Result<String, String> {
        let mut all = vec![args[0].to_string(), PIPELINE.to_string()];
        all.extend(args[1..].iter().map(|arg| arg.to_string()));
        self.run(&self.scripts.join("pipeline.py"), &all)
    }

    /// Answers every message the host would act on, as the host does, until
    /// none is left. Returns every reply the program sent.
    fn host(&self, server: &mut Server, mut replies: Vec<AppMessage>) -> Vec<AppMessage> {
        let mut seen = Vec::new();
        while let Some(message) = (!replies.is_empty()).then(|| replies.remove(0)) {
            let answer = match &message {
                AppMessage::ReadCanon { id, query } => Some(match self.pipeline(&["sql", query, "--read-only"]) {
                    Ok(out) => HostMessage::CanonRows { id: id.clone(), rows: out.lines().map(|line| Value::String(line.to_string())).collect(), error: None },
                    Err(error) => HostMessage::CanonRows { id: id.clone(), rows: Vec::new(), error: Some(error) },
                }),
                AppMessage::RunSkillScript { id, skill, script, args } => {
                    let program = self.skills.join("skills").join(skill).join("scripts").join(script);
                    Some(match self.run(&program, args) {
                        Ok(output) => HostMessage::SkillScriptRan { id: id.clone(), output, error: None },
                        Err(error) => HostMessage::SkillScriptRan { id: id.clone(), output: String::new(), error: Some(error) },
                    })
                }
                AppMessage::EditStep { id, step, script, refs, .. } => {
                    let mut args = vec!["step", step.chain.as_str(), step.step.as_str()];
                    if let Some(script) = script {
                        args.extend(["--script", script.as_str()]);
                    }
                    if let Some(refs) = refs {
                        args.extend(["--refs", refs.as_str()]);
                    }
                    Some(HostMessage::StepEdited { id: id.clone(), error: self.pipeline(&args).err() })
                }
                _ => None,
            };
            seen.push(message);
            if let Some(answer) = answer {
                replies.extend(server.handle(answer).unwrap_or_default());
            }
        }
        seen
    }
}

/// The copy, or a line saying the checks were skipped and why. A copy that
/// cannot be made where both are set fails.
fn copy_or_skip() -> Option<Copy> {
    let (Some(last_order), Some(scripts)) = (std::env::var_os("SPUMCAKE_LAST_ORDER"), std::env::var_os("SPUMCAKE_SKILLS_SCRIPTS")) else {
        eprintln!("skipped: Last order's checks need SPUMCAKE_LAST_ORDER and SPUMCAKE_SKILLS_SCRIPTS");
        return None;
    };
    Some(copy(PathBuf::from(last_order), PathBuf::from(scripts)).unwrap_or_else(|why| panic!("Last order could not be copied: {why}")))
}

/// Every check, on one copy, put back between them.
#[test]
fn last_order_on_a_test_copy() {
    let Some(copy) = copy_or_skip() else { return };
    let checks: [(&str, fn(&Copy)); 6] = [
        ("opens and saves back unchanged", opens_from_its_step_and_saves_back_unchanged),
        ("the step decides, not the newest file", the_step_decides_not_the_newest_file),
        ("Studio's retime is what build-animatic assembles", studios_retime_is_what_build_animatic_assembles),
        ("a retime in Claude Code shows in Studio", a_retime_in_claude_code_shows_in_studio),
        ("Claude Code's retime with Studio's files gone", claude_codes_retime_needs_none_of_studios_files),
        ("the step found by its skill", the_step_is_found_by_its_skill),
    ];
    for (name, check) in checks {
        copy.reset();
        eprintln!("check: {name}");
        check(&copy);
    }
}

fn tool(server: &mut Server, copy: &Copy, tab: &str, name: &str, input: Value) -> Value {
    tool_and_tab(server, copy, tab, name, input).0
}

/// A tool's output, and the tab shown once the host has answered.
fn tool_and_tab(server: &mut Server, copy: &Copy, tab: &str, name: &str, input: Value) -> (Value, String) {
    let replies = server
        .handle(HostMessage::Invoke {
            call: format!("check-{name}"),
            tab: tab.to_string(),
            tool: protocol::tool_key("storyboard-generator", "clips", name),
            input,
            group: format!("check-{name}"),
        })
        .unwrap();
    let mut output = Value::Null;
    for reply in &replies {
        if let AppMessage::ToolResult(result) = reply {
            output = result.output.clone();
        }
    }
    let shown = shown_tab(&copy.host(server, replies)).unwrap_or_else(|| tab.to_string());
    (output, shown)
}

fn shown_tab(seen: &[AppMessage]) -> Option<String> {
    seen.iter().rev().find_map(|message| match message {
        AppMessage::Tabs(tabs) => Some(tabs.active.clone()),
        _ => None,
    })
}

fn command(server: &mut Server, copy: &Copy, tab: &str, id: &str) -> Vec<AppMessage> {
    let replies = server
        .handle(HostMessage::Command { id: id.into(), tab: Some(tab.to_string()), arg: None, node: None, viewport: [800.0, 600.0] })
        .unwrap();
    copy.host(server, replies)
}

/// A program in the copy, with Last order's animatic opened. Returns the
/// tab it opened in.
fn opened(copy: &Copy) -> (Server, String) {
    let mut server = Server::new(None);
    server.greeting();
    server.handle(HostMessage::Project { folder: copy.project.display().to_string(), skills: true });
    let seen = command(&mut server, copy, "1", "open_animatic");
    let tab = shown_tab(&seen).expect("a tab");
    (server, tab)
}

fn spans(film: &Value) -> Vec<(u64, u64)> {
    film["clips"].as_array().unwrap().iter().map(|clip| {
        let start = clip["start"].as_u64().unwrap() - 1;
        (start, start + clip["length"].as_u64().unwrap())
    }).collect()
}

/// The step's script and references, as canon holds them now.
fn step(copy: &Copy) -> (String, Value) {
    step_named(copy, "animatic")
}

fn step_named(copy: &Copy, name: &str) -> (String, Value) {
    let script = copy.pipeline(&["sql", &format!("SELECT script FROM step WHERE chain = 'the animatic' AND name = '{name}'"), "--read-only"]).unwrap();
    let refs = copy.pipeline(&["sql", &format!("SELECT refs FROM step WHERE chain = 'the animatic' AND name = '{name}'"), "--read-only"]).unwrap();
    (script.trim().to_string(), serde_json::from_str(refs.trim()).unwrap())
}

fn opens_from_its_step_and_saves_back_unchanged(copy: &Copy) {
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(spans(&film), [(0, 72), (72, 101), (101, 151), (151, 192)], "{film}");
    let mut voices: Vec<(u64, u64)> = film["sound_tracks"].as_array().unwrap()[1..]
        .iter()
        .flat_map(|track| track["sounds"].as_array().unwrap().iter().map(|sound| (sound["start"].as_u64().unwrap() - 1, sound["from"].as_u64().unwrap() - 1)))
        .collect();
    voices.sort_unstable();
    assert_eq!(voices, [(101, 1), (115, 49)]);
    // Saved untouched, the new assembly holds the same timing, and the step's
    // words are its own.
    let (_, refs_before) = step(copy);
    let saved = tool(&mut server, copy, &tab, "save_timing", json!({}));
    assert!(saved.get("error").is_none(), "{saved}");
    let (script, refs_after) = step(copy);
    assert_eq!(refs_after, refs_before);
    let file = crate::assembly::named_in(&script).unwrap();
    assert_ne!(file, "prompts/resources/2026-11-14_16-07-animatic-assembly.json", "the step still names the old file");
    let read = |path: &str| crate::assembly::Assembly::from_value(&serde_json::from_slice(&std::fs::read(copy.project.join(path)).unwrap()).unwrap()).unwrap();
    assert_eq!(read(&file), read("prompts/resources/2026-11-14_16-07-animatic-assembly.json"));
}

/// The decoded video's frames where the image changes.
fn cuts(video: &Path) -> Vec<u32> {
    let mut decoded = media::Video::open(video).unwrap();
    let size = (192, 108);
    let frames: Vec<media::Frame> = decoded.frames(0, size).map(Result::unwrap).collect();
    let mut out = Vec::new();
    for pair in frames.windows(2) {
        let change = pair[0].rgba.iter().zip(&pair[1].rgba).map(|(a, b)| u64::from(a.abs_diff(*b))).sum::<u64>() / pair[0].rgba.len() as u64;
        // A held frame is the same image, so any change is a cut: Last
        // order's keyframes are alike enough that the change is small.
        if change > 0 {
            out.push(pair[1].index);
        }
    }
    out
}

fn studios_retime_is_what_build_animatic_assembles(copy: &Copy) {
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    let ids: Vec<u64> = film["clips"].as_array().unwrap().iter().map(|clip| clip["id"].as_u64().unwrap()).collect();
    tool(&mut server, copy, &tab, "retime_clip", json!({ "clip": ids[1], "edge": "end", "frames": 2 }));
    // Between the two gestures the length is not the canon one.
    let refused = tool(&mut server, copy, &tab, "save_timing", json!({}));
    assert!(refused["error"].as_str().unwrap().contains("length is 8.0 s"), "{refused}");
    tool(&mut server, copy, &tab, "retime_clip", json!({ "clip": ids[3], "edge": "end", "frames": -2 }));
    let saved = tool(&mut server, copy, &tab, "save_timing", json!({}));
    assert!(saved.get("error").is_none(), "{saved}");
    let (_, refs) = step(copy);
    let words: Vec<&str> = refs.as_array().unwrap().iter().map(|r| r["role"].as_str().unwrap()).collect();
    assert_eq!(&words[..4], ["frame, 0.0 to 3.0 s", "frame, 3.0 to 4.2917 s", "frame, 4.2917 to 6.375 s", "frame, 6.375 to 8.0 s"]);
    assert_eq!(&words[5..], ["clip 0.05 to 0.65 s, laid at 4.2 s, at 0 dB", "clip 2.05 to 3.35 s, laid at 4.8 s, at 0 dB"], "Otis's lines moved");
    let touched = copy.pipeline(&["touches", "--json"]).unwrap();
    assert!(touched.contains("Last order: animatic"), "{touched}");
    // The substitution test: with Studio's own folder and the storyboard's
    // workspace file gone, Skills alone assembles Studio's timing.
    let _ = std::fs::remove_dir_all(copy.project.join(protocol::PROJECT_DIR));
    let _ = std::fs::remove_dir_all(copy.project.join("work"));
    // The step writes into `.build/`, which git never holds, so a clone has
    // none: a copy made by copying the folder would. Skills is told.
    std::fs::create_dir_all(copy.project.join(".build")).unwrap();
    let ran = copy.pipeline(&["rerun", "--step", "the animatic", "animatic", "--json"]).unwrap();
    let take: Value = ran.lines().rev().find_map(|line| serde_json::from_str(line).ok()).unwrap_or_else(|| panic!("rerun said: {ran}"));
    assert_eq!(take["stand_in"], json!(false), "{take}");
    let video = copy.project.join(take["files"][0].as_str().unwrap());
    let cut = cuts(&video);
    for (want, near) in [(72u32, "clip 2"), (103, "clip 3"), (153, "clip 4")] {
        assert!(cut.iter().any(|at| at.abs_diff(want) <= 1), "no cut near {want} for {near}: {cut:?}");
    }
    nothing_published_changed(copy);
}

fn a_retime_in_claude_code_shows_in_studio(copy: &Copy) {
    let (mut server, tab) = opened(copy);
    retime_as_claude_code(copy);
    // The open storyboard is told, and Follow takes the new timing.
    let replies = server.tick();
    let seen = copy.host(&mut server, replies);
    let carded = seen.iter().chain(server.tick().iter()).any(|message| matches!(message, AppMessage::Notices { notices, .. } if notices.iter().any(|notice| notice.text == "Retimed in another tool")));
    assert!(carded, "no Retimed in another tool card");
    command(&mut server, copy, &tab, "follow_retime");
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(spans(&film)[..2], [(0, 60), (60, 101)]);
    // Opened afresh, the animatic has Claude Code's timing too.
    let (mut again, other) = opened(copy);
    let film = tool(&mut again, copy, &other, "read_film", json!({}));
    assert_eq!(spans(&film)[0], (0, 60));
}

/// The assembly as Last order has it.
fn last_orders_assembly(copy: &Copy) -> crate::assembly::Assembly {
    let path = copy.project.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json");
    crate::assembly::Assembly::from_value(&serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()).unwrap()
}

/// Files an assembly with Skills' writer, as build-animatic does, giving
/// its file in the project.
fn file_assembly(copy: &Copy, assembly: &crate::assembly::Assembly) -> String {
    let args = crate::session::project::writer_args(assembly);
    let written = copy.run(&copy.skills.join("skills/build-animatic/scripts/write-assembly.py"), &args).unwrap();
    let filed: Value = serde_json::from_str(written.lines().last().unwrap()).unwrap();
    filed["path"].as_str().unwrap().to_string()
}

/// What build-animatic does by hand: write a retimed assembly with Skills'
/// writer, with shot 1 ending at 2.5 s, and point the step at it.
fn retime_as_claude_code(copy: &Copy) {
    let mut assembly = last_orders_assembly(copy);
    assembly.frames[0].end = 2.5;
    assembly.frames[1].start = 2.5;
    let file = file_assembly(copy, &assembly);
    let (script, _) = step(copy);
    let moved = crate::assembly::repoint(&script, &file).unwrap();
    copy.pipeline(&["step", "the animatic", "animatic", "--script", &moved]).unwrap();
}

/// Neither `request.py check` nor `git diff` finds a file a request made
/// changed: only the records index and the canon and run logs grow.
fn nothing_published_changed(copy: &Copy) {
    let checked = copy.run(&copy.scripts.join("request.py"), &["check".to_string()]);
    assert!(checked.is_ok(), "request.py check: {checked:?}");
    let changed = Command::new("git").arg("-C").arg(&copy.root).args(["diff", "--name-only"]).output().unwrap();
    for line in String::from_utf8_lossy(&changed.stdout).lines() {
        let log = line.ends_with("records.jsonl") || line.ends_with("runs.jsonl") || line.ends_with("canon.sql");
        assert!(log || !(line.contains("/library/") || line.contains("/requests/") || line.contains("/prompts/")), "a file a request made changed: {line}");
    }
}

/// Check 2: with a newer assembly filed that the step does not name, the
/// animatic opens with the step's.
fn the_step_decides_not_the_newest_file(copy: &Copy) {
    let mut newer = last_orders_assembly(copy);
    newer.frames[0].end = 2.0;
    newer.frames[1].start = 2.0;
    file_assembly(copy, &newer);
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(spans(&film), [(0, 72), (72, 101), (101, 151), (151, 192)], "the newest file was opened");
    assert_eq!(film["project_animatic"]["assembly"], "prompts/resources/2026-11-14_16-07-animatic-assembly.json");
    assert_eq!(film["project_animatic"]["differs"], false);
}

/// Check 6, the second way: after Claude Code's retime, with Studio's own
/// folder and every storyboard's workspace file gone, the animatic opens
/// with Claude Code's timing, and nothing published changed.
fn claude_codes_retime_needs_none_of_studios_files(copy: &Copy) {
    // Studio has been here before: a storyboard saved in work/.
    let (mut server, tab) = opened(copy);
    command(&mut server, copy, &tab, "save");
    let kept = std::fs::read_dir(copy.project.join("work")).map(|files| files.count()).unwrap_or(0);
    assert!(kept > 0, "the storyboard was not kept in work/");
    drop(server);
    retime_as_claude_code(copy);
    let _ = std::fs::remove_dir_all(copy.project.join(protocol::PROJECT_DIR));
    let _ = std::fs::remove_dir_all(copy.project.join("work"));
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(spans(&film)[..2], [(0, 60), (60, 101)]);
    nothing_published_changed(copy);
}

/// Check 12: the animatic step is found by its skill and its script,
/// whatever it is called; with two, the one named opens.
fn the_step_is_found_by_its_skill(copy: &Copy) {
    copy.pipeline(&["sql", "UPDATE step SET name = 'the cut' WHERE chain = 'the animatic' AND name = 'animatic'"]).unwrap();
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(film["project_animatic"]["step"], "the animatic: the cut");
    // A second animatic, a shot shorter at the start.
    let mut second = last_orders_assembly(copy);
    second.frames[0].end = 2.0;
    second.frames[1].start = 2.0;
    let file = file_assembly(copy, &second);
    let script = crate::assembly::repoint(&step_named(copy, "the cut").0, &file).unwrap();
    copy.pipeline(&["step", "the animatic", "the other cut", "--order", "99", "--skill", "build-animatic", "--script", &script]).unwrap();
    let (_, other) = tool_and_tab(&mut server, copy, &tab, "open_animatic", json!({ "step": "the other cut" }));
    assert_ne!(other, tab, "the second animatic did not open");
    let film = tool(&mut server, copy, &other, "read_film", json!({}));
    assert_eq!(film["project_animatic"]["step"], "the animatic: the other cut");
    assert_eq!(spans(&film)[0], (0, 48));
}
