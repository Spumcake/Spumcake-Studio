# Recipe Creation — sub-application

Recipe Creation makes recipes: each recipe is a list of cells, such as a show's characters, references, skills, and results, that the assistant draws on when it generates. The host draws and runs every cell. Its names follow Studio's conventions guide: a new recipe is "Untitled Recipe", then "Untitled Recipe 2", free among the open tabs and the `.smx` files in the project's folder.

## Rules for this sub-application

- It is its own program. Its crate depends on `protocol`, `serde`, and `serde_json` only, never on the host or on another sub-application.
- Everything the host shows for it is declared through the protocol: its tabs, menu rows, tools, and the cells contents. Nothing about it is hard-coded in the host.
- It owns its recipes: the order of cells, their prompts, their finished results, which cell is selected, how a recipe is saved, and its undo. It never draws a cell and never calls a network.
- A recipe's `.smx` file keeps the keys and version 2 that 0.13 wrote, so every recipe saved before still opens.
- It runs with no host, driven only through its standard input and standard output.
- It speaks the protocol defined in `protocol/` and nothing else. When it needs something the protocol cannot say, change the protocol, the host, and the scaffolding skill together. Do not work around the protocol.
- `manifest.json` registers it with the host. Its id, folder, crate, and program names are the same.
