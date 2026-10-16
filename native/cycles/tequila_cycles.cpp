/* SPDX-License-Identifier: Apache-2.0
 *
 * The C interface of tequila_cycles.h over Cycles' C++ API. Kept thin: it
 * creates nodes, sets sockets by name and moves pixels. Deciding *what* to
 * build is Tequila's job, in Rust. */

#define TEQUILA_CYCLES_BUILD
#include "tequila_cycles.h"

#include "device/device.h"
#include "graph/node.h"
#include "graph/node_type.h"
#include "scene/background.h"
#include "scene/camera.h"
#include "scene/film.h"
#include "scene/image.h"
#include "scene/integrator.h"
#include "scene/light.h"
#include "scene/mesh.h"
#include "scene/object.h"
#include "scene/pass.h"
#include "scene/scene.h"
#include "scene/shader.h"
#include "scene/shader_graph.h"
#include "scene/shader_nodes.h"
#include "session/buffers.h"
#include "session/output_driver.h"
#include "session/session.h"
#include "util/log.h"
#include "util/path.h"
#include "util/string.h"
#include "util/time.h"
#include "util/transform.h"
#include "util/unique_ptr.h"

#include <atomic>
#include <cstring>
#include <map>
#include <memory>
#include <mutex>
#include <set>
#include <string>
#include <vector>

#ifdef FSC_WITH_OCIO
#  include <OpenColorIO/OpenColorIO.h>
namespace OCIO = OCIO_NAMESPACE;
#endif

using namespace ccl;

namespace {

thread_local std::string last_error;
std::string device_list;

void fail(const std::string &message)
{
  last_error = message;
}

/* Pixels handed over from Tequila, shared by every node that uses them. */
struct Pixels {
  std::string name;
  int width, height, channels;
  /* One of the two. */
  std::vector<float> data;
  std::vector<unsigned char> bytes;
  bool srgb = false;
};

class MemoryLoader : public ImageLoader {
 public:
  explicit MemoryLoader(std::shared_ptr<const Pixels> pixels) : pixels_(std::move(pixels)) {}

  bool load_metadata(const ImageDeviceFeatures & /*features*/, ImageMetaData &metadata) override
  {
    metadata.width = pixels_->width;
    metadata.height = pixels_->height;
    metadata.channels = pixels_->channels;
    const bool bytes = !pixels_->bytes.empty();
    if (bytes) {
      metadata.type = pixels_->channels == 1 ? IMAGE_DATA_TYPE_BYTE : IMAGE_DATA_TYPE_BYTE4;
    }
    else {
      metadata.type = pixels_->channels == 1 ? IMAGE_DATA_TYPE_FLOAT : IMAGE_DATA_TYPE_FLOAT4;
    }
    /* sRGB bytes stay bytes and are decoded in the kernel. */
    metadata.colorspace = (bytes && pixels_->srgb) ? u_colorspace_srgb : u_colorspace_raw;
    metadata.compress_as_srgb = false;
    return true;
  }

  bool load_pixels(const ImageMetaData & /*metadata*/,
                   void *out,
                   const size_t out_size,
                   const bool associate_alpha) override
  {
    /* Cycles' rows run bottom to top; Tequila's top to bottom. */
    const int w = pixels_->width, h = pixels_->height, c = pixels_->channels;
    const size_t row = size_t(w) * c;
    if (out_size < row * h) {
      return false;
    }
    if (!pixels_->bytes.empty()) {
      unsigned char *dst = static_cast<unsigned char *>(out);
      for (int y = 0; y < h; y++) {
        memcpy(dst + size_t(h - 1 - y) * row, pixels_->bytes.data() + size_t(y) * row, row);
      }
      /* Images are added with raw parameters, so Cycles never asks for
       * associated alpha here: colour and coverage stay apart. */
      return true;
    }
    float *dst = static_cast<float *>(out);
    for (int y = 0; y < h; y++) {
      memcpy(dst + size_t(h - 1 - y) * row, pixels_->data.data() + size_t(y) * row,
             row * sizeof(float));
    }
    if (associate_alpha && c == 4) {
      for (size_t i = 0; i < size_t(w) * h; i++) {
        float *p = dst + i * 4;
        p[0] *= p[3];
        p[1] *= p[3];
        p[2] *= p[3];
      }
    }
    return true;
  }

  string name() const override
  {
    return pixels_->name;
  }

  bool equals(const ImageLoader &other) const override
  {
    return pixels_ == static_cast<const MemoryLoader &>(other).pixels_;
  }

