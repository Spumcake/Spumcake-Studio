//! What the person does on the film and inside a clip, by hand or through a
//! tool: the same calls either way. Each change to the film is one undo step,
//! made in the record; going somewhere, choosing a layer, and selecting on
//! the timeline never are.

use super::Session;
use crate::film::{CopiedFrames, Film, NewDrawing, Onion};
use crate::record::View;

/// What is selected on the timeline of one storyboard: clips at the film's
/// level, and a block of frames on layers inside a clip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picked {
    pub clips: Vec<u64>,
    pub cells: Option<Cells>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cells {
    pub layers: Vec<u64>,
    /// Frames `start ..= end`, inside the clip.
    pub start: u32,
    pub end: u32,
}

/// A text of a clip the panel and the tools set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipText {
    Name,
    Shot,
    Action,
    Dialogue,
    Camera,
    Animator,
}

/// A layer's switch the timeline's toggles and the tools set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerFlag {
    Visible,
    Locked,
    Outline,
    Open,
}

impl Session {
    pub fn film(&self) -> &Film {
        self.active_storyboard().film()
    }

    pub fn view(&self) -> View {
        self.active_storyboard().view()
    }

    /// Whether the panel shows the active layer's page, after its row was
    /// chosen, until a tool is chosen.
    pub fn layer_page(&self) -> bool {
        self.layer_page
    }

    pub fn show_layer_page(&mut self, on: bool) {
        self.layer_page = on;
    }

    /// The view a level first shows, fitted to the window, without a change.
    pub fn fit_view(&mut self, origin: [f64; 2], zoom: f64) {
        let zoom = zoom.clamp(draw::document::MIN_ZOOM, draw::document::MAX_ZOOM);
        self.active_storyboard_mut().fit(draw::document::Camera { origin, zoom });
    }

    /// An open storyboard, by id.
    pub fn record(&self, id: draw::document::DrawingId) -> Option<&crate::record::Record> {
        self.storyboards.iter().find(|record| record.id() == id)
    }

    pub fn open_panel(&mut self) {
        self.panel_open = true;
    }

    pub fn picked(&self) -> Picked {
        self.picked.get(&self.active_id()).cloned().unwrap_or_default()
    }

    fn set_picked(&mut self, picked: Picked) {
        let id = self.active_id();
        self.picked.insert(id, picked);
    }

    /// Whether the drawing tools work: inside a clip, on a layer that shows
    /// and is not locked. Everywhere else they are greyed out.
    pub fn can_draw(&self) -> bool {
        self.active_storyboard().can_draw()
    }

    /// One change to the film, as one undo step.
    fn change_film(&mut self, change: impl FnOnce(&mut Film) -> Option<Vec<NewDrawing>>) -> bool {
        self.settle();
        let palette = self.brush_palette.clone();
        let index = self.active;
        let changed = self.storyboards[index].edit_film(&mut self.ids, &palette, change);
        if changed {
            self.tidy_picked();
        }
        changed
    }

    /// Drops what the film no longer has from the selection.
    fn tidy_picked(&mut self) {
        let film = self.film().clone();
        let view = self.view();
        let mut picked = self.picked();
        picked.clips.retain(|id| film.clip(*id).is_some());
        if let Some(cells) = picked.cells.as_mut() {
            let clip = view.clip.and_then(|clip| film.clip(clip));
            cells.layers.retain(|layer| clip.is_some_and(|clip| clip.layer(*layer).is_some()));
            if cells.layers.is_empty() {
                picked.cells = None;
            }
        }
        self.set_picked(picked);
    }

    fn go(&mut self, view: View) {
        self.settle();
        self.active_storyboard_mut().go(view);
    }

    /// The clip being edited, when one is.
    pub fn open_clip(&self) -> Option<u64> {
        self.view().clip
    }

    /// The layers a frame change acts on: the selected block's, or the active
    /// layer.
    pub fn acting_layers(&self) -> Vec<u64> {
        match self.picked().cells {
            Some(cells) if !cells.layers.is_empty() => cells.layers,
            _ => self.view().layer.into_iter().collect(),
        }
    }

