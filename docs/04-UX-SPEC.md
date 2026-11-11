# 04 — Interaction and UI Specification

Exact behavior of every interaction. This document exists because "drag a material onto an object" contains a dozen decisions that determine whether the application feels professional or broken, and none of them are guessable.

---

## 1. Window layout

### 1.1 Default arrangement

```
+-----------------------------------------------------------------------------+
| (o) Tequila  File Edit View Workspace Help   Untitled *   lib con ai | save Import [Render] |
+------------+--------------------------------------------+-------------+-----+
| Materials x| [o Solid  studio light] [frame | shadings | dn pause | shot] | Material x | Sce |
| drag a ... | +----+                       (HUD, H)       | what it's for| Mat |
| [search  ] | |move|                                     |-------------| --- |
| All|Fav|.. | |rot |                                     | [preview]   | Env |
| [category] | |scal|          the picture                | fields ...  | Lgt |
| +--+ +--+  | |----|                                     |             | Cam |
| |  | |  |  | |spc |                                     |             | Img |
| +--+ +--+  | |snap|                                     |             | --- |
| name name  | +----+   (axis legend)                      |             | Std |
|            |                                            |             | Scr |
|            |                                            |             |  ?  |
+------------+--------------------------------------------+-------------+-----+
| Part: Frame - status message                         Graphics card - 792 x 830 |
+-----------------------------------------------------------------------------+
```

- **Top bar**: the mark and the menus on the left, the document's name in the middle (a dot when it has unsaved changes), and on the right the panel toggles (Library, Console, Assistant), Save, Import and the one filled button in the window, **Render**. While a render runs, the button shows its progress and opens it.
- **The viewport** keeps the panel's full height. Its controls float over it on dark islands (§2.2) instead of a toolbar across the top.
- **The inspector rail** runs down the window's right edge, one icon per Project tab in three groups: the scene and its surfaces (Scene, Material); light, lens and film (Environment, Lighting, Camera, Image); versions and automation (Studios, Scripting). Help sits at the bottom. A tab opens the inspector at that page; the tab already showing closes it. The rail is always there, so the inspector is one click from open and from closed.
- **The inspector** sits against its rail: the tab's name and what it is for, then its page. The Assistant opens between it and the viewport. *As built, November 6* (`app/src/assistant.rs`): while a request runs, the line above the prompt shows a spinner, what it is doing (*Loading the model*, *Reading the request*, *Thinking*, *Writing*, *Calling* a tool, *Carrying out its steps*, *Looking at the scene*), a running count of the tokens written and the step, and on the right how long it has been working, where **Undo request** sits otherwise. Reasoning streams in dimmed and italic under a **Thinking…** heading that folds away; a finished reply's reasoning stays folded as *Thought for 42 s*. Problems are red; notes about the model, once per conversation, are dimmed italic. See `docs/ASSISTANT.md`.
- **The Snapshots page** (the Studios page until November 8) lists the project's Snapshots, each as a picture of its shot with its name, what it holds and its buttons. Click the picture to apply the Snapshot; drag it, or its grip, to move the Snapshot in the list. *As built, November 5:* while a Snapshot is dragged it is lifted (tinted), an accent line with a dot at each end marks the gap it would land in, and a card of its picture and name follows the pointer; letting go moves it as one undo step, Escape cancels, and near the panel's top or bottom the list scrolls. Up and Down stay beside the name for the keyboard. A picture out of date - the scene edited under it - stays until it is rendered again, with a small spinner in its corner meanwhile; one not made yet is a grey card with a spinner. The pictures are saved with the project (`docs/05` M6, Studios).
- **The library** is on the left: search, shelves (All, Favorites, Recent, In project), category, and tiles that fill the panel's width. Preview size and texture-pack import are in its ⋯ menu.
- **The status line** spans the window: the selection and the last message on the left, the viewport's engine and size on the right.
- `Space` toggles the Project window. `M` toggles the Library. `Tab` toggles all panels (full-viewport mode); the viewport's islands stay.
- The Material Graph, the Render window, Shortcuts and Connect an AI open as windows of their own (`app/src/windows.rs`), not panels drawn inside the main window. They can be moved off it, onto another display, and sized past it.
  - The first time, each opens centred on the main window. After that it reopens where it was left, at the size it was left.
  - Each is created hidden and shown once it is in place, so it does not appear in one spot and jump to another.
  - Their title bars, and the main window's, follow the interface's light or dark mode rather than the desktop's.
  - Undo, redo and save work from any of them.
  - Where a platform cannot open another window, they are drawn inside the main window instead.
  - Two short prompts stay inside the main window, as dialogs belonging to it: recovering unsaved work at start-up, and the report after a model is updated.