 private:
  std::shared_ptr<const Pixels> pixels_;
};

/* The colour of the ground shadow (fsc_set_shadow_tint). */
struct ShadowTint {
  std::atomic<float> rgb[3] = {0.0f, 0.0f, 0.0f};
};

/* The shadow tint laid into a tile's combined pixels, as read (w x h RGBA).
 *
 * In an approximate shadow catcher's image the alpha is how much of each
 * pixel an object covers, plus how much of the rest the catcher's shadow
 * darkens: A = a + (1 - a)(1 - m), with m the catcher's light ratio, its
 * pass averaged and clamped as Cycles does (film_calculate_shadow_catcher_
 * matte_with_shadow). The shadow's own part, (1 - a)(1 - m), is then
 * (1 - A)(1 - m) / m. Where m is all but nothing - full shadow, A all but
 * one - that tells nothing of a, and the share of the pixel's samples that
 * reached the catcher stands in for 1 - a: without it the edge of a part
 * standing in its own contact shadow took the whole tint. */
static void tint_shadow(const OutputDriver::Tile &tile, const ShadowTint *tint, std::vector<float> &rgba, int w, int h)
{
  if (tint == nullptr) {
    return;
  }
  const float r = tint->rgb[0], g = tint->rgb[1], b = tint->rgb[2];
  if (r == 0.0f && g == 0.0f && b == 0.0f) {
    return;
  }
  std::vector<float> ratio(size_t(w) * h * 3);
  if (!tile.get_pass_pixels("catcher", 3, ratio.data())) {
    return;
  }
  std::vector<float> reached(size_t(w) * h, 1.0f);
  tile.get_pass_pixels("catcher samples", 1, reached.data());
  const auto unit = [](const float x) { return std::min(std::max(x, 0.0f), 1.0f); };
  for (size_t i = 0; i < size_t(w) * h; i++) {
    const float m = unit((ratio[3 * i] + ratio[3 * i + 1] + ratio[3 * i + 2]) / 3.0f);
    const float open = 1.0f - unit(rgba[4 * i + 3]);
    const float shadow = m > 1e-4f ? unit(open * (1.0f - m) / m) : unit(reached[i]) * (1.0f - m);
    rgba[4 * i] += r * shadow;
    rgba[4 * i + 1] += g * shadow;
    rgba[4 * i + 2] += b * shadow;
  }
}

/* Collects the finished render into a caller's buffer. */
class CaptureDriver : public OutputDriver {
 public:
  float *target = nullptr;
  bool written = false;
  const ShadowTint *tint = nullptr;

  void write_render_tile(const Tile &tile) override
  {
    if (!(tile.size == tile.full_size) || target == nullptr) {
      return;
    }
    const int w = tile.size.x, h = tile.size.y;
    std::vector<float> pixels(size_t(w) * h * 4);
    if (!tile.get_pass_pixels("combined", 4, pixels.data())) {
      return;
    }
    tint_shadow(tile, tint, pixels, w, h);
    const size_t row = size_t(w) * 4;
    for (int y = 0; y < h; y++) {
      memcpy(target + size_t(h - 1 - y) * row, pixels.data() + size_t(y) * row,
             row * sizeof(float));
    }
    written = true;
  }
};

/* Keeps the newest progressive picture of a live session. */
class LiveDriver : public OutputDriver {
 public:
  std::mutex mutex;
  std::vector<float> pixels, albedo, normal, object;
  int width = 0, height = 0, samples = 0;
  unsigned long long generation = 0;
  Progress *progress = nullptr;
  const ShadowTint *tint = nullptr;
  /* When the latest change was asked for. A tile from a render that started
   * before it shows the view as it was, and is dropped. */
  double requested = 0.0;

  void write_render_tile(const Tile &tile) override
  {
    keep(tile);
  }

  bool update_render_tile(const Tile &tile) override
  {
    keep(tile);
    return true;
  }

 private:
  /* Cycles' rows run bottom to top; Tequila's top to bottom. */
  static void flip(const std::vector<float> &from, std::vector<float> &to, int w, int h, int c)
  {
    to.resize(from.size());
    const size_t row = size_t(w) * c;
    for (int y = 0; y < h; y++) {
      memcpy(to.data() + size_t(h - 1 - y) * row, from.data() + size_t(y) * row, row * sizeof(float));
    }
  }

