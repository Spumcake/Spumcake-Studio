//! A tab's requests: its cells on the Prompt page, the rounds the host
//! says they made, the take shown, a take's menu, layers written out, and
//! the previews the host asks for.

use super::*;
use crate::rounds::{self, IMAGE_TYPES};
use protocol::Round;

/// The start of a layer written out's filing id, so its `filed` is known.
pub(super) const WRITTEN_OUT: &str = "written-out-";

impl Session {
    /// The cells of every tab whose cells the host has not been told, each
    /// tab with a cell for its Prompt page, an empty draft when it had none.
    pub(super) fn cells_due(&mut self) -> Vec<AppMessage> {
        let mut out = Vec::new();
        for image in &mut self.images {
            image.gallery.ensure_cell();
            let now = (image.gallery.cells.clone(), image.gallery.cell.clone());
            if image.cells_sent.as_ref() != Some(&now) {
                out.push(AppMessage::Cells { tab: image.id.clone(), cells: now.0.clone(), selected: now.1.clone() });
                image.cells_sent = Some(now);
            }
        }
        out
    }

    /// Every round a tab's cell has made, as `takes` sends them: the strip
    /// is laid out again, each take read from its file, and nothing of
    /// them is kept in the tab's file.
    pub(super) fn takes(&mut self, cell: &str, rounds: Vec<Round>, out: &mut Vec<AppMessage>) {
        let Some(index) = self.images.iter().position(|image| image.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let root = self.project.clone().unwrap_or_default();
        let cache = self.cache.clone();
        let mut next = self.next_revision;
        let image = &mut self.images[index];
        let seen = image.frame().uid;
        let order: Vec<String> = image.gallery.cells.iter().map(|own| own.id.clone()).collect();
        // The artifact the cell's step publishes, as its newest round says
        // it, kept with the cell while the program runs: it is the chain's,
        // never the file's.
        if let Some(artifact) = rounds.iter().rev().find_map(|round| round.artifact.clone()) {
            image.gallery.artifacts.insert(cell.to_string(), artifact);
        }
        for gone in image.rounds.set(cell, rounds, &order) {
            forget_files(&cache, &image.id, gone);
        }
        image.rounds.read_files(&root, &image.id, &cache, &mut next);
        if image.frame().uid != seen {
            image.send_camera = true;
        }
        self.next_revision = next;
        if index == self.active {
            out.extend(self.emit());
        }
    }

    /// A click on a take's tile shows it. With Ctrl or Shift, a take of
    /// another round than the one shown shows the two rounds one above the
    /// other, as ig-rounds-side-by-side does.
    pub(super) fn select_take(&mut self, tile: usize, extend: bool, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(image) = self.images.get_mut(index) else {
            return;
        };
        let Some(chosen) = image.rounds.tiles.get(tile).filter(|chosen| chosen.file.is_some()) else {
            return;
        };
        let round = (chosen.cell.clone(), chosen.request.clone());
        let shown = image.rounds.shown.and_then(|shown| image.rounds.tiles.get(shown)).map(|shown| (shown.cell.clone(), shown.request.clone()));
        if extend && shown.is_some_and(|shown| shown != round) {
            image.rounds.view = View::Pair(round.1);
            image.send_camera = true;
            return;
        }
        if let Err(reason) = self.show_take(index, tile) {
            out.push(notice("This image could not be opened", &reason));
        }
    }

    /// Shows a take in the working area, read from its file as a linked
    /// layer the first time. Its pixels are locked: a take is never
    /// painted on, as Edit puts it on a layer of an image of the gallery.
    fn show_take(&mut self, index: usize, tile: usize) -> Result<(), String> {
        let cache = self.cache.clone();
        let Some(image) = self.images.get_mut(index) else {
            return Ok(());
        };
        let seen = image.frame().uid;
        let before = image.doc().edits;
        image.doc_mut().finish_edit();
        if image.doc().edits != before {
            image.dirty = true;
        }
        self.take_frame(index, tile)?;
        let image = &mut self.images[index];
        if let Some(gone) = image.rounds.show(tile) {
            forget_files(&cache, &image.id, gone);
        }
        if image.frame().uid != seen {
            image.send_camera = true;
        }
        Ok(())
    }

    /// A take's frame, read from its file once: a layer linked to the file,
    /// its pixels locked, with the notes the tab's file kept for those
    /// bytes, if any.
    fn take_frame(&mut self, index: usize, tile: usize) -> Result<(), String> {
        let root = self.project.clone().unwrap_or_default();
        let Some(image) = self.images.get_mut(index) else {
            return Ok(());
        };
        if image.rounds.tiles.get(tile).is_none_or(|chosen| chosen.frame.is_some()) {
            return Ok(());
        }
        let Some(file) = image.rounds.tiles[tile].file.clone() else {
            return Ok(());
        };
        let (pixels, sha256) = files::import_linked(&files::resolve(&root, &file.path))?;
        let notes = image.rounds.kept_notes(&file.path, &sha256);
        let name = file_title(Path::new(&file.path));
        let mut layered = LayeredImage::linked(&name, pixels, layers::link::Link { path: file.path.clone(), sha256 });
        layered.nodes[0].locks.pixels = true;
        let mut doc = Doc::new(layered);
        if let Some(notes) = notes {
            doc.notes = notes;
        }
        let frame = image.gallery.frame_for(&name, doc);
        image.rounds.tiles[tile].frame = Some(Box::new(frame));
        Ok(())
    }

    /// The round views, drawn as one image the working area's size, with
    /// the host's camera put on it whole, as the Video Generator's views
    /// are. The pointer only pans it.
    pub(super) fn round_view(&mut self, index: usize) -> Option<RasterView> {
        let width = (self.viewport[0].max(200.0) as u32) & !1;
        let height = (self.viewport[1].max(200.0) as u32) & !1;
        let root = self.project.clone().unwrap_or_default();
        let cache = self.cache.clone();
        let image = self.images.get_mut(index)?;
        let file = cache.join(format!("{}-round.rgba", image.id));
        let key = image.rounds.key(width, height);
        let drawn = image.rounds.drawn.as_ref().filter(|(drawn, _)| *drawn == key).map(|(_, revision)| *revision);
        let revision = match drawn {
            Some(revision) => revision,
            None => {
                let canvas = image.rounds.compose(&root, width, height)?;
                if files::write_atomic(&file, &canvas.data).is_err() {
                    self.display_failed = true;
                }
                self.next_revision += 1;
                image.rounds.drawn = Some((key, self.next_revision));
                self.next_revision
            }
        };
        let camera = std::mem::take(&mut image.send_camera).then_some(DocumentCamera { origin: [0.0, 0.0], zoom: 1.0, rotation: 0.0, flip: false });
        let film = declare::film(image);
        Some(RasterView {
            tab: image.id.clone(),
            width,
            height,
            image: file.display().to_string(),
            revision,
            patch: None,
            tool: RasterTool::Hand,
            selection: Vec::new(),
            crop: None,
            bounds: None,
            camera,
            camera_owner: protocol::RasterCamera::Host,
            transport: None,
            film: Some(film),
            text: None,
        })
    }

    /// Next round from this: a draft of the take's round, with the same
    /// references in order and the take itself as a reference; when the
    /// take has notes, their words end the prompt and the marked image is
    /// sent too, written beside the tab's file in `work/` and filed, then
    /// joining the draft when `filed` names it. A draft not yet run takes
    /// it in place; otherwise it is a new cell beside the one it came from.
    /// One step of the strip's history.
    pub(super) fn next_round(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(image) = self.images.get_mut(index) else {
            return;
        };
        let Some(tile) = image.rounds.tile_of(path) else {
            return;
        };
        let Some(round) = image.rounds.round_of(&image.rounds.tiles[tile]).cloned() else {
            return;
        };
        let Some(take) = image.rounds.tiles[tile].file.clone() else {
            return;
        };
        // Notes the file kept for a take not shown since are read with it.
        if image.rounds.tiles[tile].frame.is_none() && image.rounds.kept.iter().any(|kept| kept.link.path == take.path) {
            if let Err(reason) = self.take_frame(index, tile) {
                out.push(notice("This image could not be opened", &reason));
            }
        }
        let image = &mut self.images[index];
        let (words, marked) = match image.rounds.tiles[tile].frame.as_deref_mut() {
            Some(frame) if !frame.doc.notes.is_empty() => (frame.doc.notes_words(), Some(frame.doc.marked_image())),
            _ => (Vec::new(), None),
        };
        let mut prompt = round.prompt.trim_end().to_string();
        if !words.is_empty() {
            let notes = format!("{NOTES_WORDS}: {}", words.join(" "));
            prompt = if prompt.is_empty() { notes } else { format!("{prompt} {notes}") };
        }
        let mut input = vec![CellPart::Text { text: prompt }];
        for reference in &round.references {
            input.push(CellPart::Path { path: reference.path.clone(), media_type: media_type_of(&reference.path), role: reference.role.clone(), field: None });
        }
        input.push(CellPart::Path { path: take.path.clone(), media_type: take.media_type.clone(), role: Some(rounds::ROLES[0].to_string()), field: None });
        let image = &mut self.images[index];
        let current = image.gallery.current_cell().cloned();
        let from = current.as_ref().filter(|cell| cell.step.is_none());
        // The takes' size, or the image's, for a draft's Size.
        let size = image.rounds.shape().unwrap_or_else(|| {
            let shown = &image.gallery.frame().doc.image;
            (shown.width, shown.height)
        });
        let mut draft = match from.filter(|cell| !image.rounds.has_rounds(&cell.id)) {
            Some(cell) => cell.clone(),
            None => crate::gallery::empty_draft(size),
        };
        draft.input = input;
        draft.model = Some(round.model.clone()).filter(|model| !model.is_empty()).or_else(|| from.and_then(|cell| cell.model.clone()));
        draft.settings = from.and_then(|cell| cell.settings.clone()).or_else(|| Some(crate::gallery::size_setting(size)));
        draft.endpoint = from.and_then(|cell| cell.endpoint.clone());
        let mut cells = image.gallery.cells.clone();
        match cells.iter_mut().find(|cell| cell.id == draft.id) {
            Some(cell) => *cell = draft.clone(),
            None => cells.push(draft.clone()),
        }
        let (made, cell) = (draft.input.clone(), draft.id.clone());
        if image.gallery.set_cells(cells, Some(draft.id)) {
            image.dirty = true;
        }
        // The marked image, written and filed now; it joins the draft when
        // `filed` comes.
        if let Some(marked) = marked {
            let stem = format!("{} notes", self.images[index].gallery.frame().name);
            let filed = files::encode_png(marked.width, marked.height, &marked.data)
                .and_then(|png| self.file_for_draft(index, &stem, &png, &take.path, &cell, made, out));
            if let Err(reason) = filed {
                out.push(notice("The image could not be exported", &reason));
            }
        }
        self.page = Page::Prompt;
        out.extend(self.emit());
    }

    /// Ask the assistant to use this take: a prepared request naming the
    /// take, put in the conversation's composer for the person to send, as
    /// a take enters the chain only through the assistant.
    pub(super) fn ask_about(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let Some(image) = self.active_image() else {
            return;
        };
        let Some(tile) = image.rounds.tile_of(path).map(|tile| &image.rounds.tiles[tile]) else {
            return;
        };
        let text = format!("Put take {} of round {}, {path}, into the chain.", tile.take + 1, tile.number);
        out.push(AppMessage::AskAssistant { text, items: vec![ItemRef { path: path.to_string(), item: None }] });
    }

    /// Edit: the take on a layer linked to its file, in a new image of the
    /// gallery, shown, to paint on.
    pub(super) fn edit_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let root = self.project.clone().unwrap_or_default();
        let file = files::resolve(&root, path);
        match files::import_linked(&file) {
            Ok((pixels, sha256)) => {
                let title = file_title(Path::new(path));
                let link = self.link_to(&file, sha256);
                let doc = Doc::new(LayeredImage::linked(&title, pixels, link));
                self.film(|gallery| gallery.add(&title, doc, None));
            }
            Err(reason) => out.push(notice("This image could not be opened", &reason)),
        }
    }

