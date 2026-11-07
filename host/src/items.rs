//! The host's index of items: what each workspace declared with `items`,
//! and nothing else. From it the host works out which items are out of
//! date, tells each owner with `stale`, and finds the owner of an item a
//! cell or a workspace needs rendered. It keeps no history of its own: what
//! an item was made from is what its owner declares, so the index is the
//! same after a restart once the owners have declared again. It knows items
//! only as paths, ids, and fingerprints.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use protocol::{Item, ItemRef, StaleItem};

/// One file's items, and the application that declared them.
struct File {
    owner: String,
    items: Vec<Item>,
}

#[derive(Default)]
pub(crate) struct Index {
    files: BTreeMap<String, File>,
    /// What each file's owner was last told was out of date.
    told: HashMap<String, Vec<StaleItem>>,
}

impl Index {
    /// `owner` declared every item of `path`; an empty list says the file is
    /// gone.
    pub(crate) fn declare(&mut self, owner: &str, path: String, items: Vec<Item>) {
        if items.is_empty() {
            self.files.remove(&path);
        } else {
            self.files.insert(path, File { owner: owner.to_string(), items });
        }
    }

    /// The application whose file this is.
    pub(crate) fn owner(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(|file| file.owner.as_str())
    }

    /// An item's fingerprint as last declared; for a whole file, one made
    /// from every item's. None when it is gone.
    pub(crate) fn current(&self, item: &ItemRef) -> Option<String> {
        let file = self.files.get(&item.path)?;
        match &item.item {
            Some(id) => file.items.iter().find(|declared| &declared.id == id).map(|declared| declared.fingerprint.clone()),
            None => {
                let mut whole: Vec<String> = file.items.iter().map(|declared| format!("{}:{}", declared.id, declared.fingerprint)).collect();
                whole.sort();
                Some(protocol::fingerprint(whole.join("\n").as_bytes()))
            }
        }
    }

    /// Every out-of-date item, by file: an item is out of date when an item
    /// it was made from is gone, has another fingerprint than the one
    /// recorded, or is out of date itself.
    pub(crate) fn stale(&self) -> BTreeMap<String, Vec<StaleItem>> {
        let mut known: HashMap<ItemRef, Vec<ItemRef>> = HashMap::new();
        let mut out: BTreeMap<String, Vec<StaleItem>> = BTreeMap::new();
        for (path, file) in &self.files {
            for item in &file.items {
                let at = ItemRef { path: path.clone(), item: Some(item.id.clone()) };
                let changed = self.changed(&at, &mut known, &mut BTreeSet::new());
                if !changed.is_empty() {
                    out.entry(path.clone()).or_default().push(StaleItem { id: item.id.clone(), changed });
                }
            }
        }
        out
    }

    /// Which items `at` was made from changed, or are out of date
    /// themselves. A loop of items made from each other ends where it
    /// began.
    fn changed(&self, at: &ItemRef, known: &mut HashMap<ItemRef, Vec<ItemRef>>, visiting: &mut BTreeSet<ItemRef>) -> Vec<ItemRef> {
        if let Some(done) = known.get(at) {
            return done.clone();
        }
        if !visiting.insert(at.clone()) {
            return Vec::new();
        }
        let mut changed = Vec::new();
        if let Some(made) = self.item(at).and_then(|item| item.made.as_ref()) {
            for used in made.used.iter().chain(&made.sources) {
                let moved = self.current(&used.item).is_none_or(|now| now != used.fingerprint);
                let upstream = !moved && used.item.item.is_some() && !self.changed(&used.item, known, visiting).is_empty();
                if (moved || upstream) && !changed.contains(&used.item) {
                    changed.push(used.item.clone());
                }
            }
        }
        visiting.remove(at);
        known.insert(at.clone(), changed.clone());
        changed
    }

    /// An item's name as its owner declared it.
    pub(crate) fn name(&self, at: &ItemRef) -> Option<&str> {
        self.item(at).map(|item| item.name.as_str())
    }

