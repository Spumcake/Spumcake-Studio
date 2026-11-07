//! Video, sound, and encoding for the workspaces that play and write them:
//! the Storyboard Generator and the Video Generator, as `draw` is shared.
//! Never used by the host, which only draws the raw frames a workspace
//! writes. Its interface is the lifecycle contract's "Media: one shared
//! library", in the team repository; the Image and Video role session
//! writes it.
