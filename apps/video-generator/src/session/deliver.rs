//! Deliver…, as vg-deliver and vg-delivering show it, in a Skills project
//! for a round's take whose round names its artifact. Each sound layer
//! inside `work/` is filed first with `file_resource`, and one outside the
//! project stops the flow; the mix is made by the build-animatic skill's
//! `mix.py` through `run_skill_script`, from the take, its own sound, and
//! every sound layer, as `mix_args` passes them; its answer's film goes to
//! the host with `deliver`, whose card asks the person's yes, publishes it,
//! and delivers it. Every notice is this workspace's, at the working
//! area's bottom left: "Delivering…" while the host delivers, then
//! "Delivered <file name>.", or why it failed; a Cancel removes it. The
//! mixed film stays a take in the library either way, which is the
//! host's.

use super::*;
use super::mix::{MixSound, mix_args};
use protocol::{Action, Notice, NoticeCorner, NoticeState};

/// A round take's menu row, the guide's Deliver.
pub const DELIVER: &str = "Deliver…";
/// The notices, pending the owner's word.
pub const DELIVERING: &str = "Delivering…";
pub const DELIVERED: &str = "Delivered {file}.";
pub const NOT_IN_PROJECT: &str = "{file} is not in the project.";
pub const NO_FILM: &str = "The mix gave no film.";
/// The skill and script that make the mix.
const SKILL: &str = "build-animatic";
const SCRIPT: &str = "mix.py";

/// A Deliver under way.
#[derive(Debug, Clone)]
pub(super) struct Delivery {
    tab: String,
    artifact: String,
    /// The take's path in the project, and its caption.
    take: String,
    caption: String,
    take_sound: crate::gallery::TakeSound,
    /// Each sound layer, top first: its path in the project once known,
    /// its gain, start, and mute, and its name.
    sounds: Vec<(Option<String>, f64, f64, bool, String)>,
    /// The filings still to be answered, by id, each a sound's place.
    waiting: Vec<(String, usize)>,
    /// The mix's script, once asked.
    script: Option<String>,
}

/// The tab's notices at the working area's bottom left: one, or none.
pub(super) fn notices(tab: &str, notice: Option<(String, NoticeState)>) -> AppMessage {
    let notices = notice
        .map(|(text, state)| Notice {
            id: String::from("deliver"),
            text,
            detail: String::new(),
            progress: None,
            state,
            actions: Vec::new(),
            close: (state != NoticeState::Working).then(|| Action::new("notice_close")),
        })
        .into_iter()
        .collect();
    AppMessage::Notices { tab: tab.to_string(), notices, corner: NoticeCorner::BottomLeft }
}

fn failed(tab: &str, text: String) -> AppMessage {
    notices(tab, Some((text, NoticeState::Failed)))
}

/// Names joined as a sentence says them: "Voice", "Voice and Bed", "Voice,
/// Bed and Door".
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a delivered film uses, as the Deliver card lists it: the take's
/// caption, then ", with " and the heard sound layers' names.
pub fn uses(caption: &str, heard: &[String]) -> String {
    if heard.is_empty() { caption.to_string() } else { format!("{caption}, with {}", join_names(heard)) }
}

impl Session {
    /// Whether a round's take at `path` can be delivered: in a Skills
    /// project, its round naming its artifact.
    pub fn can_deliver(&self, tab: &Tab, path: &str) -> bool {
        self.skills
            && tab.rounds.tile_of(path).and_then(|tile| tab.rounds.round_of(&tab.rounds.tiles[tile])).is_some_and(|round| round.artifact.as_deref().is_some_and(|artifact| !artifact.is_empty()))
    }

    /// Deliver… on a round's take: its sound layers filed first where they
    /// must be, then its mix.
    pub(super) fn deliver(&mut self, path: &str, out: &mut Vec<AppMessage>) {
        let index = self.active;
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        if !self.can_deliver(tab, path) {
            return;
        }
        let Some(tile) = tab.rounds.tile_of(path).map(|tile| &tab.rounds.tiles[tile]) else {
            return;
        };
        let artifact = tab.rounds.round_of(tile).and_then(|round| round.artifact.clone()).unwrap_or_default();
        let caption = tab.take_caption(tile);
        let mut sounds = Vec::new();
        let mut filing = Vec::new();
        for (place, layer) in tab.gallery.sounds.iter().enumerate() {
            let Some(relative) = self.relative(&layer.file) else {
                let name = layer.file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(failed(&tab.id, NOT_IN_PROJECT.replace("{file}", &name)));
                return;
            };
            // A file in `work/` is the workspace's own, filed first.
            let known = if relative.starts_with("work/") {
                filing.push((place, relative));
                None
            } else {
                Some(relative)
            };
            sounds.push((known, layer.gain_db, layer.start, layer.muted, layer.name.clone()));
        }
        let mut delivery = Delivery {
            tab: tab.id.clone(),
            artifact,
            take: path.to_string(),
            caption,
            take_sound: tab.gallery.take_sound,
            sounds,
            waiting: Vec::new(),
            script: None,
        };
        for (place, relative) in filing {
            let id = format!("deliver-{}", self.next_file);
            self.next_file += 1;
            out.push(AppMessage::FileResource { tab: delivery.tab.clone(), path: relative, from: Vec::new(), id: Some(id.clone()) });
            delivery.waiting.push((id, place));
        }
        self.delivery = Some(delivery);
        self.mix_when_filed(out);
    }

