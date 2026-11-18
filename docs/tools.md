# Tool catalogue

Every tool Spumcake Studio's assistant can be offered, as the host and each workspace declare them. This file is written from the code by `UPDATE_TOOL_CATALOGUE=1 cargo test -p inpainter tool_catalogue`, and a check fails while it is out of date; do not edit it by hand. A workspace's tools also take `tab`, the tab they act on, which the host adds. Besides the tools below, each skill a person makes in Settings is offered as a tool of its own, in the host's "Your skills" group.

118 tools.

## The host

### Prompt (`prompt`)

Write a prompt and run it as a cell in the conversation, and find the project's work that is out of date.

- **`inpainter__prompt__run_text_prompt`**: Show a text prompt as a cell and run it. Takes `prompt` (string, required).
- **`inpainter__prompt__list_out_of_date`**: List every item of the project that is out of date, because something it was made from changed: each with its file, its id, its name, its workspace, and what changed, those made from others after the others. Takes nothing.
- **`inpainter__prompt__edit_image`**: Open the image an Image cell in the conversation shows as new work, in its own tab, ready to paint. The cell is unchanged. Takes `cell` (string).

## The host, in a project

### Project (`project`)

Read the project and the skills, look at its images, write by hand where the agent may, run Skills' scripts and requests on the user's yes, and research.

- **`inpainter__project__list_files`**: List a folder of the project, or of a skill's repository: each entry's name, whether it is a folder, and its size. Takes `path` (string).
- **`inpainter__project__read_file`**: Read a text file of the project, or of a skill's repository. A request's report gives its record as JSON. A long file is read in parts, from a line. Takes `path` (string, required), `from_line` (integer).
- **`inpainter__project__look_at_file`**: Look at an image of the project, or of a skill's repository: PNG, JPEG, WebP, or GIF. The image is shown to you, and the user sees it with its full path. Takes `path` (string, required).
- **`inpainter__project__write_file`**: Write a page of brief/, or add a new file to the agent's folder, named by the local time and what it is, with its line in the folder's index. Never anything else. Takes `folder` (string, required), `name` (string, required), `text` (string, required), `about` (string).
- **`inpainter__project__read_skill`**: Read a skill, unchanged, with its folder and its repository, which its relative links resolve in. Takes `name` (string, required).
- **`inpainter__project__run_command`**: Run a command in the project, such as a Spumcake-Skills script (request.py and pipeline.py may be named by their file name alone), a builder, or a script of the agent's own, after the user approves it. Its output is answered when it ends. A command that spends is refused: send it with send_request or rerun_step. Takes `command` (array, required), `why` (string, required).
- **`inpainter__project__send_request`**: Send one request to a model with request.py, from its spec: its plan, worked out offline, is shown to the user with its cost, its prompt as sent, and every reference in order, and it runs only on their yes, exactly as approved. Answers with what request.py tell says. Takes `spec` (string, required), `stand_in` (boolean).
- **`inpainter__project__rerun_step`**: Make a take of a pipeline's step with pipeline.py rerun: the first that is out of date, the first a change reaches, or the one named. Its plan is shown to the user and runs only on their yes. It stops after that one step; publishing the take comes first. Takes `pipeline` (string, required), `artifact` (string), `chain` (string), `fact` (string), `stand_in` (boolean), `step` (string).
- **`inpainter__project__web_search`**: Search the web, or its images, with Brave Search, for research such as a skill's references. All the searches one research request needs are asked for together: the user approves them with their cost first. Answers each search's results, with each one's title, address, and the page it is on. Takes `searches` (array, required), `why` (string, required).
- **`inpainter__project__fetch_page`**: Read a web page's text, to learn what an image found is, who made it, and the use its licence allows. Costs nothing and needs no approval. Takes `url` (string, required).
- **`inpainter__project__download_file`**: Download one file found in research, after the user approves it: the file, the page it is on, and its licence are shown. It waits in Studio's own folder until you file it into sources/ with send_request and a spec of request.py's research kind, whose outputs are the files and whose sources give each file's address, page, and the use allowed. Takes `licence` (string, required), `page` (string, required), `url` (string, required).
- **`inpainter__project__publish_take`**: Put a take into the chain: publish it as a canon artifact through the publish card, which shows the user the version it writes and what a commit takes along, and runs only on their yes. Read pipeline.py touches --artifact "<artifact>" --json first and tell the user what it reaches. A take made from inputs that are no longer canon is refused. Takes `artifact` (string, required), `take` (string, required), `from` (array), `with` (array).

