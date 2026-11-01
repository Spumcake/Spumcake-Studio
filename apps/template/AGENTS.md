# Template — sub-application

The smallest correct Spumcake Studio sub-application. It reports one tab named Template, declares the shared grid and the properties panel on the right, and provides the assistant one skill, which renames that tab. Started with `--restore`, it opens its tab only when the host sends `open`. It does nothing else. The **scaffolding** skill in `skills/scaffolding/` copies this folder to start every new sub-application, so keep it minimal and working.

## Rules for this sub-application

- It is its own program. Its crate depends on `protocol` and `serde_json` only, never on the host or on another sub-application.
- Everything the host shows for it is declared through the protocol: its tabs, menu rows, tools, properties, and the shared components it uses. Nothing about it is hard-coded in the host.
- It owns its document: how the work is arranged, how it is saved, and its undo.
- It runs with no host, driven only through its standard input and standard output.
- It speaks the protocol defined in `protocol/` and nothing else. When it needs something the protocol cannot say, change the protocol, the host, and the scaffolding skill together. Do not work around the protocol.
- `manifest.json` registers it with the host. Its id, folder, crate, and program names are the same.
