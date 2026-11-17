//! The shot's animatic, as vg-frame-from and vg-take-over-animatic show
//! it: the Start frame taken from the Prompt page's Motion reference at
//! the time shown, and a round's take shown over the animatic, its line
//! art in white and its pre-vis edges in cyan, with the toolbar's pre-vis
//! check.

use super::*;
use crate::decode::Key;
use crate::view::Animatic;
use protocol::{Cell, CellPart, ItemRef};

/// The role of the reference a Start frame is taken from, and of the
/// Start frame, as the Prompt page's roles name them.
const MOTION: &str = "Motion reference";
const START: &str = "Start frame";

/// Whether a reference's role is `label`: the role's menu word, alone or
/// before the person's own words, "Start frame, the keeper".
fn role_is(role: Option<&str>, label: &str) -> bool {
    role.is_some_and(|role| role == label || role.strip_prefix(label).is_some_and(|rest| rest.starts_with(',')))
}

/// A cell's Motion reference: the first reference whose role is Motion
/// reference, when it is a file.
pub fn motion_reference(cell: &Cell) -> Option<&str> {
    cell.input.iter().find_map(|part| match part {
        CellPart::Path { path, role, .. } if role_is(role.as_deref(), MOTION) => Some(path.as_str()),
        _ => None,
    })
}

impl Session {
    /// The time the Video Generator's own transport shows: the Player's
    /// while playing, or else the frame shown's, in seconds.
    pub(super) fn time_shown(&self) -> f64 {
        if let Some(time) = self.play_time() {
            return time;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return 0.0;
        };
        let shown = tab.round_view();
        let gallery = shown.as_ref().unwrap_or(&tab.gallery);
        gallery.take().map_or(0.0, |take| view::seconds_at(take, tab.frame_shown()))
    }

    /// A file a cell or a round names, relative to the project or whole.
    fn project_file(&self, path: &str) -> PathBuf {
        if Path::new(path).is_absolute() { PathBuf::from(path) } else { self.project.clone().unwrap_or_default().join(path) }
    }

    /// What a video file is, read once for each file.
    fn animatic_info(&mut self, file: &Path) -> Option<media::Info> {
        if let Some(info) = self.animatics.get(file) {
            return *info;
        }
        let info = media::Video::open(file).ok().map(|video| video.info());
        self.animatics.insert(file.to_path_buf(), info);
        info
    }

    /// The animatic's frame at the playhead: the Prompt page's draft's
    /// Motion reference's frame at the time shown, written as a PNG item of
    /// the tab's file, put in the draft's Start frame in place of any
    /// other. One step of the gallery's history.
    pub(super) fn frame_from_animatic(&mut self, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(cell) = self.tabs.get(index).and_then(|tab| tab.gallery.current_cell()).filter(|cell| cell.step.is_none()).cloned() else {
            return;
        };
        let Some(path) = motion_reference(&cell).map(str::to_string) else {
            return;
        };
        let time = self.time_shown();
        let file = self.project_file(&path);
        let item = match self.file_animatic_frame(index, &file, time) {
            Ok(item) => item,
            Err(reason) => {
                out.push(notice("This video could not be opened", &reason));
                return;
            }
        };
        let frame = CellPart::Item { item, media_type: String::from("image/png"), part: None, role: Some(START.into()), field: None };
        let mut draft = cell.clone();
        // The Start frame's place, or after the prompt's words.
        let at = draft
            .input
            .iter()
            .position(|part| match part {
                CellPart::Path { role, .. } | CellPart::Item { role, .. } => role_is(role.as_deref(), START),
                _ => false,
            })
            .unwrap_or_else(|| draft.input.iter().take_while(|part| matches!(part, CellPart::Text { .. })).count());
        draft.input.retain(|part| match part {
            CellPart::Path { role, .. } | CellPart::Item { role, .. } => !role_is(role.as_deref(), START),
            _ => true,
        });
        draft.input.insert(at.min(draft.input.len()), frame);
        let tab = &mut self.tabs[index];
        let mut cells = tab.gallery.cells.clone();
        if let Some(own) = cells.iter_mut().find(|own| own.id == draft.id) {
            *own = draft.clone();
        }
        if tab.gallery.set_cells(cells, Some(draft.id)) {
            tab.dirty = true;
        }
        out.extend(self.emit());
        out.extend(self.items_due());
    }