  void keep(const Tile &tile)
  {
    const int w = tile.size.x, h = tile.size.y;
    if (w <= 0 || h <= 0) {
      return;
    }
    static const bool trace = getenv("TEQUILA_CYCLES_TRACE") != nullptr;
    if (progress && progress->get_current_sample() == 0) {
      /* Nothing rendered yet: an empty, black picture. */
      if (trace) {
        fprintf(stderr, "live: drop empty %dx%d\n", w, h);
      }
      return;
    }
    if (progress) {
      /* The session stamps its start time as it applies a reset, so a render
       * that started before the change was asked for is the old view. */
      double total = 0.0, render = 0.0;
      progress->get_time(total, render);
      const double started = time_dt() - total;
      const std::lock_guard<std::mutex> lock(mutex);
      if (started < requested) {
        if (trace) {
          fprintf(stderr, "live: drop stale %dx%d sample %d (started %.3f < asked %.3f)\n", w, h,
                  progress->get_current_sample(), started, requested);
        }
        return;
      }
    }
    std::vector<float> rows(size_t(w) * h * 4);
    if (!tile.get_pass_pixels("combined", 4, rows.data())) {
      return;
    }
    tint_shadow(tile, tint, rows, w, h);
    std::vector<float> albedo_rows(size_t(w) * h * 3), normal_rows(size_t(w) * h * 3);
    const bool guides = tile.get_pass_pixels("albedo", 3, albedo_rows.data()) &&
                        tile.get_pass_pixels("normal", 3, normal_rows.data());
    std::vector<float> object_rows(size_t(w) * h, 0.0f);
    const bool objects = tile.get_pass_pixels("object", 1, object_rows.data());
    const std::lock_guard<std::mutex> lock(mutex);
    flip(rows, pixels, w, h, 4);
    if (guides) {
      flip(albedo_rows, albedo, w, h, 3);
      flip(normal_rows, normal, w, h, 3);
    }
    else {
      albedo.assign(size_t(w) * h * 3, 0.0f);
      normal.assign(size_t(w) * h * 3, 0.0f);
    }
    if (objects) {
      flip(object_rows, object, w, h, 1);
    }
    else {
      object.assign(size_t(w) * h, 0.0f);
    }
    width = w;
    height = h;
    samples = progress ? progress->get_current_sample() : 0;
    generation++;
    if (trace) {
      fprintf(stderr, "live: keep %dx%d sample %d\n", w, h, samples);
    }
  }
};

const SocketType *find_socket(Node *node, const char *socket)
{
  if (node == nullptr) {
    fail("no node");
    return nullptr;
  }
  const ustring wanted(socket);
  for (const SocketType &input : node->type->inputs) {
    if (input.name == wanted || input.ui_name == wanted) {
      return &input;
    }
  }
  fail(string("node ") + node->type->name.string() + " has no input \"" + socket + "\"");
  return nullptr;
}

Transform to_transform(const float *m)
{
  Transform t;
  t.x = make_float4(m[0], m[1], m[2], m[3]);
  t.y = make_float4(m[4], m[5], m[6], m[7]);
  t.z = make_float4(m[8], m[9], m[10], m[11]);
  return t;
}

bool device_from_name(const char *name, DeviceInfo &out)
{
  const DeviceType type = Device::type_from_string(name);
  if (type != DEVICE_NONE) {
    vector<DeviceInfo> devices = Device::available_devices(DEVICE_MASK(type));
    if (!devices.empty()) {
      out = devices.front();
      return true;
    }
    fail(string("no ") + name + " device is available");
    return false;
  }
  for (const DeviceInfo &info : Device::available_devices()) {
    if (info.id == name) {
      out = info;
      return true;
    }
  }
  fail(string("unknown device ") + name);
  return false;
}

}  // namespace

struct FscSession {
  SessionParams session_params;
  SceneParams scene_params;
  unique_ptr<Session> session;
  CaptureDriver *capture = nullptr;
  LiveDriver *live = nullptr;
  /* Between fsc_edit_begin and fsc_edit_end the scene mutex is already held
   * by the caller, so the calls in between must not take it again. */
  bool editing = false;
  /* Graphs being built, by shader, until committed. */
  std::map<Shader *, unique_ptr<ShaderGraph>> pending;
  /* The ground shadow's colour, and whether the pass it needs is there. */
  ShadowTint tint;
  bool catcher_pass = false;
  std::vector<std::shared_ptr<const Pixels>> images;

  Scene *scene()
  {
    return session->scene.get();
  }
};

static FscSession *create(const char *device, int threads, const char *kernel_dir, bool live);

/* Cycles' log, and where it finds its GPU kernels: once a process, by the
 * first caller (fsc_init). */
static void init_once(const char *kernel_dir)
{
  static std::once_flag initialised;
  std::call_once(initialised, [&] {
    /* To standard error, which is unbuffered: a log that sits in a buffer
     * when something hangs is no log at all. */
    log_init([](const LogLevel /*level*/, const char *file_line, const char * /*func*/, const char *msg) {
      fprintf(stderr, "cycles %s: %s\n", file_line, msg);
    });
    if (const char *level = getenv("TEQUILA_CYCLES_LOG")) {
      log_level_set(string(level));
    }
    path_init(kernel_dir ? kernel_dir : "");
  });
}

/* The scene lock, unless an edit already holds it. */
struct EditLock {
  FscSession *s;
  explicit EditLock(FscSession *session) : s(session)
  {
    if (!s->editing) {
      s->scene()->mutex.lock();
    }
  }
  ~EditLock()
  {
    if (!s->editing) {
      s->scene()->mutex.unlock();
    }
  }
};

static bool start_session(FscSession *s)
{
  s->session = make_unique<Session>(s->session_params, s->scene_params);
  if (s->session->progress.get_error()) {
    fail(s->session->progress.get_error_message());
    return false;
  }
  if (s->session_params.background) {
    auto capture = make_unique<CaptureDriver>();
    capture->tint = &s->tint;
    s->capture = capture.get();
    s->session->set_output_driver(std::move(capture));
  }
  else {
    auto live = make_unique<LiveDriver>();
    live->progress = &s->session->progress;
    live->tint = &s->tint;
    s->live = live.get();
    s->session->set_output_driver(std::move(live));
  }

  /* Denoised mode (Cycles' default for a pass, made explicit): reading it
   * gives the denoised image once the denoiser has run, and the noisy one
   * before, or when denoising is off. A second pass of the same type and mode
   * would be one the denoiser never writes - a picture of zeros. */
  Pass *pass = s->scene()->create_node<Pass>();
  pass->set_name(ustring("combined"));
  pass->set_type(PASS_COMBINED);
  pass->set_mode(PassMode::DENOISED);
  if (!s->session_params.background) {
    /* A live view is denoised by its caller, which needs the guides. */
    Pass *albedo = s->scene()->create_node<Pass>();
    albedo->set_name(ustring("albedo"));
    albedo->set_type(PASS_DENOISING_ALBEDO);
    albedo->set_mode(PassMode::NOISY);
    Pass *normal = s->scene()->create_node<Pass>();
    normal->set_name(ustring("normal"));
    normal->set_type(PASS_DENOISING_NORMAL);
    normal->set_mode(PassMode::NOISY);
    /* Which object each pixel shows, for the caller's selection outline. */
    Pass *object = s->scene()->create_node<Pass>();
    object->set_name(ustring("object"));
    object->set_type(PASS_OBJECT_ID);
    object->set_mode(PassMode::NOISY);
  }
  return true;
}

