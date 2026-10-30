//! Safe wrappers over Embree and the two-level acceleration structure.
//!
//! `docs/01-RENDERER-CORE.md` §14: a **two-level hierarchy** — a BLAS per
//! unique mesh, a TLAS over instances — is "non-negotiable for CAD scenes with
//! thousands of repeated fasteners". Instancing has to survive the whole
//! pipeline, and this is the end of it.
//!
//! §2.1 of the master spec is equally direct about not writing the BVH
//! ourselves: "Do not write your own BVH; you will not beat it and it is not
//! where the value is."

use crate::embree_sys as sys;
use tequila_core::ids::{InstanceId, MaterialId};
use tequila_core::math::{Ray, Vec2, Vec3};
use tequila_render_core::intersect::{shadow_ray, Hit, Intersector};
use tequila_render_core::scene::SceneSnapshot;
use std::sync::Arc;

/// An Embree device. One per process is plenty; it holds no scene state.
#[derive(Debug)]
pub struct Device(sys::RTCDevice);

// Embree devices and scenes are documented as safe to use from several threads
// concurrently for read-only queries, which is all the renderer does after the
// build completes.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

static SHARED_DEVICE: std::sync::OnceLock<Result<Arc<Device>, String>> = std::sync::OnceLock::new();

impl Device {
    /// The process-wide Embree device.
    ///
    /// One per process, created once. A device holds no scene state, so there
    /// is nothing to gain from having several — and there is something to
    /// lose: Embree brings up and tears down its task scheduler with the
    /// device, and creating or destroying devices concurrently on several
    /// threads deadlocks. That is easy to hit by accident (a test harness
    /// running cases in parallel does it immediately) and hard to diagnose,
    /// since the stack is inside a third-party scheduler.
    pub fn shared() -> Result<Arc<Device>, String> {
        SHARED_DEVICE.get_or_init(Device::create).clone()
    }

    /// Create a fresh, independent device.
    ///
    /// Prefer [`Device::shared`]. This exists for the case where a caller
    /// genuinely needs an isolated device — a headless worker process pinned
    /// to a subset of cores, say — and it must not be called concurrently
    /// with itself.
    pub fn create() -> Result<Arc<Device>, String> {
        // SAFETY: a null config string selects Embree's defaults.
        let handle = unsafe { sys::rtcNewDevice(std::ptr::null()) };
        if handle.is_null() {
            return Err("Embree device creation failed; is embree4.dll on the path?".into());
        }
        Ok(Arc::new(Device(handle)))
    }

    fn raw(&self) -> sys::RTCDevice {
        self.0
    }

    fn take_error(&self) -> Option<u32> {
        // SAFETY: the device handle is valid for the lifetime of `self`.
        let code = unsafe { sys::rtcGetDeviceError(self.0) };
        (code != sys::RTC_ERROR_NONE).then_some(code)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: the handle was produced by `rtcNewDevice` and is released
        // exactly once, here.
        unsafe { sys::rtcReleaseDevice(self.0) }
    }
}

/// A committed Embree scene.
#[derive(Debug)]
struct RawScene {
    handle: sys::RTCScene,
    /// Keeps the device alive: Embree scenes must not outlive their device.
    _device: Arc<Device>,
    /// Keeps the vertex and index buffers alive. `rtcSetSharedGeometryBuffer`
    /// does not copy — Embree reads through these pointers for the lifetime of
    /// the scene, so dropping them early is a use-after-free.
    _buffers: Vec<Arc<VertexBuffers>>,
    /// Child scenes referenced by instances, kept alive for the same reason.
    _children: Vec<Arc<RawScene>>,
}

unsafe impl Send for RawScene {}
unsafe impl Sync for RawScene {}

impl Drop for RawScene {
    fn drop(&mut self) {
        // SAFETY: released exactly once, and every geometry attached to it was
        // already released after attachment (Embree refcounts them).
        unsafe { sys::rtcReleaseScene(self.handle) }
    }
}