    /// Write out as PNG…: a layer of the image shown, as a PNG of the
    /// image's size, a plain file next to the tab's file, named as the
    /// layer, free among the files there; with no file and no project,
    /// where the person chooses. Inside a project, what leaves the
    /// workspace is filed by the runner: `file_resource` asks it to, with
    /// `request.py resource` the PNG `--from` the tab's file in `work/`,
    /// then the layer's own linked file, then the file of every linked
    /// layer beneath it, edited or not, with an id, so a refusal is said.
    pub(super) fn write_out(&mut self, node: u64, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(image) = self.images.get(index) else {
            return;
        };
        // A take is no image of the gallery: Edit puts it on one first.
        if image.take_shown() {
            return;
        }
        let name = image.gallery.frame().name.clone();
        let tab = image.id.clone();
        let linked = linked_from(image.doc(), node);
        let (layer, png) = match layer_png(image.doc(), node) {
            Ok(written) => written,
            Err(reason) => {
                out.push(notice("The image could not be exported", &reason));
                return;
            }
        };
        let file = match (self.images[index].path.clone(), self.project.is_some()) {
            (Some(path), _) => Some(path),
            (None, true) => {
                self.images[index].dirty = true;
                match self.write_into_project(index) {
                    Ok(path) => path,
                    Err(reason) => {
                        out.push(notice("The image could not be exported", &reason));
                        return;
                    }
                }
            }
            (None, false) => {
                let filter = FileFilter { name: "PNG".into(), extensions: vec!["png".into()] };
                self.ask_file(FileKind::Save, Some(format!("{layer}.png")), Purpose::WriteOut { tab, node }, filter, out);
                return;
            }
        };
        // In the project's work/, or beside the tab's file with no project.
        let Some(folder) = self.work_folder().or_else(|| file.as_deref().and_then(Path::parent).map(Path::to_path_buf)) else {
            return;
        };
        let stem = match safe_name(&layer) {
            stem if stem.is_empty() => safe_name(&name),
            stem => stem,
        };
        let stem = protocol::free_name(&stem, |stem| folder.join(format!("{stem}.png")).exists());
        let written = folder.join(format!("{stem}.png"));
        if let Err(reason) = files::write_atomic(&written, &png) {
            out.push(notice("The image could not be exported", &reason));
            return;
        }
        if let (Some(path), Some(own)) = (self.relative(&written), file.as_deref().and_then(|file| self.relative(file)))
        {
            // A link names its file in the project, or whole when it is
            // outside; only a file in the project can be named here.
            let inside = linked.iter().filter_map(|link| if Path::new(link).is_absolute() { self.relative(Path::new(link)) } else { Some(link.clone()) });
            let mut from = vec![own];
            for link in inside {
                if !from.contains(&link) {
                    from.push(link);
                }
            }
            // With an id, so a filing refused is said, at the bottom left.
            let id = format!("{WRITTEN_OUT}{}", self.next_file);
            self.next_file += 1;
            out.push(AppMessage::FileResource { tab, path, from, id: Some(id) });
        }
        // As ig-layer-export shows it, at the working area's bottom left.
        out.push(AppMessage::Notices {
            tab: self.images[index].id.clone(),
            notices: vec![protocol::Notice {
                id: String::from("written-out"),
                text: format!("Wrote {stem}.png. It points back to this image's {layer} layer."),
                detail: String::new(),
                progress: None,
                state: protocol::NoticeState::Done,
                actions: Vec::new(),
                close: Some(protocol::Action::new("notice_close")),
            }],
            corner: protocol::NoticeCorner::BottomLeft,
        });
    }