## 3D Viewport

### 3D Viewport (`view`)

Look at a place's pre-vis in 3D: through its builder's cameras, or with a camera's published pre-vis render beside or over it.

- **`3d-viewport__view__list_views`**: Every view of the open place, and which is chosen. Takes nothing.
- **`3d-viewport__view__look_through`**: Choose a view of the camera list by its name, as list_views gives it. Takes `view` (string, required).
- **`3d-viewport__view__set_look`**: Draw the scene grey, colour-coded by kind, or simple (white faces and grey edges). Takes `look` (string, required).

## Image Generator

### Image Generator (`image`)

See the images in an Image Generator tab's gallery, arrange them on its filmstrip (show, add, duplicate, rename, reorder, and remove images), and manage the shown image's layers: add, rename, hide or show, reorder, and delete them; add and set texts, masks, and Curves and Color Grade adjustments.

- **`image-generator__image__look`**: See an image as it is composited now, scaled down to at most 1024 pixels on its longer side: the shown one, or another. Takes `image` (integer).
- **`image-generator__image__list_images`**: List the gallery's images in filmstrip order, with each one's position, name, size, and number of layers, and which is shown. Takes nothing.
- **`image-generator__image__show_image`**: Show an image, with its layers, as a click on its thumbnail does. Takes `image` (integer, required).
- **`image-generator__image__add_image`**: Add a new, empty 1920 × 1080 image, one transparent layer, at the end of the filmstrip, and show it. Returns its position and name. Takes `name` (string).
- **`image-generator__image__duplicate_image`**: Copy an image, with its layers, beside it, named "… copy", and show the copy. Returns its position and name. Takes `image` (integer, required).
- **`image-generator__image__rename_image`**: Rename an image. Takes `image` (integer, required), `name` (string, required).
- **`image-generator__image__move_image`**: Move an image along the filmstrip. Takes `image` (integer, required), `to` (integer, required).
- **`image-generator__image__remove_image`**: Remove an image from the gallery. The person can undo it from the filmstrip. The last image cannot be removed. Takes `image` (integer, required).
- **`image-generator__image__list_layers`**: List the image's size and every layer and group, top first, with ids, names, kinds, visibility, blend mode, opacity, fill, locks, masks, and each text's, curve's, or grade's settings. Takes nothing.
- **`image-generator__image__add_layer`**: Add a clear layer the image's size at the top of the image, or at the top of a group. Returns its id. Takes `name` (string, required), `group` (integer).
- **`image-generator__image__rename_layer`**: Rename a layer or group. Takes `layer` (integer, required), `name` (string, required).
- **`image-generator__image__set_visible`**: Show or hide a layer or group. Takes `layer` (integer, required), `visible` (boolean, required).
- **`image-generator__image__move_layer`**: Move a layer or group up or down among its siblings. Takes `layer` (integer, required), `to` (integer, required).
- **`image-generator__image__delete_layer`**: Delete a layer, or a group with everything in it. The image keeps at least one layer. Takes `layer` (integer, required).
- **`image-generator__image__add_text`**: Add a text layer that stays editable, its box's top-left corner at x, y in image pixels, above the active layer. Returns its id. Takes `text` (string, required), `x` (number, required), `y` (number, required), `style` (object), `width` (number).
- **`image-generator__image__set_text`**: Change a text layer's words, style, or box. Takes `layer` (integer, required), `box` (object), `style` (object), `text` (string).
- **`image-generator__image__add_mask`**: Put a white mask on a layer, text, adjustment, or group, which shows all of it. The person paints it to hide parts. Returns the mask's id. Takes `layer` (integer, required).
- **`image-generator__image__set_mask`**: Set a layer's mask: its feather in pixels, and invert: true flips it once, white for black. Takes `layer` (integer, required), `feather` (number), `invert` (boolean).
- **`image-generator__image__add_adjustment`**: Add a Curves or Color Grade layer, which changes everything beneath it within its group, above a layer or above the active one. Returns its id. Takes `kind` (string, required), `above` (integer).
- **`image-generator__image__set_curves`**: Set one curve of a Curves layer: rgb for all three channels, applied after each channel's own, or red, green, or blue. Takes `channel` (string, required), `layer` (integer, required), `points` (array, required).
- **`image-generator__image__set_grade`**: Set a Color Grade layer: exposure in stops from -3 to 3, and contrast, saturation, and temperature from -100 to 100, each 0 at rest. Leave out what stays. Takes `layer` (integer, required), `contrast` (number), `exposure` (number), `saturation` (number), `temperature` (number).