/// Mesh data in the layout Embree wants.
///
/// Positions are padded to 4 floats. Embree's `RTC_FORMAT_FLOAT3` vertex
/// buffer is documented as requiring 16-byte-aligned, 16-byte-strided storage
/// so its SIMD kernels can load a vertex in one instruction; handing it tightly
/// packed `Vec3`s reads past the end of the last vertex.
#[derive(Debug)]
struct VertexBuffers {
    positions: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

/// The ray-mask bit every instance in the BVH answers: closest-hit queries,
/// which is the camera, every bounce, and anything else that has to see a
/// part whatever its shadow does.
const MASK_SURFACE: u32 = 1 << 0;
/// The bit only shadow-casting instances answer, carried by occlusion rays.
///
/// A part with its shadow switched off is still a surface, so it cannot simply
/// be left out of the BVH; this lets one `rtcOccluded1` skip it instead of the
/// tracer walking hit after hit to step past it. Embree tests
/// `ray.mask & geometry.mask` at the instance and again at the triangles
/// inside it, so the mesh geometries take every bit and the instance alone
/// decides.
const MASK_SHADOW: u32 = 1 << 1;

/// The CPU backend: an Embree two-level BVH over a scene snapshot.
#[derive(Debug)]
pub struct EmbreeIntersector {
    scene: SceneSnapshot,
    top: Arc<RawScene>,
    /// Maps an Embree instance geometry ID back to our instance index.
    instance_lookup: Vec<u32>,
    /// The reverse: each instance's Embree geometry ID, or `None` for an
    /// instance that was not attached. Kept so a transform can be updated
    /// without rebuilding. See [`EmbreeIntersector::update_transforms`].
    instance_geometry: Vec<Option<u32>>,
    /// Some instance casts no shadow and the device cannot mask it out, so
    /// occlusion has to walk past it hit by hit. Only an Embree built without
    /// ray masks gets here; the shipped SDK has them.
    walk_shadows: bool,
}

impl EmbreeIntersector {
    /// Build the acceleration structure for a snapshot.
    ///
    /// `refit_quality` selects Embree's build quality. §14 notes refit is
    /// roughly an order of magnitude cheaper than a rebuild and is the right
    /// choice for animation playback, where topology is unchanged.
    pub fn build(device: &Arc<Device>, scene: SceneSnapshot) -> Result<Self, String> {
        // SAFETY: the device handle is valid for the lifetime of `device`.
        let masks = unsafe {
            sys::rtcGetDeviceProperty(device.raw(), sys::RTC_DEVICE_PROPERTY_RAY_MASK_SUPPORTED)
        } != 0;

        // ---- Bottom level: one scene per unique mesh -------------------
        let mut mesh_scenes: Vec<Option<Arc<RawScene>>> = vec![None; scene.meshes.len()];

        for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
            if let Some(fibres) = &mesh.fibres {
                if fibres.segment_count() > 0 {
                    mesh_scenes[mesh_index] = Some(Arc::new(fibre_scene(device, fibres, masks)?));
                }
                continue;
            }
            if mesh.triangle_count() == 0 {
                continue;
            }
            let buffers = Arc::new(VertexBuffers {
                positions: mesh
                    .positions
                    .iter()
                    .map(|p| [p.x, p.y, p.z, 0.0])
                    .collect(),
                indices: mesh.indices.clone(),
            });

            // SAFETY: every pointer handed to Embree below points into
            // `buffers`, which is moved into the returned `RawScene` and so
            // outlives the Embree scene.
            unsafe {
                let blas = sys::rtcNewScene(device.raw());
                if blas.is_null() {
                    return Err("failed to create an Embree scene".into());
                }
                sys::rtcSetSceneBuildQuality(blas, sys::RTC_BUILD_QUALITY_HIGH);

                let geom = sys::rtcNewGeometry(device.raw(), sys::RTC_GEOMETRY_TYPE_TRIANGLE);
                sys::rtcSetSharedGeometryBuffer(
                    geom,
                    sys::RTC_BUFFER_TYPE_VERTEX,
                    0,
                    sys::RTC_FORMAT_FLOAT3,
                    buffers.positions.as_ptr().cast(),
                    0,
                    std::mem::size_of::<[f32; 4]>(),
                    buffers.positions.len(),
                );
                sys::rtcSetSharedGeometryBuffer(
                    geom,
                    sys::RTC_BUFFER_TYPE_INDEX,
                    0,
                    sys::RTC_FORMAT_UINT3,
                    buffers.indices.as_ptr().cast(),
                    0,
                    std::mem::size_of::<[u32; 3]>(),
                    mesh.triangle_count(),
                );
                if masks {
                    // Embree 4's default is 1, not every bit.
                    sys::rtcSetGeometryMask(geom, u32::MAX);
                }
                sys::rtcCommitGeometry(geom);
                sys::rtcAttachGeometry(blas, geom);
                // Embree refcounts geometries; the scene holds the reference
                // from here on, so drop ours.
                sys::rtcReleaseGeometry(geom);
                sys::rtcCommitScene(blas);

                if let Some(code) = device.take_error() {
                    sys::rtcReleaseScene(blas);
                    return Err(format!("Embree error {code} while building a mesh BVH"));
                }

                mesh_scenes[mesh_index] = Some(Arc::new(RawScene {
                    handle: blas,
                    _device: device.clone(),
                    _buffers: vec![buffers],
                    _children: Vec::new(),
                }));
            }
        }

        // ---- Top level: one instance per placement ---------------------
        let mut instance_lookup = Vec::with_capacity(scene.instances.len());
        let mut instance_geometry = vec![None; scene.instances.len()];
        let mut children = Vec::new();

        // SAFETY: as above; every child scene is kept alive in `children`.
        let top_handle = unsafe {
            let tlas = sys::rtcNewScene(device.raw());
            if tlas.is_null() {
                return Err("failed to create the top-level Embree scene".into());
            }
            sys::rtcSetSceneBuildQuality(tlas, sys::RTC_BUILD_QUALITY_HIGH);

            for (index, instance) in scene.instances.iter().enumerate() {
                let Some(blas) = mesh_scenes
                    .get(instance.mesh.index())
                    .and_then(|s| s.as_ref())
                else {
                    continue;
                };
                if !instance.visible_to_camera
                    && !instance.casts_shadows
                    && !instance.seen_by_diffuse
                    && !instance.seen_by_glossy
                    && !instance.seen_by_transmission
                {
                    continue;
                }

                let geom = sys::rtcNewGeometry(device.raw(), sys::RTC_GEOMETRY_TYPE_INSTANCE);
                sys::rtcSetGeometryInstancedScene(geom, blas.handle);
                let xfm = instance.transform.matrix.to_cols_array_3x4();
                sys::rtcSetGeometryTransform(
                    geom,
                    0,
                    sys::RTC_FORMAT_FLOAT3X4_COLUMN_MAJOR,
                    xfm.as_ptr().cast(),
                );
                if masks {
                    let shadow = if instance.casts_shadows { MASK_SHADOW } else { 0 };
                    sys::rtcSetGeometryMask(geom, MASK_SURFACE | shadow);
                }
                sys::rtcCommitGeometry(geom);
                let geom_id = sys::rtcAttachGeometry(tlas, geom);
                sys::rtcReleaseGeometry(geom);

                if geom_id as usize >= instance_lookup.len() {
                    instance_lookup.resize(geom_id as usize + 1, u32::MAX);
                }
                instance_lookup[geom_id as usize] = index as u32;
                instance_geometry[index] = Some(geom_id);
                children.push(blas.clone());
            }

            sys::rtcCommitScene(tlas);
            if let Some(code) = device.take_error() {
                sys::rtcReleaseScene(tlas);
                return Err(format!("Embree error {code} while building the scene BVH"));
            }
            tlas
        };

