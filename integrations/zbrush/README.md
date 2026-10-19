# Send to Tequila for ZBrush (beta)

A button, **ZPlugin > Tequila > Send to Tequila**, that sends the visible
SubTools of the current Tool to Tequila with their polypaint, as the KeyShot
bridge does. Press it again after sculpting and the model in Tequila
updates: each SubTool keeps the material you gave it there, matched by its
name.

Needs **ZBrush 2026 or later**, which added the Python plugin API.

**Beta.** It has been checked against Maxon's Python API documentation and a
stand-in for it (`tests/`), but not yet run in ZBrush. If you try it, please
say how it went.

## Install

1. In ZBrush: Preferences > Asset Directory > Open Directory. Open the
   `Python` folder there (make it if it is not there).
2. Copy `tequila_bridge/init.py` and `tequila_bridge/objmerge.py` into it.
3. Restart ZBrush. The button is in ZPlugin > Tequila.

Tequila is found by its installer's registry entry, or in the usual install
folders; otherwise the button asks where it is, once. `TEQUILA_EXE` overrides
it.

## What it does

1. Exports each visible SubTool (its eye on, and its folder's) as OBJ,
   which carries polypaint, and puts them together as one model, each
   SubTool a part named after it. Polygroups do not split a SubTool.
2. Runs `Tequila --send` with it: an open Tequila updates or imports the
   model; otherwise Tequila starts with it.

The SubTool you had selected is selected again afterwards.

## Notes

- Each SubTool is exported at its current subdivision level.
- Two SubTools with one name are told apart as `Name (2)`; rename one if you
  want each to keep its own material reliably.
- ZBrush's export scale (Tool > Export > Scale) applies. OBJ files say no
  unit, so check the model's size in Tequila the first time.

`tests/check_bridge.py` runs the plugin against the stand-in.
