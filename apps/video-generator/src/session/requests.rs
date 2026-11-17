//! A tab's requests, as the Image Generator's are: its cells on the Prompt
//! page, the rounds the host says they made, the take shown, a take's menu,
//! and the tab's items and their renders.

use super::*;
use protocol::{Cell, CellPart, Item, ItemRef, Round, StepRef};

impl Session {
    /// A new draft's size: the gallery's first take's, or its rounds'
    /// takes', or 1280 × 720.
    fn draft_size(&self, index: usize) -> (u32, u32) {
        let Some(tab) = self.tabs.get(index) else {
            return crate::gallery::SIZE;
        };
        tab.gallery.takes.iter().find_map(|take| take.info).map(|info| (info.width, info.height)).or_else(|| tab.rounds.shape()).unwrap_or(crate::gallery::SIZE)
    }

    /// The cells of every tab whose cells the host has not been told, each
    /// tab with a cell for its Prompt page, an empty draft when it had none.
    pub(super) fn cells_due(&mut self) -> Vec<AppMessage> {
        let mut out = Vec::new();
        for index in 0..self.tabs.len() {
            let size = self.draft_size(index);
            let tab = &mut self.tabs[index];
            tab.gallery.ensure_cell(size);
            let now = (tab.gallery.cells.clone(), tab.gallery.cell.clone());
            if tab.cells_sent.as_ref() != Some(&now) {
                out.push(AppMessage::Cells { tab: tab.id.clone(), cells: now.0.clone(), selected: now.1.clone() });
                tab.cells_sent = Some(now);
            }
        }
        out
    }

    /// Every round a tab's cell has made, as `takes` sends them: the strip
    /// is laid out again, each take read from its file, and nothing of
    /// them is kept in the tab's file.
    pub(super) fn takes(&mut self, cell: &str, rounds: Vec<Round>, out: &mut Vec<AppMessage>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let root = self.project.clone().unwrap_or_default();
        let tab = &mut self.tabs[index];
        // The artifact the cell's step publishes, as its newest round says
        // it, while the program runs.
        if let Some(artifact) = rounds.iter().rev().find_map(|round| round.artifact.clone()) {
            tab.artifacts.insert(cell.to_string(), artifact);
        }
        let order: Vec<String> = tab.gallery.cells.iter().map(|own| own.id.clone()).collect();
        tab.rounds.set(cell, rounds, &order);
        tab.rounds.read_files(&root);
        if index == self.active {
            out.extend(self.emit());
        }
        out.extend(self.items_due());
    }

    /// A click on a take's tile shows it alone, as any take is shown.
    /// Which take of a round is shown is no change to the file.
    pub(super) fn select_take(&mut self, tile: usize) {
        self.stop_playing();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.rounds.tiles.get(tile).is_some_and(|chosen| chosen.file.is_some()) {
            tab.rounds.show(Some(tile));
        }
    }

