//! The 3D Viewport with no host, driven through the protocol alone on a test
//! copy of Last order's diner files: where Last order is on disk, set
//! `LAST_ORDER` to its folder. Without it, the test says so and checks
//! nothing. Last order itself is never written: the program writes decoded
//! renders into its project's cache, so it is given a copy.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A project holding the diner's scene files and the renders they name, in
/// a folder of the test's own, since tests run at once.
fn diner_copy(test: &str) -> Option<PathBuf> {
    let Some(folder) = std::env::var_os("LAST_ORDER") else {
        eprintln!("LAST_ORDER is not set, so the 3D Viewport is not driven");
        return None;
    };
    let from = PathBuf::from(folder).join("prompts/resources");
    let project = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("3d-viewport-{test}"));
    let _ = std::fs::remove_dir_all(&project);
    let to = project.join("prompts/resources");
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(&from).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().contains("-diner-scene-") && path.extension().is_some_and(|e| e == "glb") {
            let scene = scene::read(&std::fs::read(&path).unwrap()).unwrap();
            for reference in scene.cameras.iter().filter_map(|c| c.reference_file.as_ref()) {
                let name = Path::new(reference).file_name().unwrap();
                std::fs::copy(from.join(name), to.join(name)).unwrap();
            }
            std::fs::copy(&path, to.join(path.file_name().unwrap())).unwrap();
        }
    }
    Some(project)
}

/// Runs the program with these messages, and returns what it said.
fn drive(lines: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_3d-viewport"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for line in lines {
            writeln!(stdin, "{line}").unwrap();
        }
        writeln!(stdin, "{}", json!({ "type": "shutdown" })).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn pick(row: u64) -> Value {
    json!({ "type": "command", "id": "pick", "tab": "3d-viewport", "arg": { "row": row, "value": false }, "viewport": [1000, 600] })
}

#[test]
fn every_render_shows_with_the_scene_that_made_it() {
    let Some(project) = diner_copy("every-render") else { return };
    let resources = project.join("prompts/resources");
    // Which scene file names each render, and from which camera.
    // Read oldest export first, so a builder exported again is mapped by its
    // newest export, as the 3D Viewport lists only that one.
    let mut made_by: HashMap<String, (String, String)> = HashMap::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&resources).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        if path.extension().is_some_and(|e| e == "glb") {
            let scene = scene::read(&std::fs::read(&path).unwrap()).unwrap();
            let file = format!("prompts/resources/{}", path.file_name().unwrap().to_string_lossy());
            for camera in &scene.cameras {
                if let Some(reference) = &camera.reference_file {
                    made_by.insert(reference.clone(), (file.clone(), camera.name.clone()));
                }
            }
        }
    }
    assert_eq!(made_by.len(), 10);

    let mut lines = vec![json!({ "type": "project", "folder": project.display().to_string(), "skills": true })];
    lines.extend((0..10).map(|i| pick(1000 + i)));
    let said = drive(&lines);
    let views: Vec<&Value> = said.iter().filter(|m| m["type"] == "view3d").collect();
    assert_eq!(views.len(), 11, "one view on opening, then one a pick");
    let tabs = said.iter().rfind(|m| m["type"] == "tabs").unwrap();
    assert_eq!(tabs["tabs"][0]["label"], "Diner");

    let properties = said.iter().rfind(|m| m["type"] == "properties").unwrap();
    let rows: Vec<&Value> = properties["layers"]["rows"].as_array().unwrap().iter().collect();
    assert!(properties["layers"]["plain"].as_bool().unwrap());
    assert_eq!(rows.iter().filter(|r| r["selected"].as_bool().unwrap()).count(), 1, "only one row is ever selected");

    for (i, view) in views[1..].iter().enumerate() {
        let reference = &view["reference"];
        let image = reference["image"].as_str().unwrap();
        let stem = Path::new(image).file_stem().unwrap().to_string_lossy().to_string();
        let (render, (scene, camera)) =
            made_by.iter().find(|(file, _)| stem.starts_with(Path::new(file).file_stem().unwrap().to_str().unwrap())).unwrap();
        assert_eq!(view["scene"].as_str().unwrap(), scene, "row {i}: {render} shown over another scene");
        assert_eq!(view["camera"], json!({ "kind": "named", "name": camera }), "row {i}");
        let simple = render.ends_with("-simple.mp4");
        assert_eq!(view["look"].as_str().unwrap_or("grey"), if simple { "simple" } else { "grey" }, "row {i}: {render}");
        assert!(view["status"].as_str().unwrap().starts_with(&format!("{camera} · ")), "row {i}: the status line starts with the camera");
        // A video's decoded frame is removed when the next one is written, so
        // only a still render's file, and the last view's, are still there.
        if !render.ends_with(".mp4") || i == views.len() - 2 {
            let raw = std::fs::metadata(project.join(image)).unwrap().len();
            assert_eq!(raw, reference["width"].as_u64().unwrap() * reference["height"].as_u64().unwrap() * 4, "row {i}: {image}");
        }
        assert!(image.starts_with(".spumcake/cache/3d-viewport/"), "row {i}: {image} is not in the project's cache");
    }
}

