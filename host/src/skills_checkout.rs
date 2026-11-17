//! Where Studio finds the user's Spumcake-Skills checkout, so a project
//! opens and runs with no setting, as the owner's decision A sets until
//! Studio's installer brings Spumcake-Skills itself: the folders named in
//! Settings when one of them holds Spumcake-Skills' scripts, and otherwise
//! the checkout found beside Studio, installed next to its program or,
//! while Studio is developed, cloned in the workspace's source folder.
//! No folder of a particular computer is named here.

use std::path::{Path, PathBuf};

/// Said when no project is open and something needs one.
pub(crate) const NO_PROJECT: &str = "No project is open.";

/// Said when Spumcake-Skills cannot be found, and what to do about it.
pub(crate) const MISSING: &str = "Studio can't find Spumcake-Skills, which runs every step of a project. Add its folder in Settings.";

/// A checkout's folder named outside Settings, as a test or a developer's
/// shell may set it.
const NAMED: &str = "SPUMCAKE_SKILLS";

/// Whether `folder` is a Spumcake-Skills checkout: its scripts and skills.
fn is_checkout(folder: &Path) -> bool {
    folder.join("scripts/request.py").is_file() && folder.join("skills").is_dir()
}

/// Whether one of these folders, or a folder above it, holds Spumcake-Skills'
/// scripts, as `runner::scripts_folder` finds them from a listed skill.
fn names_a_checkout(folders: &[String]) -> bool {
    folders.iter().any(|folder| Path::new(folder).ancestors().take(4).any(is_checkout))
}

/// The checkout Studio finds by itself: the one `SPUMCAKE_SKILLS` names;
/// one installed beside Studio's program; or, in a development build, the
/// clone in the source folder Studio was built from, `spumcake-skills/`
/// beside `spumcake-studio/`, or its `worktrees/dev/`. A test finds only
/// the one named, so it never reads the developer's own checkout.
pub(crate) fn found() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os(NAMED).map(PathBuf::from).filter(|folder| is_checkout(folder)) {
        return Some(named);
    }
    if cfg!(test) {
        return None;
    }
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("spumcake-skills")));
    if let Some(beside) = beside.filter(|folder| is_checkout(folder)) {
        return Some(beside);
    }
    if cfg!(debug_assertions) {
        for above in Path::new(env!("CARGO_MANIFEST_DIR")).ancestors() {
            for candidate in ["spumcake-skills/worktrees/dev", "spumcake-skills", "Spumcake-Skills"] {
                let folder = above.join(candidate);
                if is_checkout(&folder) {
                    return Some(folder);
                }
            }
        }
    }
    None
}

/// The scripts folder of the Spumcake-Skills checkout these skill folders,
/// or the one Studio finds, hold.
pub(crate) fn scripts(settings: &[String]) -> Option<PathBuf> {
    settings
        .iter()
        .filter_map(|folder| Path::new(folder).ancestors().take(4).find(|above| is_checkout(above)).map(Path::to_path_buf))
        .next()
        .or_else(found)
        .map(|checkout| checkout.join("scripts"))
}

/// Lays a new project out in `folder` the way Last order is, with
/// Spumcake-Skills' own `request.py init`, so a project made in Studio
/// continues in an agentic harness. It runs with no key, and a refusal is
/// said as Skills says it.
pub(crate) fn lay_out(folder: &Path, settings: &[String]) -> Result<(), String> {
    // A test that names no checkout makes its project bare, so the window's
    // checks never need Spumcake-Skills.
    let Some(scripts) = scripts(settings) else {
        return if cfg!(test) { Ok(()) } else { Err(MISSING.to_string()) };
    };
    let parent = folder.parent().filter(|parent| parent.is_dir()).map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    let mut command = std::process::Command::new("python3");
    command.arg(scripts.join("request.py")).arg("init").arg(folder).arg("--json").current_dir(parent);
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if text.ends_with("_KEY") || text.ends_with("_TOKEN") || text.ends_with("_SECRET") {
            command.env_remove(&name);
        }
    }
    let out = command.output().map_err(|error| format!("Spumcake-Skills could not lay the project out: {error}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let sentence = said.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or("Spumcake-Skills could not lay the project out.");
    Err(sentence.trim_start_matches("refused: ").to_string())
}

/// The skill folders a project's setup lists: those named in Settings, and
/// the checkout Studio found when none of them is Spumcake-Skills.
pub(crate) fn skill_folders(settings: &[String]) -> Vec<String> {
    let mut folders = settings.to_vec();
    if !names_a_checkout(settings)
        && let Some(found) = found()
    {
        folders.push(found.display().to_string());
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("skills-checkout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("scripts")).unwrap();
        std::fs::create_dir_all(folder.join("skills/project-requests")).unwrap();
        std::fs::write(folder.join("scripts/request.py"), "").unwrap();
        folder
    }

    /// Catches a checkout named in Settings being joined by a second one,
    /// or none being found when Settings names none.
    #[test]
    fn settings_win_and_otherwise_the_found_checkout_is_listed() {
        let named = checkout("named");
        let skills = named.join("skills").display().to_string();
        assert_eq!(skill_folders(std::slice::from_ref(&skills)), vec![skills.clone()]);
        assert!(names_a_checkout(std::slice::from_ref(&skills)));
        // Without SPUMCAKE_SKILLS, a test finds no checkout of its own.
        if std::env::var_os(NAMED).is_none() {
            assert_eq!(skill_folders(&[]), Vec::<String>::new());
        }
        assert!(!names_a_checkout(&["/no/such/folder".into()]));
        let _ = std::fs::remove_dir_all(named);
    }
}
