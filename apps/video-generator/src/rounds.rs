//! A tab's rounds: what each of its cells' requests made, as the host
//! sends them with `takes`, and never kept in the `.vgx`, as the Image
//! Generator's rounds are. Each round is a run of the filmstrip: its takes,
//! each read through `media` when it is new or changed on disk, and a place
//! for each take still being made, or one for a round that failed. A take
//! shown in the working area is drawn as any take of the gallery is.

use protocol::{FilmPending, Round, RoundState, TakeFile};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What a take of a round can be, as this workspace shows them.
pub const TAKE_TYPES: [&str; 3] = ["video/mp4", "video/quicktime", "video/webm"];

/// The roles a reference can have, as the Prompt page's menu offers them,
/// the guide's words for 0.15, in vg-request-inputs' order.
pub const ROLES: [&str; 5] = ["Start frame", "End frame", "Reference", "Motion reference", "Sound"];

/// The roles shown as their own rows with "Choose…" until filled.
pub const SLOTS: [&str; 2] = ["Start frame", "End frame"];

/// What "Add reference" offers: images, videos, and sounds.
pub const REFERENCE_TYPES: [&str; 8] =
    ["image/png", "image/jpeg", "image/webp", "video/mp4", "video/quicktime", "audio/wav", "audio/mpeg", "audio/flac"];

/// One tile of the filmstrip a round made: a take, or a place for one.
#[derive(Debug, Clone)]
pub struct Tile {
    pub cell: String,
    pub request: String,
    /// The round's number among its cell's rounds.
    pub number: u32,
    /// The take's place in its round, from 0.
    pub take: usize,
    /// Its file, or none for a take still being made or a round that failed.
    pub file: Option<TakeFile>,
    pub pending: Option<FilmPending>,
}

impl Tile {
    pub fn path(&self) -> Option<&str> {
        self.file.as_ref().map(|file| file.path.as_str())
    }
}

/// A take's file as the strip shows it: read once, and again when it
/// changes on disk.
#[derive(Debug, Clone)]
pub struct Read {
    stamp: Option<(u64, std::time::SystemTime)>,
    /// The file itself, as an absolute path.
    pub file: PathBuf,
    /// What `media` says it is, when it opened.
    pub info: Option<media::Info>,
    /// The sha256 of its bytes, by which its notes are kept.
    pub sha256: String,
}

/// What a round's take shown is shown with: alone; beside another take,
/// of this round or another, by its path, or of the gallery, by its uid,
/// on the left with `first`; or its round whole as frame strips, as
/// vg-frame-strips and vg-takes-compare show. A view of the working area,
/// never a step of the gallery's history, and never kept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum With {
    #[default]
    Alone,
    Tile(String),
    Own { uid: u64, first: bool },
    Strips,
}

#[derive(Debug, Default)]
pub struct Rounds {
    /// Each cell's rounds, oldest first, as the host last sent them, in
    /// the order of the tab's cells.
    by_cell: Vec<(String, Vec<Round>)>,
    /// The strip's tiles: every round's in turn.
    pub tiles: Vec<Tile>,
    /// The tile shown in the working area in place of a take of the
    /// gallery.
    pub shown: Option<usize>,
    /// What the take shown is shown with.
    pub with: With,
    reads: HashMap<String, Read>,
}

