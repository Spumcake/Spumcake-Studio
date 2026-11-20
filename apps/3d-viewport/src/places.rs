//! A project's places, read from their scene files, and what the camera list
//! shows for one: Free, the newest scene's cameras, and every published
//! pre-vis render across the place's scene files, each with its date, as the
//! approved camera list (vp-camera-list) shows. Which render is in use is
//! the project's canon's: each camera's canon artifact's latest iteration,
//! as the scene contract has it since the owner's data model of 2026-11-18.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where a project keeps its scene files, and every other resource.
pub const RESOURCES: &str = "prompts/resources";

/// One place, such as the diner: every scene file made for it, oldest
/// builder first, so the last is the newest and is shown by default.
pub struct Place {
    /// The place's name as its scene files spell it, such as "diner".
    pub key: String,
    pub scenes: Vec<SceneFile>,
}

/// One scene file and what it holds.
pub struct SceneFile {
    /// Its path in the project, such as
    /// "prompts/resources/2026-11-16_12-33-diner-scene-4195103.glb".
    pub path: String,
    pub scene: scene::Scene,
}

impl Place {
    /// The place's name as the user reads it: "Diner".
    pub fn name(&self) -> String {
        let mut letters = self.key.chars();
        match letters.next() {
            Some(first) => first.to_uppercase().chain(letters).collect::<String>().replace('-', " "),
            None => String::new(),
        }
    }

    /// The scene shown by default: the newest builder's.
    pub fn newest(&self) -> usize {
        self.scenes.len().saturating_sub(1)
    }

    /// The canon artifacts the place's cameras were published as, each once.
    pub fn artifacts(&self) -> Vec<String> {
        let found: BTreeSet<String> =
            self.scenes.iter().flat_map(|file| file.scene.cameras.iter().filter_map(|camera| camera.canon.clone())).collect();
        found.into_iter().collect()
    }

    /// Every published render across the place's scene files, each from the
    /// scene whose own builder made it, and each camera's latest iteration
    /// that no builder made, in date order; each marked in use when it is its
    /// artifact's latest iteration, and outlined when that is out of date.
    pub fn renders(&self, canon: &Canon) -> Vec<Render> {
        let mut renders: Vec<Render> = self
            .scenes
            .iter()
            .enumerate()
            .flat_map(|(index, file)| {
                file.scene.cameras.iter().enumerate().filter_map(move |(order, camera)| {
                    let reference = camera.reference_file.clone()?;
                    Some(Render::of(index, order, camera, reference, true))
                })
            })
            .collect();
        // A newer iteration of a camera's render, such as one made in a
        // workspace, was made by no scene here: it is listed with the newest
        // scene that has its camera, and is never laid over it.
        for artifact in self.artifacts() {
            let Some(latest) = canon.latest.get(&artifact) else { continue };
            if renders.iter().any(|r| r.file == *latest) {
                continue;
            }
            let newest = self.scenes.iter().enumerate().rev().find_map(|(index, file)| {
                let order = file.scene.cameras.iter().position(|c| c.canon.as_deref() == Some(artifact.as_str()))?;
                Some((index, order, &file.scene.cameras[order]))
            });
            if let Some((index, order, camera)) = newest {
                renders.push(Render::of(index, order, camera, latest.clone(), false));
            }
        }
        for render in &mut renders {
            let artifact = render.canon.as_deref().unwrap_or_default();
            render.in_use = render.canon.is_some() && canon.latest.get(artifact) == Some(&render.file);
            render.outlined = render.in_use && canon.out_of_date.contains(artifact);
            render.dim = !render.in_use && canon.latest.contains_key(artifact);
        }
        // By the time in each file's name, then in the builder's own camera
        // order, as the approved list has them.
        let made = |r: &Render| r.file.rsplit('/').next().and_then(|n| n.get(..16)).unwrap_or_default().to_string();
        renders.sort_by(|a, b| made(a).cmp(&made(b)).then(a.order.cmp(&b.order)));
        renders
    }
}

