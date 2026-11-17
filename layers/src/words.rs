//! A round's words, as both workspaces write them over a round shown
//! whole: how many takes it has, and each reference by its order and role,
//! as ig-references-sheet shows them.

use protocol::SentReference;

/// "1 take", "4 takes".
pub fn takes_word(count: usize) -> String {
    if count == 1 { String::from("1 take") } else { format!("{count} takes") }
}

/// "1st", "2nd", "3rd", "4th", … "11th", "21st".
pub fn ordinal(number: usize) -> String {
    let suffix = match (number % 10, number % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{number}{suffix}")
}

/// A file's name, without its folders.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A reference's words on the references sheet: its order and role, "1st,
/// control: the room's layout.", or its order and file. A role that is one
/// of `roles`, the workspace's own menu words, is written lower case.
pub fn sheet_label(index: usize, reference: &SentReference, roles: &[&str]) -> String {
    let lower = |word: &str| if roles.contains(&word) { word.to_lowercase() } else { word.to_string() };
    let what = match reference.role.as_deref().map(str::trim).filter(|role| !role.is_empty()) {
        Some(role) => match role.split_once(", ") {
            Some((word, words)) => format!("{}: {words}", lower(word)),
            None => lower(role),
        },
        None => file_name(&reference.path).to_string(),
    };
    format!("{}, {what}.", ordinal(index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(path: &str, role: Option<&str>) -> SentReference {
        SentReference { path: path.into(), role: role.map(str::to_string) }
    }

    /// Catches a reference's words out of order or in the wrong case: its
    /// ordinal, a menu word lower case, the person's words after a colon,
    /// and a file's name with no role.
    #[test]
    fn a_reference_reads_by_order_and_role() {
        let roles = ["Start frame", "Motion reference"];
        assert_eq!(sheet_label(0, &reference("a.png", Some("Start frame, the keeper")), &roles), "1st, start frame: the keeper.");
        assert_eq!(sheet_label(1, &reference("b.mp4", Some("Video 1, the motion and the acting")), &roles), "2nd, Video 1: the motion and the acting.");
        assert_eq!(sheet_label(2, &reference("refs/c.png", None), &roles), "3rd, c.png.");
        assert_eq!((ordinal(11), ordinal(22), takes_word(1), takes_word(4)), ("11th".into(), "22nd".into(), "1 take".into(), "4 takes".into()));
    }
}
