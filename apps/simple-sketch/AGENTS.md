# Simple Sketch — sub-application

Simple Sketch, the pressure-sensitive infinite-canvas sketching app, as a Spumcake Studio sub-application.

## Rules for this sub-application

- It is its own program. Its crate depends on `protocol` and the shared `draw`, never on the host or on another sub-application.
- Everything the host shows for it is declared through the protocol: its tabs, menu rows, tools, properties, and the shared components it uses. Nothing about it is hard-coded in the host.
- It owns its document: how the work is arranged, how it is saved, and its undo. The host only previews a gesture and sends it when it ends; this program commits it and sends the whole document back.
- Its session, registry, store, and preferences are Simple Sketch 1.0.1's, moved whole. What its window drew is declared in `src/session/declare.rs`, and each action the host sends back runs the session call that window made, in `src/serve.rs`. Change those two files, not the moved session, when the window's parts change.
- Its skills for the assistant are declared in `src/skills.rs`, built from its own hand surface: the gestures, the commands that change contents, every property the panel and outliner edit on objects, text, and palette colors, and the selection. Each tool runs the session's path for that hand action, in `src/session/assist.rs`, and puts the person's tool settings and panel page back afterwards. Undo, redo, files, the view, the panel, and the tools' own settings are never tools. A new command, action, property, gesture, or selection operation needs a tool there, or a place in the left-out list of `client/src/coverage.rs`, or `sketch-client` fails.
- Every change under one request's `group` is one undo step, merged in `src/record.rs`; a change by hand ends the group (`is_hand` in `src/serve.rs`).
- The looking tool's picture is drawn in `src/look.rs`: ink with `tiny-skia`, and text through `draw::text` in an egui context with no window, with its own copy of the host's Inter faces in `assets/fonts/`.
- Workspaces: each tab reports its `path` and `saves: true`; `open` and `keep` are handled in `src/serve.rs`; with `--restore` it opens no sketch until `open`, and its recovery offers wait for that first tab.
- It runs with no host, driven only through its standard input and standard output. `sketch-client` drives every tool that way.
- It speaks the protocol defined in `protocol/` and nothing else. When it needs something the protocol cannot say, change the protocol, the host, and the scaffolding skill together. Do not work around the protocol.
- `manifest.json` registers it with the host. Its id, folder, crate, and program names are the same.
