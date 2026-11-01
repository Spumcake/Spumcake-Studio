# Spumcake Studio

Spumcake Studio is the open-source production software of [Spumcake](https://spumcake.tv), an AI-native entertainment and technology company: a Codex for creatives. It is a computer graphics (CG) authoring application for creative professionals, from indie artists up, that standardizes their production pipeline and helps enthusiasts and professionals make an animated pilot in months instead of years.

A person and an assistant shape the staged intermediates of a production together, before anything goes to a final render. Studio opens on a conversation with the assistant inside a project, and the work of its sub-applications, such as sketching, image editing, and notebooks of cells, opens beside that conversation. It works fully on its own, with the person's own generation endpoints, such as OpenAI, fal.ai, or ComfyUI, or with the Spumcake platform. What Studio is in detail is in [docs/overview.md](docs/overview.md).

Its earlier name was Inpainter. The program and the data folders still use that name until a version moves them.

## Build and run

From this directory, with Rust 1.95, build the host and its sub-applications:

```
cargo build
```

The programs land in `target/debug`. Open the window:

```
cargo run -p inpainter
```

The host finds each sub-application by its `apps/<id>/manifest.json`. To add a sub-application, follow `skills/scaffolding/SKILL.md`.