    // The film's level.

    /// A new clip at `start`, or after the last. Returns its id.
    pub fn add_clip(&mut self, start: Option<u32>, length: u32) -> Option<u64> {
        let mut made = None;
        self.change_film(|film| {
            let (id, drawings) = film.add_clip(start, length);
            made = Some(id);
            Some(drawings)
        });
        if let Some(id) = made {
            self.set_picked(Picked { clips: vec![id], cells: None });
        }
        made
    }

    pub fn duplicate_clip(&mut self, clip: u64) -> Option<u64> {
        let mut made = None;
        self.change_film(|film| {
            let (id, drawings) = film.duplicate_clip(clip)?;
            made = Some(id);
            Some(drawings)
        });
        made
    }

    pub fn delete_clip(&mut self, clip: u64) -> bool {
        self.change_film(|film| film.delete_clip(clip).then(Vec::new))
    }

    pub fn resize_clip(&mut self, clip: u64, end: bool, frames: i64) -> bool {
        self.change_film(|film| film.resize_clip(clip, end, frames).then(Vec::new))
    }

    pub fn move_clips(&mut self, clips: &[u64], frames: i64, push: bool) -> bool {
        self.change_film(|film| film.move_clips(clips, frames, push).then(Vec::new))
    }

    pub fn set_clip_text(&mut self, clip: u64, which: ClipText, text: &str) -> bool {
        self.change_film(|film| {
            let clip = film.clip_mut(clip)?;
            let field = match which {
                ClipText::Name => &mut clip.name,
                ClipText::Shot => &mut clip.shot,
                ClipText::Action => &mut clip.action,
                ClipText::Dialogue => &mut clip.dialogue,
                ClipText::Camera => &mut clip.camera,
                ClipText::Animator => &mut clip.animator,
            };
            if which == ClipText::Name && text.trim().is_empty() {
                return None;
            }
            (*field != text).then(|| {
                *field = text.to_string();
                Vec::new()
            })
        })
    }

    /// The storyboard's title, scene, frame size, and paper.
    pub fn set_film_settings(
        &mut self,
        title: Option<&str>,
        scene: Option<&str>,
        size: Option<[u32; 2]>,
        paper: Option<u8>,
    ) -> bool {
        self.change_film(|film| {
            let before = film.clone();
            if let Some(title) = title {
                film.title = title.to_string();
            }
            if let Some(scene) = scene {
                film.scene = scene.to_string();
            }
            if let Some(size) = size {
                film.size = [size[0].clamp(16, 16384), size[1].clamp(16, 16384)];
            }
            if let Some(paper) = paper {
                film.paper = paper.min(crate::film::PAPER_NAMES.len() as u8 - 1);
            }
            (*film != before).then(Vec::new)
        })
    }

    /// The playhead on the film, at the film's level.
    pub fn seek_film(&mut self, frame: u32) {
        let mut view = self.view();
        view.film_frame = frame;
        self.go(view);
    }

    pub fn select_clips(&mut self, clips: Vec<u64>) {
        self.set_picked(Picked { clips, cells: None });
    }

    /// Opens a clip on its sheet, at its first frame and first layer.
    pub fn enter_clip(&mut self, clip: u64) -> bool {
        let Some(at) = self.film().clip(clip) else {
            return false;
        };
        let view = View {
            clip: Some(clip),
            frame: 0,
            film_frame: at.start,
            layer: at.first_drawn_layer(),
        };
        self.go(view);
        self.set_picked(Picked::default());
        true
    }

    /// Back to the film, the playhead where the clip was.
    pub fn leave_clip(&mut self) -> bool {
        let view = self.view();
        let Some(clip) = view.clip else {
            return false;
        };
        let start = self.film().clip(clip).map_or(0, |clip| clip.start);
        self.go(View {
            clip: None,
            frame: 0,
            film_frame: start + view.frame,
            layer: None,
        });
        self.set_picked(Picked { clips: vec![clip], cells: None });
        true
    }

