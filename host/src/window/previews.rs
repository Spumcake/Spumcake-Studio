//! The Prompt page's reference previews: each reference file's small image
//! and corner words, such as a video's first frame and "8.0 s" or a sound's
//! waveform, asked once of the workspace that declared `previews` for its
//! media type, or else of one whose `edits` covers it, and asked again when
//! the file changes. The plan card's references are asked the same way.
//! The host decodes no media itself.

use super::HostWindow;
use crate::panel::ReferencePreview;
use protocol::{Cell, CellPart, HostMessage};
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Default)]
pub(crate) struct Previews {
    /// What each reference shows, by `reference_key`.
    pub shown: HashMap<String, ReferencePreview>,
    /// Each reference asked for, with its file's time when asked.
    asked: HashMap<String, Option<SystemTime>>,
}

/// The workspace to ask for a preview of `media`: one that declared
/// `previews` for it, else one whose `edits` covers it.
pub(crate) fn previewer<'a>(apps: impl Iterator<Item = (&'a [String], &'a [String])> + Clone, media: &str) -> Option<usize> {
    let covers = |types: &[String]| types.iter().any(|known| known == media);
    apps.clone().position(|(previews, _)| covers(previews)).or_else(|| apps.clone().position(|(_, edits)| covers(edits)))
}

impl HostWindow {
    /// Asks for a preview of each file reference of the card's cell that
    /// is new or has changed since it was asked.
    pub(super) fn ask_previews(&mut self, cell: &Cell) {
        let root = self.project.as_ref().map(|project| project.folder.clone());
        for part in &cell.input {
            let CellPart::Path { path, media_type, .. } = part else { continue };
            let file = match &root {
                Some(root) if !std::path::Path::new(path).is_absolute() => root.join(path),
                _ => std::path::PathBuf::from(path),
            };
            self.ask_preview(crate::panel::reference_key(part), &file, media_type);
        }
    }

    /// Asks for a preview of each reference the plan card shows that is
    /// not an image, such as a video's first frame, by its path.
    pub(super) fn ask_plan_previews(&mut self) {
        let Some(asking) = self.runner.asking() else { return };
        let root = self.project.as_ref().map(|project| project.folder.clone());
        for path in crate::plan_card::reference_paths(asking) {
            let media_type = crate::prompt_page::media_type(&path);
            if media_type.starts_with("image/") {
                continue;
            }
            let file = match &root {
                Some(root) if !std::path::Path::new(&path).is_absolute() => root.join(&path),
                _ => std::path::PathBuf::from(&path),
            };
            self.ask_preview(path, &file, media_type);
        }
    }

    /// Asks for a preview of each project file named that is not an image,
    /// such as a report's video output, kept under its path in the project.
    pub(super) fn ask_file_previews(&mut self, paths: &[String]) {
        let Some(root) = self.project.as_ref().map(|project| project.folder.clone()) else { return };
        for path in paths {
            let media_type = crate::prompt_page::media_type(path);
            if !media_type.starts_with("image/") {
                self.ask_preview(path.clone(), &root.join(path), media_type);
            }
        }
    }

    /// Asks the workspace that previews `media_type` for `file`'s preview,
    /// kept under `key`, when it is new or has changed since it was asked.
    fn ask_preview(&mut self, key: String, file: &std::path::Path, media_type: &str) {
        if media_type.is_empty() {
            return;
        }
        let changed = std::fs::metadata(file).and_then(|meta| meta.modified()).ok();
        if self.previews.asked.get(&key) == Some(&changed) {
            return;
        }
        let apps = self.apps.iter().filter(|started| !started.running.exited()).map(|started| (started.running.declared.previews.as_slice(), started.running.declared.edits.as_slice()));
        let Some(found) = previewer(apps, media_type) else { return };
        // `found` counts only running workspaces; find its index among all.
        let Some(app) = self.apps.iter().enumerate().filter(|(_, started)| !started.running.exited()).nth(found).map(|(app, _)| app) else { return };
        self.previews.asked.insert(key.clone(), changed);
        self.send(app, HostMessage::Preview { id: key, path: file.display().to_string(), media_type: media_type.to_string() });
    }

    /// Keeps the previews workspaces answered.
    pub(super) fn take_previews(&mut self) {
        for started in &mut self.apps {
            for (id, preview) in started.running.declared.previewed.drain(..) {
                self.previews.shown.insert(id, preview);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a preview asked of a workspace that only edits the type
    /// when another declared it previews it, or none asked when only an
    /// editor covers it.
    #[test]
    fn a_preview_goes_to_its_previewer_before_its_editor() {
        let none: Vec<String> = Vec::new();
        let images = vec![String::from("image/png")];
        let sound = vec![String::from("audio/wav"), String::from("video/mp4")];
        let apps = [(none.as_slice(), images.as_slice()), (sound.as_slice(), none.as_slice())];
        assert_eq!(previewer(apps.iter().copied(), "audio/wav"), Some(1));
        assert_eq!(previewer(apps.iter().copied(), "image/png"), Some(0));
        assert_eq!(previewer(apps.iter().copied(), "text/plain"), None);
    }
}
