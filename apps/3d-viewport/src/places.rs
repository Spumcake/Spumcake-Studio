//! A project's places, read from their scene files, and what the camera list
//! shows for one: Free, the newest scene's cameras, and every published
//! pre-vis render across the place's scene files, each with its date, as the
//! approved camera list (vp-camera-list) shows.

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

    /// Every published render across the place's scene files, each from the
    /// scene whose own builder made it, in date order.
    pub fn renders(&self) -> Vec<Render> {
        let mut renders: Vec<Render> = self
            .scenes
            .iter()
            .enumerate()
            .flat_map(|(index, file)| {
                file.scene.cameras.iter().enumerate().filter_map(move |(order, camera)| {
                    let reference = camera.reference_file.clone()?;
                    let name = reference.rsplit('/').next().unwrap_or(&reference).to_string();
                    Some(Render {
                        scene: index,
                        order,
                        camera: camera.name.clone(),
                        label: render_label(&camera.name, &name),
                        date: date_of(&name),
                        video: is_video(&name),
                        file: reference,
                    })
                })
            })
            .collect();
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
        let rows: Vec<(String, String)> = diner.renders().into_iter().map(|r| (r.label, r.date)).collect();
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
