// The transport kernel's variants (docs/07 R3, variants.rs's
// KernelFeatures): each constant is a part of the kernel a scene may not
// need, and a pipeline built with it false leaves that part out - the
// driver drops a branch whose condition is a constant false before it
// spends its time on it. Every one is true unless the pipeline says
// otherwise: the whole kernel, which every other kernel that includes a
// module reading them gets.
//
// Each guards code that the scene's own data already turns away at run
// time where the scene has no such part - a flag in the frame, a material's
// record, a program's header or words, an instance's - so a variant leaves
// out only what the whole kernel would never have run for that scene, and
// its picture is the whole kernel's to the bit: the same draws in the same
// order. The Rust that chooses them reads the same data
// (KernelFeatures::of), and the parity test holds each scene's variant to
// the whole kernel's picture bit for bit. A part ported later joins with a
// constant here, its gate where the kernel already turns it away, and its
// bit and records in variants.rs.
//
// Leaving code out can still move a picture by rounding, where the driver
// then sees through what is left and fuses or reorders the arithmetic of
// what the scene does run: so the anisotropic frame is not gated, and the
// glowing triangles' light sample is left out behind a stand-in the driver
// cannot see through (emitters.wgsl) rather than skipped. The parity test
// finds such a place; tests/variants.rs's which_place_... says which.

// Glass and media: the transmission lobe, the media a path is in and the
// surfaces between them, the split at the first plain glass, absorption.
override FEATURE_GLASS: bool = true;
// Clear glass a shadow ray passes, and the glass a seeing bounce sees.
override FEATURE_PANES: bool = true;
// Cutouts: a hole crossed at a hit, the camera's coverage, shadow rays and
// the floor's walking through them.
override FEATURE_CUTOUTS: bool = true;
// Cutouts whose opacity a program drives: a walk asking the program, and a
// segment run again once it knows.
override FEATURE_CUT_PROGRAMS: bool = true;
// The glowing triangles as lights: their light sample down the tree, and
// the weight of a glow a bounce hits against it.
override FEATURE_EMITTERS: bool = true;
// A glowing surface met at a hit.
override FEATURE_GLOW: bool = true;
// A glowing volume.
override FEATURE_VOLUME_GLOW: bool = true;
// The analytic lamps: their light samples, the lamps a bounce meets, a
// stand-in's face.
override FEATURE_LAMPS: bool = true;
// The shadow catcher's floor: its shadow, its reflection, a floor passed.
override FEATURE_FLOOR: bool = true;
// The material programs.
override FEATURE_PROGRAMS: bool = true;
// Which of the interpreter's instructions the programs hold, a bit each by
// opcode (program.wgsl's prog_holds): an arm of prog_run that no program
// holds is left out.
override FEATURE_OPS: u32 = 0xFFFFFFFFu;
// A picture's normal map (the picture's own arm is OP_TEXTURE's).
override FEATURE_TEXTURES: bool = true;
// The measures at the mesh's corners, weighed at a hit (their arm is
// OP_ATTRIBUTE's).
override FEATURE_ATTRIBUTES: bool = true;
// Rounded edges: the bevel's probes at the surface a path meets first.
override FEATURE_BEVEL: bool = true;
// A background colour or backplate past the parts, along a specular chain.
override FEATURE_BACKGROUND: bool = true;
// The backplate's picture, read at the film point.
override FEATURE_BACKPLATE: bool = true;
// The thin lens: a camera ray from a point on the opening (a frame's part:
// the camera's, which the scene's records do not hold).
override FEATURE_THIN_LENS: bool = true;
// An opening of blades rather than a round one (a frame's part).
override FEATURE_BLADES: bool = true;
// A dome of a size: the sky looked up, sampled and weighed from where a
// path is, its shadow rays stopping at the dome.
override FEATURE_DOME: bool = true;
// A shifted lens's film (a frame's part).
override FEATURE_SHIFT: bool = true;
// Parallel rays from the film plane (a frame's part).
override FEATURE_ORTHOGRAPHIC: bool = true;
// Every direction round the camera (a frame's part).
override FEATURE_EQUIRECTANGULAR: bool = true;
// A pixel filter other than one pixel's box (a frame's part: the film's).
override FEATURE_FILTER: bool = true;
// A part some kind of ray does not see - passed by that ray, and by the
// camera's coverage - or held out, a hole where the camera sees it.
override FEATURE_UNSEEN: bool = true;
// A lamp's shadow links: the parts they leave out let its light by, so its
// shadow rays walk the parts rather than ask whether any is in the way.
override FEATURE_SHADOW_LINKS: bool = true;
// The light past a part its shadow rays pass, met where a path stops at the
// part (a part that casts no shadow, a lamp that casts none or whose shadow
// links leave parts out), and a floor's nearness measured past such parts.
override FEATURE_PASSES_PARTS: bool = true;
// An IES lamp: its profile read towards the way its light leaves it.
override FEATURE_IES: bool = true;
// Metallic paint's flakes: the flake field at a point (flakes.wgsl) and
// the metal lobe under the coat in the flake's frame (openpbr.wgsl).
override FEATURE_FLAKES: bool = true;
// A thin film over the metal and the dielectric interface: the Airy sum in
// place of their Fresnel (film.wgsl).
override FEATURE_FILM: bool = true;
// Glass that disperses light, in an RGB render: a wavelength drawn at the
// first such glass a path meets, the index at it (dispersion.wgsl).
override FEATURE_DISPERSION: bool = true;
// Paths that carry four wavelengths rather than three channels (a frame's
// part: its spectral mode), every colour read at them by the spectral
// tables (spectral.wgsl).
override FEATURE_SPECTRAL: bool = true;
// A volume that scatters: the random walk through it, and the lights a
// collision in one whose surface bends nothing samples directly.
override FEATURE_SCATTERING: bool = true;
