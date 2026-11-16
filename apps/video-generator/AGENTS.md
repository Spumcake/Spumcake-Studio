# Video Generator — sub-application

The Video Generator is the Image Generator for video, as the owner set on 2026-11-16: a gallery of takes per tab, a filmstrip of takes to open, duplicate, rename, delete, and reorder, each a link to a video file that is never copied or written; the shown take at a frame the keys step; two takes side by side at the same frame, which at 100% is the before/after crop; every take as a row of eight frames, the frame strips; undo for the gallery and the view; and autosave in the project as a `.vgx` file. The host draws the working area's image and the filmstrip as a `raster` tab and sends each click on the strip, the keys, and the Move tool's drag. The contract is milestone 1 of the Image and Video build plan, "The Video Generator's gallery and `.vgx`", in the team repository; making takes, playing them with their sound, the wipe over the animatic, notes, and the mix come with its later milestones.

## Rules for this sub-application

- It is its own program. Its crate depends on `protocol`, `serde`, `serde_json`, `media` (decoding video, shared with the Storyboard Generator), `image`, and `layers` (linked files and laying out takes, shared with the Image Generator), as Studio 0.15's plan approved, and nothing else, never on the host or on another sub-application.
- Everything the host shows for it is declared through the protocol: its tabs, toolbar, menu, keys, the filmstrip, and the raster contents. Nothing about it is hard-coded in the host.
- It owns every tab's gallery, its history, its file, and every frame it decodes. The host only draws the image and thumbnails this program writes.
- A take is a link to its file, `layers::link::Link`: the path as the `.vgx` names it and the file's sha256. The file is read, never written, moved, or copied.
- The gallery has one history. Adding, duplicating, removing, renaming, and moving takes, and changing the view (one take, side by side, or frame strips), are its steps. Choosing a take on its own, the frame, and the zoom are not.
- It runs with no host, driven only through its standard input and standard output.
- It speaks the protocol defined in `protocol/` and nothing else. When it needs something the protocol cannot say, change the protocol, the host, and the scaffolding skill together. Do not work around the protocol.
- `manifest.json` registers it with the host. Its id, folder, crate, and program names are the same.

## Layout

- `src/gallery.rs` — one tab's gallery: its takes in order, each with its link, its file, and what `media::Info` says it is; the take shown; the view; and the history, which holds a removed take until it is put back. Side by side names its other take by a number unique in the tab, so it holds through reorders, and shows one take while that take is gone.
- `src/view.rs` — what the working area shows, as a `Scene`: where each frame is drawn and at what size it is decoded, laid out with `layers::layout`; and drawing it into one image, with `layers::layout::place` where a frame is drawn at its decoded size, and scaled smoothly within its half where it is zoomed.
- `src/decode.rs` — frames decoded off the message loop: the frame shown, one or two, from a `media::Seeker` for each file, the newest request winning; the frame strips and the filmstrip's thumbnails from one thread of its own, thumbnails first, then the strips last asked for, in order.
- `src/session.rs` — the tabs, the protocol messages, the filmstrip's commands, the frame, the zoom and its drag, files and autosave, and the image and thumbnails written for the host.
- `src/declare.rs` — the toolbar, the menu, the keys, the filmstrip, and the tab list.
- `src/assist.rs` — the assistant's "Video Generator" tool group.
- `src/files.rs` — the `.vgx` file and whole-file writes.

## What it writes for the host

The loop in `main.rs` wakes after 4 ms without a line while frames are being decoded, to take them in as they come, and after 250 ms otherwise, for `tick`, which writes a project's changes that are due. The host's `shown`, which revision it drew, is dropped as it is read, as in the Image Generator, until playback needs it.

The tab's image is the working area's own size, as `viewport` and each command report it, made even, and the host's camera is put on it whole, origin `[0, 0]` and zoom 1, with the first view and again whenever the area changes. A camera the host moves itself is left as it is: its zoom only scales the image, until a raster tab can own its camera. The zoom menu (Fit, 50%, 100%, 200%) and the Move tool's drag, `drag_part` from the press, choose what the image shows instead, so a 4K take is never decoded whole to be shown small: each frame is decoded through `media` at the size it is drawn, never larger than its file. Fitted, a take is as large as the area less 40 pixels around, and never larger than its own pixels. Zoomed, it is drawn at that zoom about a point kept in the middle, as far as its edges let it be, and the drag moves that point. Side by side, the area is halved with 16 between, and at a zoom both takes are drawn at the wider one's scale about the same point, so they show the same region of each: at 100%, the larger take at its own pixels, which is the before/after crop. Frame strips put every take's row, in filmstrip order, in a grid of eight columns with 8 between, each frame at the end of an eighth of the take's length, so the last is its last frame.