    /// The animatic's frame at `time`, at its own size, as an item of the
    /// tab's file: its PNG rendered now under `protocol::render_path`,
    /// named by its fingerprint, so `render` finds it, and the tab's file
    /// written first, so the item has a file to be of.
    fn file_animatic_frame(&mut self, index: usize, file: &Path, time: f64) -> Result<ItemRef, String> {
        let Some(project) = self.project.clone() else {
            return Err("The video is not in a project.".into());
        };
        let mut video = media::Video::open(file).map_err(|error| error.0)?;
        let info = video.info();
        let frame = ((time.max(0.0) * info.fps()).round() as u32).min(info.frames.saturating_sub(1));
        let decoded = video.frame(frame, crate::decode::even((info.width, info.height))).map_err(|error| error.0)?;
        if self.tabs[index].path.is_none() {
            self.tabs[index].dirty = true;
        }
        let path = self.write_into_project(index)?.ok_or_else(|| String::from("The video could not be written."))?;
        let relative = self.relative(&path).ok_or_else(|| String::from("The video is not in the project."))?;
        let png = files::encode_png(decoded.width, decoded.height, &decoded.rgba)?;
        let fingerprint = protocol::fingerprint(&png);
        let id = format!("{ANIMATIC_ITEM}{fingerprint}");
        let item = ItemRef { path: relative, item: Some(id.clone()) };
        files::write_atomic(&project.join(protocol::render_path(OWNER, &item, None, &fingerprint, "png")), &png)?;
        let tab = &mut self.tabs[index];
        if !tab.frames_taken.iter().any(|(own, _)| *own == id) {
            tab.frames_taken.push((id, fingerprint));
        }
        Ok(item)
    }

    /// Open in the Image Generator: the reference at `reference`, counted
    /// among the cell's files and items as the host counts them, opened in
    /// the workspace that edits it, when it is an image. A Start frame
    /// taken from the animatic opens as its PNG.
    pub(super) fn frame_open_image(&mut self, reference: usize, out: &mut Vec<AppMessage>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(cell) = tab.gallery.current_cell() else {
            return;
        };
        let part = cell.input.iter().filter(|part| matches!(part, CellPart::Path { .. } | CellPart::Item { .. })).nth(reference);
        let opened = match part {
            Some(CellPart::Path { path, media_type, .. }) if media_type.starts_with("image/") => Some((path.clone(), media_type.clone())),
            Some(CellPart::Item { item, .. }) => item.item.as_deref().and_then(|own| own.strip_prefix(ANIMATIC_ITEM)).map(|fingerprint| {
                (protocol::render_path(OWNER, item, None, fingerprint, "png"), String::from("image/png"))
            }),
            _ => None,
        };
        if let Some((path, media_type)) = opened {
            out.push(AppMessage::OpenIn { path, media_type });
        }
    }

    /// Whether the Prompt page's cell can take a Start frame from its
    /// animatic: a draft with a Motion reference.
    pub fn has_animatic(&self) -> bool {
        self.active_tab()
            .and_then(|tab| tab.gallery.current_cell())
            .is_some_and(|cell| cell.step.is_none() && motion_reference(cell).is_some())
    }

    /// The animatic a round's take shown is laid over: its round's Motion
    /// reference, or else its Start frame. None for a take not in a round,
    /// or a round with neither.
    pub fn animatic_of(&self, tab: &Tab) -> Option<String> {
        let tile = tab.rounds.tiles.get(tab.rounds.shown?)?;
        let round = tab.rounds.round_of(tile)?;
        let find = |label: &str| round.references.iter().find(|reference| role_is(reference.role.as_deref(), label)).map(|reference| reference.path.clone());
        find(MOTION).or_else(|| find(START))
    }

    /// The pre-vis check's button: the take shown over its animatic, on
    /// or off for the tab, never kept.
    pub(super) fn previs_toggle(&mut self) {
        let can = self.active_tab().is_some_and(|tab| self.animatic_of(tab).is_some());
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.over_animatic = can && !tab.over_animatic;
        }
    }

    /// The animatic's frame to draw over the take shown, when the pre-vis
    /// check is on: at the take's time, its last when the take runs past
    /// it, decoded at the size the take's frame is.
    pub(super) fn animatic_over(&mut self, scene: &Scene) -> Option<Animatic> {
        let tab = self.tabs.get(self.active)?;
        if !tab.over_animatic {
            return None;
        }
        let path = self.animatic_of(tab)?;
        let place = scene.places.first()?;
        let size = place.key.as_ref().map(|key| (key.width, key.height))?;
        let time = self.time_shown();
        let file = self.project_file(&path);
        let info = self.animatic_info(&file)?;
        let index = ((time.max(0.0) * info.fps()).round() as u32).min(info.frames.saturating_sub(1));
        Some(Animatic { place: 0, key: Key::new(file, index, size) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a role taken for another: "Motion reference" alone or with
    /// the person's words is one, and a longer menu word is not.
    #[test]
    fn a_role_is_its_menu_word() {
        assert!(role_is(Some("Motion reference"), MOTION));
        assert!(role_is(Some("Motion reference, the walk"), MOTION));
        assert!(!role_is(Some("Motion references"), MOTION));
        assert!(!role_is(Some("Reference"), MOTION));
        assert!(!role_is(None, MOTION));
    }
}
