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

### 1.2 Look and identity

Tequila's look is its own, not KeyShot's or Blender's: friendly but professional, in light or dark, in the person's choice of theme (`app/src/theme.rs`).

- **Appearance** (the sun or moon at the top right): Light, Dark or Auto (the desktop's); one of six themes; and a size, Compact, Regular or Large. Kept in `appearance.txt` beside the user's library. Every colour is read from the current palette, so all of it changes together: panels, viewport tools, the node editor, the console, the Render window.
- **Themes**: Graphite (neutral greys, teal), Ocean (blue), Orchid (violet), Ember (coral), Meadow (green), Rose (pink). Each has a light and a dark palette made for it. Every one is tested for contrast: body text at 7:1, secondary text and the accent's text at 4.5:1 (WCAG).
- **Graphite is the neutral one**, with equal channels in every grey. A tinted surround tints the render, so Graphite is the default and the picker says so. The others trade a little neutrality for character. The viewport itself is always the render's own picture.
- **Layout**: the frame (top bar, rail, status line) is a step off the panels; groups of controls sit on cards; floating windows and menus one step further.
- **The accent** means *active*: the selected part's outline, the chosen tab, a toggle that is on, a ticked box, a field being dragged. Apart from those, it appears only on the brand mark and the Render button.
- **Page hues**: each Inspector page has its own colour on its rail icon and its heading badge. Scene is blue, Material pink, Environment green, Lighting amber, Camera violet, Image orange, Studios sky and Scripting slate. A page is found by colour as much as by shape.
- **Busy amber** means work in progress somewhere else: a render converging, a script driving the session.
- **First run**: a welcome card over the viewport gives three steps (import, dress, render), each with its button, until it is put away. Help brings it back.
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

### 2.2 Overlays

- **Islands**: the viewport's tools float over it on dark translucent plates, and take their own clicks and drags (a drag that starts on one never turns the camera).
  - Top left, a chip naming what the view shows. A hollow dot marks a preview, not the render, so a material is not judged by it. In Rendered, the dot is amber while the image converges, teal when it has settled and grey when paused, with a thin progress line under it.
  - Top right: frame all and frame selection, the four shadings, the viewport's denoise switch and pause, and Screenshot.
  - *As built, November 8:* the denoise switch (Lucide's wand-sparkles, lit while on) denoises the Rendered view or shows its raw samples, apart from the final render's Denoise in the Render window and Image › Denoise; strength, cadence and guides are shared. It is the render settings' `preview_denoise`, saved with the project and a step to undo; a project from before takes the render's `denoise`, and a snapshot leaves it alone. Switching it keeps the samples: the picture showing is denoised or not again at once (`differs_only_in_denoise` in `orchestrator.rs`, test `switching_the_viewports_denoising_keeps_the_samples`). The Screenshot button (Lucide's camera), **P**, and View › Screenshot save the viewport's picture as a PNG: developed and graded as shown, at the preview's own size, still refining or not, without the statistics, the gizmo, the outline or the grid - the renderer's image with what the outline drew over put back (`DisplayImage::without_outline`), or the graphics card's Solid read back as a frame draws it (`SolidView::take_capture`). It goes to `Pictures/Tequila Screenshots` (a portable copy's `data/Tequila Screenshots`), or the folder View › Screenshot folder… chose, kept in `screenshots.txt` beside the other settings. The name is the project's, or Untitled, with the date and time, and a second in the same second is `(2)`: never over a file (`app/src/screenshot.rs`). A notice names the file with an **Open folder** link, which opens the folder on it; a notice with a link stays six seconds, and while pointed at.
  - *As built, November 9:* the preview is held to the final render's pixels. It has the viewport's own pixels (a Retina screen's, or a screen set above 100%), but no more of them than the Render settings' size has, in the viewport's shape, and is drawn stretched over the rest (`preview_pixels` in `main.rs`; the Rendered view and Material Preview's path-traced frames, Cycles' and Tequila's renderer's alike; the graphics card's Solid draws at the screen's size, as it costs nothing). On a 27-inch Retina screen the viewport was 5120 x 2880 pixels, traced and denoised, for a 1920 x 1080 render: on the starter scene a clean picture took 8.9 s after a move and 9.7 s after letting go, orbiting 12.3 pictures a second; held to 1920 x 1080, 1.2 s, 2.7 s and 26.6 (`retina_viewport_benchmark`, an M3 Max). A viewport with fewer pixels than the render keeps its own; changing the render's size in the Render window resizes the preview. The statistics' size is the preview's. Test: `the_preview_has_no_more_pixels_than_the_render`.
  - Down the left: Move, Rotate, Scale (dimmed with nothing selected), World/Local space, and the snapping menu.
- **Axis legend** (bottom-left), toggleable. Its backing disc is faint until the pointer reaches it.
- **Ground grid** with real-world spacing labels when enabled in the Camera tab.
- **Selection outline**: a crisp 2 px outline in an accent colour, drawn as a post-process from the instance-ID buffer (not a wireframe overlay — outlines read better and cost nothing since the ID buffer already exists).
- **Toast notifications**, bottom-centre, auto-dismissing: "Denoising", "Rebuilding BVH", "Re-tessellating", "GPU memory exceeded — fell back to CPU", "Feature X unsupported on GPU backend". Never a modal dialog for these.
- **Safe-frame / aspect guides** when the render aspect differs from the viewport.
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

*As built, November 8:* **P** saves a Screenshot and **Shift+P** pauses the viewport, which P did before; **Ctrl+A** selects all only without Shift, and **Ctrl+Shift+A** (Cmd on a Mac) deselects all, beside Esc. The Shortcuts window and the right-click menu's *Deselect all* say so. Clicking empty space deselects in every view: a click on the floor - the shadow catcher Material Preview and Rendered trace, which has no part of its own - was taken as a click on something and kept the selection (`clicked_part` in `main.rs`, test `a_click_on_the_floor_deselects_and_a_click_on_a_part_selects`). The viewport's denoise switch is on its toolbar rather than `,`.

`E` conflicts between "Background = Environment" and "Rotate gizmo". Resolve by context: gizmo shortcuts apply when the viewport has focus and a transformable selection exists; otherwise the background shortcut applies. Document the resolution, expose both in the hotkey editor, and let users rebind.

### 6.3 Hotkey editor

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
- **Previews:** every texture and pattern node shows its output, made on a worker thread. A node is redone only when something upstream of it changed, and the old picture shows until the new one arrives.
- **The swatch:** the material rendered with the real path tracer on a sphere or slab the size of the model, so a pattern shows at the scale it will have on the part. A rough pass comes at once and a clean one follows.
- **Solo:** Ctrl Shift click a node to show its output on the model, unlit. Clicking again cycles through its outputs. **End solo** in the toolbar, or closing the editor, puts the material back.
- **Undo** is the global command stack: every edit is a `material.set_graph` command, and a drag is one step. Ctrl Z, Ctrl Y and Ctrl S work with the pointer over the editor.
- Still to come: node groups with exposed inputs, and a minimap for very large graphs.

### 7.1 UV editor

A part's surface laid out flat, with its labels and the material's UV-mapped textures placed on it by hand, laid out as a drawing program is (`app/src/uv_editor/`). It opens from View › UV editor, from a label's **UV editor** button in the Labels section (which unwraps a part that has no layout yet, as one undo step), or as the **UV layout** workspace, which puts the Material page beside it. It is a window of its own and remembers its size.

- **Layout:** a toolbar along the top - the tools, what a click picks, the island tools, what the canvas shows, View options, and on the right the part's name, Unwrap, Export layout and the shortcuts; rulers along the canvas's top and left; Layers, Transform, Align and Islands down the right; the pointer's place, what a drag snapped to, the selection with its size, and the editor's last word (what it did, or in red why it would not) and the zoom along the bottom.
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

---

## 9. Library window

*As built, November 8:* the two things called Studios now have names of their own. The Library keeps **Studios** - whole studios (lights, cameras and views, floor, look) put on the product in one step, called studio presets in the files and the scripting channel (`docs/studio-presets.md`). The Project panel's Studios tab is **Snapshots** - the project's saved shots, with its turntable, model sets, variants and configurator - and one saved shot is a Snapshot ("+ Snapshot from the current shot", Snapshot 1, Next and Previous Snapshot on the 3D mouse, `U`). Only the words changed: the files, the commands (`studio.set`, `studio.apply`), the scripting channel's `studios`, the command line's `--studio` and `--all-studios`, and `{studio}` in file names keep their names, and a script may still open the tab as `studios`. Screenshot (`P`) is the viewport's picture saved as a file, and is neither.

- **Categories**: Materials, Environments, Backplates, Textures, Colours, Models. Tree of folders plus flat tag filtering.
- **Sources**: bundled Standard Library (read-only), user library (writable, path configurable), project-local (materials in the open document), and optional shared network folders.
- **Search**: fuzzy over name, tags, and category; instant filtering.
- **Thumbnails**: rendered by the actual renderer on a standard preview scene (sphere/cloth/shaderball selectable), generated lazily on first display, cached on disk keyed by content hash, regenerated when the material changes. Size slider. *As built:* the standard library ships its thumbnails, rendered with the release (`thumbnails/look-N` beside the presets), so the shelf is full the first time it opens; anything else is rendered by one background worker, the tiles on screen first and then the rest of the shelf, whether or not the window is drawing.
- **Favorites**, **recently used**, and **used in this project** as virtual folders.
- Right-click: Apply to Selection · Rename · Duplicate · Edit in Material Graph · Show in File Browser · Delete · Export.

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

The other sections below are not built yet.

Sections: **General** (language, autosave interval, undo depth, startup scene) · **Interface** (theme light/dark, UI scale, panel layout presets, tooltips) · **Navigation** (application preset, invert axes, sensitivity) · **Hotkeys** (§6.3) · **Colour Management** (OCIO config path, display transform, view transform per output) · **Units** (default scene unit, display precision) · **Rendering** (default backend, GPU device selection, CPU thread count, texture cache size, default lighting preset) · **Folders** (library, output, temp, autosave) · **Advanced** (developer overlays, shading-language cache, telemetry off by default and opt-in only).

---

## 13. The progressive-feedback contract

What the user sees while an image converges determines whether the app feels fast — often more than raw throughput does.

1. **Never show a blank or black viewport.** On any restart, the first pass renders at reduced resolution and is denoised immediately, so the user sees a plausible image within ~100 ms rather than watching noise resolve.
2. **Never block input.** Camera moves interrupt the current pass immediately; the renderer discards in-flight work rather than finishing it.
3. **Converge visibly but calmly.** The image should get quieter, not flicker. Denoise cadence backs off as samples accumulate (§01 §9.2) precisely so the picture stops visibly changing once it is close.
4. **Show progress that means something.** Percentage converged, not just sample count.
5. **Be honest about degraded modes.** When the GPU fell back to CPU, when a feature is unsupported on the current backend, when displacement is still rebuilding — say so in a toast, briefly, once.
6. **Parameter edits must feel instantaneous.** Image Style edits re-tonemap the existing buffer with no re-render at all (§01 §9.3) — dragging exposure must be as smooth as dragging a slider in a photo editor. Material edits restart only the affected region. If a user can perceive a stall while dragging a slider, that is a bug with a specific, fixable cause.
