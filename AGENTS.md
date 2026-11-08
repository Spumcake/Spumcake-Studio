# AGENTS.md — Spumcake Studio

Spumcake Studio is the open-source production software of Spumcake, an AI-native entertainment and technology company: a Codex for creatives, the computer graphics (CG) authoring application behind the Spumcake platform. Its earlier name was Inpainter, which the program, its app ID, the keyring entry, and the data folders still use until a version moves them; what a person reads on screen says Spumcake Studio. One host program draws a shared shell, and each kind of work is a separate sub-application that the host starts and draws through one protocol. This repository holds the code, and what Studio is lives in `docs/overview.md`. Spumcake's mission, which says how Studio fits with Spumcake's productions, community platform, and streaming platform, and Studio's version documents live in the team repository, https://github.com/Spumcake/spumcake-team.

The stack is Rust, egui through eframe, and Iconoir icons.

## Layout

- `host/` — the `inpainter` program: the window, the session's conversation and the session menu (`sessions.rs`), the shell's common UI (tab strip, menu button, toolbar) shown when the workspace is open, the projects and their sessions (`workspace.rs`, `window/flows.rs`), the routing of the assistant's tool calls to sub-applications, the shared components a sub-application can declare (the properties panel with its outliner, the grid, the drawing surface, and cells), the questions and file dialogs a sub-application asks for, and finding and running sub-applications. It draws every toolbar item, menu row, panel row, tab, and cell from what was declared, and sends back the action each control carries. It runs every cell. It never depends on a sub-application.
- `protocol/` — the contract between the host and sub-applications: the messages, one JSON object per line, the declarations, the cell, the tool groups a sub-application offers the assistant, items and what each was made from (`protocol/src/items.rs`), and the manifest.
- `assistant/` — the assistant: the conversation, the agent loop, and the client for OpenRouter and OpenAI-compatible endpoints. The host tells it the endpoint, key, and Discussion prompt with `Configure`, and each request carries its model and effort. It runs on its own thread, and the host talks to it through the types in `assistant/src/lib.rs`. A chat answer can arrive in parts. It knows no sub-application, tool group, or tool by name; everything it offers the model comes from declared tool groups, including the host's own.
- `draw/` — the shared document model of the drawing surface: documents, steps, ink, text, the camera, and gestures. The host draws and previews with it; a sub-application that owns such documents commits and records with it. It has no session, history, registry, store, or preferences.
- `apps/<id>/` — one sub-application per folder: its crate, its `manifest.json`, and its own `AGENTS.md`. It depends on `protocol`, and on `draw` when it uses the drawing surface, never on the host or another sub-application.
- `apps/template/` — the smallest correct sub-application, hidden from people. New ones are copied from it.
- `apps/storyboard-generator/` — the Storyboard Generator: storyboards drawn on canvases.
- `apps/recipe-creation/` — Recipe Creation: recipes, lists of cells. It stores cells and declares them. The host draws and runs them.
- `apps/video-generator/` — the Video Generator: shots, to come; a scaffold copied from the template, which the Image and Video role session fills.
- `apps/image-generator/` — the Image Generator: layered images. It owns every pixel, layer, and step of history, composites the image, and declares a raster tab; the host draws it, previews strokes, and sends gestures. Its `.ipx` format is in its own AGENTS.md.
- `.project/scripts/check.sh` — the one check to run before calling work done (see Working rules).
- `media/` — video, sound, and encoding, shared by the workspaces that play and write them, as `draw/` is; never used by the host. The Image and Video role session writes it, to the interface in the lifecycle contract.
- `docs/tools.md` — the catalogue of every tool the assistant can be offered, written from the code by `UPDATE_TOOL_CATALOGUE=1 cargo test -p inpainter tool_catalogue`; a check fails while it is out of date, so regenerate it whenever a tool changes.
- `client/` — `storyboard-client`, the no-host check: it drives `storyboard-generator` through the protocol alone, including every tool, one undo per request, and `open` and `keep`, and compares every reply with an in-process session. Its coverage check fails when a hand action that changes a storyboard has no tool.

## Rules

- Every name a person reads, and the word for each thing in code, documents, and the assistant's tools, comes from Studio's conventions guide in the team repository, `.project/projects/spumcake-studio/documents/guides/conventions-guide.md`. A new name goes into the guide, with the owner's yes, before it is built. `host/tests/conventions.rs` holds the code to it.
- The host and the assistant know sub-applications only through their manifests and the protocol. Do not put a sub-application's name, label, tool group, tool, or rule in either.
- Every sub-application runs with no host, driven only through the protocol.
- A change to the protocol updates the **scaffolding** skill in the same commit.
- A skill is an agent skill, a `SKILL.md`. A workspace's tools are tools, never skills. A sub-application declares its tools in tool groups, with `tools`; before 2026-11-05 that message was `skills`, a stored spelling the protocol still reads. Do not add new uses of the old sense.
- The full lifecycle's shared contract, the lifecycle's `systems.md` in the team repository, says who owns what while the role sessions build it: items, fingerprints, `render`, `need`, `stale`, `pick`, and cells in any workspace.