*As built, November 11, from a design pass over the menus:*
- **File, Edit, View**, the order every menu bar has them in; Edit came after View.
- **File** begins with the project - New, Open, Open recent, Save, Save as - then what comes in (the model, its up axis and reloading it; then texture packs, Substance materials, material files and a new library material), then **Export**, a menu of its five. It began with six kinds of import, and New, Open and Save were half way down.
- **Edit** has what the keys and the right-click menu did and no menu said: Group, Delete, Select all, Deselect all, Hide the selection, Isolate the selection, Show everything, beside Undo, Redo, Duplicate, Pattern and Preferences.
- **View** ticks each panel that shows - Material library, Inspector, Console, Full viewport - then what is drawn over the view. The library's was a button among the tick boxes ("Toggle material library"), the console had none, and Model edges stood apart at the top.
- **Keys are shown at the right of every command that has one**, from the table Preferences › Keyboard changes; some were in tooltips and two were written in ("P", "F5") and would not have followed a change.
- **One place for each setting where the second was only a duplicate:** View › Screenshot folder… is gone, Preferences › Folders having it (the Screenshot's tooltip says where it saves and where to change that). File › Up axis of imports stays beside Import, where it is wanted, as well as in Preferences.
- Removing a graph's hold on a parameter, and a gradient's colour, are "×" as everywhere else; they were the letter x.

### 1.2 Look and identity

Tequila's look is its own, not KeyShot's or Blender's: friendly but professional, in light or dark, in the person's choice of theme (`app/src/theme.rs`).

- **Appearance** (the sun or moon at the top right): Light, Dark or Auto (the desktop's); one of six themes; and a size, Compact, Regular or Large. Kept in `appearance.txt` beside the user's library. Every colour is read from the current palette, so all of it changes together: panels, viewport tools, the node editor, the console, the Render window.
- **Themes**: Graphite (neutral greys, teal), Ocean (blue), Orchid (violet), Ember (coral), Meadow (green), Rose (pink). Each has a light and a dark palette made for it. Every one is tested for contrast: body text at 7:1, secondary text and the accent's text at 4.5:1 (WCAG).
- **Graphite is the neutral one**, with equal channels in every grey. A tinted surround tints the render, so Graphite is the default and the picker says so. The others trade a little neutrality for character. The viewport itself is always the render's own picture.
- **Layout**: the frame (top bar, rail, status line) is a step off the panels; groups of controls sit on cards; floating windows and menus one step further.
- **The accent** means *active*: the selected part's outline, the chosen tab, a toggle that is on, a ticked box, a field being dragged. Apart from those, it appears only on the brand mark and the Render button.
- **Page hues**: each Inspector page has its own colour on its rail icon and its heading badge. Scene is blue, Material pink, Environment green, Lighting amber, Camera violet, Image orange, Studios sky and Scripting slate. A page is found by colour as much as by shape.
- **Busy amber** means work in progress somewhere else: a render converging, a script driving the session.
- **First run**: the start screen greets the person at launch (and is under Help › Start screen): a new project, one to open or a model to import, the projects opened lately, and the finished scenes to start from (`app/src/start_screen.rs`). A welcome card over the viewport with three steps (import, dress, render) came before it; it was taken out on November 9, the start screen saying the same and more.
- **Empty states** say what to do: a page with nothing to show has an icon in its colour, a title, and the next step. The library's shelves (no favourites yet, nothing recent, a search with no results) do the same.
- **Tooltips** have three parts: what the control is, in bold; a line saying more; and its keys as key caps. A greyed-out control's tooltip says why it is greyed out. The Shortcuts window and the node editor's sidebar draw keys the same way.
- **Choices with consequences are cards**: the lighting presets each show an icon in their own colour, what the preset is for, and how fast it renders as five rising bars. The chosen card is outlined in the accent with a ticked radio.
- **Motion**: hover states ease in and out over a tenth of a second. Nothing else in the interface moves on its own.
- **Type**: Segoe UI on Windows, San Francisco on a Mac, with egui's fonts behind it for missing characters. Headings use the semibold face rather than a larger size.
- **Icons** are [Lucide](https://lucide.dev)'s (ISC licence, `scripts/notices/lucide`), from its font (`app/assets/fonts/lucide.ttf`). They are one consistent set on a 24-unit grid with round strokes, rasterised by the font renderer, so they stay crisp at any size and scale, in whatever colour the state needs. Tequila draws its own where no set has the right picture (`app/src/icons.rs`): the brand mark, and the four viewport shadings as spheres (wires; matte; glossy; glossy in its own shadow). A test checks every glyph is in the font. Each icon also has a drawn fallback for contexts without the font. The window's icon is the brand mark, rasterised at start-up.
- **Number fields** (`app/src/field.rs`) replace egui's slider. Each is one control: the name on the left, the value on the right, the proportion of the range as a fill behind them. Drag across it to change it (Shift for fine), or click it to type a number, a unit or an `=` expression. A field with a unit takes a length in any other, so the focal length accepts `2in`.
- **Colour fields** (`app/src/color_picker.rs`) have the same shape: the name on the left, the hex and a swatch on the right, and the picker below on a click. An empty texture slot is the outline of a field. In the Material page every row is the same width, with its texture button in a column at the right, so the fields line up down the panel.

### 1.3 Sizing and density

Target a professional-tool density: 24 px row height at 100% scale, 11–12 px UI text, full DPI scaling support (100–300%). Every numeric field supports drag-to-scrub, type-to-enter, arrow-key increment (Shift = ×10, Ctrl = ×0.1), unit suffixes (`85mm`, `2.5in`), and simple expression entry (`= 36/1.5`).

---

## 2. Real-time View

### 2.0 Shading

The viewport has Blender's four shadings, chosen on the island at the viewport's top right. **Z** steps through them and **D** toggles Rendered against the last of the other three (`app/src/view_mode.rs`). The first three are drawn by the graphics card from the scene as it stands (`app/src/solid_gpu.rs`): they are instant, and the render thread sleeps.

- **Wireframe**: every edge of every mesh over a dark ground. The edges behind the model show faint, so its structure reads through itself. Selected parts are drawn in the accent colour.
- **Solid**: form and colour under a flat studio rig. A metal shows as the colour it is seen to be (its measured reflectance times its tint), not its tint alone.
- **Material Preview**: the materials lit by the scene's own environment, with the backdrop the render would show behind them.
  - Lighting is the split-sum approximation: a mipmapped radiance map for reflections that blur with roughness, and nine spherical-harmonic coefficients for the diffuse light.
  - It shows the clear coat, and lets the environment through transmissive parts.
  - The result goes through the render's own exposure and display transform.
  - It casts no shadows on the model itself; a metal or a gloss under the studio's softboxes looks as it will in the render.
  - On the floor, when the render has one, it lays a soft ground shadow: a tight dark band where the model meets the floor, and a broad soft one leaning away from the environment's light. The model is drawn from above into a map that is blurred once per change of geometry or environment, so orbiting and editing materials cost nothing.
- **Rendered**: the path tracer, Tequila's own or Cycles.

On a machine without usable OpenGL, the first three show the CPU's Solid view.

### 2.1 HUD

Top-right, semi-transparent, toggleable (`H`):
- Accumulated samples per pixel.
- Elapsed render time since last restart.
- **Percentage of pixels converged** (from adaptive sampling, §01 §4.4) — a better progress signal than sample count and worth showing prominently.
- Triangle count / NURBS-surface count.
- Active backend (CPU/GPU) and device name.
- Resolution and whether the view is at reduced interactive resolution.
- Memory: geometry / textures / BVH / buffers.
- *As built, November 9:* the device line names what made the picture: Tequila or Cycles, on the processor (by name, "Apple M3 Max") or the graphics card; a Cycles that would not start and fell back says Tequila (`DisplayImage::engine`). Cycles' frames show their samples without a converged share, which Cycles doesn't count. Solid, Wireframe and Material Preview, which the graphics card draws, show the view, the triangles and the card as OpenGL names it, with no samples to count. Under every view, the speed and the memory (`app/src/readout.rs`): the frames drawn in the last second and the slowest of them while the view moves (dragged, scrolled, the 3D mouse, a camera move), "idle" while it is still, since a still window draws only when asked and a few frames a second then read as a slow computer; and the memory Tequila holds of what the computer has, as its own monitor counts it - the footprint Activity Monitor shows on a Mac, private bytes on Windows, the resident set on Linux - read every half second. While shown, the statistics are drawn again once a second, so the memory stays current. Scripts show them with `{"op":"ui","statistics":true}`. Memory by kind (geometry, textures, BVH, buffers) is not broken out yet.

*As built, November 11, from a pass over every screen and a measurement of each view idle and orbiting* (M3 Max; the Headphones sample and a 1.07-million-triangle STEP model): the window idles at 3-6 frames a second and 3-4% of a core in every view and with every tab, and orbits at the screen's 120 frames a second in Solid and Material Preview with frames of 0.2-0.6 ms. What was wrong was in what the screen said and drew:
- **The frame rate counted still frames with a move's.** It was the frames of the last second, and for the first second of every orbit most of those were the still window's few: "13 fps, slowest 185 ms" on a view drawing 120. It is the rate of the move's own frames now, from when the move began (the first after a quarter-second pause) to when it last moved, and it stays the move's while still frames follow it (`readout.rs`, `the_frame_rate_is_the_moving_frames_alone`).
- **A window the desktop was not showing drew as fast as it could.** Covered or on another space it is not held to the screen's rate, and an orbit kept up there ran at 518 frames a second, one and a half cores. A frame now starts no sooner than 4 ms after the last (`frame_pause`), which no screen's own rate reaches.
- **The viewport's chip ran under the tools.** In a narrow view or the large interface its second half ("environment light, no shadows") was drawn under the island at the top right; it is left out where there is no room (`chip_has_room`).
- **The start screen's samples ran over its footer** on a 1024x640 window: the banner kept its depth and the lists got what was left. The banner now leaves room for the three samples at their smallest, their rows are spaced exactly, and they scroll where a window is shorter still (`hero_height`, `the_samples_fit_under_the_banner_on_a_small_window`).

### 2.2 Overlays

- **Islands**: the viewport's tools float over it on dark translucent plates, and take their own clicks and drags (a drag that starts on one never turns the camera).
  - Top left, a chip naming what the view shows. A hollow dot marks a preview, not the render, so a material is not judged by it. In Rendered, the dot is amber while the image converges, teal when it has settled and grey when paused, with a thin progress line under it.
  - Top right: frame all and frame selection, the composition guides, the four shadings, the viewport's denoise switch and pause, and Screenshot.
  - *As built, November 8:* the denoise switch (Lucide's wand-sparkles, lit while on) denoises the Rendered view or shows its raw samples, apart from the final render's Denoise in the Render window and Image › Denoise; strength, cadence and guides are shared. It is the render settings' `preview_denoise`, saved with the project and a step to undo; a project from before takes the render's `denoise`, and a snapshot leaves it alone. Switching it keeps the samples: the picture showing is denoised or not again at once (`differs_only_in_denoise` in `orchestrator.rs`, test `switching_the_viewports_denoising_keeps_the_samples`). The Screenshot button (Lucide's camera), **P**, and View › Screenshot save the viewport's picture as a PNG: developed and graded as shown, at the preview's own size, still refining or not, without the statistics, the gizmo, the outline or the grid - the renderer's image with what the outline drew over put back (`DisplayImage::without_outline`), or the graphics card's Solid read back as a frame draws it (`SolidView::take_capture`). It goes to `Pictures/Tequila Screenshots` (a portable copy's `data/Tequila Screenshots`), or the folder View › Screenshot folder… chose, kept in `screenshots.txt` beside the other settings. The name is the project's, or Untitled, with the date and time, and a second in the same second is `(2)`: never over a file (`app/src/screenshot.rs`). A notice names the file with an **Open folder** link, which opens the folder on it; a notice with a link stays six seconds, and while pointed at.
  - *As built, November 9:* the preview is held to the final render's pixels. It has the viewport's own pixels (a Retina screen's, or a screen set above 100%), but no more of them than the Render settings' size has, in the viewport's shape, and is drawn stretched over the rest (`preview_pixels` in `main.rs`; the Rendered view and Material Preview's path-traced frames, Cycles' and Tequila's renderer's alike; the graphics card's Solid draws at the screen's size, as it costs nothing). On a 27-inch Retina screen the viewport was 5120 x 2880 pixels, traced and denoised, for a 1920 x 1080 render: on the starter scene a clean picture took 8.9 s after a move and 9.7 s after letting go, orbiting 12.3 pictures a second; held to 1920 x 1080, 1.2 s, 2.7 s and 26.6 (`retina_viewport_benchmark`, an M3 Max). A viewport with fewer pixels than the render keeps its own; changing the render's size in the Render window resizes the preview. The statistics' size is the preview's. Test: `the_preview_has_no_more_pixels_than_the_render`.
  - Down the left: Move, Rotate, Scale (dimmed with nothing selected), World/Local space, and the snapping menu.
- **Axis legend** (bottom-left), toggleable. Its backing disc is faint until the pointer reaches it.
- **Ground grid** with real-world spacing labels when enabled in the Camera tab.
- **Selection outline**: a crisp 2 px outline in an accent colour, drawn as a post-process from the instance-ID buffer (not a wireframe overlay — outlines read better and cost nothing since the ID buffer already exists).
- **Toast notifications**, bottom-centre, auto-dismissing: "Denoising", "Rebuilding BVH", "Re-tessellating", "GPU memory exceeded — fell back to CPU", "Feature X unsupported on GPU backend". Never a modal dialog for these.
- **Safe-frame / aspect guides** when the render aspect differs from the viewport.
- **Composition guides** (*as built, November 9*): View › Composition guides, and the grid button beside the framing buttons top right, choose any number of the lines photographers compose by: the **rule of thirds**; the **golden section** (the phi grid, at 0.382 and 0.618, 1/φ² from each edge); the **golden spiral** through the golden rectangle's squares, winding to any of the four corners, and flipped so its first square spans the frame's long side rather than its short one; the **golden triangle** (one diagonal, and a line from each other corner meeting it square); the **harmonious triangles** (the same lines carried on to the frame's edge, as Blender's harmony triangles); the **diagonals**, baroque (rising to the right) and sinister (falling); **dynamic symmetry** (both diagonals and their four reciprocals, the frame's armature); **45° diagonals** from each corner; the **centre cross**; the **pyramid**, point up or down; sixteen **radial** lines from the centre; and the **safe areas**, action safe at 93% and title safe at 90%. Under a chosen one-sided guide its ways round are small pictures of it at the render's shape, to click. **O** shows or hides the guides chosen (the rule of thirds, the first time), and **Shift+O** turns the one-sided ones, as a photo editor's crop overlay does: the spiral clockwise to the next corner and, after the fourth, flipped; the triangles to the other diagonal; the pyramid over. They are drawn in the render's own frame - the render frame, or with it off, where the render falls in the view (a perspective camera keeps its width and a shift lens moves it up or down; an orthographic one keeps its height) - so they divide the final image, and a line square to a diagonal is square in it whatever its shape. A light line, 1 point and 60% opaque, on a dark edge 3 points wide at 25%, so they read on a white studio and a black one; over the picture and the grid, under the gizmos and the statistics. Never in a Screenshot (the picture alone), a render or Presentation mode. Which are chosen, and their ways round, are kept in `guides.txt` beside the other settings, not in the project. The spiral is stretched over the frame as photo editors draw it, so in a frame that is not golden its squares are not square; Blender's golden triangles, which meet the edges at the golden section rather than square to the diagonal, are not drawn - the golden triangle here is the photographers' (`app/src/composition.rs`; tests `the_golden_spirals_squares_and_arcs_close_in_on_its_eye`, `the_golden_triangles_lines_meet_the_diagonal_square_to_it`, `the_harmonious_triangles_carry_on_to_the_edge`, `the_render_region_is_where_the_render_falls_in_the_view`, and the channel's `a_script_shows_turns_and_hides_the_composition_guides`).
- **Region-render rectangle** with draggable handles.

### 2.3 Context menu (right-click on geometry)

Assign Material ▸ · Unlink Material · Copy/Paste Material · Select Similar (same material) · Hide · Isolate · Focus (frame selection) · Set Camera Target Here · Set Pivot Here ▸ · Add Label Here · Move ▸ · Properties. Right-click on empty space: Environment ▸ · Camera ▸ · Render Region ▸ · View presets ▸.

---

## 3. Material application — the defining interaction

This section is normative. Get any of it wrong and the application feels unlike the thing it is meant to replace.

### 3.1 Drag from Library to viewport

1. **Drag start**: cursor becomes the material thumbnail at ~48 px with a slight drop shadow.
2. **Drag over viewport**: on every mouse-move, ray-pick under the cursor at interactive rate (this must be a cheap dedicated pick, not a full render). The part under the cursor gets a **highlight outline** and a floating label showing the part name and its current material name.
3. **Modifier preview** (do this — it removes an entire category of mistakes): show in the floating label what the drop will affect, updating live with modifiers:
   - no modifier → "Apply to: `Housing` (3 parts share this material)"
   - `Alt` held → "Apply to: `Housing` only (unlink)"
   - `Shift` held → "Apply to: all parts in group `Front Assembly`"
   - `Ctrl` held → "Apply to: all parts with the same current material"
   - no modifier, on a part that is selected (or inside a selected group) with others → "Apply to: the 12 selected parts". A drop beside a selection adds "Drop on a selected part for all 12". *(2026-10-16: a CAD body that arrives as a piece per face colour took one drop per piece.)*
   - A selected group always means the parts in it: a group's own material is never drawn. Double-clicking a part selects its group, and again the group around that.
4. **Drop**: apply, restart accumulation for the affected screen region only (§01 §9.3), and select the affected part so the Material tab immediately shows the new material's parameters.
5. **Undo**: one undo step reverts the assignment.

### 3.2 The linking model — state this in the UI, not just the docs

Materials are shared instances (§02 §8.2). The consequences users must be able to see:

- The Material tab shows, next to the material name, a **reference count badge**: `Brushed Aluminium · used by 7 parts`. Clicking it selects all 7.
- Editing a parameter warns nothing but visibly changes all 7 — which is correct and expected once the badge makes it legible.
- **Unlink** is available in the material header, the context menu, and as `Alt`+drop. After unlinking, the badge reads `used by 1 part` and the name gets a numeric suffix.
- A **Link To…** action lets a part rejoin an existing material instance.

Users coming from other tools frequently expect per-object materials. The badge is what teaches them the model without a tutorial.

### 3.3 Other application paths

- **Drag onto a node in the Scene Tree** — applies to that node and, if it is a group, to all parts beneath it (with the same modifier semantics).
- **Drag onto the material preview** in the Material tab — replaces the material on the currently selected part(s).
- **Double-click a library material** with a selection active — applies to the selection.
- **Copy/paste material** (`Ctrl+C` / `Ctrl+V` with parts selected).
- **Material Templates** — rule-based bulk assignment: match by CAD colour, part name regex, metadata field, or layer, and assign a material to every match. Show a preview table of what will be assigned before committing. This is how a 4,000-part assembly gets materials in a minute rather than a day.

*Texture mapping you can see* (as built, November 9): a texture laid on flat, wrapped round a cylinder, round a sphere or from three sides has **Place in view** beside its Mapping controls in the Material tab (`app/src/mapping_gizmo.rs`). While it is on, the viewport draws the projection on the part: the projector's plane, the cylinder with its seam drawn stronger, the sphere with its seam, or the part's box. One repeat of the texture is outlined on it, the one holding the anchor, where the projection faces the camera through the part (the part's middle on a flat projection, the side of a cylinder or sphere turned toward you; `decal_fit::facing_coords`). There are handles at the anchor: a cross, a square to size the texture about it, and a circle to turn it about it. They are kept 36 to 120 points out, so a repeat bigger than the view (a metre of texture on a 20 cm part) still has its handles on the part in sight. A drag on the part slides the texture so the point grabbed follows the cursor. Ctrl and Shift with the wheel size and turn it, as for a label. Each change is the Mapping panel's own `material.set_mapping`, and a drag merges into one step to undo. Escape or the button again ends it; the other buttons still look around. Scripts show it with `{"op":"ui","place_texture":{"material":n,"param":"base_color"}}`. A UV-mapped texture has no shape in space, so it has no button.

### 3.4 Dropping other asset types

| Dragged asset | Dropped on viewport | Dropped on Scene Tree |
|---|---|---|
| Environment | Sets the lighting environment | Same |
| Backplate | Sets background to Backplate mode with that image | Same |
| Colour swatch | Sets `base_color` of the material under the cursor (keeps the material type) | Same |
| Texture | Opens a small chooser: apply as Colour / Bump / Roughness / **Label** | Applies as colour map |
| Model | Imports into the scene at the drop point (raycast to ground plane) | Imports as a child of the drop target |
| Label image | Creates a label at the drop point (§02 §7.3) | Adds a label at default orientation |
| Substance material (`.sbsar`, `.sbs`) | A material with its settings (§02 §5.6), on the selected parts | Same |

### 3.5 Substance materials

*As built, November 5* (`app/src/substance.rs`). An Adobe Substance file comes in by **File › Import Substance material…**, the Library panel's **+ › Substance material**, the Material tab's **Substance…** (onto every part using the material shown), a file dropped on the window, or the scripting channel's `substance open`; all take one route. A Substance material's Material tab opens with a **Substance** section above the usual ones: the file's own picture, its name, author, category and description; a line saying how its maps stand (rendered at 1024 × 1024 · rendering, with a spinner · could not be rendered, with what the tool said and *Try again* · the file is not there, with *Find it…*); then **Preset**, **Output size** (256 to 4096) and **Random seed**, and the graph's inputs in the file's groups, each with the control its widget asks for — a slider for one number, number fields for two to four, a colour button, a switch, a drop-down, a text field, a picture chooser — a hover with its description and identifier, and ↺ back to the file's value. Every change is one undo step (a drag is one), and the maps follow a moment after it settles. Below: which outputs became which maps and which were not used and why, **Reset all**, **Read the file again** (after changing it in Designer) and **Keep the maps, let go of the file**. Without Adobe's tools the section says so in a card — install Substance 3D Designer or the Substance 3D Automation Toolkit, or **Set the folder of Adobe's tools…**, or **Look again** — and that the material shows the maps saved with the project, or its plain colours until it can render. The rest of the Material tab works on it as on any textured material: texture size, projection, type.

---

## 4. Selection model

### 4.1 Levels

Selection has an explicit **level**, changed with a modifier or a toolbar toggle:

| Level | Clicking selects | Default binding |
|---|---|---|
| **Part** | the individual part | click |
| **Object/Group** | the top-level group containing it | `Alt`+click, or double-click to step up |
| **Material** | all parts sharing the clicked part's material | `Ctrl`+click |
| **Face/Surface** | a single BREP face or triangle group (for per-face material assignment) | `F`+click |

`Shift`+click adds, `Ctrl+Shift`+click removes, box-select with a drag on empty space, `Ctrl+A` selects all, `Esc` deselects.

The current selection level is shown in the status bar. Selection is synchronized bidirectionally between viewport, Scene Tree, and Material tab at all times — no exceptions.

### 4.2 Scene Tree behavior

Standard tree conventions: expand/collapse, `Shift` range-select, drag-to-reparent (with a clear insertion indicator), rename in place (`F2`), and a search box that filters live and highlights matches while keeping ancestors visible. Per-row: visibility eye, lock, material swatch (drag from here to reassign). Right-click: Isolate, Hide Others, Group (`Ctrl+G`), Ungroup, Create Model Set from Selection, Select Similar, Delete.

The toolbar's Add menu adds a shape, a light or a cutaway. **Add › Shape** is a plane, cube, cylinder, sphere, disc or backdrop (a floor sweeping up into a wall), each a part with a plastic material of its own, selected once added, for staging a product or standing in for a missing part (`crates/tequila-scene/src/shapes.rs`). A shape comes in at half the scene's size, beside it on the same floor. A backdrop is 4.5 times the scene's size, the floor under the product and well out in front, the wall a little behind it and three times as wide as it is deep, so a three-quarter view has wall behind the product to the frame's edges. An empty scene gets a 20 cm shape at the origin. Shapes are lifted a thousandth of the scene's size off the floor, so a flat one doesn't fight the ground plane for pixels. They are added at the root, since they are placed in the scene's own coordinates.

*As built, November 11: any part a shadow catcher.* **Visible to**, under the tree for the selected parts, has a seventh box, **Shadow catcher** (`Rays::catcher`, `scene.set_rays`): the camera sees the shadows that fall on the part and nothing else of it, the background showing where it is, as the floor does. A product on a photographed table, the table modelled to catch its shadow - V-Ray's matte object, with the boxes above it that keep a part out of reflections, bounced light and the picture. On a group it is every part under it. To the renderers the part is a catcher as the floor is (`ShadowCatcher::part`), in the floor's shadow colour; in the working views it is drawn as the part it is, since it has to be seen to be worked on. Kept in the file only where it is set. Tests: `a_part_made_a_shadow_catcher_is_one_to_the_renderers`, `a_nodes_rays_survive_and_default_ones_write_nothing`, and in both renderers `a_catcher_part_shows_only_its_shadow_in_both_renderers`.

*As built, November 9:* a **Wall** joins them: a flat back wall standing on the floor, placed as a backdrop is. A shape keeps its dimensions on its part (`ShapeSpec`, saved with the project), and selecting it shows **Shape** under the tree, with its dimensions in the scene's units: a plane's width and depth; a cube's width, depth and height; a cylinder's diameter and height; a sphere's and a disc's diameter; a wall's width and height; and a backdrop's width, floor depth, wall height and curve (none is a sharp corner). A change builds the mesh again from them, as one undoable command, `scene.set_shape`, whose dragged values merge into one step. The mesh it had is let go unless a duplicate shares it. A backdrop and a wall stand at the foot of their wall, so a deeper floor reaches further toward the camera and the wall stays where it was. Handles in the viewport are not built yet.

**Patterns** (*as built, November 9*): Edit › Pattern, the viewport's right-click and the Scene tab's Pattern menu repeat one selected part **along a line** or **around a circle** (`crates/tequila-scene/src/pattern.rs`, `app/src/pattern_panel.rs`). The part goes into a group named for the pattern, with copies beside it: five along its width with a fifth of it between each, or six around a circle beside it, about the vertical. The copies are parts like any other (picked, exported, framed and rendered as the rest are) that share the original's mesh and material, so a pattern of a hundred screws holds one screw's triangles, and a material on any of them is on all of them. Selecting the group, the original or a copy shows **Pattern** under the tree: Line or Circle; the count, the original included (2 to 500); a line's step along X, Y and Z, or a circle's centre, axis and angle (360° spaces them evenly, with none on top of the original). A change lays the copies out again from the original as it is, as one undoable command, `scene.set_pattern`, whose dragged values merge into one step. Moving the original afterwards puts it out of step with its copies, and **Lay out again** puts them back. **Make parts** keeps the copies as parts of their own; **Remove copies** keeps only the original. A copy dragged out of the group becomes a part of its own, and the pattern leaves it alone. A duplicated pattern is a pattern of its own copies. The settings are saved on the group in the project.

---

## 5. Transform gizmo and pivots

- **Move / Rotate / Scale** modes (`W` / `E` / `R`), plus a Universal gizmo.
- Axis handles, plane handles, screen-space handle; hold `Shift` to constrain to the last-used axis.
- **Numeric entry** in the Position/Rotation/Scale fields with absolute and relative modes.
- **Snapping**: grid snap, angle snap (15° default), vertex/edge/face snap when hovering geometry, and snap-to-ground (drop the object onto the ground plane — a one-click operation people use constantly in product staging).
- **Pivot modes** (§03 §2.4): Origin, Bounding-box centre, Corner, Cylinder axis, Sphere centre, Custom. Setting a pivot is `Insert`+drag or via the context menu, and pivots persist for animation use.
- Transform in world or local space, toggled in the gizmo header.
- *As built, November 8:* the gizmo is the same size on screen whatever the window: its arms, and its rings' radius, are `GIZMO_POINTS` (90) points long - 90 pixels at 100% display scaling, 180 on a Retina screen - turned into a length at the pivot's depth from the camera's vertical field of view and the view's height, or from an orthographic camera's scale (`world_length_of_points` in `gizmo_ui.rs`). It was a seventh of the camera's distance, a share of the view's height, which on a large monitor was a sixth of the screen. Its handles are picked at the size they are drawn (`the_gizmo_is_the_same_size_on_screen_at_any_window_size`). The label and pin placement tools do not share this code (they draw the label's own outline, at its size on the part, and a banner), nor does the UV editor, whose handles are drawn on its own canvas in points.
- *As built, November 9:* the Universal gizmo, **Q** and the first of the tools down the viewport's left (Lucide's axis-3d), is the default: move, turn and scale at once, as KeyShot's gizmo does, with W, E and R still picking one. From the centre out (`gizmo::universal`): the screen disc, which moves parallel to the view; the three plane squares; a scale cube on each axis; the rotation rings; and the arrows, which start outside the rings, as Blender's combined gizmo keeps them apart - arrows from the centre crossed every ring twice. A drag takes the mode of the handle grabbed (`pick_operation`), so each is exactly the single-mode drag, snapping and all. It is drawn 1.35 times the others' size (`UNIVERSAL_POINTS`), so its rings are not crowded inside fifty points, and every handle of every mode has a thin dark edge, so a red ring shows on a red part. Uniform scaling stays with R's centre handle.

---

## 6. Navigation and hotkeys

### 6.1 Camera navigation (must match exactly — muscle memory)

| Action | Binding |
|---|---|
| Orbit | LMB drag |
| Pan | MMB drag |
| Zoom | Scroll |
| Focal length / FOV | `Alt`+Scroll |
| Dolly | `Alt`+RMB drag |
| Twist / roll | `Ctrl`(⌘)+`Alt`+Scroll |
| Distance + perspective + FOV | `Shift`+`Alt`+RMB drag |
| Standard views 1–7 | `Ctrl`(⌘)+`Alt`+`1..7` |
| Set focus/orbit target | `Ctrl`(⌘)+`Alt`+RMB click |
| Next camera | `Shift`+`N` |
| Frame selection | `F` |
| Frame all | `Shift`+`F` |

Ship alternative navigation presets emulating SolidWorks, Rhino, Blender, Maya, and Fusion 360, selectable in Preferences. This is a five-minute feature that removes a real adoption barrier.

*As built, November 11:* Preferences › Navigation › *Move the view as in* (`app/src/navigation.rs`). Which button, with which keys held, orbits, pans and zooms by dragging is a scheme of three drags, and the viewport asks it what a drag does rather than knowing. The presets, each its program's own:

| Preset | Orbit | Pan | Zoom by dragging |
|---|---|---|---|
| Tequila (as KeyShot), the default | Left | Middle | `Alt` + Right |
| Blender | Middle | `Shift` + Middle | `Ctrl` + Middle |
| Maya, Cinema 4D | `Alt` + Left | `Alt` + Middle | `Alt` + Right |
| 3ds Max | `Alt` + Middle | Middle | `Ctrl` + `Alt` + Middle |
| SolidWorks | Middle | `Ctrl` + Middle | `Shift` + Middle |
| Rhino | Right | `Shift` + Right | `Ctrl` + Right |
| Fusion 360 | `Shift` + Middle | Middle | `Ctrl` + `Shift` + Middle |

Below the preset, *Orbit*, *Pan* and *Zoom* each choose any button with any of Ctrl, Shift and Alt; changing one makes the preset **Custom**, starting from the drags in use, and a drag another move had is swapped with it, so no two share one. Of the drags that fit a press, the one asking for the most keys wins (Blender's Shift and the middle button pans, though the middle button alone orbits), and keys a drag does not ask for do no harm, as before: Shift and a left drag still orbits in Tequila's own. The wheel zooms, Alt and the wheel change the focal length, a click selects and a right-click opens the menu in every preset; where the right button orbits (Rhino), a right-click that does not move is still the menu. The left button is the gizmo's and the axis legend's before it is the view's. The Shortcuts window's Mouse rows show the scheme in use. Kept in `preferences.txt` (`navigation.preset`, `navigation.orbit_drag`, `navigation.pan_drag`, `navigation.zoom_drag`), and set by scripts as any setting. Not built: the wheel's direction per preset (SolidWorks' and Fusion's is the other way; *Reverse the zoom direction* is its own setting), twist, and the other §6.1 bindings as choices. Tests: `navigation.rs`'s, and `a_navigation_preset_and_a_custom_drag_are_kept`.

#### 6.1.1 3D mouse

A 3Dconnexion 3D mouse (SpaceMouse, SpaceNavigator and their kin) turns, pans and zooms the view with one hand while the other keeps the mouse, as in CAD applications. The cap moves the model: push it right and the model goes right, lift it and the model rises, pull it toward you and the model comes closer; twist it and the model turns about the vertical, tilt it forward and the model tips away. The two main buttons step to the next Studio (the left, Menu on the larger devices) and fit the view (the right, Fit). Its settings are in Edit > Preferences (§12).

*As built, November 5* (`app/src/spacemouse/`): **untested on a device** - none was at hand; everything below is held by tests that feed it synthetic reports.
- **Reading it, without 3Dconnexion's SDK.** On Windows and macOS the device's own HID reports are read on a thread of its own through `hidapi` (Windows' HID calls; IOKit on a Mac), from the first 3Dconnexion device on its multi-axis interface (usage page 1, usage 8): any device under 3Dconnexion's vendor id 0x256F, and the named SpaceMouse family under Logitech's 0x046D (SpaceNavigator, SpaceExplorer, SpacePilot and Pro, SpaceMouse Pro and Classic, and the rest). On Windows any number of programs read a HID device at once, so it works beside 3Dconnexion's driver, 3DxWare; if 3DxWare also acts on Tequila (scrolling or zooming the view itself), turn its own actions off for Tequila. On a Mac 3DxWare takes the device for itself and nothing else can open it while it runs, so Tequila reads it through the driver, as other applications there do: it loads the `3DconnexionClient.framework` the person's driver installed in `/Library/Frameworks`, at run time, and registers as a client by its executable's name. The driver's own settings for Tequila (speed, which axes, which way) then apply as well as Tequila's. Tequila ships nothing of the framework, is built without it, and uses no part of 3Dconnexion's SDK: the few functions and the one message it reads are declared in its own words (`app/src/spacemouse/connexion.rs`). Without 3DxWare, Tequila opens the device for itself, which also keeps macOS from moving the pointer with it. On Linux, and on a Mac where it runs, the reader talks to spacenavd over its socket (`$SPNAV_SOCKET`, `$XDG_RUNTIME_DIR/spnav.sock` or `/var/run/spnav.sock`), protocol 0, as libspnav does without asking for more. A device unplugged is looked for again every two seconds.
- **The reports.** Report 1 with six bytes is translation and report 2 rotation (the older devices); report 1 with twelve is all six at once (the newer wireless ones and the Enterprise), told by its length; report 3 is the buttons, bit 0 the left and bit 1 the right. Each axis is a signed 16-bit number, full deflection about 350. The device's X right, Y toward the person and Z down become the view's x right, y up and z out of the screen, and each rotation follows its axis. spacenavd's numbers are the device's, unless its own configuration swaps or inverts them.
- **The camera.** Each frame the axes go through the dead zone (taken out of the middle of each axis, so full deflection still gives the full rate), a gentle curve (half the rate at half way), the inverts and, when asked, only the strongest axis. They are then smoothed with a 50 ms lag and moved by their exact mean over the frame, so the view moves alike at 30 frames a second and at 144. At full deflection and speed 1:
  - turning, 1.6 radians a second, about the selection's centre or the model's (chosen when a motion starts; the camera swings round it without the view jumping) or about the view's centre;
  - panning, 1.2 times the height the view frames at that point a second, so the model crosses the view alike near and far;
  - zooming, the distance changed by a factor of e^1.5 a second, as the wheel's zoom is a share a notch.

  The orbit camera keeps the horizon level, as the mouse's orbit does, so the roll is read and shown and moves nothing. The window keeps drawing while the cap is off centre, and each report wakes it. A motion that comes to rest is one step to undo. A motion never put back at rest is dropped after two seconds, so a lost report cannot leave the view turning.
- **Buttons.** Fit view frames the selection, or everything (`F`). Next and Previous Snapshot apply the Snapshots in turn, stepping through the saved cameras when there are none. Either button can also do nothing.

### 6.2 Command hotkeys (defaults)

| Key | Action | Key | Action |
|---|---|---|---|
| `Space` | Toggle Project window | `M` | Toggle Library |
| `Tab` | Full-viewport mode | `H` | Toggle HUD |
| `E` | Background = Environment | `C` | Background = Colour |
| `B` | Background = Backplate | `G` | Flatten ground |
| `U` | Snapshots | `L` | Add label to selection |
| `Q` | Move, turn and scale at once | `W`/`E`/`R` | Move/Rotate/Scale |
| `F`/`Shift+F` | Frame selection / all | | |
| `Ctrl+G` | Group | `Ctrl+Shift+G` | Ungroup |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / Redo | `Ctrl+S` | Save |
| `Ctrl+I` | Import | `Ctrl+R` | Render |
| `P` | Screenshot of viewport | `Ctrl+D` | Duplicate |
| `Del` | Delete | `Ctrl+H` | Hide selection |
| `Alt+H` | Unhide all | `/` | Isolate selection |
| `Shift+P` | Pause/resume rendering | `,` | Toggle denoiser |
| `Ctrl+A` | Select all | `Ctrl+Shift+A` / `Esc` | Deselect all |
| `Shift+H` | Show/hide selection outline | `Shift+G` | Show/hide gizmo |

*As built, November 8:* **P** saves a Screenshot and **Shift+P** pauses the viewport, which P did before; **Ctrl+A** selects all only without Shift, and **Ctrl+Shift+A** (Cmd on a Mac) deselects all, beside Esc. The Shortcuts window and the right-click menu's *Deselect all* say so. Clicking empty space deselects in every view: a click on the floor - the shadow catcher Material Preview and Rendered trace, which has no part of its own - was taken as a click on something and kept the selection (`clicked_part` in `main.rs`, test `a_click_on_the_floor_deselects_and_a_click_on_a_part_selects`). The viewport's denoise switch is on its toolbar rather than `,`.

*As built, November 9:* **Ctrl+D** (Cmd+D on a Mac), Edit › Duplicate, the viewport's right-click menu and the Scene tab's Duplicate button and right-click menu copy the selection in place. Each copy shows its original's mesh and wears its material - a node, not another copy of the triangles, drawn as an instance of the same mesh - and a group is copied with everything under it; a part chosen with its group comes with the group once. A copy is named "… copy" and sits beside its original in the same group; it is not matched to the CAD file by a later re-import, which updates the original alone. The copies are one step to undo, as an import of those nodes alone (`scene.import`), and are selected, so the gizmo moves them off the originals (`tequila_scene::duplicate`; tests `a_copy_of_a_part_shares_its_mesh_and_material`, `a_group_is_copied_whole_and_once`, `duplicating_selects_a_copy_that_shares_the_mesh`). Patterns - copies along a line or around a circle, kept editable - are not built yet.

*As built, November 10:* **Shift+H** hides the selection's outline and **Shift+G** the gizmo, and each shows it again; so do View › Selection outline and View › Gizmo, and the `ui` operation's `selection_outline` and `gizmo`. The selection stays, so a selected part can be seen as it is while its material is edited: deselecting it to see it put its material away. A notice says what was hidden and the key that brings it back, as nothing else would say why the gizmo has gone; both come back with each new session. Hidden, the gizmo takes no drag either, and a drag under way when it was hidden is let finish. Ctrl+Alt+H and Ctrl+Alt+G were asked for, but Ctrl+H hides parts and Alt+H shows them all, Cmd+Option+H is macOS's Hide Others, and Ctrl+G groups (test `shift_h_hides_the_outline_and_shift_g_the_gizmo_keeping_the_selection`).

`E` conflicts between "Background = Environment" and "Rotate gizmo". Resolve by context: gizmo shortcuts apply when the viewport has focus and a transformable selection exists; otherwise the background shortcut applies. Document the resolution, expose both in the hotkey editor, and let users rebind.

### 6.3 Hotkey editor

*As built, November 11:* Preferences › Keyboard (`app/src/shortcuts.rs`, `app/src/preferences.rs`). The main window's keys were written into the code that read them; they are a table now, forty-eight actions each with a name (`edit.undo`), a heading, its default keys and up to two keys of the person's own. Each frame the window asks which actions its key presses give and carries them out, so a key is a command only through the table.
- **Changing a key.** Each action is a row with its keys as buttons, and room to add a second. Click one and the next key pressed, with Ctrl, Shift or Alt held, is that action's; click it again to leave it. *Remove* empties a place. While a key is being chosen no key is a command, and the press that chose it is not also the undo or save a window passes to the main one.
- **Conflicts.** A key another action has is not taken: the row says "Ctrl + G is already Group." with *Use it for …*, which leaves the other without that key, and *Cancel*. Rotate and "background = Environment" share a key by design (§6.2) and are never a conflict: with something selected it turns it, with nothing it sets the background, whichever key that is.
- **Exactly the chord.** A press gives the action bound to exactly its key and modifiers: Ctrl+Shift+G is not Ctrl+G, which it was. On a Mac, Ctrl is Cmd, and Control is its own modifier, so hiding parts stays Control+H there (Cmd+H is the system's).
- **Reset.** A changed action has the mark that puts its defaults back, taking them from whichever action was given one; *Restore the default keys* does all of them.
- **Shown everywhere as they are.** The Shortcuts window lists every action with the keys it has now, and has *Change the keys…*; the menus' and toolbars' tooltips and the outline's and gizmo's notices read the same table.
- **Kept and scripted.** Changed actions are lines of `preferences.txt` (`shortcut.view.screenshot=F12`); the exported settings file holds every action's. The `preferences` operation lists them (`shortcuts`), sets one from a key written out, a list or `""` (`"shortcut.edit.duplicate": ["Ctrl + D", "Ctrl + Shift + D"]`) and resets one or the `keyboard` page; from a script a key is taken from the action that had it without asking.

Not built: presets (KeyShot, Blender), mouse actions and camera controls as bindings, the Material Graph editor's and UV editor's own keys (theirs, in their own tables), and the keys the Mac's menu bar shows (it shows none). Tests: `shortcuts.rs`'s, and in `preferences.rs` `a_shortcut_given_another_key_answers_to_it_and_is_kept`, `a_key_in_use_is_asked_about_before_it_moves`, `the_press_that_chooses_a_key_is_not_passed_on` and `a_script_reads_and_changes_shortcuts`.

Full remapping, organized as Simple Hotkeys / Mouse Actions / Camera Controls. Conflict detection with a clear warning. Import/export of hotkey sets. Reset to defaults. Application presets (§6.1).

---

## 7. Material Graph editor

The editor is driven as Blender's node editor is: the same keys, the same mouse, and the same Add menu. A graph built in one is built the same way in the other. The node vocabulary is Blender's too (`docs/02` §6.6). The material is rendered large beside the graph as it is edited, as V-Ray's material editor keeps one in view (`app/src/graph_editor/`).

- **Canvas:** infinite pan with a middle-drag or Alt-drag, or two fingers on a trackpad. The mouse wheel and a pinch zoom towards the pointer. A dot grid marks every fifth dot heavier. **Fit** (Home) shows everything, and `.` shows the selection.
- **Node bodies, as Blender lays them out:** outputs first, then a preview of what the node makes (for textures and patterns), then its settings as dropdowns, numbers and switches, then its inputs.
  - Each unwired input shows its value in place: a slider (filled for a value with a range), a colour swatch, a switch, or a vector's X/Y/Z.
  - Drag a number to scrub it (Shift is fine, Ctrl snaps). Click it to type a value or an expression.
  - The header's arrow collapses the node.
- **Adding nodes:** Shift A, Tab, or a right-click on empty canvas opens the Add menu.
  - The families run down the left, and hovering one lists its nodes.
  - Typing searches everything, including the operations inside nodes: "multiply" offers Math › Multiply, Vector Math › Multiply and Mix › Multiply, each added already set.
  - Arrow keys and Enter choose without the mouse.
  - Dropping a wire in empty space opens the menu filtered to nodes that fit, and wires the chosen node in.
- **Wiring:** drag socket to socket. An incompatible pair shows red, with the reason.
  - Pulling a wire off a connected input picks it up from its source.
  - Ctrl and a right-drag cuts every wire it crosses.
  - A node dragged onto a wire goes in between.
- **Selection and editing:**
  - **Select:** click; Shift-click adds or removes; dragging on empty canvas draws a box (Shift adds, Ctrl removes); A selects all and Alt A none.
  - **Move and copy:** G moves the selection, Shift D duplicates it, and Ctrl C and Ctrl V copy and paste with the wires among the copied nodes.
  - **Delete:** X or Delete deletes; Ctrl X deletes a node and joins its wires through it.
  - **Mute and collapse:** M mutes, so the input passes through as Blender bypasses a node and the node draws with red pass-through wires. H collapses.
  - **Frame and rename:** Ctrl J frames the selection in a titled box, and F2 renames.
  - A right-click on a node offers all of these.
- **The browser:** *as built, November 11* (`graph_editor/browser.rs`). Down the left side, as the Slate Material Editor of 3ds Max has its Material/Map Browser, for the people who come from there and from V-Ray and look to the left for what can be made: a search field over five groups.
  - **Materials** are Shader nodes, one of each kind of material under Basic, Advanced, Special and Light, each named for its kind: another surface to mix with the material's own. **Maps** are the Texture and Pattern families, the project's pictures among them. **Utilities** are the other families. **Scene materials** are the document's own, the one being edited marked. **Library** is the material library.
  - A node's row is dragged into the graph and is a node where it is let go, or double-clicked and is one in the middle of the view, each a step from the last; either is one undo step. A scene material's row is clicked and the editor is on that material, with no trip back to the Material tab.
  - The search is the Add menu's (`menu::score`), over all four groups and the operations inside a node ("multiply" finds Math › Multiply), without its loosest tier: a list shows everything it finds, and letters merely in order in a name are not a find.
  - **Browser** at the left end of the toolbar and **T** hide and show it; **Settings** at the right end and **N** do the same for the swatch and the node's settings, as Blender's T and N. The browser's choice is kept: Preferences › Interface › Material Graph editor › *Show the browser* (`interface.graph_browser`). The toolbar names the material the graph is.
  - To a script: the setting, and `ui` `graph_search`, which types in the search and shows the browser.
  - **A material brought in whole** (`bring_in`): a scene material's row dragged into the graph, or a row of the fifth group, **Library** - the material library by its categories - dragged in or double-clicked. Its nodes come with the wires among them, and where its Surface was there is a Shader of the same surface under the material's name with the same wires into it: a Shader takes every input a Surface does, so nothing of the material is left behind. More than one node comes in a frame with its name. Dropped, the Shader is under the pointer; double-clicked, the material goes under everything that is there and the view takes it all in. A library material's pictures are added to the document with the graph, numbered as the document numbers them, as one undo step. The row of the material being edited does not drag. To a script: `ui` `graph_bring`, `{"library": "<name>"}` or `{"material": n}`.
  - **A row dropped on a wire** it fits goes in between, the wire lit while it is held over it, as a node dragged onto one on the canvas does (`wire_fitting`, for a node that is not in the graph yet).
  - **A graph is laid out for the person only when nobody has laid it out** (`tidy::unplaced`): two nodes at the very same place, as an importer and the Material tab leave them. It was any two nodes overlapping, which a person does all the time, and each time the whole graph was re-laid and kept so: a node dropped on another, Shift D's copy set down a step from its original, a paste over the graph (`a_node_dropped_on_another_does_not_rearrange_the_graph` failed before, with a node that was not touched moved). The editor steps a node it has put exactly on another a grid step off it (`apart`), and a row double-clicked in the browser looks for a place clear of the nodes that are there.
  - **Setups** (`browser::Setup`, `setup_graph`): rows that add several nodes already wired, for what is one node in the material editors of 3ds Max and V-Ray. Under Materials › Layered, **Blend (3 coats)** - V-Ray's blend material - is three Shader coats, each brought in by a Mix Shader on the Layer of the one under it, with a **Blend Stack** node (`groups/blend_stack.rs`, a node group, so every engine renders it as the nodes inside it) giving the Mix Shaders their factors from each coat's amount: each coat over everything under it, where coats nested by their own amounts show only inside the coat under them. **Two-sided** is another surface on the back of every face, a Mix Shader by Geometry's Backfacing. Under Maps › Layered, **Composite (4 layers)** is four colours mixed one over another, each Mix with its blend and factor, left for the person to wire in. Each is added in a frame with its name, wired to the material's Layer when that is free. Found also by the names they go by there ("VRayBlendMtl", "two sided"). To a script: `graph_bring` `{"setup": "blend"}`.
    - Under Maps › Per part, three that read the part's own random number (Object Info's Random, the same number in every engine), so parts that share a material need not look alike: **Random colour per part** - V-Ray's multi-sub texture, at random - is that number through a Color Ramp of four held colours; **Vary colour per part** shifts a colour's hue and lightness by it; **Random placement per part** - V-Ray's UVW randomizer - moves and turns a texture's coordinates by it, through a Mapping, for any texture that takes a Vector. Left for the person to wire in. Not built: a colour or a number set on a part by hand for the material to read, and the placement of a picture, which takes a Projection and no other coordinates. In both renderers `a_parts_random_colour_is_the_same_in_both_renderers`.
    - The compiler dropped a coat on a coat when the material and its first coat were of a kind: mixed into one surface parameter by parameter, the first coat had no Layer left for the second to be on (`a_layer_on_a_layer_of_the_same_kind_is_kept` failed before). Two of a kind are still mixed into one when there is nothing on the second.
    - Tests: `the_blend_stack_weighs_each_coat_as_a_stack_does`, `a_setup_is_added_wired_and_compiles`, and in both renderers `blended_coats_are_a_stack_in_both_renderers` (a grey material under a red coat at a half and a blue one at a half is the colour of one surface a quarter grey, a quarter red and half blue).
  - **Tiles** (`crates/tequila-materials/src/groups/tiles.rs`): two patterns among the Maps, **Herringbone** and **Basketweave**, for planks, parquet and paving - the bonds a floor is laid in, where Brick Texture lays a wall's courses. Each takes the tile's width, its length in widths (a whole number), the joint, and the plane it is laid in (flat, or on a wall facing the front or the side), and gives Tile (1 on a tile, 0 in the joint), Random (a number of the tile's own, for a Color Ramp), Tile Coordinates (along and across each tile, each from a place of its own: into a wood's Vector, the grain runs along every plank whichever way it lies) and Edge (millimetres in from the tile's edge, for a bevel). Node groups of Math nodes and a White Noise, so every engine renders them as the nodes inside. Found also as "tiles", "parquet", "paving", "VRayBerconTile".
    - Tests: shaded over a grid of points (`crates/tequila-render-core/tests/tiles.rs`), every tile is a whole oblong a width across and its length long, lying or standing, half of each, with nothing between them; the two bonds differ as they should; a joint takes its share of the floor. In both renderers, `tiles_are_laid_alike_in_both_renderers`: each tile the same grey on the same pixels.
    - A node found by its own name is one row in the browser, where it was one for every other word it goes by.
  - **Distance**, among the Utilities with Curvature and Occlusion: how far the nearest other part is (`docs/02` §6.2), with its reach in millimetres.
  - **A picture beside each row** (`graph_editor/thumbs.rs`): a small ball of a material - a kind of material, a scene material, a library material - and a small square of what a texture or a pattern makes. The other nodes keep their family's dot. Made for the rows in view only, on one worker thread, and kept by what they are of, so a material edited is pictured again. The library's are its own shipped or kept thumbnails where it has them (`LibraryTab::small_picture`). A map is tried over six sizes of surface, two metres to two millimetres, and shown at the widest that is clear (`clarity`: its contrast, less what of it is from one pixel to the next): at any one size half of them were grey speckle or one flat colour, a metre of a weave and a centimetre of a marble.
  - **A material's labels come in with it**: they are part of it, and are printed on this material after its own, over whatever the two are mixed into, as a label is.
  - Tests: `the_browser_lists_what_the_add_menu_does`, `the_browsers_search_finds_nodes_operations_and_materials`, `the_browser_adds_nodes_where_they_are_put`, `a_row_is_dragged_out_of_the_browser_into_the_graph`, `the_browser_opens_another_of_the_scenes_materials`, `t_and_n_hide_the_side_panels`, `a_script_shows_hides_and_searches_the_graph_browser`, `a_material_is_brought_in_as_a_shader_with_its_nodes`, `a_scene_material_is_dragged_into_the_graph`, `a_library_material_comes_in_with_its_pictures`, `a_row_dropped_on_a_wire_goes_in_between`, `a_duplicate_does_not_rearrange_the_graph`, `a_script_brings_a_material_into_the_graph`, `a_rows_picture_is_of_what_the_row_is`, `a_picture_is_kept_by_what_it_is_of`.
- **Previews:** every texture and pattern node shows its output, made on a worker thread. A node is redone only when something upstream of it changed, and the old picture shows until the new one arrives.
- **The swatch:** the material rendered with the real path tracer on a sphere or slab the size of the model, so a pattern shows at the scale it will have on the part. A rough pass comes at once and a clean one follows.
  - *As built, November 11:* the full-size picture is shown as it is made, after 2, 8, 24 and all 64 samples a pixel (`STEPS` in `graph_editor/swatch.rs`, `render_thumbnail_in_steps` in the renderer), and a change to the material is looked for every 4 samples (`CHUNK`). It was the small rough picture, blown up, until the whole clean one was done, and a change made meanwhile waited for that. Measured on an M3 Max with the Headphones sample, to the first full-size picture and to the finished one: plain plastic 0.5 s and 5 s, Stainless Hairline 0.7 s and 11.5 s, Felt Grey 0.9 s and 17 s; before, the blur stayed for 3.5 s, 10 s and 15 s. The finished picture is the same. A clear material's is rendered at the screen's size with three times the samples, where it was also twice the size each way. `TEQUILA_SWATCH_TRACE=1` prints when each picture is handed over. Tests: `a_preview_in_steps_adds_up_to_the_whole`, `a_rougher_picture_never_covers_a_finer_one_of_the_same_thing`, `the_steps_are_rendered_in_small_pieces`.
- **Solo:** Ctrl Shift click a node to show its output on the model, unlit. Clicking again cycles through its outputs. **End solo** in the toolbar, or closing the editor, puts the material back.
- **Undo** is the global command stack: every edit is a `material.set_graph` command, and a drag is one step. Ctrl Z, Ctrl Y and Ctrl S work with the pointer over the editor.
- Still to come: node groups with exposed inputs.

*As built, November 11, from a pass over the two editors:* the line under the graph's canvas says what is there - how many nodes, how many selected, the zoom - and one key to start with (`status_line` in `graph_editor/mod.rs`, `the_status_line_counts_and_names_this_computers_keys`). It was six of the sidebar's eighteen shortcuts over again, with Ctrl where a Mac has Cmd. The right-click menu's keys and Solo's tooltip are in the computer's own names too (`theme::keys`). With nothing selected the sidebar opens on what the graph is and how to wire it, then the keys; that sentence was under them, below the fold. The toolbar's **Map** toggle is **Overview**: the small map of the whole graph in the corner, when it does not fit the view.

### 7.1 UV editor

A part's surface laid out flat, with its labels and the material's UV-mapped textures placed on it by hand, laid out as a drawing program is (`app/src/uv_editor/`). It opens from View › UV editor, from a label's **UV editor** button in the Labels section (which unwraps a part that has no layout yet, as one undo step), or as the **UV layout** workspace, which puts the Material page beside it. It is a window of its own and remembers its size.

- **Layout:** a toolbar along the top - the tools, what a click picks, the island tools, what the canvas shows, View options, and on the right Unwrap with the angle it splits at, Export layout and the shortcuts; rulers along the canvas's top and left; Layers, Transform, Align and Islands down the right; the pointer's place, what a drag snapped to, the selection with its size, and the editor's last word (what it did, or in red why it would not) and the zoom along the bottom.
  - *As built, November 11, from a pass over the two editors:* the toolbar wraps onto a second row in a window too narrow for it (`TOOLBAR_END`); it filled the window's first width, 1240 points, to the point, and in anything narrower its right end lay over View options. The part's name leads the status bar: in the toolbar it had room for six letters ("Cushio…"), beside a "No layout yet" that ran under View options. The angle beside Unwrap says what it is, **Split at 66°**, where it was a bare number. The zoom reads "Zoom 100%", apart from the Fit, 100% and 200% it can be set to. A part with no layout shows one line in the sidebar where Arrange, Texel density, Islands, Unwrap, Shape and Symmetry each opened on its own "Unwrap the part to…"; the canvas says there is no layout and holds the button. The snapping and zoom tooltips name this computer's keys.
- **Tools:** Select (V), Move (W), Rotate (E), Scale (R) and Measure (M), as in the viewport. Select picks, Shift-click adds, and a drag on empty space draws a box. The others act on the selection from anywhere. Handles on the selection's frame scale it (corners keep its shape, Shift frees it, Alt scales about the middle) and its stalk turns it (Shift by 15°).
- **What a click picks:** Labels (1) - the labels and textures, as before - or the layout's own: Islands (2), Edges (3) and Vertices (4), found by the layout engine's hit tests. Click, Shift-click, a box (an island it reaches into or holds the middle of, the edges and vertices inside it), Ctrl A and Esc work in each. An island is named by one of its triangles, a vertex by its mesh vertex, an edge by the mesh vertices at its ends, so a selection lasts through a cut or a stitch.
- **Island transforms:** the tools and handles move, turn and scale the selected islands (or edges' and vertices' places), snapping by the selection's box to the grid, guides, the square and the other islands' edges and middles. The drag is drawn as it goes and sent as one `uv.transform` when it is let go; the Transform panel types the box's place, size and a turn, with flips and quarter turns; arrows nudge. A label whose middle is on an island an edit moves goes with it - moved, turned, scaled, mirrored - in the same step, so it prints where it printed.
- **Seams** are drawn in orange, the part's own edge in black. In Edges or Islands, the seam under the pointer is lit with its other side, wherever that is. Cut (C) parts the layout along the selected edges; Stitch (S) joins the selected seams, or the seams between the selected islands. A refusal - a stitch that would fold the layout - says why.
- **Pins:** Pin (P) and Unpin (Shift P) the selected vertices, drawn as pins on the layout and on the part; each is a step to undo (`uv.set_pins`, the set's whole list), and the pins are saved with the part, set by set, and follow the layout's edits (a set copied takes its pins, an unwrap numbering the vertices anew keeps them on the same corners). Re-flatten lays the selected islands (or all) out flat again with their pins held, packing after if asked.
- **Layout:** Pack (Ctrl P), with a popover for the margin in millimetres or pixels of a texture size, turning islands to fit, evening their sizes first, and packing into the square or the box between the outermost guides; Straighten (by each island's axis, or by one selected edge); Even size; Unwrap again, which lays the selected islands, or the whole set in place, out flat again keeping their cuts, as the Unwrap panel's options say. Each works on the selected islands, or on all of them.
- **Stretch:** View options (or the flame in the toolbar) shows a heatmap of how much the layout stretches the part, by area (blue coarser, red finer) or by angle (yellow to red), with its key in the corner.
- **UV sets:** the Islands panel's picker lists the part's sets and what reads each, and adds, copies, moves up or down and deletes the set shown (`uv.add_set` and the rest); labels and textures keep to the layout they were on.
- **Export layout:** a dialog writes the layout as an SVG or a PNG - its size, what fills the islands, the lines' colour, a clear square - with a small picture of what it will write.
- **The part in the viewport:** what is under the pointer on the layout - an island, a face, an edge, a vertex - is lit on the part, with the layout's seams in orange in its layout modes, what is selected and the pins; the faces another part hides faintly; lines and vertices behind the part's own surface are not drawn, and those behind another part faintly, found by rays through the geometry the viewport picks against, so it is the same over every view; the face under the viewport's pointer is lit on the layout with a crosshair where it is. Faces picked at Face level are drawn on both, and the Islands panel selects their islands or cuts them out along their border as an island of their own.
- **Islands panel:** the set picker; the selection's size, area on the part and stretch, and a word where an island is mirrored; the island tools; and the labels' tools for the island last clicked (Fit, Centre, Along the island, Square).
- **2D cursor:** Shift and a right click put a cursor down on the layout, landing on whatever a drag would snap to (a plain right click still opens the menu, a right drag still pans); the menu moves it to the selection's middle (Shift C) or the layout's. One for each part, kept for the session, drawn as a red and white ring (View options › Show the cursor). Align lines things up against it, and the Transform panel's "Turn and scale about the 2D cursor" makes typed turns and sizes, the Rotate and Scale tools, quarter turns and flips go about it.
- **Arrange panel:** Line up edges (edges within a tolerance, 15° to start, of across or down made exactly so, moving only their vertices, or where that would turn a face over carrying the island's other vertices along so they still end exactly straight; a picked path that runs near square end to end lined up whole; a quad's long diagonal left alone, and an edge that would put two vertices on one place or a face's corners on one line left, and said); Upright (Shift U: each island turned so the part's up, as it stands in the scene, runs up the layout; islands facing straight up or down are left and said so); Sort (Shift O: a row, a column or a grid, by X, height or Z on the part, round an axis from the widest gap, a grid of rows and columns on the part, or where they are on the layout; aligned at the start, centre or end of each row; a gap in millimetres or pixels; wrapping after so many or at the square's edge; grouped by size, shape or nearness on the part, a wider gap between groups; reversed); Match size (to the island clicked, the largest or the average, by width or height, keeping proportions or not); Fit to square (Shift F: together, or each on its own, into the square or between the guides as Pack's options say); Stack similar (Shift S: the islands of the same piece as the one clicked laid exactly over it, corner for corner, mirrored pieces too if asked, or each a step from the last); Copy shape and Paste shape (onto selected islands of the same piece, each where it was or over the one copied, from this part or another); Swap (Shift X: two trade places by their middles, more go round in a seeded order); As on the part (laid out as the part is seen from its front, side or top, or whichever spreads them most: each island turned to read as seen - one seen from behind as through the part - and scaled to the layout's own scale, each optional, its middle where the part's middle is seen, then pushed apart until no two boxes overlap and drawn back toward where they are seen as far as they go). Each is one step, on the selection or every island, and refuses with a reason.
- **Align**, in the layout's modes, works on what the mode picks: islands whole; vertices to one line, or for the middles onto the straight line between the two farthest apart; each picked edge path on its own. Spread across or down evens islands' gaps or their middles; in Vertices and Edges it spreads a picked path's vertices along it, evenly or as the part's edges run, its ends staying put.
- **Texel density panel:** the density of the faces picked on the part, the selected islands or all, in pixels per centimetre of the part at 512, 1K, 2K, 4K or 8K; Get reads it into a field, Apply scales the selected islands about their middles to it. Below, how much of the square the islands cover, overlaps counted once, and how much of it overlaps.
- **Select menu** (toolbar, and the right-click menu): Similar islands (the same piece and shape as the selected, within the shape tolerance), Island borders, Horizontal edges and Vertical edges (within the edge tolerance, of the picked edges or the islands'), Faces with no area and Flipped faces (picked on the part, as the viewport picks faces, and ringed on the layout where they are too small to see).
- **Behind the layout and padding** (View options): one of the material's UV textures shown behind the layout in every mode, and an exposure, in stops, for the artwork behind the lines (remembered); the padding a texture of a size bleeds into round each island, drawn as a soft band half the padding wide each side, islands closer than the padding marked in red and counted; Suggest picks 4 px at 1K, 8 at 2K, 16 at 4K; Use for Pack makes it Pack's margin.
- **Checker on the part:** a toolbar toggle (and View options, with the number of squares, a plain, lettered or numbered pattern and the UV set) draws the part in the viewport with a checker in place of its colour, so stretch, turns and mirrored islands show on the part. It follows the UV set the editor shows, or one picked. It is drawn in Wireframe, Solid and Material Preview, by the graphics card or, where it cannot, the processor's Solid; Behind the layout can show it on the canvas too. Never in Rendered, never in a render, not saved, the material untouched and nothing in the history.
- **Shape tools** (the Unwrap, Shape and Symmetry panels, below Islands; `app/src/uv_editor/shape_tools.rs`): each works in the selection mode there is - on the picked vertices, edges or islands, on the faces picked on the part where it takes faces, or on every island when nothing is picked - and says in red why when what is picked is not what it works on ("Could not lay out as a grid: 12 faces aren't four-sided"). The engine works out where the layout's vertices go and the editor sends it as one `uv.set_positions` (with a pack after, or a cut before, where the tool needs one): one step to undo, the labels on the islands carried along.
- **Unwrap panel:** Unwrap again (Alt U) with a popover: Direction (both, across only, down only - one way solves only that coordinate again and keeps the other exactly), Method (conformal, or least stretch: as-rigid-as-possible rounds from the conformal layout, also used by the toolbar's Unwrap) and what each island keeps of where it was (its place, size and turn, each on its own; without its place the layout is packed after, and the whole set unwrapped again cuts where an island will not lie flat, as before). The stretch left is reported. Then: Straight line (Alt L) lays a picked path of edges or vertices on a straight line across or down, spaced and as long as on the part, the rest of the island laid out again round it (Keep pins holds the pins too); Grid (Alt G) lays four-sided faces - triangle pairs across their longest edge - out in straight rows and columns, spaced as on the part or evenly, fitted to the square if asked, a few odd triangles at a piece's end following; Rectangle (Alt B) makes an island's border, or four picked corners' border, a rectangle - sides spaced as on the part or evenly, sized by the average or the longer of opposite sides, the inside laid out again or spread evenly, the border pinned after if asked; Project (Alt P) lays faces flat as seen straight on, each piece on its own or all down one direction, taking in the whole island if asked, at the layout's density times a scale. Project is also in the Islands panel's section on the faces picked on the part.
- **Shape panel:** Relax (Alt R) smooths the picked vertices, or the inside of the selected islands, toward less stretch, a number of rounds, stopping before a fold, the border and pins held; Circle (Alt O) makes a picked loop - or, with Whole region, an island's border - a circle as long round as it was, spaced as on the part or evenly; Grow and Shrink move the selected islands' outer borders out or in by a distance in millimetres or pixels of a texture size, the inside following, eased where an island is thinner; Weld nearby (Alt W) brings picked vertices within a distance together, nearest pairs first, never two corners of one triangle, joining seams where copies meet; Match seam sides (Alt M) lines the picked seam vertices' copies up - at one height, one above the other, or at one place a hair apart so the seam stays a seam - both to the middle or the other side to the picked side, the islands carried along; Cut by angle cuts along edges sharper than an angle (30° by default) on the selected islands or the whole part, and unwraps again keeping the cuts if asked; while its options are open the edges it would cut are lit in pink.
- **Symmetry panel:** the part's mirror plane - X, Y, Z through the middle of its box, or Auto, the one pairing the most vertices - with the share of vertices that have a mirror; the mirror line on the layout, vertical or horizontal, through the layout's middle, the selection's, the 2D cursor, or the best fit (where the layout is nearest symmetric about, through the places on the plane of an island that crosses it); the side to copy from (Auto: the side with more of the picked vertices, else plus). Symmetrize (Alt Y) makes the other side the mirror image of that one, island by island with its own mirror, carrying along the places with none, and counts vertices with no mirror on the part; Snap to mirror puts picked vertices on their pairs' images, across both lines with Both ways for a part symmetric in two planes; Select one side selects what is on +X, −X, +Y, −Y, +Z or −Z of the part's middle in the mode there is; Select mirror selects the mirror image of the selection, in its place or added to it.
- **Snapping** is on by default and shown by the magnet. Drags land on the grid, guides, the layout's edges and middle, islands' boxes and middles, their vertices and edge middles once the view is close enough to tell them apart, and the other labels' edges and middles. The distance is 8 points on screen whatever the zoom. What was snapped to is drawn in pink, with a note by the pointer ("guide", "island centre"). Ctrl held during a drag drags freely.
- **Rulers** read in millimetres, centimetres or inches from the part's own size where its layout is one size all over, and in UV units, 1 across the square, where it is not. Their ticks keep a readable spacing at any zoom, and the pointer's place is marked on both. The corner switches the unit.
- **Guides** are dragged out of a ruler, moved by dragging, and removed by dragging back onto the ruler or with Delete. A double-click types a guide's place. They can be locked and cleared, and are saved with the part (`scene.set_uv_guides`).
- **Grid:** a spacing as a length on the part or a fraction of the square, split into fainter subdivisions.
- **Measure:** a drag draws a dimension with its length on the part and its angle. It stays until the next one or Esc. Shift keeps it to 0, 45 or 90°.
- **Layers:** the labels, top of the stack first, each with its artwork, an eye, a lock, its name (double-click to rename), its opacity and a grip to drag it up or down. Duplicate, delete and add a label from an image file. Below them, the material's textures on the layout: one selected is placed by its tile's outline (offset, repeats and turn). A hidden or locked label cannot be picked on the canvas.
- **Transform:** X and Y of one of nine points of the selection's frame, W and H with a proportion lock, the turn in degrees, flips and opacity, for one label or several.
- **Align:** to the selection, the island last clicked, the layout's square or the 2D cursor, by left, centre, right, top, middle or bottom, or by a corner; and spread across or down with equal gaps.
- **Keys** work only while the editor is under the pointer or has the keyboard: 1 to 4 pick what a click picks, arrows nudge (Shift ten times as far), Delete, Ctrl D, Ctrl A, Esc, F frames the selection and Home everything, Ctrl ' the grid, Ctrl ; guides, Ctrl Shift ; snapping, Ctrl R rulers; C cut, S stitch, P pin, Shift P unpin, Ctrl P pack, ] and [ quarter turns, Shift H and Shift V flips; Shift C the cursor to the selection, Shift U upright, Shift O sort, Shift S stack similar, Shift F fit to square, Shift X swap; Alt with U, L, G, B, P, R, O, W, M or Y for the shape tools. The keyboard button (or ?) lists them all.
- **Right-click** without dragging offers what can be done to the selection - in the layout's modes, the island tools, the shape tools in Shape and Symmetry submenus, an Arrange and a Select menu - and moves the 2D cursor; Shift and a right click puts the cursor down; a right-drag pans.
- **Undo:** each edit is one step: a label's as `material.set_decals`, a texture's as `material.set_mapping`, the guides' as `scene.set_uv_guides`, the pins' as `uv.set_pins`, the layout's as the `uv.*` commands - the arranging tools' as `uv.place_islands` (each island by a map of its own), and theirs and the shape tools' as `uv.set_positions` (vertices put where the tool worked out) - with the labels it carries in the same step.

---

## 8. The command bus, undo, and API parity

### 8.1 Contract

Every mutation is a command object with: a unique name, typed parameters, `apply`, `revert`, a **dirty-classification** (§01 §9.3), and a merge policy for coalescing (dragging a slider produces one undo entry, not two hundred).

```rust
trait Command {
    fn name(&self) -> &'static str;                  // "material.set_parameter"
    fn apply(&mut self, doc: &mut SceneDocument) -> Result<Dirty>;
    fn revert(&mut self, doc: &mut SceneDocument) -> Result<Dirty>;
    fn merge_with(&mut self, prev: &dyn Command) -> bool;   // slider coalescing
}
```

### 8.2 Command catalog (representative; the full list is the API surface)

`scene.import` · `scene.delete_nodes` · `scene.group` / `ungroup` · `scene.set_transform` · `scene.set_visibility` · `scene.set_pivot` · `scene.reparent` · `scene.set_units` · `scene.retessellate`
`material.assign` · `material.unlink` · `material.set_type` · `material.set_parameter` · `material.graph_edit` · `material.save_to_library` · `multimaterial.create` / `set_active`
`label.add` / `remove` / `set_transform` / `set_parameter`
`environment.set` · `environment.set_parameter` · `environment.pin_add` / `pin_edit` / `pin_remove`
`light.add` / `remove` / `set_parameter`
`camera.create` / `set_active` / `set_transform` / `set_lens` / `set_dof`
`lighting.set_preset` / `set_parameter`
`image.set_style` / `set_resolution`
`render.set_settings` / `start` / `cancel` / `queue_add`
`animation.add_track` / `add_keyframe` / `set_interpolation` / `set_range`
`studio.create` / `update` / `delete` / `activate` · `modelset.*` · `configurator.*`

### 8.3 Python API is generated from this catalog

```python
import tequila as fs

scene = fs.open("product.tequila")
red = scene.materials["Paint Base"].duplicate(name="Paint Red")
red.base_color = (0.72, 0.05, 0.06)

for part in scene.find(metadata={"Material": "ABS"}):
    part.assign(red)

scene.environment.rotation = 0.35
scene.camera("Hero").activate()
scene.render(output="out/red_hero.png", samples=512, denoise=True,
             passes=["Beauty", "Cryptomatte", "Depth"])
```

Every method is a thin wrapper over a command. **CI asserts that every command in the catalog is reachable from Python and from the UI** (§05 §3.8). This is what keeps the automation surface honest as the app grows.

### 8.4 Undo

Unlimited by default (configurable), with a visible history panel listing named steps. Commands journal to disk for crash recovery (§03 §4.5). Non-mutating actions (selection, camera navigation, panel layout) are **not** undoable — putting camera moves in the undo stack is a common and infuriating mistake.

*As built, November 11:* an import or an update (`scene.import`, `scene.reimport`) that brings a mesh, a material, a texture or a node under a key the scene already uses is refused, before anything is written. Taken, it put what it brought in place of what was there, and its undo then took the key away altogether: a part was left with no mesh, and the project saved after it would not open ("node2 references missing mesh 1"). Tequila's own imports reserve their keys and never met it; a script's command need not, and the documentation's own example of an update, applied to the Headphones sample and undone, did. Found by applying every command's example to a live scene, undoing each and opening the project saved after each step (`an_update_under_a_key_in_use_is_refused`).

---

## 9. Library window

*As built, November 8:* the two things called Studios now have names of their own. The Library keeps **Studios** - whole studios (lights, cameras and views, floor, look) put on the product in one step, called studio presets in the files and the scripting channel (`docs/studio-presets.md`). The Project panel's Studios tab is **Snapshots** - the project's saved shots, with its turntable, model sets, variants and configurator - and one saved shot is a Snapshot ("+ Snapshot from the current shot", Snapshot 1, Next and Previous Snapshot on the 3D mouse, `U`). Only the words changed: the files, the commands (`studio.set`, `studio.apply`), the scripting channel's `studios`, the command line's `--studio` and `--all-studios`, and `{studio}` in file names keep their names, and a script may still open the tab as `studios`. Screenshot (`P`) is the viewport's picture saved as a file, and is neither.

- **Categories**: Materials, Environments, Backplates, Textures, Colours, Models. Tree of folders plus flat tag filtering.
- **Sources**: bundled Standard Library (read-only), user library (writable, path configurable), project-local (materials in the open document), and optional shared network folders.
- **Search**: fuzzy over name, tags, and category; instant filtering.
- **Thumbnails**: rendered by the actual renderer on a standard preview scene (sphere/cloth/shaderball selectable), generated lazily on first display, cached on disk keyed by content hash, regenerated when the material changes. Size slider. *As built:* the standard library ships its thumbnails, rendered with the release (`thumbnails/look-N` beside the presets), so the shelf is full the first time it opens; anything else is rendered by one background worker, the tiles on screen first and then the rest of the shelf, whether or not the window is drawing.
- **Favorites**, **recently used**, and **used in this project** as virtual folders.
- Right-click: Apply to Selection · Rename · Duplicate · Edit in Material Graph · Show in File Browser · Delete · Export.

*As built, November 11: MaterialX documents* (`crates/tequila-import/src/materialx.rs`). File › Import material file, a file dropped on the window and a script's `library` `import` take a `.mtlx`: the file V-Ray, Arnold, Karma, Redshift and Blender exchange a material in, and the one the free libraries publish in. Every material in the document goes into the user's library, its pictures copied in with it.

- Read: the four surface models everything is written in - Autodesk Standard Surface, OpenPBR Surface (whose inputs are Tequila's parameters by name), glTF PBR and USD Preview Surface - and each input that is a picture (`image`, `tiledimage`, the glTF and USD picture nodes; straight, or through a `normalmap`, an `extract` or a node graph's output) as an Image node on that parameter, by the mesh's UVs with the picture's own tiling. A number read from a picture is read from the channel the document names.
- Said, input by input, in a notice: an input driven by any other network of nodes, which keeps the value the model gives it; a picture that is missing; an input with no place here. A document from before MaterialX 1.38 is refused with that said.
- Not read: V-Ray's own `.vrmat`, which there was no file to build against.
- Tests: `a_standard_surface_comes_across_with_its_pictures`, `the_other_models_come_across_and_what_does_not_is_said`, `a_document_goes_into_a_library_with_its_pictures`.

*As built, November 11: four ways to see the shelf* (`app/src/library_tab.rs`, `View`). Four small buttons beside the count choose it, each a drawing of what it gives, and Preferences › Interface › *Show materials as* is the same choice, kept between sessions (`interface.library_view`):
- **Thumbnails**, as it was: pictures with their names, sized by the "…" menu's slider.
- **Icons**: small pictures alone, four or more across, the name in the tooltip - the whole shelf at a glance.
- **List**: a line each, a swatch and the name, with what the material is at the end where the shelf is wide enough. Three times and more the materials of thumbnails in the same height, for finding one by name.
- **Cards**: a row each with the picture large, the name, its type and category, and whose it is (Tequila's, yours, a plugin's) with the words the search finds it by.

Every view has the same star, the same dot for a material already in the project, the same right-click menu, double-click and drag, and asks previews only for what it shows and the next screenful, at the size it draws them (`the_shelf_is_shown_four_ways`). The shelf's five filters are each as wide as their word needs: in equal shares "Favorites" and "In project" ran into each other (`segment_widths` in `theme.rs`, which every such control uses).

---

## 10. Animation timeline

- Track list mirroring the scene tree, one row per animated node with sub-rows per animated property.
- Animation blocks are draggable and resizable; dragging the edge changes duration, dragging the body moves it, `Alt`+drag duplicates.
- **Curve editor** below or beside: keyframe handles (auto, linear, stepped, Bezier), per-keyframe visibility toggle (2026.1 behavior), box-select on keys, value/time numeric entry.
- Playback: play/pause (`Spacebar` when the timeline has focus — note the conflict with Project-window toggle; timeline focus wins), loop, frame step (`,` / `.`), scrub, and a real-time-vs-every-frame quality toggle.
- Motion Blur and Motion Ease per animation block.
- **Animation Wizard**: a modal offering Turntable (object or camera, duration, revolutions, axis), Exploded View (auto-computes explosion vectors from part bounding boxes and assembly hierarchy — offer this, it is the single most-requested product animation), Camera Fly-through (pick waypoints), Part Reveal, and Studio Sequence. Every wizard result is an ordinary set of tracks the user can then edit; nothing is a black box.
- Studio Switch events as markers on a dedicated track.

---

## 11. Render window, queue, and gallery

- **Render dialog** tabs: Output (name, folder, format, resolution, print calculator, alpha, metadata sidecar), Options (quality mode and parameters, lighting preset, denoise, region), Layers & Passes (checkbox list with per-pass format), Queue.
- **Queue**: add still/animation/configurator-batch jobs; reorder; pause/resume; per-job progress with time remaining; render locally or send to network workers. Jobs persist across app restarts.
- **Gallery**: browse completed renders with their settings; compare two renders side-by-side or with an A/B wipe; re-open a render's settings into the current scene ("restore these settings"); reveal in file browser.
- **Progress during render**: live image with sample count, elapsed and estimated remaining time, and cancel/pause. A rendering job must never block the UI or the viewport.
- **Spectral** (Quality section): Auto, On or Off, for Tequila's renderer (§01 §7.4). Auto, the default, traces wavelengths wherever a material disperses light — glass or a gem with an Abbe number — so a diamond shows its fire, and RGB everywhere else, which is faster; On always traces wavelengths; Off is always RGB, where a dispersive material still splits light but by one wavelength per path, with more colour noise. The viewport follows it. Greyed out, with a tooltip saying why, when the engine is Cycles, which renders RGB.
- **Render wireframe**: beside Render, the same job drawn as the model's edges instead of traced - in a second or two, sharp at any size, into the same folder under the same name template. The Wireframe section chooses every triangle's edges (the viewport's Wireframe) or the parts' own edges and outline (a line drawing), hidden lines faint, hidden or shown, the line width in pixels of the image, and the colours, with Dark and Paper presets; a file with alpha leaves the background out. The project's own settings stay a photo, so the next Render renders. From the command line, `--wireframe` (`--edges`, `--hidden-lines`, `--line-width`, `--paper`).

*As built, November 8: a final render shows itself refining, and its floor's reflection no longer doubles its time* (`app/src/batch.rs`: `render_cycles`, `cycles_preview`, `FloorReflection`, `floor_layer`; `native/cycles/tequila_cycles.cpp`: `fsc_create_watched`, `CaptureDriver::update_render_tile`, `fsc_render_preview`; `tequila-render-cycles`: `Session::watched`, `Session::render_watched`; `tequila-render-core` `catch_shadow`). Found recording the launch video: the OVRA headphones, dressed, in the Library's soft studio (reflective floor), queued at 1920 x 1080, 256 samples, denoised, on Cycles on the graphics card (RTX 2060, Ryzen 7 2700X).
- **The picture refines on either engine.** A Cycles render showed "Preparing the first preview…" until it was done, however it was started: Tequila's own renderer sent the window a picture every second, and the Cycles path never did - its sessions ran headless, which turns Cycles' display updates off, and nothing asked it for one. A queued Cycles job now runs in a watched session: not headless, so Cycles hands its output driver the render so far after each stretch of samples, as Blender's render window is fed. The library shrinks that by averaging to the size of the Render window's stage (`BatchRenderer::show_previews_at`; none at all while the window is closed), and the batch lays it over the backdrop and develops it on a thread of its own. Each picture reads Cycles' whole render buffer back from the card, 60 to 100 ms at 1920 x 1080, so the library makes one a second at most and keeps them to a hundredth of the render's time: five in a 56-second render. While the floor's reflection is still being made, Cycles' finished picture shows. A Cycles library from before renders without pictures, as before.
- **The floor's reflection is made alongside Cycles.** Cycles is left without the floor's reflection, and Tequila's renderer made it on the processor after Cycles was done - 57 seconds after Cycles' 56, with the header frozen at "Floor reflection · 256 / 256 samples · 0m 56s". It now starts with the job, on threads of its own below normal priority, so Cycles' own threads and the interface come first; once the job is only waiting for it, at normal priority. It is the same reflection at the same samples, half the image's: its shadow, which was traced and thrown away, is no longer traced (the floor sampled the same), and it is denoised round the band it fills, with 128 pixels of margin, instead of over the whole frame (11 s of the processor at 1920 x 1080 became about 5). While the job waits for it, the header says how far it has got ("Floor reflection 96 / 128") and the time goes on.
- **The time is the job's.** A job's seconds - its card, `query jobs`, the finished image in the gallery, the sidecar - are its whole time, from its start to its file saved; a Cycles job reported Cycles' part alone.
- Measured on that scene, the Render window open, the viewport rendered, two runs each: before, Cycles 56.3 and 57.4 s, then the reflection 57.2 and 57.3 s, 114.5 and 115.7 s to the file, the job saying 56.9 and 57.8 s; now Cycles 57.3 and 59.2 s, the reflection done 63.1 and 61.9 s into the job, 65.0 and 63.8 s to the file, the job saying 64.8 and 63.6 s. The new application with the Cycles library from before (no pictures) took 56.1 s for Cycles and 65.3 s in all. The pictures took 0.35 s of a render's 57 - within the spread between runs. The final image is the one it was, but for a few reflection pixels the band's denoise sees a little differently: a mean difference of 0.006 of 255, at most 13, more than 2 in 0.02% of the pixels; the shadow left out changes nothing. Held by `a_queued_cycles_render_shows_its_picture_as_it_goes` (Cycles on the processor: pictures at the window's size while it renders, none with the window closed), `a_job_reports_its_whole_time`, `the_floor_stage_shows_its_progress`, `a_preview_fits_the_window_at_the_image_shape` and `the_floor_reflection_is_denoised_round_its_band`.
- What is left: the last 18 s or so of Cycles' 56 are its own denoise, on the processor (the library leaves its denoise device at Cycles' default, the CPU), which takes every core and holds the floor's reflection back for its last samples; denoised on the card it would be seconds, and the wait for the reflection would go. Rendering the reflection at three eighths of the image's samples instead of half (96 for 256) finished it well within Cycles' time - 59.7 s to the file - at a mean difference of 0.016 and at most 15 of 255, so the samples were left as they were.

---

## 12. Preferences

*As built, November 5:* where Adobe's Substance tools are, when they are not in a default folder, is set from the Substance section of the Material tab (**Set the folder of Adobe's tools…**) or the scripting channel's `substance tools`, and kept in `substance.txt` beside the person's other settings; `TEQUILA_SUBSTANCE_TOOLS` overrides it.

*As built, November 5:* Edit > Preferences opens a window with the 3D mouse's settings, kept in `3d-mouse.txt` beside the person's library:
- on or off, with the device found shown;
- speed (0.1 to 4);
- dead zone (0 to 40%);
- what turning goes about;
- only the strongest axis;
- each axis inverted, with a live bar of what the view takes from it;
- what the two buttons do.

*As built, November 9:* on a Mac with 3DxWare running, the device is read through the driver (§6.1). When opening it fails with the exclusive-access error, the reader loads `/Library/Frameworks/3DconnexionClient.framework`, installs its message handlers (`SetConnexionHandlers`, or `InstallConnexionHandlers` on a driver older than 10) and registers as a client: signature `'****'`, the executable's name as a Pascal string (`Tequila` in the application), mode 1 so the buttons are Tequila's and its own button settings apply, mask `0x3FFF` for the six axes and the buttons, and every button. Each `'3dSR'` state addressed to this client is read: command 3, the six axes as the driver's settings make them, through the same conversion as the HID reports with room up to six times full deflection for a driver set fast; command 2, the buttons. The messages come through a run loop, the reader thread's (which it turns) or the main thread's, and are handed to the reader by a channel. HID still looks every two seconds whether the device is plugged in; when it is gone, or reading stops, the client is unregistered, the handlers removed and the view let go. The preferences say "Connected: <device>, through 3Dconnexion's driver" and that 3Dconnexion's own settings for Tequila apply as well. The framework is loaded once and kept. When it is missing or the driver refuses, the preferences say "3Dconnexion's driver is using the 3D mouse, and Tequila couldn't connect to the driver. Updating 3DxWare may help; or quit 3DxWare, and Tequila reads the mouse itself.", with the reason and hidapi's words under **Details**. A copy signed with a Developer ID for the hardened runtime carries one entitlement, `com.apple.security.cs.disable-library-validation`, on the application only, so the framework signed by 3Dconnexion may load (`scripts/package-macos.sh`). Held by `a_state_reads_as_the_hid_reports_do`, `the_client_is_named_after_the_executable` and `states_from_the_driver_arrive_as_events`, which drives the reader through a stand-in for the framework. **Not tested on a device with the driver**: no SpaceMouse or 3DxWare was at hand.

*As built, November 8:* on a Mac, 3Dconnexion's driver (3DxWare) opens the device for itself, and opening it fails with IOKit's exclusive-access error, `0xE00002C5`. The preferences then say "3Dconnexion's driver is using the 3D mouse, so Tequila can't open it while the driver runs. Support through the driver is coming." with hidapi's own words under **Details**, and the log keeps them, once rather than at each retry (`OpenProblem` in `app/src/spacemouse/mod.rs`, test `the_driver_holding_the_device_is_said_plainly`). Any other failure is shown as it was, no longer as "Connected:". `query ui` reports it as `spacemouse.problem`.

*As built, November 6:* in a portable copy, one with a `portable.txt` beside the program, every one of these files is in its `data` folder instead of the person's settings folder (`docs/05-ROADMAP-AND-TESTING.md` §5). Help > About says the copy is portable and opens that folder.

*As built, November 11:* the Preferences window of §12.1 and §12.2, its first part (`app/src/preferences.rs`). Edit › Preferences, **Ctrl+,** (**⌘,** on a Mac) and, in a Mac's menu bar, Edit › Settings… open it: a window of its own with the pages down the side, a search across every setting's name, explanation and choices, and each change acted on at once. A setting not at its default has a mark beside it that puts it back and says what the default is; each page has *Restore this page's defaults*, and Advanced has *Reset every setting…*, which asks first, with *Export…* and *Import…* of every setting as one text file, the 3D mouse's included.

Every setting is a row of one table (`SETTINGS`): its name (`viewport.gizmo_size`), page, heading, words, what it takes, its default, and how the application reads and takes it. The window, the search, the reset marks, the exported file and the scripting channel's `preferences` operation (`docs/SCRIPTING.md`; `tequila_preferences` for an AI application) are drawn from it, so a setting added to the table is in all of them. A value is checked before it is taken: a number held to its range, a choice one of its own.

The pages and settings built:
- **General**: the start screen at launch *(was the appearance picker's)*; the recovery copy on or off and how often (1 to 60 minutes; off, each change is still journalled); how many recent projects are listed (0 to 30), and *Clear the list*.
- **Interface**: light, dark or auto, the colour theme and the interface size *(the appearance picker's, which stays)*; tooltips on or off and their delay.
- **Viewport**: what a session starts with - its shading, the ground grid, axis legend, render frame, statistics, model edges, the selection's outline and the gizmo (each still switched for the session from the View menu or its key); the gizmo's size (50 to 250%) and the mode it starts in; for the Rendered view, *While the view moves* - Smooth, Balanced or Sharp, the time a moving picture is given (22, 33 or 50 ms, `cycles_view`) - and the samples a still Cycles view refines to (16 to 16,384).
- **Navigation**: whose way the mouse moves the view, and the three drags of one's own (§6.1); orbit, pan and zoom speed (0.25× to 4×); the wheel's zoom reversed (a pinch keeps its direction); whether a trackpad's two-finger swipe pans or orbits, Shift doing the other; and the 3D mouse's settings, which were the whole window before.
- **Keyboard**: every key of the main window, changed there (§6.3).
- **Import**: the up axis of imports *(File › Up axis of imports, which stays)*.
- **Folders**: the screenshot folder *(it was View › Screenshot folder…)*, and Tequila's settings and library folders, opened.
- **Updates**: the quiet look for a new version at launch.

Tequila's newer choices are kept in `preferences.txt` beside the others, a `name=value` line each, and a line that is not understood keeps its default. The older ones - the look, the up axis, the screenshot folder, updates, the 3D mouse - are still kept in their own files by the code that owns them: **the one settings file of §12.1 is not built**, and nor is telling a project's defaults from the application's choices, since none of these settings is a project's. Not built either: the pages Rendering, Performance and memory, Colour management, Materials and library, Assistant, and Plugins and scripting; autosave versions, undo depth, units and language; the outline's colour and width; power settings; and, on a Mac, the item in the application menu (it is in Edit: the application menu is winit's). Held by the tests in `preferences.rs`, `ctrl_comma_opens_preferences`, `a_script_opens_preferences_and_changes_a_setting` and `a_swipe_follows_the_navigation_settings`.

The rest is specified below and not built yet (planned November 10). The aim is that a person can make every choice Tequila makes for them, in one window: today's few settings are spread over the appearance picker, the File and View menus, the Material tab and this window, and much of the rest is fixed in the code.

### 12.1 How the window works

- **One window, opened the usual way:** Edit › Preferences, `Ctrl+,` on Windows and Linux, and Tequila › Settings… with `⌘,` on a Mac. A page list down the side, in the order of §12.2.
- **Search** across every page by a setting's name and its explanation; matches are lit and their pages listed.
- **Applied at once,** with no OK or Apply button. Each setting can be put back to its default on its own (a reset mark appears beside a changed one), and each page has *Restore defaults*.
- **Tequila's own choices and a new project's defaults are told apart.** A page or group that sets defaults for new projects says so; an open project's own settings stay in the Inspector and the Render window, and are not changed by changing a default. Where a setting has both, the project's wins.
- **Every setting explains itself** in its tooltip, and says what it costs where it trades speed or memory for quality.
- **Export and import** all settings as one file, to move to another computer or to give a team one standard; *Reset everything* asks first.
- **A portable copy** keeps them in its `data` folder, as today (§5 of `docs/05`).
- **Scripts and the assistant** read and change every setting through the scripting channel (a `preferences` operation, with its names in `docs/SCRIPTING.md`), as they do every command (§8).
- **One settings file.** Today's are many small ones beside the library: `appearance.txt`, `import.txt`, `guides.txt`, `screenshots.txt`, `3d-mouse.txt`, `substance.txt`, `updates.txt`, `farm.txt`, `assistant.json` and others. They become one readable file, read from the old ones once, on the first start after the change.

### 12.2 Pages

Marked *(built)*: a setting that exists today, which moves here from where it is.

**General**
- On start: the start screen *(built: appearance)*, the last project, an empty scene, or a chosen scene.
- Autosave on or off, how often (fixed at five minutes today, `project.rs`), and how many earlier versions to keep.
- How many steps undo keeps. How many recent files the start screen and File › Open recent list, with *Clear*.
- Ask before closing with unsaved changes, and before deleting parts.
- Default units (mm, cm, m, in, ft) and how many decimals fields show.
- Language, once Tequila is translated.

**Interface**
- Light, dark or automatic, and the colour theme *(built: appearance)*.
- Interface size *(built: appearance)* and text size.
- Tooltips on or off, their delay, and whether they show the shortcut.
- Panel layout presets, and *Reset layout*.
- Fewer animations.

**Viewport**
- The shading a project opens in: Solid, Material Preview or Rendered.
- While the view moves, smooth or sharp: *Smooth*, *Balanced*, *Sharp*, the picture time the Rendered view aims at (33 ms today, `cycles_view.rs`).
- The Rendered view's sample limit (1024 today) and time limit.
- The viewport's denoising: on, off, or only once the view is still.
- Shown when a project opens: ground grid, axis legend, render frame, statistics, model edges, composition guides with their colour and opacity.
- The selection's outline: shown or hidden (`Shift+H`), its colour and its width.
- The gizmo: shown or hidden (`Shift+G`), its size (90 points today, `gizmo_ui.rs`), and the mode it starts in (`Q`, `W`, `E` or `R`).
- Power: pause the viewport while Tequila is in the background, and lighter viewport settings on battery.

**Navigation and input**
- The mouse's navigation preset: KeyShot (the default, §6.1), Blender, Maya, SolidWorks, Rhino, Fusion 360, Cinema 4D, or Custom, which shows each binding to change.
- Orbit, pan and zoom speed (fixed today, `orbit.rs`), and the wheel's zoom turned round.
- Zoom toward the pointer or the view's centre; orbit about the selection, the point under the pointer or the view's centre; keep the horizon level or orbit freely.
- On a Mac's trackpad: whether two fingers pan or orbit, and pinch to zoom.
- The 3D mouse: everything in this window today *(built)*.

**Keyboard shortcuts** (the hotkey editor of §6.3)
- Every command, found by its name or by its key, given any key.
- A conflict said plainly as it is made ("Ctrl+G is already Group"), and the choice to swap or cancel.
- Presets (Tequila, KeyShot, Blender), import and export, and reset.

**Rendering** (a new project's defaults)
- Engine: Cycles on the graphics card, Cycles on the processor, or Tequila's own; which graphics card or cards; how many processor threads.
- Resolution, samples or time limit, denoising and its strength.
- Output: format (PNG, JPEG, TIFF, EXR), bit depth, transparent background, the file name pattern (for example `{project}_{view}_{date}`) and the folder.
- When a render finishes: open the image, a sound, a notification.
- The network render farm *(built: Render window)*.

**Performance and memory**
- How much of the graphics card's memory Cycles and the denoiser may take (capped in the code today; `docs/01` says how).
- Texture memory and texture compression.
- How long the Cycles view is kept after the viewport leaves Rendered (ten seconds today).
- A low-power mode, and how many threads background work may use.
- Caches: where they are, how big, and *Clear* for each.

**Import**
- The up axis of imports *(built: File › Up axis of imports)*, the units of a file that names none, and setting models on the floor.
- How finely STEP and IGES surfaces are cut into triangles.
- Parts divided by body, colour or layer.
- Materials made from CAD colours, and the CAD file kept in the project.

**Colour management**
- The OCIO configuration, the display, the view transform (Standard, Filmic, AgX, ACES), and the output transform for each file format.

**Materials and library**
- The library's folders, a shared network folder among them, and favourites.
- Where Adobe's Substance tools are *(built: Material tab)*.
- Thumbnails' size and quality.
- What a material dropped on a part does by default: that part alone, or every part sharing its material (§3.2).
- The material imported parts wear.

**Folders and storage**
- Library, renders, screenshots *(built: View › Screenshot folder)*, autosaves and caches, each with its size, *Open* and *Clear*.

**Assistant**
- Provider and model: Claude, OpenAI, Ollama or a local model *(built: the Assistant)*, keys kept in the system's keychain.
- Whether the assistant asks before changing the scene, and whether it may look at screenshots.
- Connect an AI *(built)*.

**Plugins and scripting**
- Plugins on or off *(built)*, where Python is, the plugins folder.
- Whether other applications and AI tools may drive Tequila at all (the scripting channel and the MCP server), and whether each new one is asked about first.

**Updates and privacy**
- Look for updates daily *(built: daily, Help › Check for updates)*, weekly or never; stable or beta releases.
- Crash reports only when the person agrees; no telemetry.

**Advanced**
- Developer overlays and logs: the profile (`TEQUILA_PROFILE`), the Cycles trace (`TEQUILA_CYCLES_TRACE`).
- Open the settings folder; reset everything.

---

## 13. The progressive-feedback contract

What the user sees while an image converges determines whether the app feels fast — often more than raw throughput does.

1. **Never show a blank or black viewport.** On any restart, the first pass renders at reduced resolution and is denoised immediately, so the user sees a plausible image within ~100 ms rather than watching noise resolve.
2. **Never block input.** Camera moves interrupt the current pass immediately; the renderer discards in-flight work rather than finishing it.
3. **Converge visibly but calmly.** The image should get quieter, not flicker. Denoise cadence backs off as samples accumulate (§01 §9.2) precisely so the picture stops visibly changing once it is close.
4. **Show progress that means something.** Percentage converged, not just sample count.
5. **Be honest about degraded modes.** When the GPU fell back to CPU, when a feature is unsupported on the current backend, when displacement is still rebuilding — say so in a toast, briefly, once.
6. **Parameter edits must feel instantaneous.** Image Style edits re-tonemap the existing buffer with no re-render at all (§01 §9.3) — dragging exposure must be as smooth as dragging a slider in a photo editor. Material edits restart only the affected region. If a user can perceive a stall while dragging a slider, that is a bug with a specific, fixable cause.