    // Inside a clip.

    /// The playhead inside the open clip.
    pub fn seek_clip(&mut self, frame: u32) {
        let mut view = self.view();
        view.frame = frame;
        self.go(view);
    }

    pub fn select_layer(&mut self, layer: u64) -> bool {
        let mut view = self.view();
        let Some(clip) = view.clip.and_then(|clip| self.film().clip(clip)) else {
            return false;
        };
        if clip.layer(layer).is_none() {
            return false;
        }
        view.layer = Some(layer);
        self.go(view);
        true
    }

    pub fn select_cells(&mut self, layers: Vec<u64>, start: u32, end: u32) {
        let (start, end) = (start.min(end), start.max(end));
        let cells = (!layers.is_empty()).then_some(Cells { layers, start, end });
        self.set_picked(Picked { clips: Vec::new(), cells });
    }

    fn in_clip<R>(&mut self, change: impl FnOnce(&mut Film, u64) -> Option<R>) -> Option<R> {
        let clip = self.open_clip()?;
        let mut out = None;
        self.change_film(|film| {
            let old = film.clip(clip).map_or(0, |at| at.length);
            let result = change(film, clip);
            film.follow_length(clip, old);
            let made = result.as_ref().map(|_| Vec::new());
            out = result;
            made
        });
        out
    }

    fn in_clip_making(&mut self, change: impl FnOnce(&mut Film, u64) -> Option<Vec<NewDrawing>>) -> bool {
        let Some(clip) = self.open_clip() else {
            return false;
        };
        self.change_film(|film| {
            let old = film.clip(clip).map_or(0, |at| at.length);
            let made = change(film, clip);
            film.follow_length(clip, old);
            made
        })
    }

    /// Adds a layer, or a folder, above the active layer, and makes it active.
    pub fn add_layer(&mut self, folder: bool) -> Option<u64> {
        let clip = self.open_clip()?;
        let above = self.view().layer;
        let mut made = None;
        self.change_film(|film| {
            let (id, drawings) = film.add_layer(clip, above, folder)?;
            made = Some(id);
            Some(drawings)
        });
        if let Some(id) = made
            && !folder
        {
            self.select_layer(id);
        }
        made
    }

    pub fn delete_layer(&mut self, layer: u64) -> bool {
        self.in_clip(|film, clip| film.delete_layer(clip, layer).then_some(())).is_some()
    }

    pub fn move_layer(&mut self, layer: u64, index: usize, parent: Option<u64>) -> bool {
        self.in_clip(|film, clip| film.move_layer(clip, layer, index, parent).then_some(())).is_some()
    }

    pub fn rename_layer(&mut self, layer: u64, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        self.in_clip(|film, clip| {
            let row = film.clip_mut(clip)?.layer_mut(layer)?;
            (row.name != name).then(|| row.name = name.to_string())
        })
        .is_some()
    }

    pub fn set_layer_flag(&mut self, layer: u64, flag: LayerFlag, on: bool) -> bool {
        self.in_clip(|film, clip| {
            let row = film.clip_mut(clip)?.layer_mut(layer)?;
            let field = match flag {
                LayerFlag::Visible => &mut row.visible,
                LayerFlag::Locked => &mut row.locked,
                LayerFlag::Outline => &mut row.outline,
                LayerFlag::Open => &mut row.open,
            };
            (*field != on).then(|| *field = on)
        })
        .is_some()
    }

    pub fn set_layer_color(&mut self, layer: u64, color: [u8; 3]) -> bool {
        self.in_clip(|film, clip| {
            let row = film.clip_mut(clip)?.layer_mut(layer)?;
            (row.color != color).then(|| row.color = color)
        })
        .is_some()
    }

    pub fn set_layer_opacity(&mut self, layer: u64, opacity: f64) -> bool {
        let opacity = opacity.clamp(0.0, 1.0);
        self.in_clip(|film, clip| {
            let row = film.clip_mut(clip)?.layer_mut(layer)?;
            ((row.opacity - opacity).abs() > 1e-9).then(|| row.opacity = opacity)
        })
        .is_some()
    }

