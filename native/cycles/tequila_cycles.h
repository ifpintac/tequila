/* SPDX-License-Identifier: Apache-2.0
 *
 * A small C interface over Cycles, for Tequila.
 *
 * Cycles' own API is C++ and large. Tequila needs a narrow slice of it: build
 * a scene (meshes, objects, shader graphs, images, a world), set a camera,
 * render, read pixels back. Everything here is plain C so the Rust side binds
 * it without a C++ bridge, and so the material translation lives in Rust
 * where the rest of Tequila's material code is.
 *
 * Nodes are addressed generically: shader nodes are created by their Cycles
 * type name ("principled_bsdf", "image_texture", ...) and inputs are set by
 * socket name (the internal name, "base_color", or the UI name, "Base Color").
 * Scene-level settings work the same way on the "integrator", "film",
 * "background" and "camera" nodes. A new Cycles feature is therefore usually
 * reachable from Rust without touching this file.
 */

#ifndef TEQUILA_CYCLES_H
#define TEQUILA_CYCLES_H

#ifdef _WIN32
#  ifdef TEQUILA_CYCLES_BUILD
#    define FSC_API __declspec(dllexport)
#  else
#    define FSC_API __declspec(dllimport)
#  endif
#else
#  define FSC_API __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FscSession FscSession;
/* Opaque handles to Cycles nodes (shaders, shader nodes, meshes, objects). */
typedef void *FscNode;

/* Object visibility bits, a subset of Cycles' ray types. */
enum {
  FSC_VISIBLE_CAMERA = 1,
  FSC_VISIBLE_DIFFUSE = 2,
  FSC_VISIBLE_GLOSSY = 4,
  FSC_VISIBLE_TRANSMISSION = 8,
  FSC_VISIBLE_SHADOW = 16,
  FSC_VISIBLE_SCATTER = 32,
  FSC_VISIBLE_ALL = 63,
};

/* The last error on this thread, or "" if none. Valid until the next call. */
FSC_API const char *fsc_last_error(void);

/* Set up Cycles for this process: its log, and the folder holding its lib/
 * of GPU kernels. Only the first call counts - fsc_create and
 * fsc_create_live make it too - and it has to come before fsc_devices,
 * because Cycles decides once, by whether it finds the kernels, whether a
 * graphics card can be used at all. */
FSC_API void fsc_init(const char *kernel_dir);

/* Devices Cycles can use here, one per line: "TYPE\tdescription\tid". */
FSC_API const char *fsc_devices(void);

/* A session on a device: "CPU", "OPTIX", "CUDA" or "HIP" (the first device of
 * that type), or an id from fsc_devices(). threads = 0 uses every core.
 * kernel_dir is where precompiled GPU kernels live (may be NULL for CPU).
 * Returns NULL on failure. */
FSC_API FscSession *fsc_create(const char *device, int threads, const char *kernel_dir);
FSC_API void fsc_destroy(FscSession *session);

/* Scene-level nodes: "integrator", "film", "background" or "camera". */
FSC_API FscNode fsc_scene_node(FscSession *session, const char *which);

/* Socket setters for any node. Return 1 on success, 0 (with an error) if the
 * socket does not exist or has a different type. fsc_set_string also sets
 * enum sockets by their option name. Transforms are 3x4, row major. */
FSC_API int fsc_set_float(FscNode node, const char *socket, float value);
FSC_API int fsc_set_float3(FscNode node, const char *socket, float x, float y, float z);
FSC_API int fsc_set_int(FscNode node, const char *socket, int value);
FSC_API int fsc_set_bool(FscNode node, const char *socket, int value);
FSC_API int fsc_set_string(FscNode node, const char *socket, const char *value);
FSC_API int fsc_set_transform(FscNode node, const char *socket, const float *matrix12);

/* Shaders. A shader collects nodes until fsc_shader_commit hands its graph to
 * the scene. "output" is the graph's output node and always exists. A shader
 * named "world" is the scene's background shader. */
