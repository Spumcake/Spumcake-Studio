//! A tab's requests, as the Image Generator's are: its cells on the Prompt
//! page, the rounds the host says they made, the take shown, and a take's
//! menu.

use super::*;
use protocol::{Cell, CellPart, ItemRef, Round, StepRef};

/// What "Ask the assistant to fix the dependency chain" puts in the
/// composer, as the Image Generator's.
pub(super) const FIX: &str = "Fix the dependency chain.";

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
            let mut cells = tab.gallery.cells.clone();
            // A draft that has never run says the file of the gallery's take
            // shown as its output, so the host finds that take's round; a
            // round's take shown has its round already. Never kept, and
            // never in place of a real run's output or on a step.
            let opened = if tab.rounds.shown.is_none() { tab.gallery.take().map(|take| take.link.path.clone()) } else { None };
            if let Some(path) = opened.map(|path| as_in_project(self.project.as_deref(), &path)) {
                for cell in cells.iter_mut().filter(|cell| cell.step.is_none() && cell.versions.is_empty() && cell.output.is_empty()) {
                    cell.output = vec![CellPart::Path { media_type: media_type_of(&path), path: path.clone(), role: None, field: None }];
                }
            }
            let now = (cells, tab.gallery.cell.clone());
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
        // The marked frame, drawn now and filed; it joins the draft when
        // `filed` comes.
        let marked = match (notes.is_empty(), read.as_ref().and_then(|read| read.info.map(|info| (read.file.clone(), info)))) {
            (false, Some((file, info))) => match marked_png(&file, info, &notes) {
                Ok(png) => Some(png),
                Err(reason) => {
                    out.push(notice("The image could not be marked", &reason));
                    None
                }
            },
            _ => None,
        };
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
        let made = draft.input.clone();
        let cell = draft.id.clone();
        if tab.gallery.set_cells(cells, Some(draft.id)) {
            tab.dirty = true;
        }
        if let Some(png) = marked {
            let stem = format!("{} notes", self.tabs[index].label);
            let becomes = super::filing::Becomes::Notes { made };
            if let Err(reason) = self.file_for_draft(index, &stem, &png, &take.path, &cell, becomes, out) {
                out.push(notice("The image could not be marked", &reason));
            }
        }
        out.extend(self.emit());
    }

    /// Retime… or Upscale… on a round's take: always a new draft cell of
    /// kind video for the finishing request, the take its one reference,
    /// shown on the Prompt page, one step of the gallery's history; then
    /// `run_cell`, as the Prompt page's Generate would send it, so the
    /// runner's plan card asks the person's yes with its cost. Nothing else
    /// is sent, and nothing is paid without that yes.
    pub(super) fn finish_take(&mut self, path: &str, finish: crate::finishing::Finish, out: &mut Vec<AppMessage>) {
        // Only a project's runner sends a finishing request.
        if self.project.is_none() {
            return;
        }
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
    /// one time, at 100%, so it is the before/after crop in one action. A
    /// view only.
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
            tab.zoom = Some(1.0);
        }
    }

    /// Use this take: `use_take` for the take at `path`, with the cell's
    /// item as `takes` names it, the tab's file and the cell, beside the
    /// takes in use, replacing none, so it asks nothing first. The tab's
    /// file is written first when it has none, so the item has a file.
    pub(super) fn use_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        if let Some((tab, cell)) = self.asked_of(path, out) {
            out.push(AppMessage::UseTake { tab, cell, path: path.to_string() });
        }
    }

    /// Ignore: `ignore_take` for the out-of-date take at `path`, which
    /// clears its outline and no other's.
    pub(super) fn ignore_take(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        if let Some((tab, cell)) = self.asked_of(path, out) {
            out.push(AppMessage::IgnoreTake { tab, cell, path: path.to_string() });
        }
    }

    /// "Ask the assistant to fix the dependency chain": the fix put in the
    /// composer with the take's cell, for the person to send.
    pub(super) fn ask_to_fix(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        if let Some((_, cell)) = self.asked_of(path, out) {
            out.push(AppMessage::AskAssistant { text: FIX.into(), items: vec![cell] });
        }
    }

    /// A take's stack badge: its iterations opened as a run after its
    /// round, or closed.
    pub(super) fn toggle_stack(&mut self, path: &str) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let order: Vec<String> = tab.gallery.cells.iter().map(|own| own.id.clone()).collect();
        tab.rounds.toggle_stack(path, &order);
        // The take itself and its earlier iterations are read once shown.
        let root = self.project.clone().unwrap_or_default();
        tab.rounds.read_files(&root);
    }

    /// The tab and the cell's item as `takes` names it, for the take at
    /// `path` or any iteration of its asset, the tab's file written first
    /// when it has none, so the item has a file.
    fn asked_of(&mut self, path: &str, out: &mut Vec<AppMessage>) -> Option<(String, ItemRef)> {
        let index = self.active;
        let tab = self.tabs.get(index)?;
        let tile = tab.rounds.tile_of(path).or_else(|| tab.rounds.asset_of(path))?;
        let cell = tab.rounds.tiles[tile].cell.clone();
        if self.tabs[index].path.is_none() {
            self.tabs[index].dirty = true;
        }
        let file = match self.write_into_project(index) {
            Ok(Some(file)) => file,
            Ok(None) => return None,
            Err(reason) => {
                out.push(super::deliver::notices(&self.tabs[index].id, Some((reason, protocol::NoticeState::Failed))));
                return None;
            }
        };
        let own = self.relative(&file)?;
        Some((self.tabs[index].id.clone(), ItemRef { path: own, item: Some(cell) }))
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

    /// `render`: inside a project a tab's files are filed, and nothing is
    /// declared as an item, so any item asked for is none of this video's.
    pub(super) fn render(&mut self, id: String) -> AppMessage {
        AppMessage::Rendered { id, path: None, fingerprint: None, error: Some(String::from("That is not an item of this video.")) }
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

    /// A run of a tab's cell the host ran itself, as it still does with
    /// no project open: each video it made joins the gallery, linking its
    /// file. In a project the runner runs cells, and rounds come with
    /// `takes` instead.
    pub(super) fn cell_result(&mut self, cell: &str, output: &[CellPart], out: &mut Vec<AppMessage>) {
        if self.project.is_some() {
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

/// A next round's marked frame as a PNG: the take's frame at its first
/// note's time, at the take's own size, with the notes of that moment on
/// it, as the view shows them but with no card.
fn marked_png(file: &Path, info: media::Info, notes: &layers::notes::Notes) -> Result<Vec<u8>, String> {
    let time = notes.first_time().unwrap_or(0.0);
    let frame = ((time.max(0.0) * info.fps()).round() as u32).min(info.frames.saturating_sub(1));
    let mut video = media::Video::open(file).map_err(|error| error.0)?;
    let decoded = video.frame(frame, crate::decode::even((info.width, info.height))).map_err(|error| error.0)?;
    let mut marked = layers::Rgba { width: decoded.width, height: decoded.height, data: decoded.rgba };
    crate::notes::marked(&mut marked, &notes.at_time(time), (info.width, info.height));
    files::encode_png(marked.width, marked.height, &marked.data)
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

/// A file's path as a project's reports name it: relative to the project
/// when it is inside it, with `/` between folders, or else as it is.
fn as_in_project(project: Option<&Path>, path: &str) -> String {
    let whole = Path::new(path);
    match project.filter(|_| whole.is_absolute()).and_then(|project| whole.strip_prefix(project).ok()) {
        Some(inside) => inside.to_string_lossy().replace('\\', "/"),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    /// Catches a draft's output named otherwise than the project's reports
    /// name it: a file inside the project, linked by its whole path, is
    /// named relative to the project; one outside it, or already relative,
    /// as it is.
    #[test]
    fn a_linked_file_is_named_as_the_reports_name_it() {
        let project = Path::new("/home/someone/Last order");
        assert_eq!(as_in_project(Some(project), "/home/someone/Last order/library/a.png"), "library/a.png");
        assert_eq!(as_in_project(Some(project), "library/a.png"), "library/a.png");
        assert_eq!(as_in_project(Some(project), "/elsewhere/a.png"), "/elsewhere/a.png");
        assert_eq!(as_in_project(None, "/home/someone/Last order/library/a.png"), "/home/someone/Last order/library/a.png");
    }
}