extern "C" {

const char *fsc_last_error(void)
{
  return last_error.c_str();
}

void fsc_init(const char *kernel_dir)
{
  init_once(kernel_dir);
}

const char *fsc_devices(void)
{
  device_list.clear();
  for (const DeviceInfo &info : Device::available_devices()) {
    device_list += Device::string_from_type(info.type) + "\t" + info.description + "\t" +
                   info.id + "\n";
  }
  return device_list.c_str();
}

FscSession *fsc_create(const char *device, int threads, const char *kernel_dir)
{
  return create(device, threads, kernel_dir, false);
}

void fsc_destroy(FscSession *s)
{
  delete s;
}

FscNode fsc_scene_node(FscSession *s, const char *which)
{
  Scene *scene = s->scene();
  if (strcmp(which, "integrator") == 0) {
    return scene->integrator;
  }
  if (strcmp(which, "film") == 0) {
    return scene->film;
  }
  if (strcmp(which, "background") == 0) {
    return scene->background;
  }
  if (strcmp(which, "camera") == 0) {
    return scene->camera;
  }
  fail(string("no scene node ") + which);
  return nullptr;
}

int fsc_set_float(FscNode handle, const char *socket, float value)
{
  Node *node = static_cast<Node *>(handle);
  const SocketType *input = find_socket(node, socket);
  if (!input) {
    return 0;
  }
  if (input->type != SocketType::FLOAT) {
    fail(string("input ") + socket + " is not a float");
    return 0;
  }
  node->set(*input, value);
  return 1;
}

int fsc_set_float3(FscNode handle, const char *socket, float x, float y, float z)
{
  Node *node = static_cast<Node *>(handle);
  const SocketType *input = find_socket(node, socket);
  if (!input) {
    return 0;
  }
  if (input->type != SocketType::COLOR && input->type != SocketType::VECTOR &&
      input->type != SocketType::POINT && input->type != SocketType::NORMAL)
  {
    fail(string("input ") + socket + " is not a vector or colour");
    return 0;
  }
  node->set(*input, make_float3(x, y, z));
  return 1;
}

int fsc_set_int(FscNode handle, const char *socket, int value)
{
  Node *node = static_cast<Node *>(handle);
  const SocketType *input = find_socket(node, socket);
  if (!input) {
    return 0;
  }
  if (input->type == SocketType::INT || input->type == SocketType::ENUM) {
    node->set(*input, value);
    return 1;
  }
  if (input->type == SocketType::UINT) {
    node->set(*input, uint(value));
    return 1;
  }
  fail(string("input ") + socket + " is not an integer");
  return 0;
}

int fsc_set_bool(FscNode handle, const char *socket, int value)
{
  Node *node = static_cast<Node *>(handle);
  const SocketType *input = find_socket(node, socket);
  if (!input) {
    return 0;
  }
  if (input->type != SocketType::BOOLEAN) {
    fail(string("input ") + socket + " is not a boolean");
    return 0;
  }
  node->set(*input, value != 0);
  return 1;
}

int fsc_set_string(FscNode handle, const char *socket, const char *value)
{
  Node *node = static_cast<Node *>(handle);
  const SocketType *input = find_socket(node, socket);
  if (!input) {
    return 0;
  }
  if (input->type == SocketType::ENUM) {
    const NodeEnum &options = *input->enum_values;
    const ustring name(value);
    if (!options.exists(name)) {
      fail(string("input ") + socket + " has no option " + value);
      return 0;
    }
    node->set(*input, options[name]);
    return 1;
  }
  if (input->type == SocketType::STRING) {
    node->set(*input, ustring(value));
    return 1;
  }
  fail(string("input ") + socket + " is not a string or enum");
  return 0;
}

int fsc_set_transform(FscNode handle, const char *socket, const float *m)
{
  Node *node = static_cast<Node *>(handle);
  const SocketType *input = find_socket(node, socket);
  if (!input) {
    return 0;
  }
  if (input->type != SocketType::TRANSFORM) {
    fail(string("input ") + socket + " is not a transform");
    return 0;
  }
  node->set(*input, to_transform(m));
  return 1;
}

FscNode fsc_shader_new(FscSession *s, const char *name)
{
  const EditLock lock(s);
  /* The world is Cycles' own background shader, edited in place as Blender
   * does: pointing the background at a new shader crashes the update. */
  Shader *shader = strcmp(name, "world") == 0 ? s->scene()->default_background :
                                                s->scene()->create_node<Shader>();
  shader->name = ustring(name);
  s->pending[shader] = make_unique<ShaderGraph>();
  return shader;
}

FscNode fsc_shader_node(FscSession *s, FscNode shader, const char *type)
{
  auto found = s->pending.find(static_cast<Shader *>(shader));
  if (found == s->pending.end()) {
    fail("shader is not open for editing");
    return nullptr;
  }
  ShaderGraph *graph = found->second.get();
  if (strcmp(type, "output") == 0) {
    return graph->output();
  }
  /* The background shader node shares its name with the scene node. */
  const NodeType *node_type = NodeType::find(
      ustring(strcmp(type, "background") == 0 ? "background_shader" : type));
  if (node_type == nullptr || node_type->type != NodeType::SHADER ||
      node_type->create == nullptr)
  {
    fail(string("no shader node type ") + type);
    return nullptr;
  }
  return graph->create_node(node_type);
}

int fsc_connect(FscSession *s,
                FscNode shader,
                FscNode from,
                const char *output,
                FscNode to,
                const char *input)
{
  auto found = s->pending.find(static_cast<Shader *>(shader));
  if (found == s->pending.end()) {
    fail("shader is not open for editing");
    return 0;
  }
  ShaderNode *source = static_cast<ShaderNode *>(from);
  ShaderNode *target = static_cast<ShaderNode *>(to);
  ShaderOutput *out = nullptr;
  ShaderInput *in = nullptr;
  for (ShaderOutput *o : source->outputs) {
    if (string_iequals(o->socket_type.name.string(), output) ||
        string_iequals(o->socket_type.ui_name.string(), output))
    {
      out = o;
    }
  }
  for (ShaderInput *i : target->inputs) {
    if (string_iequals(i->socket_type.name.string(), input) ||
        string_iequals(i->socket_type.ui_name.string(), input))
    {
      in = i;
    }
  }
  if (out == nullptr || in == nullptr) {
    fail(string("cannot connect ") + source->type->name.string() + "." + output + " to " +
         target->type->name.string() + "." + input);
    return 0;
  }
  found->second->connect(out, in);
  return 1;
}

int fsc_shader_commit(FscSession *s, FscNode handle)
{
  Shader *shader = static_cast<Shader *>(handle);
  auto found = s->pending.find(shader);
  if (found == s->pending.end()) {
    fail("shader is not open for editing");
    return 0;
  }
  const EditLock lock(s);
  shader->set_graph(std::move(found->second));
  shader->tag_update(s->scene());
  s->pending.erase(found);
  return 1;
}

void fsc_set_world(FscSession *s, FscNode shader)
{
  const EditLock lock(s);
  if (shader != s->scene()->default_background) {
    s->scene()->background->set_shader(static_cast<Shader *>(shader));
  }
  s->scene()->background->tag_update(s->scene());
}

int fsc_image(FscSession *s, const char *name, int width, int height, int channels,
              const float *pixels)
{
  if (width <= 0 || height <= 0 || channels < 1 || channels > 4 || pixels == nullptr) {
    fail("bad image");
    return -1;
  }
  auto image = std::make_shared<Pixels>();
  image->name = name ? name : "image";
  image->width = width;
  image->height = height;
  image->channels = channels;
  image->data.assign(pixels, pixels + size_t(width) * height * channels);
  s->images.push_back(std::move(image));
  return int(s->images.size()) - 1;
}

int fsc_image_bytes(FscSession *s, const char *name, int width, int height, int channels,
                    const unsigned char *pixels, int srgb)
{
  if (width <= 0 || height <= 0 || channels < 1 || channels > 4 || pixels == nullptr) {
    fail("bad image");
    return -1;
  }
  auto image = std::make_shared<Pixels>();
  image->name = name ? name : "image";
  image->width = width;
  image->height = height;
  image->channels = channels;
  image->bytes.assign(pixels, pixels + size_t(width) * height * channels);
  image->srgb = srgb != 0;
  s->images.push_back(std::move(image));
  return int(s->images.size()) - 1;
}

void fsc_forget_image(FscSession *s, int image)
{
  if (image >= 0 && image < int(s->images.size())) {
    s->images[image].reset();
  }
}

int fsc_set_image(FscSession *s, FscNode handle, int image)
{
  if (image < 0 || image >= int(s->images.size()) || !s->images[image]) {
    fail("no such image");
    return 0;
  }
  ShaderNode *node = static_cast<ShaderNode *>(handle);
  ImageParams params;
  if (node->type->name == "image_texture") {
    params = static_cast<ImageTextureNode *>(node)->image_params();
  }
  else if (node->type->name == "environment_texture") {
    params = static_cast<EnvironmentTextureNode *>(node)->image_params();
  }
  else {
    fail("only image_texture and environment_texture nodes take images");
    return 0;
  }
  params.colorspace = u_colorspace_raw;
  ImageSlotTextureNode *slot = static_cast<ImageSlotTextureNode *>(node);
  slot->handle = s->scene()->image_manager->add_image(make_unique<MemoryLoader>(s->images[image]),
                                                      params);
  return 1;
}

FscNode fsc_mesh(FscSession *s,
                 int vertex_count,
                 const float *positions,
                 const float *normals,
                 int triangle_count,
                 const int *triangles,
                 const float *uvs,
                 const int *tri_shader,
                 int shader_count,
                 const FscNode *shaders)
{
  const EditLock lock(s);
  Mesh *mesh = s->scene()->create_node<Mesh>();

  array<Node *> used;
  for (int i = 0; i < shader_count; i++) {
    used.push_back_slow(static_cast<Node *>(shaders[i]));
  }
  mesh->set_used_shaders(used);

  array<float3> verts;
  verts.resize(vertex_count);
  for (int i = 0; i < vertex_count; i++) {
    verts[i] = make_float3(positions[3 * i], positions[3 * i + 1], positions[3 * i + 2]);
  }
  mesh->set_verts(verts);
  mesh->reserve_mesh(vertex_count, triangle_count);
  const bool smooth = normals != nullptr;
  for (int t = 0; t < triangle_count; t++) {
    mesh->add_triangle(triangles[3 * t],
                       triangles[3 * t + 1],
                       triangles[3 * t + 2],
                       tri_shader ? tri_shader[t] : 0,
                       smooth);
  }
  if (normals) {
    Attribute *attr = mesh->attributes.add(ATTR_STD_VERTEX_NORMAL);
    float3 *n = attr->data_float3();
    for (int i = 0; i < vertex_count; i++) {
      n[i] = make_float3(normals[3 * i], normals[3 * i + 1], normals[3 * i + 2]);
    }
  }
  if (uvs) {
    Attribute *attr = mesh->attributes.add(ATTR_STD_UV, ustring("UVMap"));
    float2 *uv = attr->data_float2();
    for (int i = 0; i < triangle_count * 3; i++) {
      uv[i] = make_float2(uvs[2 * i], uvs[2 * i + 1]);
    }
  }
  /* Generated coordinates, as Blender's automatic texture space gives them:
   * the bounding box mapped to 0..1, a flat axis reading 0.5. Tequila's
   * renderer computes the same (`SceneSnapshot::generated_at`). */
  if (vertex_count > 0) {
    /* From the caller's positions: `set_verts` took `verts`' data. */
    auto vert = [positions](int i) {
      return make_float3(positions[3 * i], positions[3 * i + 1], positions[3 * i + 2]);
    };
    float3 lo = vert(0), hi = vert(0);
    for (int i = 1; i < vertex_count; i++) {
      lo = min(lo, vert(i));
      hi = max(hi, vert(i));
    }
    const float3 center = (lo + hi) * 0.5f;
    float3 half = (hi - lo) * 0.5f;
    for (int a = 0; a < 3; a++) {
      float &h = a == 0 ? half.x : (a == 1 ? half.y : half.z);
      h = h == 0.0f ? 1.0f : (h < 1e-5f ? 1e-5f : h);
    }
    Attribute *attr = mesh->attributes.add(ATTR_STD_GENERATED);
    float3 *generated = attr->data_float3();
    for (int i = 0; i < vertex_count; i++) {
      generated[i] = (vert(i) - center) / half * 0.5f + make_float3(0.5f, 0.5f, 0.5f);
    }
  }
  return mesh;
}

FscNode fsc_object(FscSession *s, FscNode mesh, const float *m, unsigned visibility,
                   int shadow_catcher)
{
  const EditLock lock(s);
  Object *object = s->scene()->create_node<Object>();
  object->set_geometry(static_cast<Geometry *>(mesh));
  object->set_tfm(to_transform(m));
  uint rays = 0;
  if (visibility & FSC_VISIBLE_CAMERA) {
    rays |= PATH_RAY_CAMERA;
  }
  if (visibility & FSC_VISIBLE_DIFFUSE) {
    rays |= PATH_RAY_DIFFUSE;
  }
  if (visibility & FSC_VISIBLE_GLOSSY) {
    rays |= PATH_RAY_GLOSSY;
  }
  if (visibility & FSC_VISIBLE_TRANSMISSION) {
    rays |= PATH_RAY_TRANSMIT;
  }
  if (visibility & FSC_VISIBLE_SHADOW) {
    rays |= PATH_RAY_SHADOW;
  }
  if (visibility & FSC_VISIBLE_SCATTER) {
    rays |= PATH_RAY_VOLUME_SCATTER;
  }
  object->set_visibility(rays);
  object->set_is_shadow_catcher(shadow_catcher != 0);
  return object;
}

static uint ray_visibility(unsigned visibility)
{
  uint rays = 0;
  if (visibility & FSC_VISIBLE_CAMERA) {
    rays |= PATH_RAY_CAMERA;
  }
  if (visibility & FSC_VISIBLE_DIFFUSE) {
    rays |= PATH_RAY_DIFFUSE;
  }
  if (visibility & FSC_VISIBLE_GLOSSY) {
    rays |= PATH_RAY_GLOSSY;
  }
  if (visibility & FSC_VISIBLE_TRANSMISSION) {
    rays |= PATH_RAY_TRANSMIT;
  }
  if (visibility & FSC_VISIBLE_SHADOW) {
    rays |= PATH_RAY_SHADOW;
  }
  if (visibility & FSC_VISIBLE_SCATTER) {
    rays |= PATH_RAY_VOLUME_SCATTER;
  }
  return rays;
}

FscNode fsc_light(FscSession *s, const char *type, FscNode shader, const float *m,
                  unsigned visibility)
{
  const EditLock lock(s);
  Light *light = s->scene()->create_node<Light>();
  const SocketType *kind = find_socket(light, "light_type");
  const NodeEnum &options = *kind->enum_values;
  if (!options.exists(ustring(type))) {
    fail(string("no light type ") + type);
    s->scene()->delete_node(light);
    return nullptr;
  }
  light->set(*kind, options[ustring(type)]);
  array<Node *> used;
  used.push_back_slow(static_cast<Node *>(shader));
  light->set_used_shaders(used);

  Object *object = s->scene()->create_node<Object>();
  object->set_geometry(light);
  object->set_tfm(to_transform(m));
  object->set_visibility(ray_visibility(visibility));
  /* Counted by shadow catchers, as Blender makes every light: one that is
   * not lights the ground the same with or without the objects on it, so
   * a catcher shows no shadow from it - the sun of an HDRI included. */
  object->set_is_shadow_catcher(true);
  return light;
}

void fsc_camera(FscSession *s, const float *m, float fov, int width, int height)
{
  const EditLock lock(s);
  Camera *camera = s->scene()->camera;
  camera->set_camera_type(CAMERA_PERSPECTIVE);
  camera->set_matrix(to_transform(m));
  camera->set_fov(fov);
  camera->set_full_width(width);
  camera->set_full_height(height);
  camera->compute_auto_viewplane();
  camera->need_flags_update = true;
}

int fsc_render(FscSession *s, int width, int height, int samples, float *rgba,
               FscProgress progress, void *user)
{
  if (s->capture == nullptr) {
    fail("fsc_render needs a session from fsc_create, not fsc_create_live");
    return 0;
  }
  BufferParams buffer;
  buffer.width = width;
  buffer.height = height;
  buffer.full_width = width;
  buffer.full_height = height;

  {
    const EditLock lock(s);
    Camera *camera = s->scene()->camera;
    camera->set_full_width(width);
    camera->set_full_height(height);
    camera->compute_auto_viewplane();
  }

  s->session_params.samples = samples;
  /* A cancel or error from the last render must not end this one. */
  s->session->progress.reset();
  s->capture->target = rgba;
  s->capture->written = false;
  if (progress) {
    Session *session = s->session.get();
    session->progress.set_update_callback(
        [session, progress, user] { progress(user, float(session->progress.get_progress())); });
  }
  s->session->reset(s->session_params, buffer);
  s->session->start();
  s->session->wait();
  s->session->progress.set_update_callback(nullptr);
  s->capture->target = nullptr;

  if (s->session->progress.get_error()) {
    fail(s->session->progress.get_error_message());
    return 0;
  }
  if (s->session->progress.get_cancel() && !s->capture->written) {
    fail("cancelled before an image was written");
    return 0;
  }
  if (!s->capture->written) {
    fail("the render finished without writing an image");
    return 0;
  }
  return 1;
}

}  // extern "C"