FSC_API FscNode fsc_shader_new(FscSession *session, const char *name);
FSC_API FscNode fsc_shader_node(FscSession *session, FscNode shader, const char *type);
FSC_API int fsc_connect(FscSession *session,
                        FscNode shader,
                        FscNode from,
                        const char *output,
                        FscNode to,
                        const char *input);
FSC_API int fsc_shader_commit(FscSession *session, FscNode shader);
/* Open a committed shader for editing again, with an empty graph: its new
 * graph replaces the old one at fsc_shader_commit, and whatever uses the
 * shader keeps using it. Returns 0 if it is open already. */
FSC_API int fsc_shader_reopen(FscSession *session, FscNode shader);
/* Use a committed shader as the world. */
FSC_API void fsc_set_world(FscSession *session, FscNode shader);

/* An image from memory: linear float pixels, rows top to bottom, 1 to 4
 * channels. Returns an id for fsc_set_image, or -1. */
FSC_API int fsc_image(FscSession *session,
                      const char *name,
                      int width,
                      int height,
                      int channels,
                      const float *pixels);
/* The same from 8-bit pixels, a quarter of the memory on the device:
 * srgb = 1 for colour encoded with the sRGB curve (Cycles decodes it as it
 * samples, as Blender does for 8-bit colour textures), 0 for data. */
FSC_API int fsc_image_bytes(FscSession *session,
                            const char *name,
                            int width,
                            int height,
                            int channels,
                            const unsigned char *pixels,
                            int srgb);
/* Let go of an image's pixels on this side. Nodes already pointing at it
 * keep it on the device until they are gone. */
FSC_API void fsc_forget_image(FscSession *session, int image);
/* Point an image_texture or environment_texture node at an image. Images
 * outlive fsc_clear_scene, so an unchanged texture is not sent again. */
FSC_API int fsc_set_image(FscSession *session, FscNode node, int image);

/* A triangle mesh. positions: 3 floats per vertex. normals: 3 per vertex, or
 * NULL for flat shading. triangles: 3 ints each. uvs: 2 floats per triangle
 * corner (3 per triangle), or NULL. tri_shader: index into shaders per
 * triangle, or NULL for all 0. */
FSC_API FscNode fsc_mesh(FscSession *session,
                         int vertex_count,
                         const float *positions,
                         const float *normals,
                         int triangle_count,
                         const int *triangles,
                         const float *uvs,
                         const int *tri_shader,
                         int shader_count,
                         const FscNode *shaders);
/* Colours painted on a mesh's vertices: 4 floats (RGBA, the rendering
 * space) per vertex, as an attribute named "tq_color" that an "attribute"
 * shader node reads. Returns 0 if the count is not the mesh's. */
FSC_API int fsc_mesh_colors(FscSession *session, FscNode mesh, int vertex_count, const float *rgba);
FSC_API FscNode fsc_object(FscSession *session,
                           FscNode mesh,
                           const float *matrix12,
                           unsigned visibility,
                           int shadow_catcher);

/* A light: "point", "spot", "area", "distant" or "background", emitting
 * through shader (for a background light, the world shader). Placed by
 * matrix12. Returns the light node, for its other sockets (size, sizeu,
 * strength, use_mis, ...). */
FSC_API FscNode fsc_light(FscSession *session,
                          const char *type,
                          FscNode shader,
                          const float *matrix12,
                          unsigned visibility);

/* A perspective camera. matrix12 is camera to world with the camera looking
 * along +Z, +Y up (Cycles' convention). fov is the full angle in radians
 * across the image's shorter side. */
/* Light linking: `membership` the receiver sets `node` (an object, or a
 * light) lights as an emitter, a bit each; `receiver_set` the set it takes
 * its light from as a receiver, 0 for every light's. */
FSC_API int fsc_light_linking(FscSession *session,
                              FscNode node,
                              unsigned long long membership,
                              unsigned receiver_set);