        let walk_shadows = !masks && scene.instances.iter().any(|i| !i.casts_shadows);
        Ok(EmbreeIntersector {
            scene,
            top: Arc::new(RawScene {
                handle: top_handle,
                _device: device.clone(),
                _buffers: Vec::new(),
                _children: children,
            }),
            instance_lookup,
            instance_geometry,
            walk_shadows,
        })
    }

    /// Update instance transforms in place, rebuilding only the top level.
    ///
    /// `docs/01` §9.3 classifies an instance transform change as needing the TLAS
    /// rebuilt and nothing else, and §14 is why: the per-mesh BLASes are the
    /// expensive part, and a part that moved has not changed its own geometry.
    /// A gizmo drag issues one of these per frame, so on a 4,000-part assembly
    /// this is the difference between dragging and watching it stutter.
    ///
    /// Returns `false`, changing nothing, when the new snapshot is not merely
    /// a re-placement of the same instances over the same meshes — the caller
    /// must then rebuild.
    pub fn update_transforms(&mut self, scene: &SceneSnapshot) -> bool {
        if scene.instances.len() != self.scene.instances.len()
            || scene.meshes.len() != self.scene.meshes.len()
        {
            return false;
        }
        // Which mesh an instance points at, and whether it is in the BVH at
        // all, are structural. Only the transform may differ.
        let same_structure = scene
            .instances
            .iter()
            .zip(&self.scene.instances)
            .all(|(new, old)| {
                new.mesh == old.mesh
                    && new.id == old.id
                    && new.visible_to_camera == old.visible_to_camera
                    && new.casts_shadows == old.casts_shadows
            });
        if !same_structure {
            return false;
        }
        // A mesh whose contents changed needs its BLAS rebuilt. Comparing
        // `Arc` pointers is the cheap way to ask: the snapshot builder shares
        // an `Arc` when a mesh is unchanged and allocates a new one when it is
        // not, so this is exact rather than a heuristic.
        if !scene
            .meshes
            .iter()
            .zip(&self.scene.meshes)
            .all(|(new, old)| Arc::ptr_eq(new, old))
        {
            return false;
        }

        // SAFETY: every ID came from `rtcAttachGeometry` on `self.top`, which
        // still holds the geometry; the transform is the 3x4 column-major
        // layout the format constant declares.
        unsafe {
            for (index, geom_id) in self.instance_geometry.iter().enumerate() {
                let Some(geom_id) = geom_id else { continue };
                let geometry = sys::rtcGetGeometry(self.top.handle, *geom_id);
                if geometry.is_null() {
                    return false;
                }
                let xfm = scene.instances[index].transform.matrix.to_cols_array_3x4();
                sys::rtcSetGeometryTransform(
                    geometry,
                    0,
                    sys::RTC_FORMAT_FLOAT3X4_COLUMN_MAJOR,
                    xfm.as_ptr().cast(),
                );
                sys::rtcCommitGeometry(geometry);
            }
            sys::rtcCommitScene(self.top.handle);
        }

        // The rest of the snapshot comes along too: material assignments are
        // answered from it, and a stale copy would shade a moved part with the
        // assignment it had before.
        self.scene = scene.clone();
        true
    }

    /// Fill in the parts of a [`Hit`] Embree does not compute: the
    /// interpolated shading normal, the UV, and the material.
    fn resolve(&self, ray: &Ray, rh: &sys::RTCRayHit) -> Option<Hit> {
        let inst_geom_id = rh.hit.inst_id[0];
        let instance_index = if inst_geom_id == sys::RTC_INVALID_GEOMETRY_ID {
            return None;
        } else {
            *self.instance_lookup.get(inst_geom_id as usize)? as usize
        };
        let instance = self.scene.instances.get(instance_index)?;
        let mesh = self.scene.meshes.get(instance.mesh.index())?;
        let primitive = rh.hit.prim_id as usize;
        if let Some(fibres) = &mesh.fibres {
            return Self::resolve_fibre(ray, rh, instance_index, instance, fibres);
        }
        if primitive >= mesh.triangle_count() {
            return None;
        }

        let t = rh.ray.tfar;
        let position = ray.at(t);
        let [i0, i1, i2] = mesh.triangle(primitive);

        // The geometric normal is recomputed from the mesh rather than taken
        // from `rh.hit.ng`.
        //
        // Embree's geometry normal follows its own winding convention, which
        // is the opposite of the one `Mesh::geometric_normal_scaled` uses and
        // the one the vertex normals were built against. Taking Embree's value
        // makes the geometric and shading normals disagree about which side is
        // out — and the failure is not subtle-looking-but-wrong, it is total:
        // `facing_forward` flips the pair together, so the shading normal ends
        // up pointing into the surface, every path terminates at its first
        // vertex, and the render comes out black while the coverage mask stays
        // perfectly correct.
        //
        // Deriving it here costs three vertex fetches and makes the convention
        // impossible to get wrong, because there is only one of them.
        let object_normal = mesh.geometric_normal_scaled(primitive);
        let geometric_normal = instance
            .transform
            .normal(object_normal)
            .try_normalize()
            .unwrap_or(Vec3::Y);

        let (u, v) = (rh.hit.u, rh.hit.v);
        let w = 1.0 - u - v;

        let shading_normal = if mesh.normals.is_empty() {
            geometric_normal
        } else {
            let n = mesh.normals[i0 as usize] * w
                + mesh.normals[i1 as usize] * u
                + mesh.normals[i2 as usize] * v;
            instance
                .transform
                .normal(n)
                .try_normalize()
                .unwrap_or(geometric_normal)
        };

        let uv = if mesh.uvs.is_empty() {
            Vec2 { x: u, y: v }
        } else {
            mesh.uvs[i0 as usize] * w + mesh.uvs[i1 as usize] * u + mesh.uvs[i2 as usize] * v
        };

        Some(Hit {
            t,
            position,
            geometric_normal,
            shading_normal,
            uv,
            instance: InstanceId(instance_index as u32),
            material: MaterialId(instance.material.0),
            primitive: primitive as u32,
        })
    }