    /// `render`: inside a project a tab's files are filed, and nothing is
    /// declared as an item, so any item asked for is none of this image's.
    pub(super) fn render(&mut self, id: String) -> AppMessage {
        not_rendered(id, "That is not an item of this image.")
    }

    /// `preview`: a PNG, JPEG, or WebP file drawn small, as a reference
    /// row's thumbnail, written for the host as a raw image, as every
    /// thumbnail is. An image has no words for its corner.
    pub(super) fn preview(&mut self, id: String, path: &str, media_type: &str) -> AppMessage {
        let failed = |id: String, error: String| AppMessage::Previewed { id, thumbnail: None, badge: String::new(), cues: Vec::new(), error: Some(error) };
        if !IMAGE_TYPES.contains(&media_type) {
            return failed(id, "Only PNG, JPEG, and WebP images are previewed here.".into());
        }
        let root = self.project.clone().unwrap_or_default();
        let pixels = match files::import(&files::resolve(&root, path)) {
            Ok(pixels) => pixels,
            Err(reason) => return failed(id, reason),
        };
        let (width, height) = if pixels.width <= PREVIEW_SIZE[0] && pixels.height <= PREVIEW_SIZE[1] {
            (pixels.width, pixels.height)
        } else {
            fit_into(pixels.width, pixels.height, PREVIEW_SIZE)
        };
        let bytes = files::resample(pixels.width, pixels.height, &pixels.data, width, height);
        let file = self.cache.join(format!("preview-{}.rgba", protocol::fingerprint(path.as_bytes())));
        if let Err(reason) = files::write_atomic(&file, &bytes) {
            return failed(id, reason);
        }
        self.next_revision += 1;
        AppMessage::Previewed {
            id,
            thumbnail: Some(Thumbnail { path: file.display().to_string(), width, height, revision: self.next_revision }),
            badge: String::new(),
            cues: Vec::new(),
            error: None,
        }
    }