/* Shadow linking: `membership` the blocker sets that cast the shadow of
 * `node` as an emitter, a bit each; `blocker_set` the set `node` belongs to
 * as a blocker, 0 for the set that blocks every light. */
FSC_API int fsc_shadow_linking(FscSession *session,
                               FscNode node,
                               unsigned long long membership,
                               unsigned blocker_set);
FSC_API void fsc_camera(FscSession *session,
                        const float *matrix12,
                        float fov,
                        int width,
                        int height);

/* Render and wait. rgba receives width*height*4 floats, rows top to bottom,
 * scene linear, alpha straight from Cycles' film (premultiplied colour).
 * progress, if given, is called with the fraction done. Returns 1 on success. */
typedef void (*FscProgress)(void *user, float fraction);
FSC_API int fsc_render(FscSession *session,
                       int width,
                       int height,
                       int samples,
                       float *rgba,
                       FscProgress progress,
                       void *user);

/* The colour of the shadow a shadow catcher casts: Cycles' is always black,
 * a darkening of what lies behind the floor, and this mixes towards the
 * colour instead, by how much the catcher is in shadow. It is added to the
 * images fsc_render and fsc_live_frame hand back. Black (the default) leaves
 * them as Cycles made them. */
FSC_API void fsc_set_shadow_tint(FscSession *session, float r, float g, float b);

/* Stop a render in progress, from any thread (a progress callback
 * included). fsc_render then returns with what it has. */
FSC_API void fsc_cancel(FscSession *session);

/* How many samples a pixel has had so far in the render under way, for a
 * progress callback: not the fraction it is given, which under a time limit
 * runs with the clock. Cycles clears the count as a render winds up. */
FSC_API int fsc_rendered_samples(FscSession *session);

/* Stop the next renders after this many seconds; 0 for no limit. */
FSC_API void fsc_set_time_limit(FscSession *session, double seconds);

/* ---- Live sessions, for an interactive viewport ------------------------
 *
 * A live session keeps rendering, refining one image, until it is reset or
 * reaches its samples. Edits to its scene go between fsc_edit_begin and
 * fsc_edit_end: everything above may be called in between, and the end
 * restarts the image with what changed. fsc_live_frame copies out the newest
 * progressive picture (denoised once the denoiser has run). */
FSC_API FscSession *fsc_create_live(const char *device, int threads, const char *kernel_dir);
FSC_API void fsc_edit_begin(FscSession *session);
FSC_API void fsc_edit_end(FscSession *session, int width, int height, int samples);
/* Remove every object, mesh, light and shader, keeping the device and its
 * loaded kernels. Inside an edit. */
FSC_API void fsc_clear_scene(FscSession *session);
/* The newest frame, if it is newer than *generation: rows top to bottom,
 * RGBA, at the size it was rendered (smaller than asked while the image is
 * being refined after a change), with its denoising guides when albedo and
 * normal are given (3 floats a pixel each), for a denoiser of the caller's
 * own, and when object is given, the pass id of the object each pixel's
 * camera ray hit first (0 for none; see the "pass_id" object socket).
 * Returns 1 and updates the outputs when a newer frame was copied, 0 when
 * there is none or it does not fit. */
FSC_API int fsc_live_frame(FscSession *session,
                           float *rgba,
                           float *albedo,
                           float *normal,
                           float *object,
                           int capacity_pixels,
                           int *width,
                           int *height,
                           int *samples,
                           unsigned long long *generation);
FSC_API void fsc_set_paused(FscSession *session, int paused);
/* What the session says it is doing ("Rendering", "Updating Scene", ...),
 * or "error: " and why it stopped. Valid until the next call on this thread. */
FSC_API const char *fsc_status(FscSession *session);

/* Remove every object, mesh, shader and image added so far, to rebuild. */
FSC_API void fsc_clear(FscSession *session);

#ifdef __cplusplus
}
#endif

#endif