    /// Next round from this: a draft of the take's round, with its prompt,
    /// its references in order with their roles, and the take itself as a
    /// Reference; when the take has notes, their words with their times
    /// end the prompt, and the marked frame is sent too, as an item of the
    /// tab's file. A draft not yet run takes it in place; otherwise it is
    /// a new cell beside the one it came from. One step of the gallery's
    /// history; the Prompt page is shown.
    pub(super) fn next_round(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let size = self.draft_size(index);
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let Some(tile) = tab.rounds.tile_of(path) else {
            return;
        };
        let Some(round) = tab.rounds.round_of(&tab.rounds.tiles[tile]).cloned() else {
            return;
        };
        let Some(take) = tab.rounds.tiles[tile].file.clone() else {
            return;
        };
        // The take's notes, kept by its file: their words, each with its
        // time, end the prompt, and the frame at the first note's time,
        // marked, is sent too.
        let read = tab.rounds.read(&take.path).cloned();
        let notes = tab.gallery.notes_of(&take.path, read.as_ref().map_or("", |read| read.sha256.as_str()));
        let words = notes.as_timed_words();
        let mut prompt = round.prompt.trim_end().to_string();
        if !words.is_empty() {
            let said = format!("{NOTES_WORDS}: {}", words.join(" "));
            prompt = if prompt.is_empty() { said } else { format!("{prompt} {said}") };
        }
        let mut input = vec![CellPart::Text { text: prompt }];
        for reference in &round.references {
            input.push(CellPart::Path { path: reference.path.clone(), media_type: media_type_of(&reference.path), role: reference.role.clone(), field: None });
        }
        input.push(CellPart::Path { path: take.path.clone(), media_type: take.media_type.clone(), role: Some(String::from("Reference")), field: None });
        if !notes.is_empty()
            && let Some(read) = read
            && let Some(info) = read.info
        {
            match self.file_marked(index, &read.file, info, &notes) {
                Ok(item) => input.push(CellPart::Item { item, media_type: String::from("image/png"), part: None, role: Some(NOTES_WORDS.into()), field: None }),
                Err(reason) => out.push(notice("The image could not be marked", &reason)),
            }
        }
        let tab = &mut self.tabs[index];
        let current = tab.gallery.current_cell().cloned();
        let from = current.as_ref().filter(|cell| cell.step.is_none());
        let mut draft = match from.filter(|cell| !tab.rounds.has_rounds(&cell.id)) {
            Some(cell) => cell.clone(),
            None => crate::gallery::empty_draft(size),
        };
        draft.input = input;
        draft.model = Some(round.model.clone()).filter(|model| !model.is_empty()).or_else(|| from.and_then(|cell| cell.model.clone()));
        draft.settings = from.and_then(|cell| cell.settings.clone()).or_else(|| Some(crate::gallery::draft_settings(size)));
        draft.endpoint = from.and_then(|cell| cell.endpoint.clone());
        let mut cells = tab.gallery.cells.clone();
        match cells.iter_mut().find(|cell| cell.id == draft.id) {
            Some(cell) => *cell = draft.clone(),
            None => cells.push(draft.clone()),
        }
        if tab.gallery.set_cells(cells, Some(draft.id)) {
            tab.dirty = true;
        }
        out.extend(self.emit());
        out.extend(self.items_due());
    }

    /// The marked frame of a next round, as an item of the tab's file: the
    /// take's frame at its first note's time, at the take's own size, with
    /// the notes of that moment on it, as the view shows them but with no
    /// card. Its PNG is rendered now under `protocol::render_path`, named
    /// by its fingerprint, so `render` finds it, and the tab's file is
    /// written first, so the item has a file to be of.
    fn file_marked(&mut self, index: usize, file: &Path, info: media::Info, notes: &layers::notes::Notes) -> Result<ItemRef, String> {
        let Some(project) = self.project.clone() else {
            return Err("The video is not in a project.".into());
        };
        let time = notes.first_time().unwrap_or(0.0);
        let frame = ((time.max(0.0) * info.fps()).round() as u32).min(info.frames.saturating_sub(1));
        let mut video = media::Video::open(file).map_err(|error| error.0)?;
        let decoded = video.frame(frame, crate::decode::even((info.width, info.height))).map_err(|error| error.0)?;
        let mut marked = layers::Rgba { width: decoded.width, height: decoded.height, data: decoded.rgba };
        crate::notes::marked(&mut marked, &notes.at_time(time), (info.width, info.height));
        if self.tabs[index].path.is_none() {
            self.tabs[index].dirty = true;
        }
        let path = self.write_into_project(index)?.ok_or_else(|| String::from("The video could not be written."))?;
        let relative = self.relative(&path).ok_or_else(|| String::from("The video is not in the project."))?;
        let png = files::encode_png(marked.width, marked.height, &marked.data)?;
        let fingerprint = protocol::fingerprint(&png);
        let id = format!("{MARKED_ITEM}{fingerprint}");
        let item = ItemRef { path: relative, item: Some(id.clone()) };
        files::write_atomic(&project.join(protocol::render_path(OWNER, &item, None, &fingerprint, "png")), &png)?;
        let tab = &mut self.tabs[index];
        if !tab.marked.iter().any(|(own, _)| *own == id) {
            tab.marked.push((id, fingerprint));
        }
        Ok(item)
    }

