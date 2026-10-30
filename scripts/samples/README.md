# The sample scenes

`assets/samples/Headphones.tequila` and `assets/samples/Perfume bottle.tequila`
are the two finished scenes on the start screen. Opened from there, each
becomes a new project, so saving asks where and the shipped file stays as it
is. Both products are made up, OVRA and OREN, and modelled here to real
proportions.

They are built in three steps:

1. **The models**, in Blender 5:

   ```
   blender -b --factory-startup --python headphones.py -- build/headphones.glb
   blender -b --factory-startup --python bottle.py -- build/bottle.glb
   ```

   The bottle's lettering is in Blender's own font, Inter, under the Open Font
   License.

2. **The scenes**, from a build of Tequila's command-line renderer:

   ```
   python make_samples.py build out 512
   ```

   This imports each model, dresses it from the material library, lights it
   low-key with environment pins, renders a 512 by 288 preview and a
   1920 by 1080 shot, and saves the project with the preview inside.
   `TEQUILA_CLI` names the renderer, or the checkout's release build is used.
   `TEQUILA_CYCLES` says where Cycles is: the headphones render in Cycles, and
   the bottle, glass with a liquid inside, in Tequila's own renderer.

3. **Copy** `out/*.tequila` into `assets/samples`.

The packages carry the `.tequila` files only. The folder's model file,
`glasses.gltf`, is a fixture for the import tests.