    /// A sound layer filed: its filed path is the mix's; a filing that
    /// failed stops the flow with why.
    pub(super) fn filed(&mut self, id: &str, file: &str, error: Option<String>, out: &mut Vec<AppMessage>) {
        let Some(delivery) = self.delivery.as_mut() else {
            return;
        };
        let Some(at) = delivery.waiting.iter().position(|(own, _)| own == id) else {
            return;
        };
        let (_, place) = delivery.waiting.remove(at);
        match error.filter(|error| !error.is_empty()) {
            Some(error) => {
                let tab = delivery.tab.clone();
                self.delivery = None;
                out.push(failed(&tab, error));
            }
            None => {
                delivery.sounds[place].0 = Some(file.to_string());
                self.mix_when_filed(out);
            }
        }
    }

    /// Once every sound layer is filed: the mix, through the build-animatic
    /// skill's `mix.py`.
    fn mix_when_filed(&mut self, out: &mut Vec<AppMessage>) {
        let id = format!("deliver-{}", self.next_file);
        let Some(delivery) = self.delivery.as_mut().filter(|delivery| delivery.waiting.is_empty() && delivery.script.is_none()) else {
            return;
        };
        let sounds: Vec<MixSound> = delivery
            .sounds
            .iter()
            .map(|(path, gain_db, start, muted, _)| MixSound { path: path.clone().unwrap_or_default(), gain_db: *gain_db, start: *start, muted: *muted })
            .collect();
        let mut args = mix_args(&delivery.take, delivery.take_sound, &sounds);
        args.extend([String::from("--name"), delivery.artifact.clone(), String::from("--json")]);
        delivery.script = Some(id.clone());
        self.next_file += 1;
        out.push(AppMessage::RunSkillScript { id, skill: SKILL.into(), script: SCRIPT.into(), args });
    }

    /// The mix's answer: why it failed, or its film, the last line of its
    /// output as `{"path", "sha256"}`, sent to the host with `deliver` and
    /// what it uses, while "Delivering…" shows.
    pub(super) fn mix_ran(&mut self, id: &str, output: &str, error: Option<String>, out: &mut Vec<AppMessage>) {
        if self.delivery.as_ref().and_then(|delivery| delivery.script.as_deref()) != Some(id) {
            return;
        }
        let Some(delivery) = self.delivery.take() else {
            return;
        };
        if let Some(error) = error.filter(|error| !error.is_empty()) {
            out.push(failed(&delivery.tab, error));
            return;
        }
        let film = output
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .and_then(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .and_then(|answer| answer.get("path").and_then(Value::as_str).map(str::to_string))
            .filter(|path| !path.is_empty());
        let Some(file) = film else {
            out.push(failed(&delivery.tab, NO_FILM.into()));
            return;
        };
        let heard: Vec<String> = delivery.sounds.iter().filter(|sound| !sound.3).map(|sound| sound.4.clone()).collect();
        out.push(AppMessage::Deliver { tab: delivery.tab.clone(), artifact: delivery.artifact.clone(), file, uses: uses(&delivery.caption, &heard) });
        out.push(notices(&delivery.tab, Some((DELIVERING.into(), NoticeState::Working))));
    }

    /// The host's answer to `deliver`: the file delivered, done; why not,
    /// failed; or nothing, a Cancel, which takes the notice away.
    pub(super) fn delivered(&mut self, tab: &str, file: &str, error: Option<String>, out: &mut Vec<AppMessage>) {
        let notice = if !file.is_empty() {
            let name = Path::new(file).file_name().map_or_else(|| file.to_string(), |name| name.to_string_lossy().into_owned());
            Some((DELIVERED.replace("{file}", &name), NoticeState::Done))
        } else {
            error.filter(|error| !error.is_empty()).map(|error| (error, NoticeState::Failed))
        };
        out.push(notices(tab, notice));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches what a delivered film uses listed other than as a sentence:
    /// the take alone, then with one, two, and three heard layers.
    #[test]
    fn a_film_lists_what_it_uses() {
        let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(uses("Take 1 · 60 fps · 4K, from Take 7", &[]), "Take 1 · 60 fps · 4K, from Take 7");
        assert_eq!(uses("Take 2 · Kling 3.0", &names(&["Voice"])), "Take 2 · Kling 3.0, with Voice");
        assert_eq!(join_names(&names(&["Voice", "Bed"])), "Voice and Bed");
        assert_eq!(join_names(&names(&["Voice", "Bed", "Door"])), "Voice, Bed and Door");
    }
}