    pub fn insert_frames(&mut self, layers: &[u64], frame: u32, count: u32) -> bool {
        self.in_clip(|film, clip| film.insert_frames(clip, layers, frame, count).then_some(())).is_some()
    }

    pub fn remove_frames(&mut self, layers: &[u64], frame: u32, count: u32) -> bool {
        self.in_clip_making(|film, clip| film.remove_frames(clip, layers, frame, count))
    }

    pub fn insert_key(&mut self, layer: u64, frame: u32, blank: bool) -> bool {
        let changed = self.in_clip_making(|film, clip| film.insert_key(clip, layer, frame, blank));
        if changed {
            let mut view = self.view();
            view.frame = frame;
            view.layer = Some(layer);
            self.go(view);
        }
        changed
    }

    pub fn clear_key(&mut self, layer: u64, frame: u32) -> bool {
        self.in_clip_making(|film, clip| film.clear_key(clip, layer, frame))
    }

    pub fn move_frames(&mut self, layers: &[u64], start: u32, end: u32, frames: i64, push: bool) -> bool {
        let changed = self.in_clip_making(|film, clip| film.move_frames(clip, layers, start, end, frames, push));
        if changed && let Some(mut cells) = self.picked().cells {
            cells.start = (i64::from(cells.start) + frames).max(0) as u32;
            cells.end = (i64::from(cells.end) + frames).max(0) as u32;
            self.set_picked(Picked { clips: Vec::new(), cells: Some(cells) });
        }
        changed
    }

    pub fn stretch_span(&mut self, layer: u64, key_frame: u32, frames: i64) -> bool {
        self.in_clip(|film, clip| film.stretch_span(clip, layer, key_frame, frames).then_some(())).is_some()
    }

    /// Copy frames: keeps them for Paste frames. Not a change.
    pub fn copy_frames(&mut self, layers: &[u64], start: u32, end: u32) -> bool {
        let Some(clip) = self.open_clip() else {
            return false;
        };
        let copied = self.film().copy_frames(clip, layers, start, end + 1);
        let found = copied.is_some();
        if found {
            self.frames_clipboard = copied;
        }
        found
    }

    pub fn has_copied_frames(&self) -> bool {
        self.frames_clipboard.is_some()
    }

    pub fn paste_frames(&mut self, layers: &[u64], frame: u32) -> bool {
        let Some(copied): Option<CopiedFrames> = self.frames_clipboard.clone() else {
            return false;
        };
        self.in_clip_making(|film, clip| film.paste_frames(clip, layers, frame, &copied))
    }

    pub fn reverse_frames(&mut self, layers: &[u64], start: u32, end: u32) -> bool {
        self.in_clip_making(|film, clip| film.reverse_frames(clip, layers, start, end + 1))
    }

    pub fn add_mark(&mut self, layer: u64, frame: u32, comment: bool, text: &str) -> Option<u64> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        self.in_clip(|film, clip| film.add_mark(clip, layer, frame, comment, text))
    }

    pub fn remove_mark(&mut self, layer: u64, mark: u64) -> bool {
        self.in_clip(|film, clip| film.remove_mark(clip, layer, mark).then_some(())).is_some()
    }

    /// The onion skin's settings. A view aid, saved with the storyboard
    /// but never an undo step.
    pub fn onion(&self) -> Onion {
        self.active_storyboard().onion()
    }

    pub fn set_onion(&mut self, change: impl FnOnce(&mut Onion)) -> bool {
        let mut onion = self.onion();
        change(&mut onion);
        onion.before = onion.before.min(9);
        onion.after = onion.after.min(9);
        onion.opacity = onion.opacity.clamp(0.0, 1.0);
        onion.fade = onion.fade.clamp(0.0, 1.0);
        self.active_storyboard_mut().set_onion(onion)
    }
}