## Recipe Creation

No tools.

## Storyboard Generator

### Film and clips (`clips`)

Make and time the film's clips and its sound, and inside a clip its layers, keyframes, holds, labels, and onion skin, as the timeline and its pages do; export the animatic.

- **`storyboard-generator__clips__read_film`**: The film as data: where the person is, the storyboard's settings, the onion skin, the selection, and every clip with its timing, notes, items, layers, keys, and marks. Takes nothing.
- **`storyboard-generator__clips__add_clip`**: Add a clip, with one blank layer. Clips it lands on move later. Takes `length` (integer), `start` (integer).
- **`storyboard-generator__clips__duplicate_clip`**: Copy a clip, drawings and all, right after it. Takes `clip` (integer, required).
- **`storyboard-generator__clips__delete_clip`**: Take a clip off the film. Its place stays empty. Takes `clip` (integer, required).
- **`storyboard-generator__clips__retime_clip`**: Move a clip's start or end edge by a number of frames. Moving its end moves the clips after it. Takes `clip` (integer, required), `edge` (string, required), `frames` (integer, required).
- **`storyboard-generator__clips__move_clips`**: Move clips by a number of frames. Takes `clips` (array, required), `frames` (integer, required), `push` (boolean).
- **`storyboard-generator__clips__set_clip`**: Set a clip's name, shot number, notes, or animator, as its page in the panel does. Takes `clip` (integer, required), `action` (string), `animator` (string), `camera` (string), `dialogue` (string), `name` (string), `shot` (string).
- **`storyboard-generator__clips__select_clips`**: Select clips on the film, as clicking them does. Takes `clips` (array, required).
- **`storyboard-generator__clips__edit_clip`**: Open a clip on its sheet, to draw inside it, as Edit clip does. Takes `clip` (integer, required).
- **`storyboard-generator__clips__leave_clip`**: Go back from a clip to the film. Takes nothing.
- **`storyboard-generator__clips__go_to_frame`**: Move the playhead: inside a clip, to its frame; on the film, to the film's frame. Takes `frame` (integer, required).
- **`storyboard-generator__clips__select_layer`**: Make a layer of the open clip the one drawn on. Takes `layer` (integer, required).
- **`storyboard-generator__clips__select_frames`**: Select a block of frames on layers of the open clip, as dragging across them does. Takes `end` (integer, required), `start` (integer, required), `layers` (array).
- **`storyboard-generator__clips__add_layer`**: Add a layer, or a folder, above the active layer of the open clip. Takes `folder` (boolean).
- **`storyboard-generator__clips__delete_layer`**: Delete a layer of the open clip, or a folder with what is in it. A clip keeps one layer. Takes `layer` (integer, required).
- **`storyboard-generator__clips__move_layer`**: Move a layer to a position in the list, 0 at the top, and into a folder or out of one. Takes `layer` (integer, required), `position` (integer, required), `folder` (integer).
- **`storyboard-generator__clips__set_layer`**: Set a layer's name, colour, opacity, eye, lock, outline, or a folder's open state. Takes `layer` (integer, required), `color` (string), `locked` (boolean), `name` (string), `opacity` (number), `open` (boolean), `outline` (boolean), `visible` (boolean).
- **`storyboard-generator__clips__insert_frames`**: Insert frames after a frame on layers, as Insert frame does: their later keys move later and the clip grows. Takes `frame` (integer, required), `count` (integer), `layers` (array).
- **`storyboard-generator__clips__remove_frames`**: Remove frames from layers, as Remove frames does: keys there go and later ones move earlier. Takes `frame` (integer, required), `count` (integer), `layers` (array).
- **`storyboard-generator__clips__insert_keyframe`**: Make a frame of a layer a keyframe: a copy of what it showed, or blank. Takes `frame` (integer, required), `layer` (integer, required), `blank` (boolean).
- **`storyboard-generator__clips__clear_keyframe`**: Clear a keyframe: the one before holds on. Takes `frame` (integer, required), `layer` (integer, required).
- **`storyboard-generator__clips__move_frames`**: Move frames start..end of layers by a number of frames. Takes `end` (integer, required), `frames` (integer, required), `start` (integer, required), `layers` (array), `push` (boolean).
- **`storyboard-generator__clips__stretch_hold`**: Lengthen or shorten the hold of the keyframe at a frame, moving the layer's later keys with it. Takes `frame` (integer, required), `frames` (integer, required), `layer` (integer, required).
- **`storyboard-generator__clips__copy_frames`**: Copy frames start..end of layers, for paste_frames. Takes `end` (integer, required), `start` (integer, required), `layers` (array).
- **`storyboard-generator__clips__paste_frames`**: Paste the copied frames at a frame, replacing what is there. Takes `frame` (integer, required), `layers` (array).
- **`storyboard-generator__clips__reverse_frames`**: Reverse frames start..end of layers: their holds come in the opposite order. Takes `end` (integer, required), `start` (integer, required), `layers` (array).
- **`storyboard-generator__clips__add_mark`**: Add a label or a comment at a frame of a layer. Takes `frame` (integer, required), `kind` (string, required), `layer` (integer, required), `text` (string, required).
- **`storyboard-generator__clips__remove_mark`**: Remove a label or a comment. Takes `layer` (integer, required), `mark` (integer, required).
- **`storyboard-generator__clips__set_onion_skin`**: Turn onion skin on or off, and set how many keys before and after it shows, its starting opacity, how much it fades, and whether it counts keys only. A view aid: not an undo step. Takes `after` (integer), `before` (integer), `fade` (number), `keys_only` (boolean), `on` (boolean), `opacity` (number).
- **`storyboard-generator__clips__set_storyboard`**: Set the storyboard's title and scene, which the sheet's header shows, and the film's frame size and paper. Takes `height` (integer), `paper` (integer), `scene` (string), `title` (string), `width` (integer).
- **`storyboard-generator__clips__import_sound`**: Import a sound, or a video's sound, from a file and place it whole on a Sound track, a new one when none has room. A sound already in the project, such as a take in library/, is placed where it is; one from outside is filed into the project with request.py resource first, then placed. Takes `path` (string, required), `frame` (integer), `track` (integer).
- **`storyboard-generator__clips__add_sound_track`**: Add an empty Sound track under the others. Takes nothing.
- **`storyboard-generator__clips__rename_sound_track`**: Rename a Sound track. Takes `name` (string, required), `track` (integer, required).
- **`storyboard-generator__clips__delete_sound_track`**: Delete a Sound track and the sounds on it. Takes `track` (integer, required).
- **`storyboard-generator__clips__set_track`**: Lock or unlock a track, show or hide the Shots track, or mute or unmute a Sound track, as its header's toggles do. Takes `track` (any, required), `locked` (boolean), `muted` (boolean), `visible` (boolean).
- **`storyboard-generator__clips__select_sounds`**: Select sounds on the Sound tracks, as clicking them does. Takes `sounds` (array, required).
- **`storyboard-generator__clips__move_sound`**: Move a sound to a frame of the film, and to another Sound track. Takes `frame` (integer, required), `sound` (integer, required), `push` (boolean), `track` (integer).
- **`storyboard-generator__clips__trim_sound`**: Set where a sound starts on the film, where it starts inside its own sound, and its length, as dragging its edges and its page do. Takes `sound` (integer, required), `at` (integer), `from` (integer), `length` (integer).
- **`storyboard-generator__clips__set_sound_volume`**: Set a sound's volume, as its page does. Takes `sound` (integer, required), `volume` (number, required).
- **`storyboard-generator__clips__delete_sound`**: Take a sound off its track. Its file stays where it is in the project. Takes `sound` (integer, required).
- **`storyboard-generator__clips__publish_sound`**: Offer a placed sound on the publish card as canon, under the film and named after its file, as Publish sound does, so the animatic can use it. The user approves it on the publish card. Takes `sound` (integer, required).
- **`storyboard-generator__clips__set_listening_volume`**: Set how loud the film plays here, as the header's volume does. A listening aid: never in the export, and not an undo step. Takes `volume` (number, required).
- **`storyboard-generator__clips__read_sound`**: How loud the film's sound is on each frame from start to end, 0 to 1, as it plays: every unmuted Sound track mixed. Use it to time clips and keys to the sound. Takes `end` (integer, required), `start` (integer, required).
- **`storyboard-generator__clips__export_animatic`**: Export the animatic: the project's animatic step makes it again, as build-animatic does, after the timing is saved when it differs, and its take is offered for publishing. Returns the step at once; list_items says when it is done. Takes nothing.
- **`storyboard-generator__clips__list_items`**: Every item of the storyboard, by item reference: the cut, each clip's first and last frames and frames, and the animatic with whether it is exporting, done, or failed. Takes nothing.
- **`storyboard-generator__clips__insert_image_key`**: Make the open clip's active layer show an image of the project from the playhead: an image key, fitted to the clip's field, its drawing over it, as Insert image key… does. Takes `image` (string, required).
- **`storyboard-generator__clips__publish_hold`**: Publish the open clip's hold under the playhead, drawing and all, as its rough board, as Publish hold does, so the animatic can use it. The user approves it on the publish card. Takes nothing.
- **`storyboard-generator__clips__save_timing`**: Save the storyboard's timing as a new assembly and point the project's animatic step at it, as Save timing to the project does, so build-animatic assembles it. The user approves the step's edit. It refuses, saying why, when the length differs, the film has a gap, the bed is muted, or a hold's drawing is not published. Takes nothing.
- **`storyboard-generator__clips__follow_retime`**: Take the timing the project's animatic step now names, after it was retimed in another tool, as one undo step, as Follow does. Takes nothing.
- **`storyboard-generator__clips__render_again`**: Render an item again from what made it: the animatic's step runs again. Takes `item` (string, required).