impl Rounds {
    /// Replaces one cell's rounds, as `takes` sends them, and lays the
    /// strip out again: the cells' rounds in the order of `cells`, each
    /// cell's oldest first. A take stays shown while its file is still
    /// among them.
    pub fn set(&mut self, cell: &str, rounds: Vec<Round>, cells: &[String]) {
        match self.by_cell.iter_mut().find(|(id, _)| id == cell) {
            Some((_, kept)) => *kept = rounds,
            None => self.by_cell.push((cell.to_string(), rounds)),
        }
        let place = |id: &str| cells.iter().position(|cell| cell == id).unwrap_or(usize::MAX);
        self.by_cell.sort_by_key(|(id, _)| place(id.as_str()));
        let shown = self.shown.and_then(|index| self.tiles.get(index)).and_then(|tile| tile.path().map(str::to_string));
        let mut tiles = Vec::new();
        for (cell, rounds) in &self.by_cell {
            for round in rounds {
                let videos: Vec<&TakeFile> = round.takes.iter().filter(|take| TAKE_TYPES.contains(&take.media_type.as_str())).collect();
                // A round being made shows a place for each take still to
                // come; one that failed with nothing shows one.
                let waiting = match round.state {
                    RoundState::Running | RoundState::Pending => {
                        (round.count.unwrap_or(0) as usize).saturating_sub(videos.len()).max(usize::from(videos.is_empty()))
                    }
                    RoundState::Failed => usize::from(videos.is_empty()),
                    RoundState::Done | RoundState::Cancelled => 0,
                };
                let pending = match round.state {
                    RoundState::Failed => FilmPending::Failed,
                    _ => FilmPending::Running { since: round.started },
                };
                for (take, file) in videos.iter().enumerate() {
                    tiles.push(Tile {
                        cell: cell.clone(),
                        request: round.request.clone(),
                        number: round.number,
                        take,
                        file: Some((*file).clone()),
                        pending: None,
                    });
                }
                for more in 0..waiting {
                    tiles.push(Tile {
                        cell: cell.clone(),
                        request: round.request.clone(),
                        number: round.number,
                        take: videos.len() + more,
                        file: None,
                        pending: Some(pending.clone()),
                    });
                }
            }
        }
        self.tiles = tiles;
        self.shown = shown.and_then(|path| self.tiles.iter().position(|tile| tile.path() == Some(path.as_str())));
        // A take beside it stays while its file is still among them.
        let gone = match &self.with {
            With::Tile(path) => self.tile_of(path).is_none(),
            _ => false,
        };
        if self.shown.is_none() || gone {
            self.with = With::Alone;
        }
        self.reads.retain(|path, _| self.tiles.iter().any(|tile| tile.path() == Some(path.as_str())));
    }

    /// Reads each take's file the strip does not hold yet, or that changed
    /// on disk: what it is, through `media`, and its sha256. A path is
    /// read from `root`, the project, unless it is whole.
    pub fn read_files(&mut self, root: &Path) {
        let paths: Vec<String> = self.tiles.iter().filter_map(|tile| tile.path().map(str::to_string)).collect();
        for path in paths {
            let file = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { root.join(&path) };
            let stamp = std::fs::metadata(&file).ok().and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
            if self.reads.get(&path).is_some_and(|read| read.stamp == stamp) {
                continue;
            }
            let sha256 = std::fs::read(&file).map(|bytes| layers::link::sha256(&bytes)).unwrap_or_default();
            let info = media::Video::open(&file).ok().map(|video| video.info());
            self.reads.insert(path, Read { stamp, file, info, sha256 });
        }
    }

    /// A take's file as read.
    pub fn read(&self, path: &str) -> Option<&Read> {
        self.reads.get(path)
    }

    /// Every take's file read, to ask for its thumbnail.
    pub fn reads(&self) -> impl Iterator<Item = &Read> {
        self.reads.values()
    }

    /// The round a tile is of.
    pub fn round_of(&self, tile: &Tile) -> Option<&Round> {
        let (_, rounds) = self.by_cell.iter().find(|(cell, _)| *cell == tile.cell)?;
        rounds.iter().find(|round| round.request == tile.request)
    }

    /// Shows the tile at `index` alone, or nothing of the rounds with none.
    pub fn show(&mut self, index: Option<usize>) {
        self.shown = index;
        self.with = With::Alone;
    }

    /// The tiles of a tile's round, in order, that have their take.
    pub fn round_tiles(&self, tile: &Tile) -> Vec<usize> {
        (0..self.tiles.len())
            .filter(|index| {
                let other = &self.tiles[*index];
                other.cell == tile.cell && other.request == tile.request && other.file.is_some()
            })
            .collect()
    }