/// A published render the list offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Render {
    /// Which of the place's scene files names it.
    pub scene: usize,
    /// Its camera's place in its builder's camera list.
    pub order: usize,
    pub camera: String,
    /// What the row reads: the camera, and for a video what it shows, such as
    /// "far-end-wall · door swing, simple".
    pub label: String,
    /// The day it was made, such as "10 Nov".
    pub date: String,
    pub video: bool,
    /// Its path in the project.
    pub file: String,
    /// The canon artifact its camera was published as, if any.
    pub canon: Option<String>,
    /// Made by the builder of the scene it is listed with, so it may be laid
    /// over that scene; a later iteration is only ever shown beside it.
    pub from_builder: bool,
    /// Its artifact's latest iteration, the render in use.
    pub in_use: bool,
    /// In use, and out of date, as `pipeline.py touches` says.
    pub outlined: bool,
    /// An earlier iteration of an artifact whose latest is known: an asset
    /// not in use, drawn dimmer.
    pub dim: bool,
}

impl Render {
    fn of(scene: usize, order: usize, camera: &scene::Camera, file: String, from_builder: bool) -> Self {
        let name = file.rsplit('/').next().unwrap_or(&file).to_string();
        Self {
            scene,
            order,
            camera: camera.name.clone(),
            label: render_label(&camera.name, &name),
            date: date_of(&name),
            video: is_video(&name),
            file,
            canon: camera.canon.clone(),
            from_builder,
            in_use: false,
            outlined: false,
            dim: false,
        }
    }
}

/// What the project's canon says of the place's renders, read with
/// `pipeline.py sql --read-only` and `pipeline.py touches --json`: each
/// artifact's latest iteration, and which artifacts are out of date. Empty
/// while unread, or where the project has no pipeline, so nothing is marked.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Canon {
    /// Each artifact's latest iteration: its file in the project.
    pub latest: BTreeMap<String, String>,
    pub out_of_date: BTreeSet<String>,
}

impl Canon {
    /// The query that reads each artifact's latest iteration, one
    /// `[id, file]` line each, or none when there are no artifacts.
    pub fn query(artifacts: &[String]) -> Option<String> {
        if artifacts.is_empty() {
            return None;
        }
        let list: Vec<String> = artifacts.iter().map(|a| format!("'{}'", a.replace('\'', "''"))).collect();
        Some(format!("SELECT json_array(id, file) FROM artifact WHERE id IN ({})", list.join(", ")))
    }

    /// The rows the query printed.
    pub fn read_rows(&mut self, rows: &[Value]) {
        self.latest = rows
            .iter()
            .filter_map(|row| {
                let pair = row.as_array()?;
                Some((pair.first()?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()))
            })
            .filter(|(_, file)| !file.is_empty())
            .collect();
    }

    /// `pipeline.py touches --json`'s answer: what is out of date now.
    pub fn read_touches(&mut self, touches: &Value) {
        self.out_of_date = touches
            .get("out_of_date")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|entry| entry.get("artifact")?.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
    }
}

/// Finds every place in a project from its scene files,
/// `<time>-<place>-scene-<builder>.glb` in `prompts/resources/`. A file that
/// cannot be read is left out and named in the returned problems.
pub fn find(project: &Path) -> (Vec<Place>, Vec<String>) {
    let folder = project.join(RESOURCES);
    let mut names: Vec<String> = match std::fs::read_dir(&folder) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect(),
        Err(_) => return (Vec::new(), Vec::new()),
    };
    names.sort();
    let mut places: Vec<Place> = Vec::new();
    let mut problems = Vec::new();
    for name in names {
        let Some(key) = place_of(&name) else { continue };
        let path = format!("{RESOURCES}/{name}");
        let read = std::fs::read(folder.join(&name)).map_err(|e| e.to_string()).and_then(|b| scene::read(&b).map_err(|e| e.to_string()));
        match read {
            Ok(scene) => {
                let file = SceneFile { path, scene };
                match places.iter_mut().find(|p| p.key == key) {
                    Some(place) => place.scenes.push(file),
                    None => places.push(Place { key, scenes: vec![file] }),
                }
            }
            Err(why) => problems.push(format!("{name}: {why}")),
        }
    }
    // A builder exported more than once keeps only its newest export; then
    // the scenes go oldest builder first. A builder read from git's history
    // is older than every builder filed by date.
    for place in &mut places {
        let entries: Vec<(&str, &str)> = place.scenes.iter().map(|f| (f.path.as_str(), f.scene.builder.as_str())).collect();
        let keep = newest_of_each(&entries);
        let mut index = 0;
        place.scenes.retain(|_| {
            index += 1;
            keep.contains(&(index - 1))
        });
        place.scenes.sort_by_key(|file| builder_order(&file.scene.builder));
    }
    (places, problems)
}

