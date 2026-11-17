//! M4.2's real-data checks: Last order's animatic, opened, retimed, and
//! followed on a whole copy of Last order, with Skills' own scripts run
//! where the host would run them. They run when `check.sh` finds Last order
//! and a Spumcake-Skills checkout (`LAST_ORDER`,
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

    /// Files a workspace's file as the host's runner does, giving its new
    /// path in the project.
    fn file(&self, path: &str, from: &[String]) -> Result<String, String> {
        let stem = Path::new(path).file_stem().unwrap().to_string_lossy().to_string();
        let mut args = vec!["resource".to_string(), path.to_string(), "--name".to_string(), stem];
        for input in from {
            args.extend(["--from".to_string(), input.clone()]);
        }
        let printed = self.run(&self.scripts.join("request.py"), &args)?;
        Ok(printed.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_string())
    }

    /// Publishes a workspace's file as the host's runner does on the
    /// publish card's yes: filed, then published as made from the canon
    /// files it names.
    fn publish(&self, artifact: &str, source: &str, from: &[String]) -> Result<String, String> {
        let filed = self.file(source, from)?;
        let mut args = vec!["publish", artifact, "--file", filed.as_str()];
        let canon = self.pipeline(&["sql", "SELECT id || char(31) || file FROM artifact WHERE file IS NOT NULL", "--read-only"])?;
        let names: Vec<String> = from.iter().filter_map(|input| canon.lines().find_map(|line| line.split_once('\u{1f}').filter(|(_, file)| file == input).map(|(name, _)| name.to_string()))).collect();
        for name in &names {
            args.extend(["--from", name.as_str(), "input"]);
        }
        self.pipeline(&args)?;
        Ok(filed)
    }

    /// Publishes a step's take as the host's runner does on the publish
    /// card's yes: made from what the step refers to, with their roles.
    fn publish_take(&self, artifact: &str, take: &str) -> Result<String, String> {
        let quoted = artifact.replace('\'', "''");
        let chain = self.pipeline(&["sql", &format!("SELECT chain FROM step WHERE artifact = '{quoted}'"), "--read-only"])?;
        let refs = self.pipeline(&["sql", &format!("SELECT refs FROM step WHERE artifact = '{quoted}'"), "--read-only"])?;
        let refs: Vec<Value> = serde_json::from_str(refs.trim()).map_err(|error| error.to_string())?;
        let chain = chain.trim().to_string();
        let mut args = vec!["publish", artifact, "--file", take, "--chain", chain.as_str()];
        let mut seen: Vec<&str> = Vec::new();
        for reference in &refs {
            let (Some(name), Some(role)) = (reference["artifact"].as_str(), reference["role"].as_str()) else { continue };
            if !seen.contains(&name) {
                seen.push(name);
                args.extend(["--from", name, role]);
            }
        }
        self.pipeline(&args)?;
        Ok(take.to_string())
    }

    /// Runs a step again as the host's runner does, giving the take.
    fn rerun(&self, chain: &str, step: &str) -> Result<Value, String> {
        let ran = self.pipeline(&["rerun", "--step", chain, step, "--json"])?;
        let take: Value = ran.lines().rev().find_map(|line| serde_json::from_str(line).ok()).ok_or_else(|| format!("rerun said: {ran}"))?;
        // The step ran for real, never as a stand-in.
        assert_eq!(take["stand_in"], json!(false), "{take}");
        Ok(take)
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
                AppMessage::FileResource { path, from, id: Some(id), .. } => Some(match self.file(path, from) {
                    Ok(file) => HostMessage::Filed { id: id.clone(), file, error: None },
                    Err(error) => HostMessage::Filed { id: id.clone(), file: String::new(), error: Some(error) },
                }),
                AppMessage::Publish { tab, artifact, source, from, .. } => {
                    let published = if source.starts_with("work/") { self.publish(artifact, source, from) } else { self.publish_take(artifact, source) };
                    Some(match published {
                        Ok(file) => HostMessage::Published { tab: tab.clone(), artifact: artifact.clone(), file, version: None, error: None },
                        Err(error) => HostMessage::Published { tab: tab.clone(), artifact: artifact.clone(), file: String::new(), version: None, error: Some(error) },
                    })
                }
                AppMessage::RunStep { id, step, .. } => Some(match self.rerun(&step.chain, &step.step) {
                    Ok(take) => HostMessage::StepRan { id: id.clone(), take: take["files"][0].as_str().map(str::to_string), error: None },
                    Err(error) => HostMessage::StepRan { id: id.clone(), take: None, error: Some(error) },
                }),
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
    let (Some(last_order), Some(scripts)) = (std::env::var_os("LAST_ORDER"), std::env::var_os("SPUMCAKE_SKILLS_SCRIPTS")) else {
        eprintln!("skipped: Last order's checks need LAST_ORDER and SPUMCAKE_SKILLS_SCRIPTS");
        return None;
    };
    Some(copy(PathBuf::from(last_order), PathBuf::from(scripts)).unwrap_or_else(|why| panic!("Last order could not be copied: {why}")))
}