    /// Retime… or Upscale… on a round's take: always a new draft cell of
    /// kind video for the finishing request, the take its one reference,
    /// shown on the Prompt page, one step of the gallery's history; then
    /// `run_cell`, as the Prompt page's Generate would send it, so the
    /// runner's plan card asks the person's yes with its cost. Nothing else
    /// is sent, and nothing is paid without that yes.
    pub(super) fn finish_take(&mut self, path: &str, finish: crate::finishing::Finish, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let Some(file) = tab.rounds.tile_of(path).and_then(|tile| tab.rounds.tiles[tile].file.clone()) else {
            return;
        };
        let size = tab.rounds.read(path).and_then(|read| read.info).map(|info| (info.width, info.height)).or_else(|| tab.rounds.shape()).unwrap_or(crate::gallery::SIZE);
        let draft = crate::finishing::draft(finish, path, &file.media_type, size.0, size);
        let mut cells = tab.gallery.cells.clone();
        cells.push(draft.clone());
        if tab.gallery.set_cells(cells, Some(draft.id.clone())) {
            tab.dirty = true;
        }
        let id = tab.id.clone();
        self.page = super::Page::Prompt;
        out.extend(self.emit());
        out.push(AppMessage::RunCell { tab: id, cell: draft.id, content: None, at: None });
    }

    /// Compare with the source: a finished take beside the take it was made
    /// from, as a Ctrl-click shows two, the source on the left, both from
    /// one time; at 100% the before/after crop. A view only.
    pub(super) fn compare_take(&mut self, path: &str) {
        self.stop_playing();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let source = tab
            .rounds
            .tile_of(path)
            .and_then(|tile| tab.rounds.round_of(&tab.rounds.tiles[tile]))
            .and_then(crate::finishing::source)
            .and_then(|source| tab.rounds.tile_of(source));
        if let Some(source) = source {
            tab.rounds.shown = Some(source);
            tab.rounds.with = crate::rounds::With::Tile(path.to_string());
        }
    }

    /// Ask the assistant to use this take: a prepared request naming the
    /// take, put in the conversation's composer for the person to send, as
    /// a take enters the chain only through the assistant.
    pub(super) fn ask_about(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(tile) = tab.rounds.tile_of(path).map(|tile| &tab.rounds.tiles[tile]) else {
            return;
        };
        let text = format!("Put take {} of round {}, {path}, into the chain.", tile.take + 1, tile.number);
        out.push(AppMessage::AskAssistant { text, items: vec![ItemRef { path: path.to_string(), item: None }] });
    }

