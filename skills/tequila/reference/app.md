# The window and Tequila's own settings

Most of the time you change the scene, not the window. Use this page to show the person something, to put the right thing on screen before a screenshot, and when they ask for one of Tequila's own settings to be changed.

## 1. What is on screen

`tequila_query {"what": "ui"}` says what is open: the `viewport` mode, the Inspector `tab`, which windows are open, the theme, and the viewport's size in pixels.

## 2. The viewport

```json
tequila_ui {"viewport": "rendered"}
tequila_ui {"viewport": "solid", "statistics": true}
tequila_ui {"full_viewport": true}
tequila_ui {"frame": true}
```

- `viewport`: `rendered` (ray traced, the true picture), `material` (fast, materials approximated), `solid` (fast, shape and colour), `wireframe`. Work in `solid` when moving things; judge looks in `rendered`.
- `statistics` shows samples, engine and frame rate. `full_viewport` hides every panel. `frame` shows the render's shape in the viewport. `gizmo` and `selection_outline` hide the move handles and the selection's outline for a clean screenshot.
- `tequila_screenshot {"viewport": false}` takes the whole main window, panels included, instead of just the picture; `{"window": "uv"}`, `"graph"`, `"render"` or `"preferences"` takes one of the windows of their own, which must be open.

## 3. Panels and windows

```json
tequila_ui {"tab": "material"}
tequila_ui {"render_window": true}
tequila_ui {"library": true, "inspector": true}
tequila_ui {"graph": 3}
```

- The Inspector's tabs: `scene`, `material`, `environment`, `lighting`, `camera`, `image`. Selecting a part first (`tequila_select`) makes the Scene and Material tabs show it.
- `render_window` is where renders, the queue and the finished pictures are. `library` is the material and studio library. `console` shows every request as it arrives. `graph` opens the Material Graph editor on a material (`false` closes it).
- Through `tequila_request {"request": {"op": "ui", ...}}` there is more: `"uv_editor": true` (the UV editor on the selected part), `"library_section": "studios"`, `"present": true` (the configurator full screen), `"guides": ["thirds"]` (composition guides), `"preferences_page": "viewport"` (the Preferences window on a page), `"mode": "light"` or `"dark"`, `"theme": "ocean"`.

Open things for the person when it helps them carry on by hand: after setting up render settings, open the Render window; after making a material, select the part and show the Material tab.

## 4. Preferences: Tequila's own settings

These belong to the person's Tequila, not to the project: they apply to every project and stay after it closes. Change one only when asked, and say what you changed. Inside the Assistant the person confirms each change.

```json
tequila_preferences {"set": {"viewport.gizmo_size": 120}}
tequila_preferences {"set": {"navigation.preset": "blender", "navigation.invert_zoom": true}}
tequila_preferences {"reset": "viewport.gizmo_size"}
```

`tequila_preferences {}` lists every setting with its value, default and what it takes, and every keyboard shortcut. That answer is long (about 19,000 characters): the settings are these, so ask for the list only for a setting's exact range.

| Setting | Takes |
|---|---|
| `general.start_screen`, `general.autosave` | true or false |
| `general.autosave_minutes`, `general.recent_projects` | A number |
| `interface.mode` | `light`, `dark`, `auto` |
| `interface.theme` | `graphite`, `ocean`, `orchid`, `ember`, `meadow`, `rose` |
| `interface.size` | `compact`, `regular`, `large` |
| `interface.tooltips`, `interface.tooltip_delay` | true or false; seconds |
| `interface.library_view` | `thumbnails`, `icons`, `list`, `cards` |
| `viewport.shading` | The mode a project opens in: `wireframe`, `solid`, `material`, `rendered` |
| `viewport.grid`, `viewport.axis_legend`, `viewport.render_frame`, `viewport.statistics`, `viewport.model_edges`, `viewport.selection_outline`, `viewport.gizmo` | true or false |
| `viewport.gizmo_size`, `viewport.samples` | A number |
| `viewport.gizmo_mode` | `all`, `move`, `rotate`, `scale` |
| `viewport.moving_picture` | While the view moves: `smooth`, `balanced`, `sharp` |
| `navigation.preset` | Mouse controls as in: `keyshot`, `blender`, `maya`, `cinema4d`, `3dsmax`, `solidworks`, `rhino`, `fusion360`, `custom` |
| `navigation.orbit_speed`, `navigation.pan_speed`, `navigation.zoom_speed` | A number, 1 as it is |
| `navigation.invert_zoom` | true or false |
| `navigation.trackpad` | Two fingers: `pan` or `orbit` |
| `import.up_axis` | `file`, `y`, `z`, `x`, `-y`, `-z`, `-x` |
| `import.blender_lights` | `ask`, `blender`, `tequila` |
| `folders.screenshots` | A folder's path |
| `updates.automatic` | true or false |

- All of a `set` are checked before any is taken; a wrong name or value is refused, naming it.
- `reset` takes `"all"`, a page (`general`, `interface`, `viewport`, `navigation`, `keyboard`, `import`, `folders`, `updates`), one setting, or a list of settings.
- **"The mouse works like in Blender / KeyShot / SolidWorks":** `navigation.preset`.
- **"Models always come in lying down":** `import.up_axis` `z`.

## 5. Keyboard shortcuts

A shortcut is a setting named `shortcut.<group>.<action>`:

```json
tequila_preferences {"set": {"shortcut.view.screenshot": "F12"}}
tequila_preferences {"reset": "shortcut.view.screenshot"}
```

The value is the key written out (`"Ctrl + G"`, `"Shift + H"`, `"F12"`), a list of two keys, or `""` for none. A key given to one action is taken from the action that had it. The exact names are in `tequila_preferences {}` under `shortcuts`, each with its `label` and current `keys`; read that list before setting one, since the names cannot be guessed.

## 6. Things in the window that the tools do not reach

Say where these are rather than trying another way:

- **Signing in to an AI service, API keys, which AI answers:** the Assistant's **Connect an AI…** window. Never ask for or handle a key.
- **Installing updates:** **Help → Check for updates**.
- **Rendering a turntable's frames:** the Snapshots tab (page `animation`).
- **Improve model** (subdivide, shade again) and **Units of this model:** right-click the model in the Scene tab (page `fixing`).
- **Network render nodes:** set up in the Render window.
