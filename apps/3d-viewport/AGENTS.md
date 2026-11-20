# 3D Viewport — sub-application

Shows a place's pre-vis in 3D, as it stands, from the place's scene files in the project's `prompts/resources/`: the user orbits it freely, looks through any of its builder's cameras, and lays each camera's published pre-vis render beside or over it, and the door's swing plays as its published video shows it. Nothing in it is editable in Studio 0.15. The host draws the scene itself from the scene file; this program chooses what is shown, as `view3d`, and declares the camera list, the toolbar, and the menu.

Which render of a camera is in use is the project's canon's, as the scene contract has it since the owner's data model of 2026-11-18: its camera's `canon` artifact's latest iteration, asked of the host with `read_canon` (`pipeline.py sql --read-only`) when a place opens, and what is out of date with `read_touches`. The list marks the render in use with the seal, draws the others dimmer, and outlines a render in use that is out of date. A later iteration no builder made is listed with the newest scene that has its camera, and is shown only beside the scene, never over it. Without a pipeline, nothing is marked.

## Rules for this sub-application

- It is its own program. Its crate depends on `protocol`, `serde_json`, `scene` (the scene reader in Spumcake-Render), `image` for a published render's PNG, and `media` for a published video's frames, never on the host or on another sub-application.
- Everything the host shows for it is declared through the protocol: its tabs, menu rows, tools, properties, and the shared components it uses. Nothing about it is hard-coded in the host.
- It writes no work of the user's: the scene files are the Skills layout's, and the only files it writes are decoded renders and thumbnails in the project's `.spumcake/cache/3d-viewport/`, which is never committed.
- It runs with no host, driven only through its standard input and standard output.
- It speaks the protocol defined in `protocol/` and nothing else. When it needs something the protocol cannot say, change the protocol, the host, and the scaffolding skill together. Do not work around the protocol.
- `manifest.json` registers it with the host. Its id, folder, and program are `3d-viewport`; its package is `viewport-3d`, since Cargo refuses a package name that starts with a digit.