/* Cycles made to work in Tequila's rendering space, ACEScg: the current
 * OpenColorIO config becomes OCIO's built-in CG config, whose scene_linear
 * role is ACEScg, and Cycles takes its working primaries, luminance and
 * colour matrices from that (ShaderManager::init_xyz_transforms). Left in
 * Rec.709, every colour beyond Rec.709 was clipped where Cycles clamps -
 * gold's reflectance to 1 in red, which turned it yellow. Nothing else in
 * the process uses this OpenColorIO library: Tequila reads OCIO configs
 * itself. */
static bool use_rendering_space()
{
#ifdef FSC_WITH_OCIO
  static std::mutex mutex;
  static int state = 0; /* 0 untried, 1 working, -1 failed. */
  const std::lock_guard<std::mutex> lock(mutex);
  if (state == 0) {
    try {
      OCIO::SetCurrentConfig(OCIO::Config::CreateFromBuiltinConfig("cg-config-latest"));
      const OCIO::ConstColorSpaceRcPtr linear = OCIO::GetCurrentConfig()->getColorSpace(
          "scene_linear");
      state = (linear && string(linear->getName()) == "ACEScg") ? 1 : -1;
    }
    catch (const OCIO::Exception &exception) {
      fprintf(stderr, "Tequila: OpenColorIO: %s\n", exception.what());
      state = -1;
    }
  }
  if (state < 0) {
    fail("Cycles could not be made to work in ACEScg, Tequila's rendering space");
    return false;
  }
  return true;
#else
  fail("this Cycles was built without OpenColorIO, so it cannot work in ACEScg");
  return false;
#endif
}

