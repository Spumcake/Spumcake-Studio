//! The pre-vis check against Spumcake-Skills' own method: `expected.png`
//! is what `build-animatic/scripts/overlay.py`'s lines make from the same
//! render and pre-vis (`fixtures/make_previs_check.py`).

use layers::{Rgba, edges};

fn load(name: &str) -> Rgba {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let image = image::open(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display())).to_rgba8();
    Rgba { width: image.width(), height: image.height(), data: image.into_raw() }
}

/// Catches a pre-vis check that only looks like overlay.py's: every pixel,
/// edges and the 85% mix toward cyan alike, matches the script's output.
#[test]
fn the_check_matches_overlay_py() {
    compare("previs.png", "expected.png");
}

/// Catches a grey conversion other than Pillow's: a coloured pre-vis's
/// edges are found where the script finds them.
#[test]
fn a_coloured_previs_matches_overlay_py() {
    compare("previs-colour.png", "expected-colour.png");
}

fn compare(previs: &str, expected: &str) {
    let mut render = load("render.png");
    edges::check(&mut render, &load(previs), edges::CYAN);
    let expected = load(expected);
    let mut differing = Vec::new();
    for y in 0..render.height {
        for x in 0..render.width {
            if render.pixel(x, y) != expected.pixel(x, y) {
                differing.push((x, y, render.pixel(x, y), expected.pixel(x, y)));
            }
        }
    }
    assert!(differing.is_empty(), "{} pixels differ, first {:?}", differing.len(), &differing[..differing.len().min(5)]);
}
