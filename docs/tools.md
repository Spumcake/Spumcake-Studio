# Tool catalogue

Every tool Spumcake Studio's assistant can be offered, as the host and each workspace declare them. This file is written from the code by `UPDATE_TOOL_CATALOGUE=1 cargo test -p inpainter tool_catalogue`, and a check fails while it is out of date; do not edit it by hand. A workspace's tools also take `tab`, the tab they act on, which the host adds. Besides the tools below, each skill a person makes in Settings is offered as a tool of its own, in the host's "Your skills" group.

80 tools.

## The host

### Prompt (`prompt`)

Write a prompt and run it as a cell in the conversation, and find the project's work that is out of date.

- **`inpainter__prompt__run_text_prompt`**: Show a text prompt as a cell and run it. Takes `prompt` (string, required).
- **`inpainter__prompt__list_out_of_date`**: List every item of the project that is out of date, because something it was made from changed: each with its file, its id, its name, its workspace, and what changed, those made from others after the others. Takes nothing.
- **`inpainter__prompt__edit_image`**: Open the image an Image cell in the conversation shows as new work, in its own tab, ready to paint. The cell is unchanged. Takes `cell` (string).

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

Make and time the film's clips, and inside a clip its layers, keyframes, holds, labels, and onion skin, as the timeline and its pages do.

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

### Tab name (`tab`)

Rename this application's tab.

- **`video-generator__tab__rename_tab`**: Set the tab's label. Takes `label` (string, required).