    fn item(&self, at: &ItemRef) -> Option<&Item> {
        let id = at.item.as_ref()?;
        self.files.get(&at.path)?.items.iter().find(|item| &item.id == id)
    }

    /// What each owner must be told now: every file whose out-of-date items
    /// differ from what its owner was last told, with an empty list for a
    /// file none of whose items are out of date any more.
    pub(crate) fn tell(&mut self) -> Vec<(String, String, Vec<StaleItem>)> {
        let now = self.stale();
        let mut messages = Vec::new();
        for (path, items) in &now {
            if self.told.get(path) != Some(items)
                && let Some(owner) = self.owner(path)
            {
                messages.push((owner.to_string(), path.clone(), items.clone()));
            }
        }
        for path in self.told.keys() {
            if !now.contains_key(path)
                && let Some(owner) = self.owner(path)
            {
                messages.push((owner.to_string(), path.clone(), Vec::new()));
            }
        }
        self.told = now.into_iter().collect();
        messages
    }

    /// Every out-of-date item, each with its owner, those made from others
    /// after the others, so rendering them in this order renders each once.
    pub(crate) fn upstream_first(&self) -> Vec<(ItemRef, String)> {
        let stale = self.stale();
        let all: Vec<ItemRef> = stale
            .iter()
            .flat_map(|(path, items)| items.iter().map(move |item| ItemRef { path: path.clone(), item: Some(item.id.clone()) }))
            .collect();
        let mut placed: Vec<ItemRef> = Vec::new();
        let mut left = all.clone();
        while !left.is_empty() {
            let before = left.len();
            left.retain(|at| {
                let waits = self.item(at).and_then(|item| item.made.as_ref()).is_some_and(|made| {
                    made.used.iter().chain(&made.sources).any(|used| all.contains(&used.item) && !placed.contains(&used.item))
                });
                if !waits {
                    placed.push(at.clone());
                }
                waits
            });
            if left.len() == before {
                placed.append(&mut left);
            }
        }
        placed
            .into_iter()
            .filter_map(|at| self.owner(&at.path).map(|owner| (at.clone(), owner.to_string())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{Made, UsedItem};

    fn at(path: &str, item: &str) -> ItemRef {
        ItemRef { path: path.into(), item: Some(item.into()) }
    }

    fn plain(id: &str, fingerprint: &str) -> Item {
        Item { id: id.into(), name: id.into(), media_types: Vec::new(), fingerprint: fingerprint.into(), made: None }
    }

    fn made_from(id: &str, fingerprint: &str, used: &[(ItemRef, &str)]) -> Item {
        let used = used.iter().map(|(item, fingerprint)| UsedItem { item: item.clone(), fingerprint: (*fingerprint).into() }).collect();
        Item { made: Some(Made { used, ..Made::default() }), ..plain(id, fingerprint) }
    }

    /// Three shots, two of them made from one reference, and a final video
    /// made from the shots, in files of workspaces the index was not
    /// written for.
    fn film() -> Index {
        let mut index = Index::default();
        index.declare("easel", "Cast.easel".into(), vec![plain("hero", "h1"), plain("street", "s1")]);
        index.declare("reel", "Shots.reel".into(), vec![
            made_from("one", "o1", &[(at("Cast.easel", "hero"), "h1")]),
            made_from("two", "t1", &[(at("Cast.easel", "hero"), "h1")]),
            made_from("three", "r1", &[(at("Cast.easel", "street"), "s1")]),
        ]);
        index.declare("cutter", "Film.cutter".into(), vec![made_from(
            "final",
            "f1",
            &[(at("Shots.reel", "one"), "o1"), (at("Shots.reel", "two"), "t1"), (at("Shots.reel", "three"), "r1")],
        )]);
        index
    }

    /// Catches an index that marks everything, or nothing: changing the
    /// reference two of three shots used marks those two and the final
    /// video, which was made from them, and nothing else.
    #[test]
    fn only_what_an_edit_affects_is_out_of_date() {
        let mut index = film();
        assert!(index.stale().is_empty());
        index.declare("easel", "Cast.easel".into(), vec![plain("hero", "h2"), plain("street", "s1")]);
        let stale = index.stale();
        let shots: Vec<&str> = stale["Shots.reel"].iter().map(|item| item.id.as_str()).collect();
        assert_eq!(shots, ["one", "two"]);
        assert_eq!(stale["Shots.reel"][0].changed, [at("Cast.easel", "hero")]);
        assert_eq!(stale["Film.cutter"][0].changed, [at("Shots.reel", "one"), at("Shots.reel", "two")]);
        assert!(!stale.contains_key("Cast.easel"));
    }

    /// Catches a comparison by anything but content: the reference changed
    /// and changed back marks nothing.
    #[test]
    fn changing_back_marks_nothing() {
        let mut index = film();
        index.declare("easel", "Cast.easel".into(), vec![plain("hero", "h2"), plain("street", "s1")]);
        assert!(!index.stale().is_empty());
        index.declare("easel", "Cast.easel".into(), vec![plain("hero", "h1"), plain("street", "s1")]);
        assert!(index.stale().is_empty());
    }

    /// Catches a source that is gone being taken as unchanged.
    #[test]
    fn a_source_that_is_gone_marks_what_used_it() {
        let mut index = film();
        index.declare("easel", "Cast.easel".into(), Vec::new());
        assert_eq!(index.stale()["Shots.reel"].len(), 3);
    }

    /// Catches owners told the same thing every frame, or never told that
    /// an item is up to date again.
    #[test]
    fn owners_are_told_only_what_changed() {
        let mut index = film();
        assert!(index.tell().is_empty());
        index.declare("easel", "Cast.easel".into(), vec![plain("hero", "h2"), plain("street", "s1")]);
        let told: Vec<(String, String)> = index.tell().into_iter().map(|(owner, path, _)| (owner, path)).collect();
        assert_eq!(told, [("cutter".to_string(), "Film.cutter".to_string()), ("reel".into(), "Shots.reel".into())]);
        assert!(index.tell().is_empty());
        // The shots are rendered again from the new reference.
        index.declare("reel", "Shots.reel".into(), vec![
            made_from("one", "o2", &[(at("Cast.easel", "hero"), "h2")]),
            made_from("two", "t2", &[(at("Cast.easel", "hero"), "h2")]),
            made_from("three", "r1", &[(at("Cast.easel", "street"), "s1")]),
        ]);
        let told = index.tell();
        let reel = told.iter().find(|(_, path, _)| path == "Shots.reel").unwrap();
        assert!(reel.2.is_empty(), "the shots are up to date again");
        // The final video is still out of date, for the same shots, so its
        // owner is not told again.
        assert!(told.iter().all(|(_, path, _)| path != "Film.cutter"));
        assert_eq!(index.stale()["Film.cutter"][0].changed, [at("Shots.reel", "one"), at("Shots.reel", "two")]);
    }

    /// Catches an order that renders a final video before the shots it is
    /// made from.
    #[test]
    fn out_of_date_items_come_upstream_first() {
        let mut index = film();
        index.declare("easel", "Cast.easel".into(), vec![plain("hero", "h2"), plain("street", "s1")]);
        let order: Vec<String> = index.upstream_first().into_iter().map(|(at, _)| at.item.unwrap()).collect();
        let final_at = order.iter().position(|id| id == "final").unwrap();
        assert!(order.iter().position(|id| id == "one").unwrap() < final_at);
        assert!(order.iter().position(|id| id == "two").unwrap() < final_at);
    }

    /// Catches a loop of items made from each other hanging the host.
    #[test]
    fn a_loop_ends() {
        let mut index = Index::default();
        index.declare("loop", "A.loop".into(), vec![
            made_from("a", "a1", &[(at("A.loop", "b"), "b1")]),
            made_from("b", "b1", &[(at("A.loop", "a"), "a0")]),
        ]);
        assert!(!index.stale().is_empty());
        assert_eq!(index.upstream_first().len(), 2);
    }
}