### Read and look at a storyboard (`look`)

Read what a storyboard holds, as data, and look at an image of it as it is drawn.

- **`storyboard-generator__look__read_storyboard`**: Everything the storyboard holds, as JSON: canvases with their layers and strokes (ids, bounds, simplified points), objects front to back with their kind, name, bounds, transform, properties, palette, and words, the selection, the brush's colors, and the visible area. Takes `canvas` (integer), `points` (boolean).
- **`storyboard-generator__look__look_at`**: A PNG image of the storyboard as it is drawn: everything visible in a region, or around one canvas or object, or the whole storyboard. The result says the region and the scale in pixels per unit. Takes `object` (integer), `region` (object), `size` (integer), `visible_area` (boolean).

### Draw by hand (`draw`)

Draw as the person draws with the tools: freehand strokes, erasing, rectangles, ellipses, lines, text, and moving, scaling, or rotating objects.

- **`storyboard-generator__draw__draw_stroke`**: One freehand brush stroke along points, in a palette color, on a canvas. Without a canvas, the highlighted canvas is used; with no canvas highlighted, the stroke makes a new canvas, as it does by hand. Takes `points` (array, required), `canvas` (integer), `color` (integer).
- **`storyboard-generator__draw__erase`**: Erase along points on a canvas, as the eraser does: ink under the path disappears, but every stroke stays, so it can still be restyled. Takes `points` (array, required), `canvas` (integer), `size` (integer).
- **`storyboard-generator__draw__add_shape`**: A rectangle or ellipse filling the box between two corners, or a line from one point to the other, as the shape tools draw them. Takes `from` (array, required), `kind` (string, required), `to` (array, required), `constrain` (boolean), `properties` (object).
- **`storyboard-generator__draw__add_text`**: New text, typed with the text tool, its box's top-left corner at a point. Without a width the box fits the words; with one the words wrap. Use \n for new lines. Takes `at` (array, required), `text` (string, required), `properties` (object), `width` (number).
- **`storyboard-generator__draw__transform`**: Move, scale, or rotate objects, as dragging them or their handles does. Scale and rotation are about the objects' center, or about a point. Text is never rotated. Takes `about` (array), `move` (array), `objects` (array), `rotate` (number), `scale` (any).
- **`storyboard-generator__draw__set_text_box`**: Set a text box's width or height, as dragging its side handles does; the words re-wrap. Takes `object` (integer, required), `height` (number), `width` (number).