/// Of several scene files, `(path, builder)`, the indices of the newest
/// export of each builder: the one whose file name starts with the latest
/// time, as `<time>-<place>-scene-<builder>.glb` does.
pub fn newest_of_each(entries: &[(&str, &str)]) -> Vec<usize> {
    let time = |path: &str| path.rsplit('/').next().and_then(|name| name.get(..16)).unwrap_or_default().to_string();
    let mut kept: Vec<usize> = Vec::new();
    for (i, (path, builder)) in entries.iter().enumerate() {
        match kept.iter().position(|&k| entries[k].1 == *builder) {
            Some(at) if time(entries[kept[at]].0) < time(path) => kept[at] = i,
            Some(_) => {}
            None => kept.push(i),
        }
    }
    kept.sort();
    kept
}

/// The place a scene file is for, from its name: "diner" in
/// "2026-11-16_12-33-diner-scene-4195103.glb".
pub fn place_of(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".glb")?;
    let (before, _) = stem.split_once("-scene-")?;
    // The time is "YYYY-MM-DD_HH-MM", sixteen characters, then a hyphen.
    let key = before.get(17..)?;
    (!key.is_empty() && before.as_bytes().get(16) == Some(&b'-')).then(|| key.to_string())
}

fn builder_order(builder: &str) -> (bool, String) {
    (!builder.starts_with("git:"), builder.to_string())
}

/// A builder as the status line names it: the time in its filed name, such
/// as "2026-11-14 19:56", or its commit, such as "4195103".
pub fn builder_name(builder: &str) -> String {
    if let Some(rest) = builder.strip_prefix("git:") {
        return rest.split(':').next().unwrap_or(rest).to_string();
    }
    let name = builder.rsplit('/').next().unwrap_or(builder);
    match (name.get(..10), name.get(11..16)) {
        (Some(day), Some(clock)) if name.as_bytes().get(10) == Some(&b'_') => format!("{day} {}", clock.replace('-', ":")),
        _ => name.to_string(),
    }
}

/// "10 Nov" from a file named "2026-11-10_05-21-…".
pub fn date_of(name: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let month = name.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    let day = name.get(8..10).and_then(|d| d.parse::<u32>().ok());
    match (month, day) {
        (Some(month), Some(day)) => format!("{day} {}", MONTHS[month - 1]),
        _ => String::new(),
    }
}

fn is_video(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".mp4", ".mov", ".webm"].iter().any(|ext| lower.ends_with(ext))
}

/// What a render's row reads. A still render reads as its camera. A video
/// adds what its file name says it shows beyond the camera, such as
/// "door swing", and a last word "simple" after a comma, as vp-camera-list
/// shows: "far-end-wall · door swing, simple".
pub fn render_label(camera: &str, name: &str) -> String {
    if !is_video(name) {
        return camera.to_string();
    }
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = stem.get(17..).unwrap_or(stem);
    let stem = stem.strip_prefix("previs-").unwrap_or(stem);
    // The part the file name shares with the camera's name, such as
    // "far-end" of "far-end-wall", is the camera; the rest is what it shows.
    let shared = camera.split('-').zip(stem.split('-')).take_while(|(a, b)| a == b).count();
    let mut words: Vec<&str> = stem.split('-').skip(shared).collect();
    if words.last() == Some(&"grey") {
        words.pop();
    }
    let shows = match words.split_last() {
        Some((&"simple", rest)) if !rest.is_empty() => format!("{}, simple", rest.join(" ")),
        _ => words.join(" "),
    };
    if shows.is_empty() { camera.to_string() } else { format!("{camera} · {shows}") }
}

