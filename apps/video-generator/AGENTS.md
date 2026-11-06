# Video Generator — sub-application

The Video Generator will make shots, each rendered from its board, its first and last frames, and its references, and rendered again as the person iterates; today it is a scaffold copied from the template, which the Image and Video role session fills, as the lifecycle's `systems.md` in the team repository describes.

## Rules for this sub-application

- It is its own program. Its crate depends on `protocol` and `serde_json` only, never on the host or on another sub-application.
- Everything the host shows for it is declared through the protocol: its tabs, menu rows, tools, properties, and the shared components it uses. Nothing about it is hard-coded in the host.
- It owns its document: how the work is arranged, how it is saved, and its undo.
- It runs with no host, driven only through its standard input and standard output.
- It speaks the protocol defined in `protocol/` and nothing else. When it needs something the protocol cannot say, change the protocol, the host, and the scaffolding skill together. Do not work around the protocol.
- `manifest.json` registers it with the host. Its id, folder, crate, and program names are the same.