static FscSession *create(const char *device, int threads, const char *kernel_dir, bool live)
{
  if (!use_rendering_space()) {
    return nullptr;
  }
  init_once(kernel_dir);

  auto s = std::make_unique<FscSession>();
  if (!device_from_name(device ? device : "CPU", s->session_params.device)) {
    return nullptr;
  }
  s->session_params.background = !live;
  /* Headless turns off progressive display updates, which are the whole
   * point of a live session. */
  s->session_params.headless = !live;
  s->session_params.threads = threads;
  s->session_params.use_auto_tile = false;
  s->session_params.tile_size = 0;
  s->session_params.use_resolution_divider = live;
  s->scene_params.shadingsystem = SHADINGSYSTEM_SVM;
  if (!start_session(s.get())) {
    return nullptr;
  }
  return s.release();
}

extern "C" {

FscSession *fsc_create_live(const char *device, int threads, const char *kernel_dir)
{
  return create(device, threads, kernel_dir, true);
}

void fsc_edit_begin(FscSession *s)
{
  if (!s->editing) {
    s->scene()->mutex.lock();
    s->editing = true;
  }
}

void fsc_edit_end(FscSession *s, int width, int height, int samples)
{
  if (s->editing) {
    /* Setting a socket marks a node modified but tells no manager; Blender's
     * sync tags each changed object and light itself, and so must this, or a
     * moved object stays where it was. */
    Scene *scene = s->scene();
    for (Object *object : scene->objects) {
      if (object->is_modified()) {
        object->tag_update(scene);
      }
    }
    for (Geometry *geometry : scene->geometry) {
      if (geometry->is_modified()) {
        if (geometry->is_light()) {
          static_cast<Light *>(geometry)->tag_update(scene);
        }
        else {
          geometry->tag_update(scene, true);
        }
      }
    }
    s->editing = false;
    s->scene()->mutex.unlock();
  }
  BufferParams buffer;
  buffer.width = width;
  buffer.height = height;
  buffer.full_width = width;
  buffer.full_height = height;
  {
    const thread_scoped_lock lock(s->scene()->mutex);
    Camera *camera = s->scene()->camera;
    camera->set_full_width(width);
    camera->set_full_height(height);
    camera->compute_auto_viewplane();
  }
  s->session_params.samples = samples;
  if (s->live) {
    /* The picture from before the change is not this view: show nothing new
     * until the restarted render has something. */
    const std::lock_guard<std::mutex> lock(s->live->mutex);
    s->live->width = 0;
    s->live->requested = time_dt();
  }
  s->session->reset(s->session_params, buffer);
  s->session->start();
}

void fsc_clear_scene(FscSession *s)
{
  const EditLock lock(s);
  Scene *scene = s->scene();
  std::set<Object *> objects(scene->objects.begin(), scene->objects.end());
  scene->delete_nodes(objects);
  std::set<Geometry *> geometry(scene->geometry.begin(), scene->geometry.end());
  scene->delete_nodes(geometry);
  std::set<Shader *> shaders;
  for (Shader *shader : scene->shaders) {
    if (shader != scene->default_surface && shader != scene->default_volume &&
        shader != scene->default_light && shader != scene->default_background &&
        shader != scene->default_empty)
    {
      shaders.insert(shader);
    }
  }
  scene->delete_nodes(shaders);
  s->pending.clear();
}

int fsc_live_frame(FscSession *s,
                   float *rgba,
                   float *albedo,
                   float *normal,
                   float *object,
                   int capacity_pixels,
                   int *width,
                   int *height,
                   int *samples,
                   unsigned long long *generation)
{
  if (s->live == nullptr) {
    fail("not a live session");
    return 0;
  }
  const std::lock_guard<std::mutex> lock(s->live->mutex);
  if (s->live->generation == *generation || s->live->width == 0) {
    return 0;
  }
  if (size_t(s->live->width) * s->live->height > size_t(capacity_pixels)) {
    fail("the frame does not fit");
    return 0;
  }
  memcpy(rgba, s->live->pixels.data(), s->live->pixels.size() * sizeof(float));
  if (albedo) {
    memcpy(albedo, s->live->albedo.data(), s->live->albedo.size() * sizeof(float));
  }
  if (normal) {
    memcpy(normal, s->live->normal.data(), s->live->normal.size() * sizeof(float));
  }
  if (object) {
    memcpy(object, s->live->object.data(), s->live->object.size() * sizeof(float));
  }
  *width = s->live->width;
  *height = s->live->height;
  *samples = s->live->samples;
  *generation = s->live->generation;
  return 1;
}

const char *fsc_status(FscSession *s)
{
  static thread_local std::string text;
  if (s->session->progress.get_error()) {
    text = "error: " + s->session->progress.get_error_message();
    return text.c_str();
  }
  string status, substatus;
  s->session->progress.get_status(status, substatus);
  text = substatus.empty() ? status : status + ": " + substatus;
  return text.c_str();
}

void fsc_set_paused(FscSession *s, int paused)
{
  s->session->set_pause(paused != 0);
}

void fsc_cancel(FscSession *s)
{
  s->session->progress.set_cancel("Cancelled");
}

int fsc_rendered_samples(FscSession *s)
{
  return s->session ? s->session->progress.get_current_sample() : 0;
}

void fsc_set_time_limit(FscSession *s, double seconds)
{
  s->session_params.time_limit = seconds > 0.0 ? seconds : 0.0;
}

void fsc_set_shadow_tint(FscSession *s, float r, float g, float b)
{
  s->tint.rgb[0] = r;
  s->tint.rgb[1] = g;
  s->tint.rgb[2] = b;
  if ((r != 0.0f || g != 0.0f || b != 0.0f) && !s->catcher_pass) {
    /* The catcher's light ratio, denoised when the image is (so the two
     * agree), merging with the pass Cycles adds for its own catcher. */
    const EditLock lock(s);
    Pass *pass = s->scene()->create_node<Pass>();
    pass->set_name(ustring("catcher"));
    pass->set_type(PASS_SHADOW_CATCHER);
    pass->set_mode(PassMode::DENOISED);
    Pass *count = s->scene()->create_node<Pass>();
    count->set_name(ustring("catcher samples"));
    count->set_type(PASS_SHADOW_CATCHER_SAMPLE_COUNT);
    count->set_mode(PassMode::NOISY);
    s->catcher_pass = true;
  }
}

void fsc_clear(FscSession *s)
{
  s->catcher_pass = false;
  s->pending.clear();
  s->images.clear();
  s->capture = nullptr;
  s->live = nullptr;
  s->session.reset();
  start_session(s);
}

}  // extern "C"