/// The folder for this program's decoded renders and thumbnails, inside the
/// project's own folder, which is never committed.
pub fn cache(project: &Path) -> PathBuf {
    project.join(protocol::PROJECT_DIR).join("cache").join("3d-viewport")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_files_name_their_place() {
        assert_eq!(place_of("2026-11-16_12-33-diner-scene-4195103.glb").as_deref(), Some("diner"));
        assert_eq!(place_of("2026-11-16_12-33-diner-scene-2026-11-14-19-56.glb").as_deref(), Some("diner"));
        assert_eq!(place_of("2026-11-16_12-33-back-room-scene-x.glb").as_deref(), Some("back-room"));
        assert_eq!(place_of("2026-11-10_05-21-previs-location-grey.png"), None);
        assert_eq!(place_of("diner-scene-x.glb"), None);
    }

    #[test]
    fn a_builder_exported_twice_keeps_its_newest_export() {
        let entries = [
            ("prompts/resources/2026-11-16_12-33-diner-scene-4195103.glb", "git:4195103:a.py"),
            ("prompts/resources/2026-11-16_12-33-diner-scene-2026-11-14-19-56.glb", "prompts/resources/b.py"),
            ("prompts/resources/2026-11-16_18-34-diner-scene-4195103.glb", "git:4195103:a.py"),
            ("prompts/resources/2026-11-16_18-34-diner-scene-2026-11-14-19-56.glb", "prompts/resources/b.py"),
            ("prompts/resources/2026-11-16_09-00-diner-scene-2026-11-10-08-05.glb", "prompts/resources/c.py"),
        ];
        assert_eq!(newest_of_each(&entries), vec![2, 3, 4]);
        // Read in any order, the newest still wins.
        let mut backwards = entries;
        backwards.reverse();
        assert_eq!(newest_of_each(&backwards), vec![0, 1, 2]);
    }

    #[test]
    fn rows_read_as_the_approved_list_does() {
        assert_eq!(date_of("2026-11-10_05-21-previs-location-grey.png"), "10 Nov");
        assert_eq!(date_of("2026-11-14_19-57-previs-far-end-door-swing-simple.mp4"), "14 Nov");
        assert_eq!(render_label("far-end-wall", "2026-11-10_05-21-previs-far-end-wall-grey.png"), "far-end-wall");
        assert_eq!(render_label("far-end-wall", "2026-11-14_19-13-previs-far-end-door-swing.mp4"), "far-end-wall · door swing");
        assert_eq!(
            render_label("far-end-wall", "2026-11-14_19-57-previs-far-end-door-swing-simple.mp4"),
            "far-end-wall · door swing, simple"
        );
        assert_eq!(builder_name("prompts/resources/2026-11-14_19-56-build-previs.py"), "2026-11-14 19:56");
        assert_eq!(builder_name("git:4195103:canonical/location/last-order-diner/look/previs/v3/build-previs.py"), "4195103");
        assert_eq!(Place { key: "diner".into(), scenes: Vec::new() }.name(), "Diner");
    }

    fn camera(name: &str, canon: &str, reference: Option<&str>) -> scene::Camera {
        scene::Camera {
            name: name.into(),
            position: [0.0; 3],
            target: [1.0, 0.0, 0.0],
            hfov: 60.0,
            image_size: [1664, 928],
            canon: Some(canon.into()),
            reference_file: reference.map(str::to_string),
            reference_look: None,
        }
    }

    fn scene_with(builder: &str, cameras: Vec<scene::Camera>) -> scene::Scene {
        let edges = scene::Edges { grey: 0, coded: 0, simple: None };
        let shading = scene::Shading { ambient: 0.5, diffuse: 0.5, light: [0.0, 0.0, 1.0] };
        scene::Scene {
            builder: builder.into(),
            builder_sha256: String::new(),
            image_size: [1664, 928],
            door_note: None,
            looks: scene::Looks { grey: BTreeMap::new(), coded: BTreeMap::new(), shading, edges, background: None, simple: None },
            objects: Vec::new(),
            cameras,
            swing: None,
        }
    }

    /// A camera rendered by two builders, and a third iteration made after
    /// both, as a workspace's Save makes one.
    fn wall() -> Place {
        const ARTIFACT: &str = "the diner: pre-vis far-end-wall";
        let old = scene_with("git:4195103:a.py", vec![camera("far-end-wall", ARTIFACT, Some("prompts/resources/2026-11-10_05-21-previs-far-end-wall-grey.png"))]);
        let new = scene_with("prompts/resources/b.py", vec![camera("far-end-wall", ARTIFACT, Some("prompts/resources/2026-11-11_17-20-previs-far-end-wall-grey.png"))]);
        Place {
            key: "diner".into(),
            scenes: vec![SceneFile { path: "a.glb".into(), scene: old }, SceneFile { path: "b.glb".into(), scene: new }],
        }
    }

    #[test]
    fn the_render_in_use_is_its_artifacts_latest_iteration() {
        let place = wall();
        assert_eq!(place.artifacts(), vec![String::from("the diner: pre-vis far-end-wall")]);
        let marks = |canon: &Canon| -> Vec<(String, bool, bool, bool, bool)> {
            place.renders(canon).into_iter().map(|r| (r.date, r.from_builder, r.in_use, r.dim, r.outlined)).collect()
        };
        // Unread, nothing is marked or dimmed.
        assert_eq!(marks(&Canon::default()), vec![("10 Nov".into(), true, false, false, false), ("11 Nov".into(), true, false, false, false)]);

        // The latest iteration is the 11 Nov render, out of date: in use and
        // outlined, the earlier one dimmer.
        let mut canon = Canon::default();
        canon.read_rows(&[serde_json::json!(["the diner: pre-vis far-end-wall", "prompts/resources/2026-11-11_17-20-previs-far-end-wall-grey.png"])]);
        canon.read_touches(&serde_json::json!({ "out_of_date": [{ "artifact": "the diner: pre-vis far-end-wall", "why": "its inputs changed" }], "unverified": [] }));
        assert_eq!(marks(&canon), vec![("10 Nov".into(), true, false, true, false), ("11 Nov".into(), true, true, false, true)]);

        // A later iteration no builder made is listed last, in use, with the
        // newest scene that has its camera, and is never laid over it.
        canon.read_rows(&[serde_json::json!(["the diner: pre-vis far-end-wall", "library/2026-11-19_10-00-far-end-wall-iteration.png"])]);
        canon.read_touches(&serde_json::json!({ "out_of_date": [] }));
        let renders = place.renders(&canon);
        assert_eq!(renders.len(), 3);
        let latest = &renders[2];
        assert_eq!((latest.date.as_str(), latest.from_builder, latest.in_use, latest.dim, latest.outlined), ("19 Nov", false, true, false, false));
        assert_eq!((latest.scene, latest.camera.as_str()), (1, "far-end-wall"));
        assert!(renders[..2].iter().all(|r| r.dim && !r.in_use));
    }

    #[test]
    fn the_canon_query_names_each_artifact_once() {
        assert_eq!(Canon::query(&[]), None);
        assert_eq!(
            Canon::query(&["the diner: pre-vis far-end-wall".into(), "Rosa's bar".into()]).unwrap(),
            "SELECT json_array(id, file) FROM artifact WHERE id IN ('the diner: pre-vis far-end-wall', 'Rosa''s bar')"
        );
        // A line that is not an [id, file] pair, or names no file, is left out.
        let mut canon = Canon::default();
        canon.read_rows(&[serde_json::json!("a | b"), serde_json::json!(["x", ""]), serde_json::json!(["y", "library/y.png"])]);
        assert_eq!(canon.latest.into_iter().collect::<Vec<_>>(), vec![(String::from("y"), String::from("library/y.png"))]);
    }

    /// Last order's diner, where Last order is on disk: set `LAST_ORDER` to
    /// its folder. Without it, the test says so and checks nothing.
    #[test]
    fn the_diner_lists_its_cameras_and_its_ten_renders() {
        let Some(folder) = std::env::var_os("LAST_ORDER") else {
            eprintln!("LAST_ORDER is not set, so the diner's list is not checked");
            return;
        };
        let (places, problems) = find(Path::new(&folder));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(places.len(), 1);
        let diner = &places[0];
        assert_eq!(diner.name(), "Diner");
        assert_eq!(diner.scenes.len(), 6);
        // Only the newest export of each builder is listed.
        let newest = diner.scenes.iter().map(|f| f.path.rsplit('/').next().unwrap()[..16].to_string()).max().unwrap();
        let exports = std::fs::read_dir(Path::new(&folder).join(RESOURCES))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("-diner-scene-"))
            .count();
        if exports > 6 {
            assert!(diner.scenes.iter().all(|f| f.path.rsplit('/').next().unwrap().starts_with(&newest)), "an older export is listed");
        }
        let newest = &diner.scenes[diner.newest()].scene;
        assert!(newest.builder.ends_with("2026-11-14_19-56-build-previs.py"), "{}", newest.builder);
        assert_eq!(newest.cameras.len(), 7);
        assert!(diner.scenes[0].scene.builder.starts_with("git:4195103"));
        let rows: Vec<(String, String)> = diner.renders(&Canon::default()).into_iter().map(|r| (r.label, r.date)).collect();
        let expected = [
            ("location", "10 Nov"),
            ("back-wall", "10 Nov"),
            ("window-wall", "10 Nov"),
            ("far-end-wall", "10 Nov"),
            ("near-end-wall", "10 Nov"),
            ("concept-shot", "10 Nov"),
            ("far-end-wall", "11 Nov"),
            ("door-close", "12 Nov"),
            ("far-end-wall · door swing", "14 Nov"),
            ("far-end-wall · door swing, simple", "14 Nov"),
        ];
        assert_eq!(rows, expected.map(|(a, b)| (a.to_string(), b.to_string())));
    }
}
