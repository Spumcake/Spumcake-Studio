//! What the Image Generator and the Video Generator do alike: a layer's
//! link to a file it never writes, notes marked on an image, the pre-vis
//! check's edges, laying out a round's takes, drawing the views' small
//! labels in Inter, which ships here, how a workspace's file keeps a
//! cell, and a round's words. Images here are straight
//! RGBA, rows top to bottom, as both workspaces keep them.

pub mod cells;
pub mod edges;
pub mod label;
pub mod layout;
pub mod link;
pub mod notes;
pub mod words;

/// An image as both workspaces hold one: `width` × `height` × 4 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Rgba {
    pub fn new(width: u32, height: u32, fill: [u8; 4]) -> Rgba {
        Rgba { width, height, data: fill.repeat((width * height) as usize) }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * self.width + x) * 4) as usize;
        [self.data[at], self.data[at + 1], self.data[at + 2], self.data[at + 3]]
    }

    pub fn set(&mut self, x: u32, y: u32, value: [u8; 4]) {
        let at = ((y * self.width + x) * 4) as usize;
        self.data[at..at + 4].copy_from_slice(&value);
    }
}