## Working rules for agents

These hold for every agent building Studio, in every session, as the owner set on 2026-10-29.

- **Done means checked.** Before calling any work done, run `.project/scripts/check.sh`. It builds everything, runs the whole workspace's tests and the no-host check, and refuses files that were only reformatted. It must end with "All checks passed." Report its output, not a partial run such as `-p inpainter`.
- **Never run `cargo fmt` or `rustfmt`.** The code is not machine-formatted. Match the formatting around each edit by hand.
- **Fix your own mistakes without asking.** That covers a compile error in any crate, a failing or newly failing test, a boundary test, and stray changes. These are not questions for the owner.
- **A change you did not make is someone else's.** The owner or another agent may edit files while you work, this one included. Keep their changes, never revert them, reread any instructions that changed, and carry on.
- **Ask the owner only for two things.** The first is a decision that changes the product, meaning what a person sees or can do, or stored data, dependencies, or spending. The second is a contract in the version's `systems.md` that proves wrong. Decide everything else inside the approved tasks and systems, and record each departure as "As built" in `systems.md`.
- **Build a version through to the end.** Work through every step of the handover without stopping between them, and report once, when the whole build passes the check. A reviewer checks the build after that.
- **Never touch the owner's own data.** Tests and scenarios use scratch folders for the data folder and for projects, never `~/.local/share/inpainter` or `~/Inpainter`. Make no real model calls. Run screenshots only on their own virtual display, never on the owner's screen, and never script the desktop's file dialogs.
- **Do not commit or push** unless the owner asks.

## Skills

Developer skills for working on this code are in `skills/`, one folder each with a `SKILL.md`. Read the one that matches the task before starting.

- `skills/scaffolding/` — when a task creates a new sub-application, or brings an existing program into Spumcake Studio as one.

These are for agents developing Spumcake Studio. They are not the tool groups a sub-application declares to Studio's assistant through the protocol, nor the agent skills Studio's assistant loads.

## Commands

From the repository root: `cargo build`, `cargo test`, and `cargo run -p inpainter`. `cargo run -p inpainter` builds only the host, so run `cargo build` first whenever a sub-application changed; the host starts the sub-application programs already built in `target/`. `cargo run -p inpainter -- --app <id>` chooses which sub-application the host starts; `--app` may be given more than once, and every one named runs in the same window. Without `--app`, every installed sub-application starts. `target/debug/storyboard-client` runs the no-host check after a build. Run tests with `TMPDIR=$PWD/target/tmp` after `cargo build`: the host's checks start the built `storyboard-generator`, `template`, and `recipe-creation` programs and use a scripted stand-in for OpenRouter. The stand-in answers a chat in parts. Where prompts go is set in Settings, opened from the logo menu: endpoints, skills, and the assistant's levels live in `settings.json` in the data folder, and keys in the system keyring, or in a private `keys.json` there when no keyring answers or `INPAINTER_NO_KEYRING` is set, as tests and screenshot scenarios do. At the first start of 0.8, the `OPENROUTER_API_KEY` from the environment or `.env` becomes the "OpenRouter" endpoint's key, at `INPAINTER_OPENROUTER_URL` when that is set; after that the environment is not read. An OpenRouter endpoint with no key replies that no key is set. With `INPAINTER_TRACE=1`, the host writes every message to and from sub-applications to standard error. A project is a folder the person chooses, by default under `~/Inpainter/`. Each holds its activities as `.smx` files and Studio's own `.inpainter/` folder: `project.json` and one file per session in `sessions/`, with paths relative to the project. The data folder, `~/.local/share/inpainter/` on this computer, keeps `projects.json`, the list of projects and which session is current. Projects from before 0.7 move into folders of their own at the first start, and their old files are kept in `projects-before-0.7/`. The logo menu holds Settings and Quit. Each session remembers the level its composer chose. A skill that makes images, such as the "Image generation" skill the first start of 0.9 adds, runs as an Image cell through OpenRouter's image API; its images are kept in the project's `images` folder, named by paths relative to the project. Every image is a paid request on the person's key, so tests answer it with a stand-in.