### Edit and arrange (`edit`)

Select, cut, copy, paste, delete, nudge, group, ungroup, order, align, rotate, and flip objects; add canvases and choose the one drawn on; and order, duplicate, or delete a canvas's color layers.

- **`storyboard-generator__edit__select`**: Change the selection as the select tool does: replace it, add or remove objects as Shift-clicking does, take everything an area touches as a drag does, or clear it. Takes `mode` (string, required), `area` (object), `objects` (array).
- **`storyboard-generator__edit__clipboard`**: Cut, copy, or paste whole objects, within Storyboard Generator. Takes `action` (string, required), `objects` (array).
- **`storyboard-generator__edit__delete`**: Delete objects, as the Delete key does. Takes `objects` (array).
- **`storyboard-generator__edit__nudge`**: Nudge objects one unit, or eight with large, per step, as the arrow keys do. Takes `direction` (string, required), `large` (boolean), `objects` (array), `steps` (integer).
- **`storyboard-generator__edit__group`**: Group two or more objects into one. Takes `objects` (array).
- **`storyboard-generator__edit__ungroup`**: Release the objects of the selected groups. Takes `objects` (array).
- **`storyboard-generator__edit__arrange`**: Bring objects forward or to the front, or send them backward or to the back. Takes `order` (string, required), `objects` (array).
- **`storyboard-generator__edit__align`**: Align two or more objects' left edges, centers, or right edges. Takes `edge` (string, required), `objects` (array).
- **`storyboard-generator__edit__turn`**: Rotate objects a quarter or half turn, or flip them, about their center. Text never turns. Takes `turn` (string, required), `objects` (array).
- **`storyboard-generator__edit__move_object`**: Move an object to a position among its siblings, as dragging its row in the layers list does. 0 is the front. Takes `object` (integer, required), `position` (integer, required).
- **`storyboard-generator__edit__set_active_canvas`**: Make a canvas the one strokes and erasing go on, as clicking it does. Takes `canvas` (integer, required).
- **`storyboard-generator__edit__duplicate_colour`**: Copy one of a drawing's colours, with its ink, below it. Takes `canvas` (integer, required), `colour` (integer, required).
- **`storyboard-generator__edit__delete_colour`**: Delete one of a drawing's colours and its ink. A drawing keeps at least one colour. Takes `canvas` (integer, required), `colour` (integer, required).
- **`storyboard-generator__edit__move_colour`**: Move one of a drawing's colours to a position; 0 paints on top. Takes `canvas` (integer, required), `colour` (integer, required), `position` (integer, required).

