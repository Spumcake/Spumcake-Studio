//! Finding sub-applications. Each one is registered by its manifest,
//! `apps/<id>/manifest.json`, and its program sits beside the host's
//! executable. The host knows sub-applications only through these files.

use protocol::Manifest;
use std::path::{Path, PathBuf};

/// A sub-application whose manifest is valid and whose program exists.
pub struct App {
    pub manifest: Manifest,
    pub executable: PathBuf,
}

/// Folders that hold one folder per sub-application, in search order:
/// `apps` beside the host's executable, then, in a development build, the
/// workspace's own `apps` folder, found from the executable's own place.
fn roots(exe_dir: &Path) -> Vec<PathBuf> {
    #[allow(unused_mut)]
    let mut roots = vec![exe_dir.join("apps")];
    // `target/debug/inpainter` sits two folders below the workspace, which
    // stays true when the workspace is moved after it was built.
    #[cfg(debug_assertions)]
    if let Some(workspace) = exe_dir.parent().and_then(Path::parent) {
        roots.push(workspace.join("apps"));
    }
    roots
}

/// Every registered sub-application, ordered by id. An id found again in a
/// later folder is skipped. A manifest that does not parse, or whose program
/// is missing, is reported on stderr and skipped.
pub fn discover() -> Vec<App> {
    let Ok(exe) = std::env::current_exe() else {
        return Vec::new();
    };
    let Some(exe_dir) = exe.parent() else {
        return Vec::new();
    };
    discover_beside(exe_dir)
}

/// Every registered sub-application whose program is in `exe_dir`, as
/// [`discover`] finds them beside the host's own.
pub fn discover_beside(exe_dir: &Path) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    for root in roots(exe_dir) {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut folders: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        for folder in folders {
            let path = folder.join(protocol::MANIFEST_FILE);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let manifest = match protocol::parse_manifest(&text) {
                Ok(manifest) => manifest,
                Err(error) => {
                    eprintln!("inpainter: {}: {error}", path.display());
                    continue;
                }
            };
            if apps.iter().any(|app| app.manifest.id == manifest.id) {
                continue;
            }
            let executable = exe_dir.join(&manifest.executable);
            if !executable.is_file() {
                eprintln!(
                    "inpainter: {}: no program at {}",
                    manifest.id,
                    executable.display()
                );
                continue;
            }
            apps.push(App {
                manifest,
                executable,
            });
        }
    }
    apps.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    refuse_claimed_ids(apps)
}

/// `apps` less any whose manifest names, among its former ids, another's
/// current id or another's former id: what was kept under an id is one
/// sub-application's, never two.
fn refuse_claimed_ids(apps: Vec<App>) -> Vec<App> {
    let refused: Vec<String> = apps
        .iter()
        .filter(|app| {
            app.manifest.formerly.iter().any(|old| {
                apps.iter().any(|other| {
                    other.manifest.id != app.manifest.id
                        && (other.manifest.id == *old || other.manifest.formerly.contains(old))
                })
            })
        })
        .map(|app| app.manifest.id.clone())
        .collect();
    for id in &refused {
        eprintln!("inpainter: {id}: a former id it names is another sub-application's");
    }
    apps.into_iter().filter(|app| !refused.contains(&app.manifest.id)).collect()
}

/// The sub-applications to start. With no `--app`, every installed one its
/// manifest does not hide, in id order. With `--app`, each one named, in the
/// order named, hidden or not. An unknown id starts nothing. What was kept
/// under an id a sub-application had before moves to its own first.
pub fn launch() -> Vec<App> {
    let found = discover();
    if let Some(data) = data_root() {
        move_former_data(&data, &found);
    }
    pick(found, &requested())
}

/// What [`launch`] starts of `found`: with no `--app`, every one its
/// manifest does not hide; with `--app`, each one named, hidden or not.
fn pick(found: Vec<App>, requested: &[String]) -> Vec<App> {
    if requested.is_empty() {
        found.into_iter().filter(|app| !app.manifest.hidden).collect()
    } else {
        choose(found, requested)
    }
}

/// The id `id` is now: the id of the installed sub-application whose
/// manifest names `id` among its former ones, or else `id` itself.
pub fn current_id(installed: &[App], id: &str) -> String {
    if installed.iter().any(|app| app.manifest.id == id) {
        return id.to_string();
    }
    installed
        .iter()
        .find(|app| app.manifest.formerly.iter().any(|old| old == id))
        .map_or_else(|| id.to_string(), |app| app.manifest.id.clone())
}

