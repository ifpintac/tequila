# 04 — Interaction and UI Specification

Exact behavior of every interaction. This document exists because "drag a material onto an object" contains a dozen decisions that determine whether the application feels professional or broken, and none of them are guessable.

---

## 1. Window layout

### 1.1 Default arrangement

```
+-----------------------------------------------------------------------------+
| (o) Tequila  File Edit View Workspace Help   Untitled *   lib con ai | save Import [Render] |
+------------+--------------------------------------------+-------------+-----+
| Materials x| [o Solid  studio light]   [frame | shadings | pause] | Material x | Sce |
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
- **The inspector** sits against its rail: the tab's name and what it is for, then its page. The Assistant opens between it and the viewport.
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
  - Top right: frame all and frame selection, the four shadings, and pause.
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

### 6.2 Command hotkeys (defaults)

| Key | Action | Key | Action |
|---|---|---|---|
| `Space` | Toggle Project window | `M` | Toggle Library |
| `Tab` | Full-viewport mode | `H` | Toggle HUD |
| `E` | Background = Environment | `C` | Background = Colour |
| `B` | Background = Backplate | `G` | Flatten ground |
| `U` | Studios | `L` | Add label to selection |
| `W`/`E`/`R` | Move/Rotate/Scale | `F`/`Shift+F` | Frame selection / all |
| `Ctrl+G` | Group | `Ctrl+Shift+G` | Ungroup |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / Redo | `Ctrl+S` | Save |
| `Ctrl+I` | Import | `Ctrl+R` | Render |
| `Ctrl+P` | Screenshot of viewport | `Ctrl+D` | Duplicate |
| `Del` | Delete | `Ctrl+H` | Hide selection |
| `Alt+H` | Unhide all | `/` | Isolate selection |
| `P` | Pause/resume rendering | `,` | Toggle denoiser |

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

- **Categories**: Materials, Environments, Backplates, Textures, Colours, Models. Tree of folders plus flat tag filtering.
- **Sources**: bundled Standard Library (read-only), user library (writable, path configurable), project-local (materials in the open document), and optional shared network folders.
- **Search**: fuzzy over name, tags, and category; instant filtering.
- **Thumbnails**: rendered by the actual renderer on a standard preview scene (sphere/cloth/shaderball selectable), generated lazily on first display, cached on disk keyed by content hash, regenerated when the material changes. Size slider.
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

---

## 12. Preferences

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