/// Every check, on one copy, put back between them.
#[test]
fn last_order_on_a_test_copy() {
    let Some(copy) = copy_or_skip() else { return };
    let checks: [(&str, fn(&Copy)); 9] = [
        ("opens and saves back unchanged", opens_from_its_step_and_saves_back_unchanged),
        ("the step decides, not the newest file", the_step_decides_not_the_newest_file),
        ("Studio's retime is what build-animatic assembles", studios_retime_is_what_build_animatic_assembles),
        ("a retime in Claude Code shows in Studio", a_retime_in_claude_code_shows_in_studio),
        ("Claude Code's retime with Studio's files gone", claude_codes_retime_needs_none_of_studios_files),
        ("the step found by its skill", the_step_is_found_by_its_skill),
        ("drawing reaches the animatic through canon", drawing_reaches_the_animatic_through_canon),
        ("import and export the Skills way", import_and_export_the_skills_way),
        ("playback is what the step's take is", playback_is_the_take),
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
    // Told the project, it shows the project's animatic.
    let replies = server.handle(HostMessage::Project { folder: copy.project.display().to_string(), skills: true }).unwrap();
    let seen = copy.host(&mut server, replies);
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
/// whatever it is called; with two, the first in its chain is shown, and
/// the storyboard takes the film's name, Last order's own element.
fn the_step_is_found_by_its_skill(copy: &Copy) {
    copy.pipeline(&["sql", "UPDATE step SET name = 'the cut' WHERE chain = 'the animatic' AND name = 'animatic'"]).unwrap();
    // A second animatic after it, a shot shorter at the start.
    let mut second = last_orders_assembly(copy);
    second.frames[0].end = 2.0;
    second.frames[1].start = 2.0;
    let file = file_assembly(copy, &second);
    let script = crate::assembly::repoint(&step_named(copy, "the cut").0, &file).unwrap();
    copy.pipeline(&["step", "the animatic", "the other cut", "--order", "99", "--skill", "build-animatic", "--script", &script]).unwrap();
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(film["project_animatic"]["step"], "the animatic: the cut");
    assert_eq!(spans(&film)[0], (0, 72));
    assert_eq!(film["title"], "Last order");
}

/// The decoded video's frame `frame`, small.
fn frame_at(video: &Path, frame: u32) -> Vec<u8> {
    let mut decoded = media::Video::open(video).unwrap();
    decoded.frames(0, (192, 108)).map(Result::unwrap).find(|decoded| decoded.index == frame).unwrap().rgba
}

/// How far apart two frames are: the mean difference of their channels.
fn apart(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / a.len().max(1) as f64
}

/// Check 4: a mark drawn on shot 2 is refused until its hold is published
/// as "Last order: rough board, Shot 2", made from shot 2's keyframe; saved, the step
/// assembles it, and `touches` follows the rough board to the animatic.
fn drawing_reaches_the_animatic_through_canon(copy: &Copy) {
    // The animatic from the keyframes alone, assembled beside the copy,
    // which files nothing.
    let published = copy.pipeline(&["sql", "SELECT file FROM artifact WHERE id = 'Last order: animatic'", "--read-only"]).unwrap();
    let assemble = copy.skills.join("skills/build-animatic/scripts/assemble.py");
    let before = copy.root.join("before.mp4");
    copy.run(&assemble, &["prompts/resources/2026-11-14_16-07-animatic-assembly.json".to_string(), before.display().to_string()]).unwrap();
    let before = (frame_at(&before, 30), frame_at(&before, 85));
    let (mut server, tab) = opened(copy);
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    let second = film["clips"][1]["id"].as_u64().unwrap();
    tool(&mut server, copy, &tab, "edit_clip", json!({ "clip": second }));
    for row in 0..6 {
        let y = 380.0 + f64::from(row) * 12.0;
        let drawn = tool_in(&mut server, copy, &tab, "draw", "draw_stroke", json!({ "points": (0..40).map(|step| json!([400.0 + f64::from(step) * 20.0, y, 1.0])).collect::<Vec<_>>() }));
        assert!(drawn.get("error").is_none(), "{drawn}");
    }
    tool(&mut server, copy, &tab, "leave_clip", json!({}));
    let refused = tool(&mut server, copy, &tab, "save_timing", json!({}));
    assert_eq!(refused["error"], "Shot 2 has drawing that is not published yet. Publish it first.");
    tool(&mut server, copy, &tab, "edit_clip", json!({ "clip": second }));
    let asked = tool(&mut server, copy, &tab, "publish_hold", json!({}));
    assert_eq!(asked["made"]["artifact"], "Last order: rough board, Shot 2", "{asked}");
    tool(&mut server, copy, &tab, "leave_clip", json!({}));
    let made_from = copy.pipeline(&["sql", "SELECT source FROM made_from WHERE artifact = 'Last order: rough board, Shot 2'", "--read-only"]).unwrap();
    assert_eq!(made_from.trim(), "Last order: keyframe, the bell");
    // Filed with the keyframe as its input, and not from work/.
    let board = copy.pipeline(&["sql", "SELECT file FROM artifact WHERE id = 'Last order: rough board, Shot 2'", "--read-only"]).unwrap();
    let record = copy.run(&copy.scripts.join("request.py"), &["show".to_string(), board.trim().to_string()]).unwrap();
    assert!(record.contains("library/2026-11-14_16-04-keyframe-the-bell.png") && !record.contains("\"work/"), "{record}");
    // Export animatic video saves the timing, runs the step, and publishes
    // its take on the card's yes.
    let exported = tool(&mut server, copy, &tab, "export_animatic", json!({}));
    assert_eq!(exported["made"]["step"], "the animatic: animatic", "{exported}");
    let (_, refs) = step(copy);
    assert_eq!(refs[1]["artifact"], "Last order: rough board, Shot 2", "{refs}");
    let take = copy.pipeline(&["sql", "SELECT file FROM artifact WHERE id = 'Last order: animatic'", "--read-only"]).unwrap();
    let touched = copy.pipeline(&["touches", "--artifact", "Last order: rough board, Shot 2", "--json"]).unwrap();
    assert!(touched.contains("Last order: animatic"), "{touched}");
    assert_ne!(take.trim(), published.trim(), "the take was not published");
    let after = copy.project.join(take.trim());
    // Shot 1 is as it was; shot 2 has the mark.
    let (shot_1, shot_2) = (apart(&before.0, &frame_at(&after, 30)), apart(&before.1, &frame_at(&after, 85)));
    assert!(shot_1 < 0.5 && shot_2 > 1.0, "the mark is not in shot 2 alone: shot 1 {shot_1:.2}, shot 2 {shot_2:.2}");
    let cut = cuts(&after);
    for want in [72u32, 101, 151] {
        assert!(cut.iter().any(|at| at.abs_diff(want) <= 1), "the take's timing moved: {cut:?}");
    }
    nothing_published_changed(copy);
}

/// Check 10: a sound from outside is filed into the project, never into
/// sounds/, and Export animatic video runs the step, never writing exports/.
fn import_and_export_the_skills_way(copy: &Copy) {
    let (mut server, tab) = opened(copy);
    let exported = tool(&mut server, copy, &tab, "export_animatic", json!({}));
    assert_eq!(exported["made"]["step"], "the animatic: animatic", "{exported}");
    // The step made the animatic, and it is the animatic's file now.
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(film["animatic"]["state"], "done", "{}", film["animatic"]);
    let take = film["animatic"]["file"].as_str().unwrap();
    assert!(copy.project.join(take).is_file() && !take.starts_with("exports/"), "{take}");
    let outside = std::env::temp_dir().join(format!("storyboard-outside-sound-{}.wav", std::process::id()));
    std::fs::write(&outside, crate::sounds::wav_bytes(crate::sounds::RATE, &crate::sounds::clicks(24, &[]))).unwrap();
    let filing = tool(&mut server, copy, &tab, "import_sound", json!({ "path": outside.display().to_string() }));
    let _ = std::fs::remove_file(&outside);
    assert!(filing["made"].get("filing").is_some(), "{filing}");
    // The import finishes on a thread of its own.
    let started = std::time::Instant::now();
    let mut placed = None;
    while placed.is_none() && started.elapsed() < std::time::Duration::from_secs(10) {
        let replies = server.tick();
        copy.host(&mut server, replies);
        let film = tool(&mut server, copy, &tab, "read_film", json!({}));
        placed = film["sound_tracks"].as_array().unwrap().iter().flat_map(|track| track["sounds"].as_array().unwrap().clone()).find_map(|sound| {
            let file = sound["file"].as_str()?.to_string();
            file.contains("storyboard-outside-sound").then_some(file)
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let placed = placed.expect("the sound was not placed");
    assert!(placed.starts_with("prompts/resources/"), "{placed}");
    assert!(!copy.project.join("sounds").exists() && !copy.project.join("exports").exists());
    assert!(!copy.project.join("work").join(outside.file_name().unwrap()).exists(), "the copy in work/ was kept");
}

fn tool_in(server: &mut Server, copy: &Copy, tab: &str, group: &str, name: &str, input: Value) -> Value {
    let replies = server
        .handle(HostMessage::Invoke {
            call: format!("check-{name}"),
            tab: tab.to_string(),
            tool: protocol::tool_key("storyboard-generator", group, name),
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
    copy.host(server, replies);
    output
}

/// Each frame's loudness in decibels, from stereo samples at `rate`.
fn loudness(samples: &[f32], rate: u32, frames: usize) -> Vec<f64> {
    let per = (rate / 24) as usize * media::Sound::CHANNELS;
    (0..frames)
        .map(|frame| {
            let part = samples.get(frame * per..((frame + 1) * per).min(samples.len())).unwrap_or_default();
            let power = part.iter().map(|sample| f64::from(*sample) * f64::from(*sample)).sum::<f64>() / part.len().max(1) as f64;
            10.0 * power.max(1e-12).log10()
        })
        .collect()
}

/// Check 9: Studio's playback of the animatic, its images and its mix, is
/// what the step's take holds, within a tolerance for `assemble.py`'s
/// scaling, padding, and encoding.
fn playback_is_the_take(copy: &Copy) {
    let (mut server, tab) = opened(copy);
    let exported = tool(&mut server, copy, &tab, "export_animatic", json!({}));
    assert_eq!(exported["made"]["step"], "the animatic: animatic", "{exported}");
    let film = tool(&mut server, copy, &tab, "read_film", json!({}));
    assert_eq!(film["animatic"]["state"], "done", "{}", film["animatic"]);
    let take = copy.project.join(film["animatic"]["file"].as_str().unwrap());
    // Studio's playback of the same animatic, as the stage and the mix
    // play it.
    let mut session = crate::session::Session::new();
    session.set_project(&copy.project);
    session.open_assembly(&copy.project.join("prompts/resources/2026-11-14_16-07-animatic-assembly.json"), "Last order").unwrap();
    let tracks = session.mix_tracks();
    let mut plan = crate::export::plan(session.active_storyboard(), tracks.clone(), Some(copy.project.as_path()));
    // Each hold, in its middle.
    let held = [36usize, 86, 126, 171];
    let mut decoded = media::Video::open(&take).unwrap();
    let frames: Vec<media::Frame> = decoded.frames(0, plan.size).map(Result::unwrap).filter(|frame| held.contains(&(frame.index as usize))).collect();
    assert_eq!(frames.len(), held.len(), "the take is missing frames");
    // Each hold is near Studio's, and nearer its own than any other
    // hold's, so a take with two holds swapped fails though the keyframes
    // are alike.
    let mut looker = crate::look::Looker::new();
    let studio: Vec<Vec<u8>> = held.iter().map(|at| {
        let shown = plan.frames[*at];
        crate::export::pixels(&mut looker, &mut plan, shown)
    }).collect();
    for (index, frame) in frames.iter().enumerate() {
        let distances: Vec<f64> = studio.iter().map(|image| apart(image, &frame.rgba)).collect();
        let own = distances[index];
        assert!(own < 4.0, "frame {} is {own:.2} apart from Studio's", frame.index);
        for (other, distance) in distances.iter().enumerate().filter(|(other, _)| *other != index) {
            assert!(own < *distance, "frame {} is nearer Studio's frame {} ({distance:.2}) than its own ({own:.2})", frame.index, held[other]);
        }
    }
    // The sound, frame by frame, wherever it is heard.
    let rate = 48_000;
    let length = session.film().length() as usize;
    let studio = loudness(&media::mix(&tracks, 0, length as u32, rate, (24, 1)), rate, length);
    let made = loudness(&media::Sound::open(&take, rate).unwrap().samples, rate, length);
    let mut compared = 0;
    for (frame, (studio, made)) in studio.iter().zip(&made).enumerate() {
        if *studio > -50.0 {
            compared += 1;
            assert!((studio - made).abs() <= 1.0, "frame {frame}: Studio's mix is {studio:.1} dB, the take's {made:.1} dB");
        }
    }
    assert!(compared > length / 2, "only {compared} frames were heard");
    // The comparison can tell: without Otis's lines, Studio's mix is more
    // than 1 dB from the take where he speaks.
    let bed: Vec<media::Track> = tracks.iter().take(1).cloned().collect();
    let quiet = loudness(&media::mix(&bed, 0, length as u32, rate, (24, 1)), rate, length);
    assert!(quiet.iter().zip(&made).any(|(quiet, made)| (quiet - made).abs() > 1.0), "the sound comparison cannot tell the voice is missing");
}