/// A tool's key with its sub-application's former id made current, such as
/// a conversation's earlier calls.
pub fn current_tool_key(installed: &[App], key: &str) -> String {
    match protocol::split_tool_key(key) {
        Some((app, skill, tool)) => protocol::tool_key(&current_id(installed, app), skill, tool),
        None => key.to_string(),
    }
}

/// Each sub-application's folder in `data`'s `apps/`, under an id it had
/// before, becomes its own when it has none yet. One it already has is
/// kept, and the former left alone.
pub fn move_former_data(data: &Path, installed: &[App]) {
    let apps = data.join("apps");
    for app in installed {
        let own = apps.join(&app.manifest.id);
        for old in &app.manifest.formerly {
            let former = apps.join(old);
            if !own.exists() && former.is_dir() {
                if let Err(error) = std::fs::rename(&former, &own) {
                    eprintln!("inpainter: {} could not move to {}: {error}", former.display(), own.display());
                }
            }
        }
    }
}

/// The sub-applications `--app` names, in that order. An unknown id starts
/// nothing. An empty `requested` is not used here: [`launch`] starts all of them.
pub fn choose(apps: Vec<App>, requested: &[String]) -> Vec<App> {
    if requested.is_empty() {
        return apps.into_iter().take(1).collect();
    }
    let mut apps = apps;
    let mut chosen = Vec::new();
    for id in requested {
        match apps.iter().position(|app| app.manifest.id == *id) {
            Some(index) => chosen.push(apps.remove(index)),
            None => eprintln!("inpainter: no sub-application with id {id:?}"),
        }
    }
    chosen
}

/// Every `--app <id>` on the host's command line, in order.
pub fn requested() -> Vec<String> {
    let mut ids = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--app"
            && let Some(id) = args.next()
        {
            ids.push(id);
        }
    }
    ids
}

/// A sub-application's own folder in Inpainter's data folder, where it
/// keeps its preferences and recovery copies.
pub fn data_dir(id: &str) -> Option<PathBuf> {
    Some(data_root()?.join("apps").join(id))
}

/// Inpainter's data folder, which keeps each sub-application's own.
fn data_root() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("io.github", "spumcake", "Inpainter")?;
    Some(dirs.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, hidden: bool) -> App {
        let text = format!(r#"{{"id":"{id}","name":"{id}","executable":"{id}","protocol":{},"hidden":{hidden}}}"#, protocol::PROTOCOL_VERSION);
        App { manifest: protocol::parse_manifest(&text).unwrap(), executable: PathBuf::from(id) }
    }

    fn formerly(id: &str, old: &str) -> App {
        let mut app = app(id, false);
        app.manifest.formerly = vec![old.to_string()];
        app
    }

    /// Catches what was kept under one id going to two sub-applications, as
    /// when a manifest is copied with its former ids: both that name it are
    /// refused, and one naming a current id is too; an installed id is never
    /// taken for a former one.
    #[test]
    fn a_former_id_belongs_to_one_sub_application() {
        let kept = refuse_claimed_ids(vec![formerly("lantern", "lamp"), formerly("beacon", "lamp"), app("harbor", false)]);
        assert_eq!(ids(&kept), ["harbor"]);
        let kept = refuse_claimed_ids(vec![formerly("lantern", "harbor"), app("harbor", false)]);
        assert_eq!(ids(&kept), ["harbor"]);
        let installed = vec![formerly("lantern", "lamp"), app("lamp", false)];
        assert_eq!(current_id(&installed, "lamp"), "lamp");
    }

    fn ids(apps: &[App]) -> Vec<&str> {
        apps.iter().map(|app| app.manifest.id.as_str()).collect()
    }

    /// Catches a hidden sub-application that starts with the others, or one
    /// hidden by its id: any manifest that hides itself is left out, and
    /// `--app` still starts it.
    #[test]
    fn a_hidden_sub_application_starts_only_when_named() {
        let started = pick(vec![app("lantern", true), app("harbor", false)], &[]);
        assert_eq!(ids(&started), ["harbor"]);
        let started = pick(vec![app("lantern", true), app("harbor", false)], &[String::from("lantern")]);
        assert_eq!(ids(&started), ["lantern"]);
    }
}
