# Tequila's change to truck-stepio 0.3.0

A STEP file whose placements leave out their reference direction — written
`AXIS2_PLACEMENT_3D('', #origin, #axis, $)`, which ISO 10303-42 allows —
imported with every circle and cylinder along the X axis turned into NaN, and
meshing it then panicked ("tolerance must be no less than 1e-6") inside
truck-geometry, far from the cause.

The standard's `first_proj_axis` fills an omitted reference direction with
+X, *unless the axis is itself ±X*, when it takes +Y. Upstream always took
+X, so for an X axis `x - (x·z)z` was the zero vector and `normalize()`
divided by zero.

The fix, in `src/in/mod.rs` (marked "TEQUILA PATCH"), takes +Y for an axis
along X. Diagnosed by meshing a failing file one face at a time: every face
that panicked had circle edges whose placement matrix was entirely NaN.
Tested by `crates/tequila-import/tests/step.rs`, whose fixture places a
cylinder along X with its reference direction left out.

The example and test targets were dropped from `Cargo.toml` because their
files are not vendored; the library itself is upstream's, unchanged
otherwise. Licence: Apache-2.0 (the text in `LICENSE`).
