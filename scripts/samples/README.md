# The sample scenes

`assets/samples/Headphones.tequila`, `assets/samples/Perfume bottle.tequila`
and `assets/samples/palma P8.tequila` are the finished scenes on the start
screen. Opened from there, each becomes a new project, so saving asks where
and the shipped file stays as it is. The headphones and the bottle are made
up, OVRA and OREN, and modelled here to real proportions. The palma P8 is a
handheld designed for Tequila; its model, `models/palma-p8.glb`, comes with
its own materials and textures, and is kept here as it was made.

They are built in three steps:

1. **The models**, in Blender 5:

   ```
   blender -b --factory-startup --python headphones.py -- build/headphones.glb
   blender -b --factory-startup --python bottle.py -- build/bottle.glb
   ```

   The bottle's lettering is in Blender's own font, Inter, under the Open Font
   License. The palma's model is not built: copy `models/palma-p8.glb` into
   `build`.

2. **The scenes**, from a build of Tequila's command-line renderer:

   ```
   python make_samples.py build out 512
   ```

   This imports each model, dresses it from the material library (the palma
   keeps its own materials, its shell made solid, its screen drawn as an LCD,
   and very faint smudges, fingerprints and dust from `wear.py` on its shell
   and screen glass), lights it low-key with environment pins, renders a 512 by 288
   preview and a 1920 by 1080 shot, and saves the project with the preview
   inside. A fourth argument builds only the samples it names, as in
   `python make_samples.py build out 512 "palma P8"`. `TEQUILA_CLI` names the
   renderer, or the checkout's release build is used. `TEQUILA_CYCLES` says
   where Cycles is: the headphones and the palma render in Cycles, and the
   bottle, glass with a liquid inside, in Tequila's own renderer.

   A project saves its textures where they were found, so the palma's go into
   `out/palma P8.textures`, named relative to the project, which Tequila finds
   beside the project wherever it is opened from.

3. **Copy** `out/*.tequila`, and `out/palma P8.textures`, into
   `assets/samples`.

The start screen's banners come from `make_banners.py OUT` and go in
`app/assets/start`.

The packages carry the `.tequila` files and their `.textures` folders. The
folder's model file, `glasses.gltf`, is a fixture for the import tests.