    /// A hit on a fibre: a ribbon facing the ray, so its normal is the way
    /// back along the ray, square to the fibre. Where across the fibre it
    /// was met is the BSDF's to work out (`SceneSnapshot::fibre_at`), from
    /// the curve's parameter along the segment kept in `uv`.
    fn resolve_fibre(
        ray: &Ray,
        rh: &sys::RTCRayHit,
        instance_index: usize,
        instance: &tequila_render_core::scene::Instance,
        fibres: &tequila_render_core::scene::Fibres,
    ) -> Option<Hit> {
        let segment = rh.hit.prim_id as usize;
        if segment >= fibres.segment_count() {
            return None;
        }
        let first = tequila_render_core::scene::Fibres::segment_start(segment);
        let tangent = instance
            .transform
            .vector(fibres.points[first + 1] - fibres.points[first])
            .try_normalize()
            .unwrap_or(Vec3::Y);
        let back = -ray.direction;
        let facing = (back - tangent * tangent.dot(back)).try_normalize().unwrap_or(back);
        let t = rh.ray.tfar;
        Some(Hit {
            t,
            position: ray.at(t),
            geometric_normal: facing,
            shading_normal: facing,
            uv: Vec2 { x: rh.hit.u, y: rh.hit.v },
            instance: InstanceId(instance_index as u32),
            material: MaterialId(instance.material.0),
            primitive: segment as u32,
        })
    }

    fn to_embree_ray(ray: &Ray, mask: u32) -> sys::RTCRay {
        sys::RTCRay {
            org_x: ray.origin.x,
            org_y: ray.origin.y,
            org_z: ray.origin.z,
            tnear: ray.t_min,
            dir_x: ray.direction.x,
            dir_y: ray.direction.y,
            dir_z: ray.direction.z,
            time: ray.time,
            tfar: ray.t_max,
            mask,
            id: 0,
            flags: 0,
        }
    }
}

/// A bottom-level scene of fibres: one flat linear curve per segment, its
/// two control points the segment's ends with their radii.
fn fibre_scene(
    device: &Arc<Device>,
    fibres: &tequila_render_core::scene::Fibres,
    masks: bool,
) -> Result<RawScene, String> {
    use tequila_render_core::scene::Fibres;
    let buffers = Arc::new(VertexBuffers {
        positions: fibres
            .points
            .iter()
            .zip(&fibres.radii)
            .map(|(p, r)| [p.x, p.y, p.z, *r])
            .collect(),
        // Each segment by the point it starts at. Consecutive within a
        // fibre and not across two, which is how Embree tells for itself
        // which segments join.
        indices: (0..fibres.segment_count()).map(|s| Fibres::segment_start(s) as u32).collect(),
    });
    // SAFETY: as for a mesh's BLAS: the buffers move into the returned
    // scene and outlive it.
    unsafe {
        let blas = sys::rtcNewScene(device.raw());
        if blas.is_null() {
            return Err("failed to create an Embree scene".into());
        }
        sys::rtcSetSceneBuildQuality(blas, sys::RTC_BUILD_QUALITY_HIGH);
        let geom = sys::rtcNewGeometry(device.raw(), sys::RTC_GEOMETRY_TYPE_FLAT_LINEAR_CURVE);
        sys::rtcSetSharedGeometryBuffer(
            geom,
            sys::RTC_BUFFER_TYPE_VERTEX,
            0,
            sys::RTC_FORMAT_FLOAT4,
            buffers.positions.as_ptr().cast(),
            0,
            std::mem::size_of::<[f32; 4]>(),
            buffers.positions.len(),
        );
        sys::rtcSetSharedGeometryBuffer(
            geom,
            sys::RTC_BUFFER_TYPE_INDEX,
            0,
            sys::RTC_FORMAT_UINT,
            buffers.indices.as_ptr().cast(),
            0,
            std::mem::size_of::<u32>(),
            buffers.indices.len(),
        );
        if masks {
            sys::rtcSetGeometryMask(geom, u32::MAX);
        }
        sys::rtcCommitGeometry(geom);
        sys::rtcAttachGeometry(blas, geom);
        sys::rtcReleaseGeometry(geom);
        sys::rtcCommitScene(blas);
        if let Some(code) = device.take_error() {
            sys::rtcReleaseScene(blas);
            return Err(format!("Embree error {code} while building a fibre BVH"));
        }
        Ok(RawScene { handle: blas, _device: device.clone(), _buffers: vec![buffers], _children: Vec::new() })
    }
}

impl EmbreeIntersector {
    /// The closest hit, before any cutaway has had its say.
    fn raw_intersect(&self, ray: &Ray) -> Option<Hit> {
        let mut rh = sys::RTCRayHit {
            ray: Self::to_embree_ray(ray, MASK_SURFACE),
            hit: sys::RTCHit::default(),
        };
        // SAFETY: `rh` is a correctly-laid-out, initialized RTCRayHit, and the
        // scene handle is valid. Embree's single-ray query is documented as
        // safe to call concurrently on a committed scene.
        unsafe {
            sys::rtcIntersect1(self.top.handle, &mut rh, std::ptr::null_mut());
        }
        if rh.hit.geom_id == sys::RTC_INVALID_GEOMETRY_ID {
            return None;
        }
        self.resolve(ray, &rh)
    }
}