After every change it sends `raster`, with the filmstrip in `film` and the Move tool. The image is `<tab>-view.rgba`, written by `media::write_frame`, in a folder of the session's own in memory, `$XDG_RUNTIME_DIR/inpainter/video-generator/<process>-<number>/`, when the system has that folder, as it is written for every frame stepped, read by the host, and never kept; it stays there when a project is named, goes when the program ends, and a folder left by a program no longer running is removed when one starts. Without it, the image is in `<project>/.spumcake/cache/video-generator/`, or a scratch folder under the system's temporary folder with no project. It is width × height × 4 bytes, RGBA, rows top to bottom, on white, replaced whole by writing `<name>.partial` and renaming it. A frame not yet decoded keeps the image last written, with its revision, until every frame of the new view has come, so stepping never shows half an image; frame strips are drawn as their frames come. A take whose file is missing or unreadable is drawn as a grey box. Each file's filmstrip thumbnail is its first frame at 44 pixels tall and no more than 176 wide, as the Image Generator's, `film-<number>.rgba` beside the image, made once for each file, so a duplicate shares it. Revisions start from the time the program started and rise with every file written.

The keys step the frame: Left and Right one frame, Shift with them one second of the shown take, Home and End its first and last; `seek`, the host's transport row's when it comes, shows a frame. Each tab keeps one frame for all its takes; a shorter take shows its last. The toolbar's grid button turns the frame strips on and off.

The filmstrip's actions are `film_focus`, `film_select` (`value`: a position, and `extend` set for a Ctrl- or Shift-click, which shows that take beside the one shown; a plain click shows a take alone), `film_reorder` (`from`, `to`), the + tile's `film_open`, and each thumbnail's `film_duplicate`, `film_rename`, and `film_delete` (`take`: a position). Rename is a row with `rename` set: the host renames in place and sends the name as `value`. A `film_rename` with no `value` tells the person to ask the assistant, since the protocol's questions have no text field.

The assistant's "Video Generator" tool group, `takes`, has `list_takes` (each take's position, name, file, size, frames, frames per second, and length, which is shown, the view, and the frame), `open_video` (a video file by its path, absolute or in the project, added at the end and shown), `show_take` (a take by its position, at a frame), and `set_view` (`single`, `strips`, or `side_by_side` `with` another take). Opening a take and changing the view are steps of the gallery's history, each call its own.

## The `.vgx` file

A tab's gallery in a project is one `.vgx` file in the project's folder, written by itself about a second after a change, when the program is idle, as the Image Generator's `.ipx` files are, and before its tab closes, a `keep` is answered, or the host quits. Outside a project, Save and Save As write it where the person chooses. Inside a project, the take shown and the frame are kept too, so they change the file; outside one, only the gallery's steps ask before a close. It holds no video, and no prompt, report, or record of what made a take: those are read from the project's own records.

Names follow the conventions guide, through `protocol::untitled` and `protocol::free_name`. A new tab is "Untitled Video", or the first free name after it among the open tabs and the `.vgx` files in the project's folder, and a tab never written goes to a file named the same way. A take is named after its file, free among the gallery's takes, and a copy is "<name> copy", numbered the same way when taken.

1. The five bytes `VGX1` and a newline.
2. One line of compact JSON, ended by a newline:
   - `version`: 1;
   - `shown`: the position of the take shown, from 0;
   - `takes`: every take, in filmstrip order, each with `name` and `link`: `path`, relative to the project's folder with `/` between folders when the file is inside it, or else absolute, and `sha256`, the file's in lowercase hex, as Spumcake-Skills records it;
   - `view`: `{"mode": "single"}`, `{"mode": "strips"}`, or `{"mode": "side", "with": n}`, side by side with the take at position `n`;
   - `frame`: the frame shown, from 0.

A relative path is read from the project's folder, or, with no project, from the `.vgx` file's own. A take whose file is gone, or no longer reads as a video, keeps its place and its link and is drawn empty.

## Checks

`cargo test -p video-generator` runs the unit tests and `tests/session.rs`, which makes two short videos with `media::Encoder` in a scratch project and starts the built program, with `INPAINTER_NO_SOUND` set, driving it through protocol lines alone: the takes, their thumbnails, the keys, side by side, zoomed, and the frame strips, each read from the image's pixels; rename, reorder, delete, and the view undone and redone; the `.vgx` reopened by a second program as it was left; the videos' sha256 unchanged; and the assistant's tools.
