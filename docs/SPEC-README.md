# Tequila — Documentation Suite

An open-source, real-time, physically-based product rendering application. Feature-equivalent to KeyShot, built from the ground up on published rendering science and permissively-licensed open-source libraries.

## How to use these documents (read this first, Claude Code)

These documents are a **complete build specification**, not marketing material. They are written so that an engineer (or an AI coding agent) can implement Tequila without needing access to KeyShot, without guessing at algorithms, and without inventing behavior.

| Document | What it is | When you need it |
|---|---|---|
| **[TEQUILA_SPEC.md](TEQUILA_SPEC.md)** | Master document. Product scope, legal ground rules, complete KeyShot feature inventory (the "what"), system architecture, phased roadmap, master checklist. | Read completely, first. Return to it to check feature coverage. |
| **[docs/01-RENDERER-CORE.md](docs/01-RENDERER-CORE.md)** | The renderer engineering spec: rendering equation, integrator, sampling, light sampling, volumetrics, caustics, camera model, color pipeline, denoising, progressive scheduling, CPU/GPU backends, acceleration structures. All math specified. | Milestones M0, M4, M9. This is the hardest and most important document. |
| **[docs/02-MATERIAL-SYSTEM.md](docs/02-MATERIAL-SYSTEM.md)** | Shading model (OpenPBR), exact parameter mapping for every material type, material-graph IR and compilation, texture pipeline, procedural texture algorithms, label/decal layering. | Milestones M2, M6. |
| **[docs/03-SCENE-AND-IO.md](docs/03-SCENE-AND-IO.md)** | Scene graph data model, instancing, units, the `.tequila` file format (with schema), import pipeline (OCCT/Assimp/USD) with tessellation rules, export, LiveLinking protocol. | Milestones M1, M8. |
| **[docs/04-UX-SPEC.md](docs/04-UX-SPEC.md)** | Exact interaction semantics: window layout, drag-and-drop material application (including the material-linking model that everything depends on), selection, gizmos, hotkeys, node editor, undo/redo command catalog, HUD. | Milestones M1–M6. |
| **[docs/05-ROADMAP-AND-TESTING.md](docs/05-ROADMAP-AND-TESTING.md)** | Milestones broken into issue-sized tasks with explicit acceptance criteria, plus the full correctness/perf test strategy (furnace tests, χ² BSDF sampling tests, golden images, backend parity, determinism). | Continuously. This is your task list. |

## The one-paragraph version

Tequila is a progressive, physically-based path tracer wrapped in a product-visualization UI. The defining property — the thing that makes it feel like KeyShot rather than like a general 3D package — is that **the viewport is the renderer**: every material, lighting, and camera edit is reflected in a converging, photoreal image immediately, with no separate preview mode and no bake step. Materials are applied by dragging a thumbnail onto an object. Lighting is primarily HDRI image-based lighting plus optional physical lights. Everything else in this suite exists to serve those three sentences.

## Non-negotiable engineering invariants

These hold across every milestone. If a change would break one, the change is wrong.

1. **The UI never blocks on the renderer, and the renderer never blocks on the UI.** They communicate through a lock-free snapshot handoff (§01-RENDERER-CORE §9).
2. **Every scene mutation goes through the Command layer.** The UI, the Python API, and the undo system are three consumers of one command bus (§04-UX-SPEC §8). No exceptions — a direct scene mutation is a bug even if it works.
3. **One shading implementation, two backends.** CPU and GPU must not have separate BSDF code. They share source via a restricted shading language/IR (§01-RENDERER-CORE §10).
4. **Scene-linear color throughout, display transform only at the end.** No sRGB values inside the integrator, ever (§01-RENDERER-CORE §7).
5. **Physically-based units end to end.** Real-world scene units, photometric-ish light intensities, physical camera exposure. "Looks right" is achieved by correct physics plus a tone curve, not by fudge factors (§03-SCENE-AND-IO §3).
6. **Determinism.** Same scene + same sample count + same seed = bit-identical image on a given backend. Non-determinism is a bug, not a performance feature (§05-ROADMAP-AND-TESTING §3.6).

## Status

Specification complete. No implementation started. Begin at [docs/05-ROADMAP-AND-TESTING.md](docs/05-ROADMAP-AND-TESTING.md) §1 (Milestone M0).
