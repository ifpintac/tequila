# Send to Tequila for SolidWorks (beta)

A macro that sends the open part or assembly to Tequila, with its
appearances, as KeyShot's add-in does. Press it again after a change and the
model in Tequila updates; the materials you gave it there stay.

**Beta.** It has been written against SolidWorks' API documentation but not
yet run in SolidWorks. If you try it, please say how it went.

## What it does

1. Exports the part or assembly as STEP AP214 (which carries colours, and
   keeps an assembly's structure and names) to
   `%TEMP%\Tequila\SolidWorks\<name>.step`. Your own STEP export options are
   put back afterwards.
2. Runs `Tequila.exe --send` with that file: an open Tequila updates or
   imports the model; otherwise Tequila starts with it.

Drawings are refused: send the part or assembly.

## Install

Pick one.

**Run it from the menu.** Tools > Macro > Run, choose
`SendToTequila.swb` (set the file type to show `.swb`). SolidWorks
converts it to a `.swp` the first time.

**As a toolbar button** (recommended):

1. Tools > Macro > New, and save it as `SendToTequila.swp`.
2. In the editor that opens, delete what is in the module, then File >
   Import File and pick `SendToTequila.bas`. (Or paste the `.bas` text in,
   leaving out its first line, `Attribute VB_Name = ...`.) Save, and close
   the editor.
3. With a document open: Tools > Customize > Commands > Macro. Drag **New
   Macro Button** onto a toolbar, browse to `SendToTequila.swp`, choose the
   method `SendToTequila.main`, and give it a tooltip.

## Notes

- Two documents with the same file name, from different folders, export to
  the same path, so Tequila takes them for one model.
- Hidden and suppressed components are exported as SolidWorks' STEP export
  treats them.
- It needs no references set in the VBA editor (it is late bound), so it
  works across SolidWorks versions; SaveAs3 (2020 SP02 and later) is used,
  and SaveAs before that.
