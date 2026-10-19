# Sending models to Tequila from other applications

| Application | What it is | Status |
|---|---|---|
| [Blender](blender/tequila_livelink) | A live link: the scene follows you as you model | Tested |
| [SolidWorks](solidworks) | A **Send to Tequila** macro for a toolbar button | Beta: not yet run in SolidWorks |
| [ZBrush](zbrush) | A **Send to Tequila** button, polypaint included (ZBrush 2026 and later) | Beta: not yet run in ZBrush |

Sending the same model again updates it in Tequila: its geometry changes,
and the materials, labels and names you gave it there stay. Parts are
matched by name, so a renamed part arrives as a new one.

If you use SolidWorks or ZBrush and try one of the betas, please say how it
went in the [Discord](https://discord.gg/Na5T2qgQBX) or an issue. Neither has
been run inside the application yet, only checked against its documentation.

## For any application: `Tequila --send`

Every bridge but Blender's does the same two things: it writes the model to
a file (STEP, OBJ, glTF, anything Tequila imports) at the same path each
time, then runs

```
"C:\...\Tequila.exe" --send "C:\path\to\model.step"
```

If Tequila is open, the model sent from that file before is updated from it,
or the file is imported. If it is not open, it starts with the file. Either
way the command returns at once, printing one line, and exits 0, or exits 1
with the reason. On a Mac the program is `Tequila.app/Contents/MacOS/Tequila`.

Where Tequila is installed on Windows: the `InstallLocation` value of
`Software\Microsoft\Windows\CurrentVersion\Uninstall\{EDD88596-57CD-4475-98DD-4D3B719E8AB3}_is1`,
under HKEY_CURRENT_USER or, for an install for all users, HKEY_LOCAL_MACHINE.

For geometry that changes as you work, without a file in between, see the
[LiveLink protocol](../docs/livelink-protocol.md) the Blender add-on speaks.