#[test]
fn a_video_render_shows_the_frame_held_at_the_time() {
    let Some(project) = diner_copy("video-frame") else { return };
    // The renders in the list's order: the six of 10 Nov, 11 Nov, 12 Nov,
    // and the two swings of 14 Nov, the simple one last.
    let simple = 1009;
    let said = drive(&[
        json!({ "type": "project", "folder": project.display().to_string(), "skills": true }),
        pick(simple),
        json!({ "type": "transport_time", "tab": "3d-viewport", "time": 3.4 }),
        json!({ "type": "command", "id": "transport.play", "tab": "3d-viewport", "arg": { "value": true }, "viewport": [1000, 600] }),
    ]);
    let views: Vec<&Value> = said.iter().filter(|m| m["type"] == "view3d").collect();
    let at = views[2];
    assert_eq!(at["time"].as_f64().unwrap(), 3.4);
    // 3.4 s at 24 frames a second falls in frame 81, which starts at
    // 3.375 s, as the scene's swing holds each key until the next.
    assert!(at["reference"]["image"].as_str().unwrap().ends_with("-simple-f00081.rgba"), "{}", at["reference"]);
    assert!(at["transport"]["cues"][0]["at"].as_f64().unwrap() > 2.9, "the door's cue");
    // While the swing plays, the render steps aside, so no still frame sits
    // over the moving door.
    let playing = views[3];
    assert!(playing["transport"]["playing"].as_bool().unwrap());
    assert!(playing.get("reference").is_none_or(Value::is_null), "the render is shown while playing");
}

fn command(id: &str, arg: Value) -> Value {
    json!({ "type": "command", "id": id, "tab": "3d-viewport", "arg": arg, "viewport": [1000, 600] })
}

/// The rest of what the user and the assistant can do: a builder's camera
/// from the newest scene, the looks, leaving the free camera somewhere, a
/// frame step, and the three tools, each answered as the hand would be.
#[test]
fn cameras_looks_steps_and_tools_answer_as_the_hand_does() {
    let Some(project) = diner_copy("controls") else { return };
    let said = drive(&[
        json!({ "type": "project", "folder": project.display().to_string(), "skills": true }),
        pick(100 + 3),
        command("look", json!({ "value": 1 })),
        command("look", json!({ "value": 2 })),
        json!({ "type": "view3d_camera", "tab": "3d-viewport", "position": [9.0, 3.0, 1.7], "target": [2.0, 3.0, 1.2], "fov": 60.0 }),
        command("transport.step", json!({ "value": 1 })),
        json!({ "type": "invoke", "call": "c1", "tab": "3d-viewport", "tool": "3d-viewport__view__list_views", "input": {}, "group": "g" }),
        json!({ "type": "invoke", "call": "c2", "tab": "3d-viewport", "tool": "3d-viewport__view__look_through", "input": { "view": "far-end-wall 11 Nov" }, "group": "g" }),
        json!({ "type": "invoke", "call": "c3", "tab": "3d-viewport", "tool": "3d-viewport__view__set_look", "input": { "look": "coded" }, "group": "g" }),
        json!({ "type": "invoke", "call": "c4", "tab": "3d-viewport", "tool": "3d-viewport__view__look_through", "input": { "view": "no such view" }, "group": "g" }),
    ]);
    let views: Vec<&Value> = said.iter().filter(|m| m["type"] == "view3d").collect();
    let newest = views[0]["scene"].as_str().unwrap().to_string();
    assert!(newest.ends_with("-diner-scene-2026-11-14-19-56.glb"), "{newest}");
    // A camera of the newest scene, by its row.
    assert_eq!(views[1]["camera"], json!({ "kind": "named", "name": "far-end-wall" }));
    assert_eq!(views[1]["scene"].as_str().unwrap(), newest);
    assert!(views[1].get("reference").is_none_or(Value::is_null), "a camera row shows no render");
    // The looks, as the toolbar's choice sends them.
    assert_eq!(views[2]["look"], "coded");
    assert_eq!(views[3]["look"], "simple");
    // Leaving the free camera somewhere makes Free the chosen row, from there.
    assert_eq!(views[4]["camera"]["kind"], "free");
    assert_eq!(views[4]["camera"]["start"]["position"], json!([9.0, 3.0, 1.7]));
    // A step moves one frame of the swing.
    assert!((views[5]["time"].as_f64().unwrap() - 1.0 / 24.0).abs() < 1e-9, "{}", views[5]["time"]);
    // The tools.
    let results: Vec<&Value> = said.iter().filter(|m| m["type"] == "result").collect();
    assert!(results[0]["ok"].as_bool().unwrap());
    assert_eq!(results[0]["output"]["views"].as_array().unwrap().len(), 18);
    assert!(results[1]["ok"].as_bool().unwrap(), "{}", results[1]);
    let through = views.iter().rfind(|v| v["reference"]["image"].is_string()).unwrap();
    assert!(through["reference"]["image"].as_str().unwrap().contains("2026-11-11_17-20-previs-far-end-wall-grey"), "{}", through["reference"]);
    assert!(results[2]["ok"].as_bool().unwrap());
    assert_eq!(views.last().unwrap()["look"], "coded");
    assert!(!results[3]["ok"].as_bool().unwrap(), "an unknown view is refused");
}