### Restyle (`style`)

Change the properties of objects, text, and palette colors, as the properties panel and the layers list do, and change a text's words.

- **`storyboard-generator__style__set_object_properties`**: Set properties of objects. Each object takes the properties it has; the result says which took each one. Takes `properties` (object, required), `objects` (array).
- **`storyboard-generator__style__set_color_properties`**: Set properties of one palette color of an object: a canvas's layer, or a shape's or text's color. Takes `color` (integer, required), `object` (integer, required), `properties` (object, required).
- **`storyboard-generator__style__set_text`**: Replace a text object's words, as editing it in place does. Empty words delete it. Takes `object` (integer, required), `text` (string, required).

## Video Generator

### Video Generator (`takes`)

See the takes in a Video Generator tab's gallery, open a video file into it as a take, show a take at a frame, and show the takes alone, as frame strips, or two side by side.

- **`video-generator__takes__list_takes`**: List the gallery's takes in filmstrip order, with each one's position, name, file, size, frames, frames per second, and length in seconds, and which is shown, the view, and the frame shown. Takes nothing.
- **`video-generator__takes__open_video`**: Add a video file to the end of the filmstrip as a take, linked and never copied, and show it. Returns its position, name, and length. Takes `path` (string, required), `name` (string).
- **`video-generator__takes__show_take`**: Show a take, as a click on its thumbnail does, at a frame. Takes `take` (integer, required), `frame` (integer).
- **`video-generator__takes__set_view`**: Show the shown take alone, every take as frame strips, or the shown take side by side with another at the same frame. Takes `view` (string, required), `with` (integer).