    /// Edit: the take added to the gallery as a take of its own, linking
    /// its file, and shown.
    pub(super) fn edit_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let root = self.project.clone().unwrap_or_default();
        let file = if Path::new(path).is_absolute() { PathBuf::from(path) } else { root.join(path) };
        match self.add_take(&file, None) {
            Ok(_) => {}
            Err(reason) => out.push(notice("This video could not be opened", &reason)),
        }
    }

    /// Open report: the report of the take's round, which the host shows.
    pub(super) fn open_report(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let Some(tile) = tab.rounds.tile_of(path).map(|tile| &tab.rounds.tiles[tile]) else {
            return;
        };
        out.push(AppMessage::OpenReport { request: tile.request.clone() });
    }

    /// The items of each tab's file the host has not been told: its takes.
    /// Only a tab with a file in the project has any.
    pub(super) fn items_due(&mut self) -> Vec<AppMessage> {
        if self.project.is_none() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for index in 0..self.tabs.len() {
            let Some(path) = self.tabs[index].path.clone().and_then(|path| self.relative(&path)) else {
                continue;
            };
            let tab = &mut self.tabs[index];
            let mut items = tab.rounds.items();
            for (id, fingerprint) in &tab.marked {
                items.push(Item { id: id.clone(), name: NOTES_WORDS.into(), media_types: vec![String::from("image/png")], fingerprint: fingerprint.clone(), made: None });
            }
            for (id, fingerprint) in &tab.frames_taken {
                items.push(Item { id: id.clone(), name: String::from("Start frame"), media_types: vec![String::from("image/png")], fingerprint: fingerprint.clone(), made: None });
            }
            let now: (String, Vec<Item>) = (path, items);
            if tab.items_sent.as_ref() != Some(&now) {
                out.push(AppMessage::Items { path: now.0.clone(), items: now.1.clone() });
                tab.items_sent = Some(now);
            }
        }
        out
    }

    /// A tab's file read again: each marked image and each Start frame
    /// taken from the animatic that one of its cells still names, whose
    /// render is still in the project under `protocol::render_path`, is
    /// declared again with `items`, so what a draft names is listed after
    /// a restart.
    pub(super) fn restore_items(&mut self, index: usize) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(relative) = self.tabs.get(index).and_then(|tab| tab.path.clone()).and_then(|path| self.relative(&path)) else {
            return;
        };
        let tab = &mut self.tabs[index];
        let named: Vec<ItemRef> = tab
            .gallery
            .cells
            .iter()
            .flat_map(|cell| cell.input.iter())
            .filter_map(|part| match part {
                CellPart::Item { item, .. } if item.path == relative => Some(item.clone()),
                _ => None,
            })
            .collect();
        for item in named {
            let Some(id) = item.item.as_deref() else {
                continue;
            };
            let found = id.strip_prefix(MARKED_ITEM).map(|fingerprint| (true, fingerprint)).or_else(|| id.strip_prefix(ANIMATIC_ITEM).map(|fingerprint| (false, fingerprint)));
            let Some((marked, fingerprint)) = found else {
                continue;
            };
            if !project.join(protocol::render_path(OWNER, &item, None, fingerprint, "png")).is_file() {
                continue;
            }
            let list = if marked { &mut tab.marked } else { &mut tab.frames_taken };
            if !list.iter().any(|(own, _)| own == id) {
                list.push((id.to_string(), fingerprint.to_string()));
            }
        }
    }

    /// `render`: a take is its own file, as it was made; a marked frame
    /// was rendered when its next round was made, and a Start frame taken
    /// from the animatic when it was taken.
    pub(super) fn render(&mut self, id: String, item: &ItemRef, media_type: &str) -> AppMessage {
        let failed = |id: String, error: &str| AppMessage::Rendered { id, path: None, fingerprint: None, error: Some(error.to_string()) };
        let found = (0..self.tabs.len())
            .find(|index| self.tabs[*index].path.as_deref().and_then(|path| self.relative(path)).as_deref() == Some(item.path.as_str()));
        let Some(index) = found else {
            return failed(id, "That video is not open.");
        };
        let own = item.item.as_deref().unwrap_or("");
        if let Some(fingerprint) = own.strip_prefix(MARKED_ITEM) {
            let relative = protocol::render_path(OWNER, item, None, fingerprint, "png");
            let kept = self.project.as_ref().is_some_and(|project| project.join(&relative).is_file());
            return match (media_type, kept) {
                ("image/png", true) => AppMessage::Rendered { id, path: Some(relative), fingerprint: Some(fingerprint.to_string()), error: None },
                ("image/png", false) => failed(id, "The marked image is gone. Make the next round from the take again."),
                _ => failed(id, "A marked image is rendered only as PNG."),
            };
        }
        if let Some(fingerprint) = own.strip_prefix(ANIMATIC_ITEM) {
            let relative = protocol::render_path(OWNER, item, None, fingerprint, "png");
            let kept = self.project.as_ref().is_some_and(|project| project.join(&relative).is_file());
            return match (media_type, kept) {
                ("image/png", true) => AppMessage::Rendered { id, path: Some(relative), fingerprint: Some(fingerprint.to_string()), error: None },
                ("image/png", false) => failed(id, "The Start frame is gone. Take it from the animatic again."),
                _ => failed(id, "A Start frame is rendered only as PNG."),
            };
        }
        match self.tabs[index].rounds.take_by_item(own) {
            Some((take, fingerprint)) if take.media_type == media_type || media_type.starts_with("video/") => {
                AppMessage::Rendered { id, path: Some(take.path), fingerprint: Some(fingerprint), error: None }
            }
            Some(_) => failed(id, "A take is rendered only as its own file."),
            None => failed(id, "That is not an item of this video."),
        }
    }

    /// The person's edit of a draft's prompt on the Prompt page, with its
    /// settings, endpoint, and model. One step of the gallery's history. An
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
        let Some(index) = self.tabs.iter().position(|tab| tab.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let tab = &mut self.tabs[index];
        let mut cells = tab.gallery.cells.clone();
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
        let current = tab.gallery.cell.clone();
        if tab.gallery.set_cells(cells, current) {
            tab.dirty = true;
        }
        out.extend(self.emit());
    }

    /// A cell the host gives the tab, which then shows it, or, with none,
    /// an empty draft. One step. A cell the tab already has is not given
    /// twice.
    pub(super) fn insert_cell(&mut self, tab: &str, at: usize, cell: Option<Cell>, out: &mut Vec<AppMessage>) {
        let Some(index) = self.tab_index(tab) else {
            return;
        };
        let size = self.draft_size(index);
        let tab = &mut self.tabs[index];
        let cell = layers::cells::stored_cell(&cell.unwrap_or_else(|| crate::gallery::empty_draft(size)));
        if tab.gallery.cells.iter().any(|own| own.id == cell.id) {
            return;
        }
        let mut cells = tab.gallery.cells.clone();
        cells.insert(at.min(cells.len()), cell.clone());
        if tab.gallery.set_cells(cells, Some(cell.id)) {
            tab.dirty = true;
        }
        out.extend(self.emit());
    }

    /// The cell the person chose to show on the Prompt page. Not a step.
    pub(super) fn select_cell(&mut self, tab: &str, cell: String, out: &mut Vec<AppMessage>) {
        if let Some(index) = self.tab_index(tab)
            && self.tabs[index].gallery.cells.iter().any(|own| own.id == cell)
        {
            self.tabs[index].gallery.cell = Some(cell);
            out.extend(self.emit());
        }
    }

    /// `cell_became_step`: the agent recorded a draft as a chain step, so
    /// the cell keeps only its pointer, and its draft goes, from the file
    /// too. The artifact it publishes is kept while the program runs, never
    /// in the file.
    pub(super) fn cell_became_step(&mut self, tab: &str, cell: &str, step: &StepRef, artifact: Option<String>, out: &mut Vec<AppMessage>) {
        let found = self
            .tab_index(tab)
            .filter(|index| self.tabs[*index].gallery.cells.iter().any(|own| own.id == cell))
            .or_else(|| self.tabs.iter().position(|tab| tab.gallery.cells.iter().any(|own| own.id == cell)));
        let Some(index) = found else {
            return;
        };
        let tab = &mut self.tabs[index];
        if let Some(artifact) = artifact {
            tab.artifacts.insert(cell.to_string(), artifact);
        }
        if tab.gallery.point_at_step(cell, step) {
            tab.dirty = true;
            out.extend(self.emit());
        }
    }

    /// A run of a tab's cell outside a Skills project, where the host runs
    /// cells itself: each video it made joins the gallery, linking its
    /// file. In a Skills project, rounds come with `takes` instead.
    pub(super) fn cell_result(&mut self, cell: &str, output: &[CellPart], out: &mut Vec<AppMessage>) {
        if self.skills {
            return;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.gallery.cells.iter().any(|own| own.id == cell)) else {
            return;
        };
        let root = self.project.clone().unwrap_or_default();
        let active = self.active;
        self.active = index;
        let mut added = false;
        for part in output {
            let CellPart::Path { path, media_type, .. } = part else {
                continue;
            };
            if !media_type.starts_with("video/") {
                continue;
            }
            let file = if Path::new(path).is_absolute() { PathBuf::from(path) } else { root.join(path) };
            match self.add_take(&file, None) {
                Ok(_) => added = true,
                Err(reason) => out.push(notice("This video could not be opened", &reason)),
            }
        }
        self.active = active;
        if added && index == self.active {
            out.extend(self.emit());
        }
    }

    /// A path inside the project, as the project names it, with `/`
    /// between folders.
    pub(super) fn relative(&self, path: &Path) -> Option<String> {
        let project = self.project.as_ref()?;
        let whole = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let inside = whole.strip_prefix(project).ok()?;
        Some(inside.to_string_lossy().replace('\\', "/"))
    }
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
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
    .to_string()
}