    /// The person's edit of a draft's prompt on the Prompt page, with its
    /// settings, endpoint, and model. One step of the strip's history. An
    /// edit to a step goes to the step, never here.
    pub(super) fn edit_cell(
        &mut self,
        cell: &str,
        input: Vec<CellPart>,
        settings: Option<Value>,
        endpoint: Option<String>,
        model: Option<String>,
        out: &mut Vec<AppMessage>,
    ) {
        let Some(index) = self.images.iter().position(|image| image.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let image = &mut self.images[index];
        let mut cells = image.gallery.cells.clone();
        let Some(own) = cells.iter_mut().find(|own| own.id == cell) else {
            return;
        };
        if own.step.is_some() {
            return;
        }
        own.input = input;
        if settings.is_some() {
            own.settings = settings;
        }
        if endpoint.is_some() {
            own.endpoint = endpoint;
        }
        if model.is_some() {
            own.model = model;
        }
        let current = image.gallery.cell.clone();
        if image.gallery.set_cells(cells, current) {
            image.dirty = true;
        }
        out.extend(self.emit());
    }

    /// A cell the host gives the tab, which then shows it, or, with none,
    /// an empty draft. One step of the strip's history. A cell the tab
    /// already has is not given twice.
    pub(super) fn insert_cell(&mut self, tab: &str, at: usize, cell: Option<Cell>, out: &mut Vec<AppMessage>) {
        let Some(index) = self.image_index(tab) else {
            return;
        };
        let image = &mut self.images[index];
        let shown = &image.gallery.frame().doc.image;
        let size = (shown.width, shown.height);
        let cell = files::stored_cell(&cell.unwrap_or_else(|| crate::gallery::empty_draft(size)));
        if image.gallery.cells.iter().any(|own| own.id == cell.id) {
            return;
        }
        let mut cells = image.gallery.cells.clone();
        cells.insert(at.min(cells.len()), cell.clone());
        if image.gallery.set_cells(cells, Some(cell.id)) {
            image.dirty = true;
        }
        out.extend(self.emit());
    }

    /// `cell_became_step`: the agent recorded a draft as a chain step, so
    /// the cell keeps only its pointer, and its draft goes, from the file
    /// too. The runner reads the step from `canon.sql`; nothing of it is
    /// kept here.
    pub(super) fn cell_became_step(&mut self, tab: &str, cell: &str, step: &protocol::StepRef, out: &mut Vec<AppMessage>) {
        let found = self
            .image_index(tab)
            .filter(|index| self.images[*index].gallery.cells.iter().any(|own| own.id == cell))
            .or_else(|| self.images.iter().position(|image| image.gallery.cells.iter().any(|own| own.id == cell)));
        let Some(index) = found else {
            return;
        };
        let image = &mut self.images[index];
        if image.gallery.point_at_step(cell, step) {
            image.dirty = true;
            out.extend(self.emit());
        }
    }

    /// Open report: the report of the take's round, which the host shows.
    pub(super) fn open_report(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let Some(image) = self.active_image() else {
            return;
        };
        let Some(tile) = image.rounds.tile_of(path).map(|tile| &image.rounds.tiles[tile]) else {
            return;
        };
        out.push(AppMessage::OpenReport { request: tile.request.clone() });
    }

    /// A run of a tab's cell the host ran itself, as it still does with no
    /// project open: each image it made joins the gallery, linked to its
    /// file. In a project the runner runs cells, and rounds come with
    /// `takes` instead.
    pub(super) fn cell_result(&mut self, cell: &str, output: &[CellPart], out: &mut Vec<AppMessage>) {
        if self.project.is_some() {
            return;
        }
        let Some(index) = self.images.iter().position(|image| image.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let root = self.project.clone().unwrap_or_default();
        let mut added = false;
        for part in output {
            let CellPart::Path { path, media_type, .. } = part else {
                continue;
            };
            if !IMAGE_TYPES.contains(&media_type.as_str()) {
                continue;
            }
            let file = files::resolve(&root, path);
            match files::import_linked(&file) {
                Ok((pixels, sha256)) => {
                    let title = file_title(&file);
                    let doc = Doc::new(LayeredImage::linked(&title, pixels, self.link_to(&file, sha256)));
                    self.film_of(index, |gallery| gallery.add(&title, doc, None));
                    added = true;
                }
                Err(reason) => out.push(notice("This image could not be opened", &reason)),
            }
        }
        if added && index == self.active {
            out.extend(self.emit());
        }
    }
}

fn not_rendered(id: String, error: &str) -> AppMessage {
    AppMessage::Rendered { id, path: None, fingerprint: None, error: Some(error.to_string()) }
}

/// A layer of pixels, as written out: the image's size, the layer's pixels
/// where they sit, and clear elsewhere, with the layer's name.
pub(super) fn layer_canvas(doc: &Doc, node: u64) -> Result<(String, Vec<u8>), String> {
    let layered = &doc.image;
    let Some(found) = layered.find(node) else {
        return Err("That layer is no longer in the image.".into());
    };
    let Some(pixels) = layered.pixels.get(&node).filter(|_| matches!(found.kind, Kind::Pixels) && !found.is_group()) else {
        return Err("Only a layer of pixels is written out as PNG.".into());
    };
    let (width, height) = (layered.width as i64, layered.height as i64);
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    let [dx, dy] = [i64::from(found.offset[0]), i64::from(found.offset[1])];
    let (x0, x1) = (dx.max(0), (dx + i64::from(pixels.width)).min(width));
    if x0 < x1 {
        for y in 0..i64::from(pixels.height) {
            let ty = y + dy;
            if ty < 0 || ty >= height {
                continue;
            }
            let from = ((y * i64::from(pixels.width) + (x0 - dx)) * 4) as usize;
            let to = ((ty * width + x0) * 4) as usize;
            let length = ((x1 - x0) * 4) as usize;
            canvas[to..to + length].copy_from_slice(&pixels.data[from..from + length]);
        }
    }
    Ok((found.name.clone(), canvas))
}

/// The files a layer was made from, as their links name them: its own
/// linked file, when it is linked, then those of the linked layers beneath
/// it, lowest first, each once. Beneath it are the layers before it as the
/// image combines them, its siblings beneath it and everything beneath its
/// groups. A layer painted on still names what it was edited from.
fn linked_from(doc: &Doc, node: u64) -> Vec<String> {
    let nodes = doc.image.every_node();
    let Some(at) = nodes.iter().position(|own| own.id == node) else {
        return Vec::new();
    };
    let mut files: Vec<String> = Vec::new();
    for linked in std::iter::once(nodes[at]).chain(nodes[..at].iter().copied()).filter_map(|own| own.link.as_ref()) {
        if !files.contains(&linked.link.path) {
            files.push(linked.link.path.clone());
        }
    }
    files
}

/// A layer written out: its name, and its PNG.
pub(super) fn layer_png(doc: &Doc, node: u64) -> Result<(String, Vec<u8>), String> {
    let (name, canvas) = layer_canvas(doc, node)?;
    Ok((name, files::encode_png(doc.image.width, doc.image.height, &canvas)?))
}

/// A reference's media type, by its file's extension.
fn media_type_of(path: &str) -> String {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// A take's frame let go: its image and thumbnails written for the host go
/// too, as they are in memory.
pub(super) fn forget_files(cache: &Path, tab: &str, uid: u64) {
    let (whole, part) = (format!("{tab}-{uid}."), format!("{tab}-{uid}-"));
    for entry in std::fs::read_dir(cache).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&whole) || name.starts_with(&part) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