    /// The tile of the take at `path`.
    pub fn tile_of(&self, path: &str) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.path() == Some(path))
    }

    /// Whether that cell has made a round: its draft is no longer only
    /// editing state.
    pub fn has_rounds(&self, cell: &str) -> bool {
        self.by_cell.iter().any(|(id, rounds)| id == cell && !rounds.is_empty())
    }

    /// The strip's runs: each round's label, "Round 14 · Kling 3.0", and
    /// how many tiles it has.
    pub fn groups(&self) -> Vec<(String, usize)> {
        let mut groups: Vec<(String, usize)> = Vec::new();
        let mut last: Option<(&str, &str)> = None;
        for tile in &self.tiles {
            let key = (tile.cell.as_str(), tile.request.as_str());
            if last == Some(key)
                && let Some(group) = groups.last_mut()
            {
                group.1 += 1;
                continue;
            }
            last = Some(key);
            let model = self.round_of(tile).map_or("", model_name);
            groups.push((format!("Round {} · {model}", tile.number), 1));
        }
        groups
    }

    /// The takes' size: the first read's that opened.
    pub fn shape(&self) -> Option<(u32, u32)> {
        self.tiles.iter().filter_map(Tile::path).filter_map(|path| self.reads.get(path)).find_map(|read| read.info).map(|info| (info.width, info.height))
    }
}

/// The model as a round's label reads it: its name, "MiniMax H3 Max", or
/// its id when the host does not know the name.
pub fn model_name(round: &Round) -> &str {
    if round.model_name.is_empty() { &round.model } else { &round.model_name }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a round labelled by its model's id when the host knows its
    /// name, or by nothing when it does not.
    #[test]
    fn a_round_reads_its_models_name() {
        let mut named = round("r1", 1, &[], RoundState::Done, None);
        assert_eq!(model_name(&named), "Model");
        named.model_name = "MiniMax H3 Max".into();
        assert_eq!(model_name(&named), "MiniMax H3 Max");
    }

    fn round(request: &str, number: u32, takes: &[&str], state: RoundState, count: Option<u32>) -> Round {
        Round {
            request: request.into(),
            number,
            model: "Model".into(),
            model_name: String::new(),
            prompt: String::new(),
            references: Vec::new(),
            left_out: Vec::new(),
            state,
            started: Some(1_793_146_000),
            count,
            takes: takes.iter().map(|path| TakeFile { path: path.to_string(), media_type: "video/mp4".into(), canon: false }).collect(),
            error: None,
            report: None,
            artifact: None,
        }
    }

    /// Catches a strip that loses a round's runs or its places: a done
    /// round has a tile for each video, one being made a place for each
    /// take still to come, and one that failed with nothing a failed
    /// place; a take of another kind is left out, and a round sent again
    /// without a take loses its tile.
    #[test]
    fn rounds_lay_out_the_strip() {
        let mut rounds = Rounds::default();
        let mut first = round("r1", 1, &["library/a.mp4", "library/b.mp4"], RoundState::Done, None);
        first.takes.push(TakeFile { path: "library/c.png".into(), media_type: "image/png".into(), canon: false });
        let sent = vec![first, round("r2", 2, &["library/d.mp4"], RoundState::Running, Some(3)), round("r3", 3, &[], RoundState::Failed, None)];
        rounds.set("c1", sent, &["c1".to_string()]);
        assert_eq!(rounds.groups(), [("Round 1 · Model".to_string(), 2), ("Round 2 · Model".to_string(), 3), ("Round 3 · Model".to_string(), 1)]);
        assert!(matches!(rounds.tiles[3].pending, Some(FilmPending::Running { since: Some(_) })));
        assert_eq!(rounds.tiles[5].pending, Some(FilmPending::Failed));
        rounds.shown = Some(1);
        rounds.set("c1", vec![round("r1", 1, &["library/b.mp4"], RoundState::Done, None)], &["c1".to_string()]);
        assert_eq!(rounds.groups(), [("Round 1 · Model".to_string(), 1)]);
        assert_eq!(rounds.shown, Some(0), "the take shown did not stay shown");
        assert_eq!(rounds.tile_of("library/a.mp4"), None);
    }
}
