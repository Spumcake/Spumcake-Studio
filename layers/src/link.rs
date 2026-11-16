//! A layer made from a file outside the workspace links to it, as
//! Photoshop's linked smart objects do, and the file is never written. The
//! link names the file as the project names it and the sha256 Spumcake-
//! Skills records for it, so the workspace can tell when the file is no
//! longer the one it linked.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// A file a layer was made from: its path, relative to the project where
/// there is one, and its sha256 when linked, in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub path: String,
    pub sha256: String,
}

impl Link {
    /// Links to the file at `path`, read from `root` when the path is
    /// relative.
    pub fn to(root: &Path, path: &str) -> std::io::Result<Link> {
        Ok(Link { path: path.to_string(), sha256: sha256_file(&root.join(path))? })
    }

    /// Whether the file is still there and still the one linked.
    pub fn holds(&self, root: &Path) -> bool {
        sha256_file(&root.join(&self.path)).is_ok_and(|now| now == self.sha256)
    }

    /// The file's name, as a layer's row shows it.
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }
}

/// The sha256 of a file's bytes, in lowercase hex, as `request.py` records
/// it.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

/// The sha256 of bytes in memory, in lowercase hex.
pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Catches a hash that is not sha256, or not hex as Skills writes it:
    /// the known digest of "abc".
    #[test]
    fn the_hash_is_skills_sha256() {
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    /// Catches a link that never notices its file changed or went.
    #[test]
    fn a_link_holds_until_its_file_changes() {
        let dir = std::env::temp_dir().join(format!("layers-link-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("library")).unwrap();
        std::fs::write(dir.join("library/take.png"), b"one").unwrap();
        let link = Link::to(&dir, "library/take.png").unwrap();
        assert!(link.holds(&dir));
        assert_eq!(link.file_name(), "take.png");
        std::fs::write(dir.join("library/take.png"), b"two").unwrap();
        assert!(!link.holds(&dir));
        std::fs::remove_file(dir.join("library/take.png")).unwrap();
        assert!(!link.holds(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
