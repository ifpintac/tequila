# Tequila's change to vdb-rs 0.6.0

A grid saved as half floats (`saveFloatAsHalf`, Houdini's 16-bit option)
failed to load with "Invalid node meta-data: 240".

OpenVDB's half writer writes nothing at all for an empty value array — no
size prefix — where the full-float path writes a blosc header even for zero
values. An internal node with no active tiles has exactly such an array, and
`read_compressed` read an eight-byte size for it anyway, taking the first
eight bytes of the next node's header and misreading everything after.

The fix, in `src/reader.rs` (marked "Tequila"), skips the read for an empty
half-float array. Diagnosed by tracing both files' reads side by side: the
full-float file read a 16-byte header where the half file read a size of
zero. Tested by `crates/tequila-texture/tests/volume.rs`, which reads a
half-float file written by OpenVDB 12.0 and compares all 32,768 voxels with
the full-float original. Everything else is upstream's, unchanged.