impl Intersector for EmbreeIntersector {
    fn intersect(&self, ray: &Ray) -> Option<Hit> {
        if self.scene.cutters().is_empty() {
            return self.raw_intersect(ray);
        }
        tequila_render_core::cutaway::intersect(&self.scene, ray, |r| self.raw_intersect(r))
    }

    fn occluded(&self, from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> bool {
        // The cutaway walk steps past parts that cast no shadow itself, so it
        // doubles as the answer for a device that cannot mask them out.
        if !self.scene.cutters().is_empty() || self.walk_shadows {
            return tequila_render_core::cutaway::occluded(&self.scene, from, from_normal, to, to_normal, |r| {
                self.raw_intersect(r)
            });
        }
        let ray = shadow_ray(from, from_normal, to, to_normal);
        let mut r = Self::to_embree_ray(&ray, MASK_SHADOW);
        // SAFETY: as above. `rtcOccluded1` signals a hit by writing a negative
        // value into `tfar` rather than by filling in a hit record.
        unsafe {
            sys::rtcOccluded1(self.top.handle, &mut r, std::ptr::null_mut());
        }
        r.tfar < 0.0
    }

    fn scene(&self) -> &SceneSnapshot {
        &self.scene
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::ids::MeshId;
    use tequila_core::math::vec3;
    use tequila_core::math::{vec2, Transform};
    use tequila_core::spectrum::Spectrum;
    use tequila_render_core::environment::Environment;
    use tequila_render_core::material::Material;
    use tequila_render_core::scene::{Instance, Mesh};

    /// A unit quad in the XY plane at z = 0, facing +Z.
    fn quad() -> Mesh {
        Mesh::new(
            vec![
                vec3(-1.0, -1.0, 0.0),
                vec3(1.0, -1.0, 0.0),
                vec3(1.0, 1.0, 0.0),
                vec3(-1.0, 1.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn scene_with(meshes: Vec<Mesh>, instances: Vec<Instance>) -> SceneSnapshot {
        SceneSnapshot::new(
            meshes.into_iter().map(Arc::new).collect(),
            instances,
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
    }

    fn device() -> Arc<Device> {
        Device::shared().expect("Embree device")
    }

    #[test]
    fn hits_a_triangle_at_the_right_distance() {
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        let hit = bvh
            .intersect(&Ray::new(vec3(0.0, 0.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .expect("ray must hit the quad");
        assert!((hit.t - 5.0).abs() < 1e-4, "t = {}", hit.t);
        assert!((hit.position - Vec3::ZERO).length() < 1e-4);
        assert_eq!(hit.instance, InstanceId(0));

        // The *sign* matters, and asserting only on the magnitude here is what
        // let a flipped convention through. The quad is wound
        // counter-clockwise in the XY plane, so its geometric normal is +Z —
        // the same answer `Mesh::geometric_normal_scaled` gives, which is what
        // the vertex normals and every downstream consumer assume.
        assert!(
            (hit.geometric_normal - vec3(0.0, 0.0, 1.0)).length() < 1e-4,
            "geometric normal is {:?}, expected +Z",
            hit.geometric_normal
        );
        assert!(!hit.is_backface(vec3(0.0, 0.0, -1.0)));
        assert!(hit.is_backface(vec3(0.0, 0.0, 1.0)));
    }

    /// The backend's geometric normal must agree with the mesh's own, in
    /// sign as well as direction. They are computed by different code with
    /// different conventions, and a disagreement makes shading normals point
    /// into surfaces.
    #[test]
    fn geometric_normals_agree_with_the_mesh() {
        let mesh = quad();
        let expected = mesh.geometric_normal_scaled(0).normalize();
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        for from in [vec3(0.0, 0.0, 5.0), vec3(0.3, -0.2, 5.0)] {
            let hit = bvh
                .intersect(&Ray::new(from, vec3(0.0, 0.0, -1.0)))
                .expect("ray must hit");
            assert!(
                (hit.geometric_normal - expected).length() < 1e-4,
                "backend says {:?}, mesh says {expected:?}",
                hit.geometric_normal
            );
            // And the shading normal must end up on the same side as the
            // geometric one once oriented to the ray.
            let (ng, ns) = hit.facing_forward(vec3(0.0, 0.0, -1.0));
            assert!(ng.dot(ns) > 0.0, "normals disagree about which side is out");
            assert!(
                ns.dot(vec3(0.0, 0.0, 1.0)) > 0.0,
                "shading normal points inward"
            );
        }
    }

    #[test]
    fn misses_report_nothing() {
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        assert!(bvh
            .intersect(&Ray::new(vec3(5.0, 5.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .is_none());
        // A ray pointing away from the geometry.
        assert!(bvh
            .intersect(&Ray::new(vec3(0.0, 0.0, 5.0), vec3(0.0, 0.0, 1.0)))
            .is_none());
    }

    /// Instancing must survive the whole pipeline (§14): one mesh, several
    /// placements, each reporting its own identity and its own transform.
    #[test]
    fn instances_share_one_mesh_and_keep_their_transforms() {
        let instances = vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(0), MaterialId(0))
                .with_transform(Transform::translation(vec3(10.0, 0.0, 0.0))),
        ];
        let scene = scene_with(vec![quad()], instances);
        assert_eq!(scene.unique_mesh_count(), 1);
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();

        let a = bvh
            .intersect(&Ray::new(vec3(0.0, 0.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .expect("first instance");
        let b = bvh
            .intersect(&Ray::new(vec3(10.0, 0.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .expect("second instance");
        assert_eq!(a.instance, InstanceId(0));
        assert_eq!(b.instance, InstanceId(1));
        assert!((b.position.x - 10.0).abs() < 1e-4);
    }

    /// Normals must transform by the inverse transpose. Under a non-uniform
    /// instance scale, using the forward matrix tilts them the wrong way.
    #[test]
    fn instance_normals_survive_non_uniform_scale() {
        // A quad rotated so its normal is not axis-aligned, then squashed.
        let mut tilted = Mesh::new(
            vec![
                vec3(0.0, 0.0, 0.0),
                vec3(1.0, 1.0, 0.0),
                vec3(1.0, 1.0, 1.0),
            ],
            vec![0, 1, 2],
        );
        tilted.compute_smooth_normals();
        let squash =
            Transform::from_matrix(tequila_core::math::Mat4::scale(vec3(1.0, 0.25, 1.0))).unwrap();
        let scene = scene_with(
            vec![tilted],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(squash)],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        let hit = bvh
            .intersect(&Ray::new(vec3(0.6, 0.05, 0.4), vec3(0.0, 1.0, 0.0)))
            .or_else(|| bvh.intersect(&Ray::new(vec3(0.6, 1.0, 0.4), vec3(0.0, -1.0, 0.0))))
            .expect("ray must hit the squashed triangle");
        assert!((hit.geometric_normal.length() - 1.0).abs() < 1e-4);
        // The normal must stay perpendicular to the surface after squashing.
        let along_edge = squash.vector(vec3(1.0, 1.0, 0.0)).normalize();
        assert!(
            hit.geometric_normal.dot(along_edge).abs() < 1e-3,
            "normal not perpendicular to the transformed surface"
        );
    }

    #[test]
    fn occlusion_agrees_with_intersection() {
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        assert!(bvh.occluded(
            vec3(0.0, 0.0, 5.0),
            vec3(0.0, 0.0, -1.0),
            vec3(0.0, 0.0, -5.0),
            vec3(0.0, 0.0, 1.0)
        ));
        assert!(!bvh.occluded(
            vec3(9.0, 9.0, 5.0),
            vec3(0.0, 0.0, -1.0),
            vec3(9.0, 9.0, -5.0),
            vec3(0.0, 0.0, 1.0)
        ));
    }

    /// One straight fibre up the y axis, 0.05 thick, as a mesh of fibres.
    fn one_fibre() -> Mesh {
        use tequila_render_core::scene::Fibres;
        let points: Vec<Vec3> = (0..Fibres::POINTS).map(|i| vec3(0.0, i as f32 / (Fibres::POINTS - 1) as f32, 0.0)).collect();
        Mesh::of_fibres(Fibres { radii: vec![0.05; points.len()], points, random: vec![0.25] })
    }

    /// A fibre is met where it stands, facing the ray, and the hit says how
    /// far across it the ray was: a fifth of the way out, on the side the
    /// fibre's frame calls negative. A ray past its edge misses, and its
    /// shadow falls.
    #[test]
    fn a_fibre_is_met_across_its_width() {
        let scene = scene_with(vec![one_fibre()], vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))]);
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        let down = vec3(0.0, 0.0, -1.0);
        let hit = bvh.intersect(&Ray::new(vec3(0.01, 0.5, 5.0), down)).expect("the ray crosses the fibre");
        assert!((hit.t - 5.0).abs() < 0.06, "t = {}", hit.t);
        assert!((hit.geometric_normal - vec3(0.0, 0.0, 1.0)).length() < 1e-4, "{:?}", hit.geometric_normal);
        let fibre = bvh.scene().fibre_at(&hit, down).expect("a fibre hit");
        assert!((fibre.tangent - Vec3::Y).length() < 1e-4);
        // Across is facing × tangent = -x, and the ray passed at +x.
        assert!((fibre.offset + 0.2).abs() < 0.02, "offset {}", fibre.offset);
        assert_eq!(fibre.random, 0.25);
        assert!(bvh.intersect(&Ray::new(vec3(0.06, 0.5, 5.0), down)).is_none(), "past the fibre's edge");
        assert!(bvh.occluded(vec3(0.0, 0.5, 5.0), down, vec3(0.0, 0.5, -5.0), -down));
    }

    /// A ray leaving a fibre, whichever way, does not meet the fibre it
    /// left: Embree's ribbons ignore their own width at a ray's origin.
    #[test]
    fn a_ray_leaving_a_fibre_does_not_meet_it_again() {
        let scene = scene_with(vec![one_fibre()], vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))]);
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        let hit = bvh.intersect(&Ray::new(vec3(0.0, 0.5, 5.0), vec3(0.0, 0.0, -1.0))).unwrap();
        for direction in [vec3(0.0, 0.0, -1.0), vec3(0.3, 0.2, -1.0), vec3(-1.0, 0.0, 0.1), vec3(0.0, 0.0, 1.0)] {
            let leaving = Ray::new(hit.position, direction.normalize());
            assert!(bvh.intersect(&leaving).is_none(), "met the same fibre again going {direction:?}");
        }
    }

    /// A part with its shadow switched off is still seen — by the camera, in
    /// reflections, by every closest-hit query — but a shadow ray goes
    /// straight through it. Only the cutaway path used to know that, so in a
    /// scene without cutters the toggle did nothing at all.
    #[test]
    fn a_part_that_casts_no_shadow_does_not_occlude() {
        let between = |casts_shadows: bool| {
            let mut quad_instance = Instance::new(InstanceId(0), MeshId(0), MaterialId(0));
            quad_instance.casts_shadows = casts_shadows;
            let scene = scene_with(vec![quad()], vec![quad_instance]);
            EmbreeIntersector::build(&device(), scene).unwrap()
        };
        // A point above the quad, and a light below it.
        let (point, light) = (vec3(0.0, 0.0, 5.0), vec3(0.0, 0.0, -5.0));
        let down = vec3(0.0, 0.0, -1.0);

        assert!(
            between(true).occluded(point, down, light, -down),
            "a shadow-casting quad must occlude"
        );
        let shadowless = between(false);
        assert!(
            !shadowless.occluded(point, down, light, -down),
            "a quad with casts_shadows = false still blocked the light"
        );
        // Off for shadows only: the camera and every bounce still meet it.
        assert!(shadowless
            .intersect(&Ray::new(point, down))
            .is_some_and(|hit| hit.instance == InstanceId(0)));
    }

    /// Passing through one part must not let a shadow ray through the next:
    /// the check is per instance, not per ray.
    #[test]
    fn a_shadow_ray_through_a_shadowless_part_still_meets_the_one_behind() {
        let mut front = Instance::new(InstanceId(0), MeshId(0), MaterialId(0));
        front.casts_shadows = false;
        let back = Instance::new(InstanceId(1), MeshId(0), MaterialId(0))
            .with_transform(Transform::translation(vec3(0.0, 0.0, -2.0)));
        // And one hidden from the camera that still casts: a render layer's
        // other parts, which must go on shadowing the layer.
        let mut hidden = Instance::new(InstanceId(2), MeshId(0), MaterialId(0))
            .with_transform(Transform::translation(vec3(10.0, 0.0, 0.0)));
        hidden.visible_to_camera = false;
        let scene = scene_with(vec![quad()], vec![front, back, hidden]);
        let mut bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        let down = vec3(0.0, 0.0, -1.0);

        // Both roads: the masked query, and the walk a device without ray
        // masks falls back to, which the shipped SDK never takes on its own.
        for walk in [false, true] {
            bvh.walk_shadows = walk;
            assert!(
                bvh.occluded(vec3(0.0, 0.0, 5.0), down, vec3(0.0, 0.0, -5.0), -down),
                "the quad behind the shadowless one must still occlude (walking: {walk})"
            );
            assert!(
                !bvh.occluded(vec3(0.0, 0.0, 5.0), down, vec3(0.0, 0.0, -1.0), -down),
                "only the shadowless quad is between these two points (walking: {walk})"
            );
            assert!(
                bvh.occluded(vec3(10.0, 0.0, 5.0), down, vec3(10.0, 0.0, -5.0), -down),
                "a part hidden from the camera must still cast its shadow (walking: {walk})"
            );
        }
    }

    /// Interpolated shading normals must be smooth across a triangle, while
    /// the geometric normal stays faceted. Conflating them is what makes
    /// tessellated CAD look faceted at any zoom.
    #[test]
    fn shading_normals_are_interpolated_geometric_ones_are_not() {
        let mut mesh = quad();
        // Hand-authored vertex normals fanning outwards.
        mesh = mesh.with_normals(vec![
            vec3(-0.3, -0.3, 1.0).normalize(),
            vec3(0.3, -0.3, 1.0).normalize(),
            vec3(0.3, 0.3, 1.0).normalize(),
            vec3(-0.3, 0.3, 1.0).normalize(),
        ]);
        let scene = scene_with(
            vec![mesh],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();

        let left = bvh
            .intersect(&Ray::new(vec3(-0.7, 0.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .unwrap();
        let right = bvh
            .intersect(&Ray::new(vec3(0.7, 0.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .unwrap();

        assert!(
            left.shading_normal.x < right.shading_normal.x - 0.05,
            "shading normals did not interpolate: {:?} vs {:?}",
            left.shading_normal,
            right.shading_normal
        );
        assert!(
            (left.geometric_normal - right.geometric_normal).length() < 1e-4,
            "geometric normals must stay faceted"
        );
        assert!((left.shading_normal.length() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn uvs_are_interpolated_when_present() {
        let mesh = quad().with_uvs(vec![
            vec2(0.0, 0.0),
            vec2(1.0, 0.0),
            vec2(1.0, 1.0),
            vec2(0.0, 1.0),
        ]);
        let scene = scene_with(
            vec![mesh],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        let centre = bvh
            .intersect(&Ray::new(vec3(0.0, 0.0, 5.0), vec3(0.0, 0.0, -1.0)))
            .unwrap();
        assert!((centre.uv.x - 0.5).abs() < 0.05, "{:?}", centre.uv);
        assert!((centre.uv.y - 0.5).abs() < 0.05, "{:?}", centre.uv);
    }

    #[test]
    fn an_empty_scene_builds_and_intersects_nothing() {
        let scene = scene_with(Vec::new(), Vec::new());
        let bvh = EmbreeIntersector::build(&device(), scene).unwrap();
        assert!(bvh
            .intersect(&Ray::new(Vec3::ZERO, vec3(0.0, 0.0, -1.0)))
            .is_none());
    }

    /// Concurrent queries against a committed scene must be safe and must
    /// return identical results — the renderer runs one of these per pixel per
    /// core.
    #[test]
    fn queries_are_thread_safe_and_consistent() {
        use std::thread;
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let bvh = Arc::new(EmbreeIntersector::build(&device(), scene).unwrap());
        let expected = bvh
            .intersect(&Ray::new(vec3(0.1, 0.2, 5.0), vec3(0.0, 0.0, -1.0)))
            .unwrap()
            .t;

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let bvh = bvh.clone();
                thread::spawn(move || {
                    for _ in 0..2_000 {
                        let hit = bvh
                            .intersect(&Ray::new(vec3(0.1, 0.2, 5.0), vec3(0.0, 0.0, -1.0)))
                            .unwrap();
                        assert_eq!(hit.t, expected);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    }

    /// The whole point: a moved instance is actually somewhere else
    /// afterwards, without any BLAS being rebuilt.
    #[test]
    fn updating_a_transform_moves_the_geometry() {
        let device = Device::shared().expect("Embree");
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let mut intersector = EmbreeIntersector::build(&device, scene.clone()).unwrap();

        // The quad sits at z = 0 spanning x and y in [-1, 1]; these rays come
        // straight down -Z at it.
        let at = |x: f32| Ray::new(vec3(x, 0.0, 5.0), vec3(0.0, 0.0, -1.0));
        assert!(intersector.intersect(&at(0.25)).is_some());
        assert!(intersector.intersect(&at(5.25)).is_none());

        // Move it five units along +X.
        let mut moved = scene.clone();
        moved.instances[0].transform = Transform::translation(vec3(5.0, 0.0, 0.0));
        assert!(
            intersector.update_transforms(&moved),
            "a pure re-placement should not need a rebuild"
        );

        assert!(
            intersector.intersect(&at(0.25)).is_none(),
            "the instance is still at its old position"
        );
        let hit = intersector
            .intersect(&at(5.25))
            .expect("the instance did not move");
        assert!((hit.position.x - 5.25).abs() < 1e-4, "{:?}", hit.position);
    }

    /// The snapshot travels with the update, or the moved part is shaded with
    /// the material it had before.
    #[test]
    fn updating_a_transform_carries_the_rest_of_the_snapshot() {
        let device = Device::shared().expect("Embree");
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let mut intersector = EmbreeIntersector::build(&device, scene.clone()).unwrap();

        let mut changed = scene.clone();
        changed.instances[0].transform = Transform::translation(vec3(0.0, 0.0, 0.0));
        changed.materials = vec![Material::diffuse(Spectrum::splat(0.9))];
        assert!(intersector.update_transforms(&changed));
        assert_eq!(
            intersector.scene().materials[0].base_color,
            Spectrum::splat(0.9),
            "the intersector kept the old snapshot"
        );
    }

    /// Anything structural has to fall back to a rebuild, and must leave the
    /// existing BVH untouched when it refuses.
    #[test]
    fn a_structural_change_refuses_the_in_place_update() {
        let device = Device::shared().expect("Embree");
        let scene = scene_with(
            vec![quad()],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        );
        let mut intersector = EmbreeIntersector::build(&device, scene.clone()).unwrap();

        // An extra instance.
        let mut more = scene.clone();
        more.instances
            .push(Instance::new(InstanceId(1), MeshId(0), MaterialId(0)));
        assert!(!intersector.update_transforms(&more));

        // A different mesh behind the same instance.
        let mut other_mesh = scene.clone();
        other_mesh.meshes = vec![Arc::new(quad())];
        assert!(
            !intersector.update_transforms(&other_mesh),
            "a rebuilt mesh needs its BLAS rebuilt"
        );

        // And the original is still intact and still hits.
        let ray = Ray::new(vec3(0.25, 0.0, 5.0), vec3(0.0, 0.0, -1.0));
        assert!(intersector.intersect(&ray).is_some());
    }

    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn measure_rebuild_against_in_place_update() {
        use std::time::Instant;

        let device = Device::shared().expect("Embree");
        // A 4,000-part assembly over 40 distinct meshes, which is the shape
        // docs/03 2.2 describes.
        let meshes: Vec<Mesh> = (0..40).map(|_| tessellated_sphere(24, 12)).collect();
        let instances: Vec<Instance> = (0..4000)
            .map(|i| {
                Instance::new(InstanceId(i), MeshId(i % 40), MaterialId(0))
                    .with_transform(Transform::translation(vec3(i as f32 * 0.01, 0.0, 0.0)))
            })
            .collect();
        let scene = scene_with(meshes, instances);

        let start = Instant::now();
        let mut intersector = EmbreeIntersector::build(&device, scene.clone()).unwrap();
        let build = start.elapsed();

        let mut moved = scene.clone();
        for (i, instance) in moved.instances.iter_mut().enumerate() {
            instance.transform = Transform::translation(vec3(i as f32 * 0.01, 1.0, 0.0));
        }

        let start = Instant::now();
        assert!(intersector.update_transforms(&moved));
        let update = start.elapsed();

        let start = Instant::now();
        let _ = EmbreeIntersector::build(&device, moved.clone()).unwrap();
        let rebuild = start.elapsed();

        println!(
            "4000 instances / 40 meshes: first build {:.1} ms, rebuild {:.1} ms, \
             in-place update {:.1} ms ({:.0}x)",
            build.as_secs_f64() * 1000.0,
            rebuild.as_secs_f64() * 1000.0,
            update.as_secs_f64() * 1000.0,
            rebuild.as_secs_f64() / update.as_secs_f64().max(1e-9),
        );
    }

    /// A sphere with enough triangles to make its BLAS worth building.
    fn tessellated_sphere(segments: usize, rings: usize) -> Mesh {
        let mut positions = Vec::new();
        for r in 0..=rings {
            let theta = r as f32 / rings as f32 * std::f32::consts::PI;
            for s in 0..=segments {
                let phi = s as f32 / segments as f32 * std::f32::consts::TAU;
                positions.push(vec3(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ));
            }
        }
        let mut indices = Vec::new();
        let stride = segments + 1;
        for r in 0..rings {
            for s in 0..segments {
                let a = (r * stride + s) as u32;
                let b = a + stride as u32;
                indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        Mesh::new(positions, indices)
    }
}
