// The OpenPBR layer stack: tequila_materials::lobes::openpbr's OpenPbrBsdf,
// fuzz over coat over base, each layer attenuating what is beneath it, the
// base mixing metal, specular and diffuse by weight (docs/02 §2.1). Shading
// frame, z along the normal; the material is a GpuMaterial (material.rs).
//
// The fuzz (Sheen), the coat (DielectricSpecular with its tint and weight),
// the metal (Conductor with metal_ior, tinted by base_color, anisotropic),
// the dielectric specular interface, the transmission lobe (glass, solid or
// thin-walled, its reflect-or-refract choice the stack's rescaled lobe
// variate) and the diffuse substrate (EON), with base_weight and
// geometry_occlusion in the lobes' scales; the interface and the diffuse
// share what the transmission leaves (`opaque`), and the coat traps light
// against the transmission colour as well as the diffuse. Six lobes in their
// slots, at most five of them in a stack, as the processor's MAX_LOBES drops
// the sixth pushed. NOT here, and not in GpuMaterial (GpuMaterial::supports
// refuses a material that has them):
// - subsurface: subsurface_weight = 0, no walk doorway, and the diffuse
//   albedo the trapped light bounces against is base_color's alone;
// - thin film: no film over the metal or the interface (thin_film_weight at
//   or below the 1e-4 the processor ignores);
// - flakes: flake coverage 0, so no Depth::Flake lobe, no flake frame, and
//   the base's scale and response are the base's whole;
// - emission and geometry_opacity: not the BSDF's at all (the integrator's;
//   GpuMaterial carries both for it).
//
// The processor's lobes cache their table reads for a shading point
// (DielectricSpecular's Seen, aimed by OpenPbrBsdf::aim): the same reads
// with the same arguments, so here the reads are simply made.
//
// Ported: build (new, to_aim, from_inside, from_inside_to_aim), aim,
// attenuation and its trapped-light series, eval, eval_selective, pdf,
// pdf_reflection, sample and sample_reflection, albedo, and of the flags
// whether there is a delta lobe and whether the stack transmits.
//#include features
//#include sheen

// Which layer of §2.1's stack a lobe belongs to (openpbr.rs's Depth), in
// its order; deeper layers are attenuated by everything above them.
const DEPTH_FUZZ: u32 = 0u;
const DEPTH_COAT: u32 = 1u;
// Depth::Flake would be 2u: flakes are not ported yet.
const DEPTH_BASE: u32 = 3u;
const DEPTH_SUBSTRATE: u32 = 4u;

// The lobes, each in a slot of its own, in the order OpenPbrBsdf::build
// pushes them; the processor's list is these slots with the absent ones
// left out, so walking the slots in order is walking its list.
const LOBE_FUZZ: u32 = 0u;
const LOBE_COAT: u32 = 1u;
const LOBE_METAL: u32 = 2u;
const LOBE_SPECULAR: u32 = 3u;
const LOBE_TRANSMISSION: u32 = 4u;
const LOBE_DIFFUSE: u32 = 5u;
const OPENPBR_LOBES: u32 = 6u;
// How many of them a stack holds at once (openpbr.rs's MAX_LOBES): a sixth
// pushed is dropped, so with fuzz, coat, metal, interface and transmission
// all there, the diffuse is not.
const OPENPBR_MAX_LOBES: u32 = 5u;

// material.rs's GpuMaterial, member for member (tests/layouts.rs holds the
// two to one layout). Colours are vec4s whose w is unused.
struct GpuMaterial {
    base_color: vec4<f32>,
    metal_n: vec4<f32>,
    metal_k: vec4<f32>,
    specular_color: vec4<f32>,
    coat_color: vec4<f32>,
    fuzz_color: vec4<f32>,
    base_weight: f32,
    base_metalness: f32,
    base_diffuse_roughness: f32,
    specular_weight: f32,
    specular_roughness: f32,
    specular_roughness_anisotropy: f32,
    specular_ior: f32,
    coat_weight: f32,
    coat_roughness: f32,
    coat_ior: f32,
    coat_darkening: f32,
    fuzz_weight: f32,
    fuzz_roughness: f32,
    geometry_occlusion: f32,
    // The anisotropic frame's turn, which the integrator reads, not the
    // stack.
    specular_rotation: f32,
    // The cutout's opacity: 1 a wall, 0 a hole. The integrator's.
    geometry_opacity: f32,
    // emission_color, and in w emission_luminance: the integrator's
    // (material_emission, material_is_emissive).
    emission_color: vec4<f32>,
    // transmission_color, and in w transmission_weight.
    transmission_color: vec4<f32>,
    // The transmission lobe's tint at each crossing (the square root of
    // the colour the surface wears, worked out on the processor), and in w
    // transmission_depth.
    transmission_tint: vec4<f32>,
    volume_density: f32,
    medium_priority: f32,
    // The scene's own material's index, for a medium entered where no
    // interface is (the integrator's).
    base_ior: f32,
    // GLASS_* (material.rs).
    glass: u32,
    // The clear glass a shadow ray passes (SceneSnapshot::pane), the
    // integrator's: its weight, opacity, index, coat and coat index, and
    // the depth its volume reaches its colour in.
    pane_weight: f32,
    pane_opacity: f32,
    pane_ior: f32,
    pane_coat: f32,
    pane_coat_ior: f32,
    pane_depth: f32,
    // Rounded edges (bevel.rs): the radius, none at 0, and the probes'
    // count. The integrator's, at the surface a path meets first.
    edge_radius: f32,
    edge_samples: u32,
    // What the volume behind gives off (Material::emitted_over): the glow,
    // and in w what it absorbs a metre before its colour. The integrator's.
    volume_glow: vec4<f32>,
};

// Material::emission, in nits: the colour times the luminance.
fn material_emission(m: GpuMaterial) -> vec3<f32> {
    return m.emission_color.xyz * m.emission_color.w;
}

// Material::is_emissive: a luminance above nought and a colour that is not
// all nought.
fn material_is_emissive(m: GpuMaterial) -> bool {
    return m.emission_color.w > 0.0 && any(m.emission_color.xyz != vec3<f32>(0.0));
}

const GLASS_THIN: u32 = 1u;
const GLASS_BOUNDARY: u32 = 2u;
const GLASS_VOLUME: u32 = 4u;
const GLASS_PANE: u32 = 8u;
const GLASS_PLAIN: u32 = 16u;
const GLASS_BENDS: u32 = 32u;
const GLASS_PANE_VOLUME: u32 = 64u;

// The assembled stack for one shading point.
struct OpenPbr {
    // Bit `slot` set for each lobe the stack holds, how many, and the slot
    // of the last one pushed.
    present: u32,
    count: u32,
    last: u32,
    // Each lobe's scale and weight, and its selection probability
    // (normalized), by slot.
    scales: array<vec3<f32>, 6>,
    weights: array<f32, 6>,
    probabilities: array<f32, 6>,
    // The lobes. The fuzz, the coat and the interface are also the layers
    // over the base, kept so their attenuation is evaluated for the actual
    // pair of directions: `has_*` says the layer is there, which it can be
    // when its lobe was not pushed (an occlusion of 0, or a dielectric
    // share too small for the interface's lobe).
    fuzz: Sheen,
    has_fuzz: bool,
    coat: Specular,
    has_coat: bool,
    metal: Conductor,
    specular: Specular,
    has_specular: bool,
    transmission: Transmission,
    diffuse: Diffuse,
    coat_weight: f32,
    coat_darkening: f32,
    // The hemispherical response of what the coat sits on, and of the
    // coat's whole answer (what the fuzz sits on).
    base_response: f32,
    coat_response: f32,
    specular_scale: f32,
    diffuse_albedo: f32,
    // What the stack above the substrate turns back, for the one
    // trapped-light series under it (stack_transmittance).
    substrate_internal: f32,
    // Whether `probabilities` holds a choice of lobes: not yet when built
    // for openpbr_aim to choose them.
    chosen: bool,
};

// Spectrum::luminance, ACEScg.
fn spectrum_luminance(c: vec3<f32>) -> f32 {
    return 0.2722287 * c.x + 0.6740818 * c.y + 0.0536895 * c.z;
}

fn openpbr_finite(x: f32) -> bool {
    return abs(x) <= 3.40282347e38;
}

fn openpbr_has(b: OpenPbr, slot: u32) -> bool {
    return (b.present & (1u << slot)) != 0u;
}

fn openpbr_depth(slot: u32) -> u32 {
    switch slot {
        case 0u: { return DEPTH_FUZZ; }
        case 1u: { return DEPTH_COAT; }
        case 5u: { return DEPTH_SUBSTRATE; }
        default: { return DEPTH_BASE; }
    }
}

// build's `rough`: path regularization (docs/01 §3.5) raises a reflection's
// roughness to the floor, never lowers it.
fn openpbr_rough(r: f32, roughness_floor: f32) -> f32 {
    return max(clamp(r, 0.0, 1.0), roughness_floor);
}

// build's `push`: occlusion scales every lobe, and a lobe with no weight or
// no scale is not there.
fn openpbr_push(b: ptr<function, OpenPbr>, slot: u32, scale_in: vec3<f32>, weight: f32, occlusion: f32) {
    let scale = scale_in * occlusion;
    if ((*b).count < OPENPBR_MAX_LOBES && weight > 1e-4 && !all(scale == vec3<f32>(0.0))) {
        (*b).present |= 1u << slot;
        (*b).count += 1u;
        (*b).last = slot;
        (*b).scales[slot] = scale;
        (*b).weights[slot] = weight;
    }
}

// layer_response: a layer over a substrate, its own reflection, what it
// passes down and gets back, and everything trapped between.
fn openpbr_layer_response(layer_in: f32, substrate_in: f32) -> f32 {
    let layer = clamp(layer_in, 0.0, 1.0);
    let substrate = clamp(substrate_in, 0.0, 1.0);
    let transmitted = 1.0 - layer;
    return clamp(layer + transmitted * transmitted * substrate / max(1.0 - layer * substrate, 1e-3), 0.0, 1.0);
}

// stack_transmittance: the cosine-weighted average of the product of the
// transmittances of every layer over the substrate, measured over sixteen
// midpoints of mu when there is more than one layer to correlate.
fn openpbr_stack_transmittance(b: OpenPbr, coat_average: f32, coat_weight: f32, specular_average: f32, specular_scale: f32) -> f32 {
    let layers = u32(b.has_fuzz) + u32(b.has_coat) + u32(b.has_specular);
    if (layers <= 1u) {
        var reflected = 0.0;
        if (b.has_fuzz) {
            reflected = sheen_hemispherical_albedo(b.fuzz);
        } else if (b.has_coat) {
            reflected = coat_average * coat_weight;
        } else if (b.has_specular) {
            reflected = specular_average * specular_scale;
        }
        return clamp(1.0 - reflected, 0.0, 1.0);
    }
    var total = 0.0;
    for (var i = 0u; i < 16u; i++) {
        let mu = (f32(i) + 0.5) / 16.0;
        let wo = vec3<f32>(sqrt(max(1.0 - mu * mu, 0.0)), 0.0, mu);
        var transmitted = 1.0;
        if (b.has_fuzz) {
            transmitted *= sheen_transmittance(b.fuzz, mu);
        }
        if (b.has_coat) {
            transmitted *= clamp(1.0 - specular_directional_albedo(b.coat, wo) * coat_weight, 0.0, 1.0);
        }
        if (b.has_specular) {
            transmitted *= clamp(1.0 - specular_directional_albedo(b.specular, wo) * specular_scale, 0.0, 1.0);
        }
        total += transmitted * 2.0 * mu;
    }
    return clamp(total / 16.0, 0.0, 1.0);
}

// A lobe's albedo seen from `wo`, as its own Bsdf::albedo gives it.
fn openpbr_lobe_albedo(b: OpenPbr, slot: u32, wo: vec3<f32>) -> vec3<f32> {
    switch slot {
        case 0u: { return sheen_albedo(b.fuzz, wo); }
        case 1u: { return b.coat.tint * specular_directional_albedo(b.coat, wo); }
        case 2u: { return conductor_albedo(b.metal, wo); }
        case 3u: { return b.specular.tint * specular_directional_albedo(b.specular, wo); }
        case 4u: { return transmission_albedo(b.transmission); }
        default: { return b.diffuse.albedo; }
    }
}

fn openpbr_lobe_eval(b: OpenPbr, slot: u32, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    switch slot {
        case 0u: { return sheen_eval(b.fuzz, wo, wi); }
        case 1u: { return specular_eval(b.coat, wo, wi); }
        case 2u: { return conductor_eval(b.metal, wo, wi); }
        case 3u: { return specular_eval(b.specular, wo, wi); }
        case 4u: { return transmission_eval(b.transmission, wo, wi); }
        default: { return diffuse_eval(b.diffuse, wo, wi); }
    }
}

fn openpbr_lobe_pdf(b: OpenPbr, slot: u32, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    switch slot {
        case 0u: { return sheen_pdf(b.fuzz, wo, wi); }
        case 1u: { return specular_pdf(b.coat, wo, wi); }
        case 2u: { return conductor_pdf(b.metal, wo, wi); }
        case 3u: { return specular_pdf(b.specular, wo, wi); }
        case 4u: { return transmission_pdf(b.transmission, wo, wi); }
        default: { return diffuse_pdf(b.diffuse, wo, wi); }
    }
}

// A lobe's draw (lobes.wgsl's diffuse_draw): its sample, but for the check
// of its own density and value, which the stack makes from its own
// evaluation in the direction drawn.
fn openpbr_lobe_draw(b: OpenPbr, slot: u32, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    switch slot {
        case 0u: { return sheen_draw(b.fuzz, wo, u); }
        case 1u: { return specular_draw(b.coat, wo, u, uc); }
        case 2u: { return conductor_draw(b.metal, wo, u, uc); }
        case 3u: { return specular_draw(b.specular, wo, u, uc); }
        case 4u: { return transmission_draw(b.transmission, wo, u, uc); }
        default: { return diffuse_draw(b.diffuse, wo, u); }
    }
}

// Lobe::reflection_roughness: the coat's, the interface's or the metal's;
// negative for the rest (the processor's None).
fn openpbr_reflection_roughness(b: OpenPbr, slot: u32) -> f32 {
    switch slot {
        case 1u: { return ggx_roughness(b.coat.distribution); }
        case 2u: { return ggx_roughness(b.metal.distribution); }
        case 3u: { return ggx_roughness(b.specular.distribution); }
        default: { return -1.0; }
    }
}

// face_on_probabilities: each lobe by its albedo seen face on (§2.4), the
// interfaces floored at 0.05 (not the transmission).
fn openpbr_face_on(b: OpenPbr) -> array<f32, 6> {
    var probabilities: array<f32, 6>;
    var total = 0.0;
    let z = vec3<f32>(0.0, 0.0, 1.0);
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        var estimate = spectrum_luminance(openpbr_lobe_albedo(b, slot, z));
        if (slot == LOBE_COAT || slot == LOBE_SPECULAR) {
            estimate = max(estimate, 0.05);
        }
        let p = max(estimate * spectrum_luminance(b.scales[slot]) * b.weights[slot], 1e-4);
        probabilities[slot] = p;
        total += p;
    }
    if (total > 0.0) {
        for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
            probabilities[slot] /= total;
        }
    }
    return probabilities;
}

// OpenPbrBsdf::build, for the restricted stack (see the top of this file):
// `from_inside` for a path already inside the material, which only the
// transmission lobe reads. `m.specular_ior` is the index the integrator
// gives the interface (the relative one between two media), which the
// interface and the transmission both read.
fn openpbr_build(m: GpuMaterial, roughness_floor: f32, to_aim: bool, from_inside: bool) -> OpenPbr {
    var b: OpenPbr;
    let occlusion = clamp(m.geometry_occlusion, 0.0, 1.0);

    // --- fuzz, the topmost layer, at a weight of one: the sheen carries
    // its own.
    b.has_fuzz = m.fuzz_weight > 1e-4;
    if (b.has_fuzz) {
        b.fuzz = sheen_new(m.fuzz_color.xyz, m.fuzz_roughness, m.fuzz_weight);
        openpbr_push(&b, LOBE_FUZZ, vec3<f32>(1.0), 1.0, occlusion);
    }

    // --- coat
    b.has_coat = m.coat_weight > 1e-4;
    if (b.has_coat) {
        b.coat = specular_new(openpbr_rough(m.coat_roughness, roughness_floor), m.coat_ior);
        b.coat.tint = m.coat_color.xyz;
        openpbr_push(&b, LOBE_COAT, vec3<f32>(1.0), m.coat_weight, occlusion);
    }

    // --- base. No flakes: the base's scale is its weight.
    let base_scale = vec3<f32>(m.base_weight);
    var specular_scale = 0.0;
    var diffuse_albedo = 0.0;
    if (!all(base_scale == vec3<f32>(0.0))) {
        let metalness = m.base_metalness;
        if (metalness > 1e-4) {
            b.metal = Conductor(
                ggx_from_roughness(openpbr_rough(m.specular_roughness, roughness_floor), m.specular_roughness_anisotropy),
                m.metal_n.xyz,
                m.metal_k.xyz,
                m.base_color.xyz,
            );
            openpbr_push(&b, LOBE_METAL, base_scale, metalness, occlusion);
        }
        let dielectric = 1.0 - metalness;
        if (dielectric > 1e-4) {
            // Where the base transmits, the interface is the transmission
            // lobe's own reflect-or-refract split, so it and the diffuse
            // share what the transmission leaves.
            let transmission_weight = m.transmission_color.w;
            let opaque = 1.0 - transmission_weight;
            let specular_weight = m.specular_weight * opaque;
            if (specular_weight > 1e-4) {
                b.specular = specular_new(openpbr_rough(m.specular_roughness, roughness_floor), m.specular_ior);
                b.specular.tint = m.specular_color.xyz;
                b.has_specular = true;
                specular_scale = specular_weight;
                openpbr_push(&b, LOBE_SPECULAR, base_scale, dielectric * specular_weight, occlusion);
            }
            if (FEATURE_GLASS && transmission_weight > 1e-4) {
                // Isotropic whatever the anisotropy; its tint the square
                // root of what the surface wears, worked out on the
                // processor (material.rs).
                b.transmission = transmission_new(openpbr_rough(m.specular_roughness, roughness_floor), m.specular_ior, m.transmission_tint.xyz, (m.glass & GLASS_THIN) != 0u, from_inside);
                openpbr_push(&b, LOBE_TRANSMISSION, base_scale, dielectric * transmission_weight, occlusion);
            }
            // No subsurface: the substrate returns base_color, and has
            // whatever the transmission did not take.
            diffuse_albedo = clamp(spectrum_luminance(m.base_color.xyz), 0.0, 1.0);
            let diffuse_weight = dielectric * opaque;
            if (diffuse_weight > 1e-4) {
                b.diffuse = diffuse_new(m.base_color.xyz, m.base_diffuse_roughness);
                openpbr_push(&b, LOBE_DIFFUSE, base_scale, diffuse_weight, occlusion);
            }
        }
    }

    // --- selection probabilities (§2.4), face on, unless openpbr_aim is
    // about to choose them.
    b.chosen = !(to_aim && b.count >= 2u);
    if (b.chosen) {
        b.probabilities = openpbr_face_on(b);
    }

    // --- what each layer sits on, from the bottom up.
    let metalness = clamp(m.base_metalness, 0.0, 1.0);
    // Under the interface: the diffuse substrate, or what the transmission
    // carries away where the base transmits (its colour, not the tint).
    let transmission = clamp(m.transmission_color.w, 0.0, 1.0);
    let under_interface = clamp(diffuse_albedo * (1.0 - transmission) + spectrum_luminance(m.transmission_color.xyz) * transmission, 0.0, 1.0);
    var specular_average = 0.0;
    if (b.has_specular) {
        specular_average = specular_hemispherical_albedo(b.specular);
    }
    var coat_average = 0.0;
    if (b.has_coat) {
        coat_average = specular_hemispherical_albedo(b.coat);
    }
    let interface_albedo = select(0.0, specular_average * specular_scale, b.has_specular);
    let dielectric_response = openpbr_layer_response(interface_albedo, under_interface);
    let metal_response = clamp(spectrum_luminance(m.base_color.xyz), 0.0, 1.0);
    // No flakes to share the base's response.
    let base_response = metal_response * metalness + dielectric_response * (1.0 - metalness);

    var substrate_internal = 0.0;
    if (diffuse_albedo > 0.0 && (b.has_fuzz || b.has_coat || b.has_specular)) {
        substrate_internal = 1.0 - openpbr_stack_transmittance(b, coat_average, m.coat_weight, specular_average, specular_scale);
    }
    var coat_response = base_response;
    if (b.has_coat) {
        coat_response = openpbr_layer_response(coat_average * m.coat_weight, base_response);
    }

    b.coat_weight = m.coat_weight;
    b.coat_darkening = m.coat_darkening;
    b.base_response = base_response;
    b.coat_response = coat_response;
    b.specular_scale = specular_scale;
    b.diffuse_albedo = diffuse_albedo;
    b.substrate_internal = substrate_internal;
    return b;
}

// OpenPbrBsdf::new: `roughness_floor` is path regularization.
fn openpbr_new(m: GpuMaterial, roughness_floor: f32) -> OpenPbr {
    return openpbr_build(m, roughness_floor, false, false);
}

// OpenPbrBsdf::to_aim: the same stack, for openpbr_aim next.
fn openpbr_to_aim(m: GpuMaterial, roughness_floor: f32) -> OpenPbr {
    return openpbr_build(m, roughness_floor, true, false);
}

// OpenPbrBsdf::from_inside and ::from_inside_to_aim.
fn openpbr_from_inside(m: GpuMaterial, roughness_floor: f32) -> OpenPbr {
    return openpbr_build(m, roughness_floor, false, true);
}

fn openpbr_from_inside_to_aim(m: GpuMaterial, roughness_floor: f32) -> OpenPbr {
    return openpbr_build(m, roughness_floor, true, true);
}

// trapped: the light that crosses a layer, comes back off the assembly
// beneath it and leaves, everything trapped in between included.
fn openpbr_trapped(down_in: f32, up_in: f32, internal_in: f32, substrate_in: f32) -> f32 {
    let down = clamp(down_in, 0.0, 1.0);
    let up = clamp(up_in, 0.0, 1.0);
    let internal = clamp(internal_in, 0.0, 1.0);
    let substrate = clamp(substrate_in, 0.0, 1.0);
    return down * up / max(1.0 - substrate * internal, 1e-3);
}

// attenuation: how much of the light travelling `wo` in and `wi` out
// survives the layers above `depth`, each crossed twice. Off the interface
// each layer sums its own trapped-light series against what is beneath it;
// off the substrate there is one series for the whole stack.
fn openpbr_attenuation(b: OpenPbr, depth: u32, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    let spreads = depth > DEPTH_BASE;
    var through = 1.0;
    if (depth > DEPTH_FUZZ && b.has_fuzz) {
        let down = sheen_transmittance(b.fuzz, wo.z);
        let up = sheen_transmittance(b.fuzz, wi.z);
        if (spreads) {
            through *= down * up;
        } else {
            through *= openpbr_trapped(down, up, 1.0 - up, b.coat_response);
        }
    }
    if (depth > DEPTH_COAT && b.has_coat) {
        let down = 1.0 - specular_directional_albedo(b.coat, wo) * b.coat_weight;
        let up = 1.0 - specular_directional_albedo(b.coat, wi) * b.coat_weight;
        var physical: f32;
        if (spreads) {
            physical = clamp(down * up, 0.0, 1.0);
        } else {
            physical = openpbr_trapped(down, up, 1.0 - up, b.base_response);
        }
        // §2.2's coat_darkening, between the physical result and none.
        through *= physical * b.coat_darkening + (1.0 - b.coat_darkening);
    }
    if (spreads) {
        if (b.has_specular) {
            let scale = b.specular_scale;
            let down = 1.0 - specular_directional_albedo(b.specular, wo) * scale;
            let up = 1.0 - specular_directional_albedo(b.specular, wi) * scale;
            through *= clamp(down, 0.0, 1.0) * clamp(up, 0.0, 1.0);
        }
        // The one series, for the whole stack at once.
        through /= max(1.0 - b.diffuse_albedo * b.substrate_internal, 1e-3);
    }
    return through;
}

fn openpbr_eval(b: OpenPbr, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let through = openpbr_attenuation(b, openpbr_depth(slot), wo, wi);
        total += openpbr_lobe_eval(b, slot, wo, wi) * (b.scales[slot] * (b.weights[slot] * through));
    }
    return total;
}

// The weighted sum over every lobe (§2.4), never only the sampled one's.
fn openpbr_pdf(b: OpenPbr, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    var total = 0.0;
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        let p = b.probabilities[slot];
        if (openpbr_has(b, slot) && p > 0.0) {
            total += p * openpbr_lobe_pdf(b, slot, wo, wi);
        }
    }
    return total;
}

// eval_selective (§5.4's visibility flags): only the diffuse lobe, only the
// others, or all of them - the same sum as openpbr_eval then - each lobe
// still attenuated by every layer over it, whether that layer's own lobe
// is in or not.
fn openpbr_eval_selective(b: OpenPbr, wo: vec3<f32>, wi: vec3<f32>, diffuse: bool, specular: bool) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let is_diffuse = slot == LOBE_DIFFUSE;
        if ((is_diffuse && !diffuse) || (!is_diffuse && !specular)) {
            continue;
        }
        let through = openpbr_attenuation(b, openpbr_depth(slot), wo, wi);
        total += openpbr_lobe_eval(b, slot, wo, wi) * (b.scales[slot] * (b.weights[slot] * through));
    }
    return total;
}

// flags().contains(DELTA): a mirror-smooth reflection among the lobes.
fn openpbr_has_delta(b: OpenPbr) -> bool {
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        switch slot {
            case 1u: { if (ggx_is_smooth(b.coat.distribution)) { return true; } }
            case 2u: { if (ggx_is_smooth(b.metal.distribution)) { return true; } }
            case 3u: { if (ggx_is_smooth(b.specular.distribution)) { return true; } }
            case 4u: { if (ggx_is_smooth(b.transmission.distribution)) { return true; } }
            default: {}
        }
    }
    return false;
}

// flags().contains(TRANSMISSION): the transmission lobe is in the stack (a
// transmission weight whose lobe was dropped transmits nothing).
fn openpbr_transmits(b: OpenPbr) -> bool {
    return openpbr_has(b, LOBE_TRANSMISSION);
}

// The share of a reflection of roughness `r` kept, faded out between
// `up_to` and `passed_from` (integrator.rs's seeing_share, 0.5 and 0.8).
fn openpbr_seeing_share(r: f32, up_to: f32, passed_from: f32) -> f32 {
    let t = clamp((r - up_to) / (passed_from - up_to), 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

// The stack's density for `wi` (openpbr_pdf), and pdf_reflection's: the
// density the reflections sample_reflection names a roughness for - the
// coat, the metal, the interface - give it, each weighed by the share
// openpbr_seeing_share keeps of its roughness (a mirror-smooth lobe has no
// density and adds none). Both from one density of each lobe.
fn openpbr_densities(b: OpenPbr, wo: vec3<f32>, wi: vec3<f32>, up_to: f32, passed_from: f32) -> vec2<f32> {
    var total = 0.0;
    var seen = 0.0;
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        let p = b.probabilities[slot];
        if (!openpbr_has(b, slot) || !(p > 0.0)) {
            continue;
        }
        let density = openpbr_lobe_pdf(b, slot, wo, wi);
        total += p * density;
        let roughness = openpbr_reflection_roughness(b, slot);
        if (roughness >= 0.0) {
            let kept = openpbr_seeing_share(roughness, up_to, passed_from);
            if (kept > 0.0) {
                seen += p * kept * density;
            }
        }
    }
    return vec2<f32>(total, seen);
}

// OpenPbrBsdf::pdf_reflection (see openpbr_densities).
fn openpbr_pdf_reflection(b: OpenPbr, wo: vec3<f32>, wi: vec3<f32>, up_to: f32, passed_from: f32) -> f32 {
    return openpbr_densities(b, wo, wi, up_to, passed_from).y;
}

// openpbr_eval's sum, and in the same pass the part of it the lobes other
// than the diffuse give (eval_selective(false, true)).
struct StackValue {
    total: vec3<f32>,
    glossy: vec3<f32>,
};

fn openpbr_values(b: OpenPbr, wo: vec3<f32>, wi: vec3<f32>) -> StackValue {
    var v = StackValue(vec3<f32>(0.0), vec3<f32>(0.0));
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let through = openpbr_attenuation(b, openpbr_depth(slot), wo, wi);
        let term = openpbr_lobe_eval(b, slot, wo, wi) * (b.scales[slot] * (b.weights[slot] * through));
        v.total += term;
        if (slot != LOBE_DIFFUSE) {
            v.glossy += term;
        }
    }
    return v;
}

struct ReflectionSample {
    sample: LobeSample,
    // The chosen lobe's reflection_roughness; negative where it is not a
    // reflection off the coat, the interface or a metal.
    roughness: f32,
    // For a sample that is not a delta's: pdf_reflection in its direction,
    // and the part of its value the lobes other than the diffuse give.
    seen: f32,
    glossy: vec3<f32>,
    // The lobe it took (LOBE_*): a bounce off the diffuse one is a diffuse
    // ray, any other a glossy one (the integrator's Seen).
    slot: u32,
};

// sample_reflection: a lobe chosen by `uc` against the probabilities, `uc`
// rescaled for it, its sample, and then the whole stack's value and density
// in that direction - or, for a delta lobe, its value through the layers
// over it divided by the chance of having chosen it.
fn openpbr_sample_reflection(b: OpenPbr, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> ReflectionSample {
    return openpbr_sample_seen(b, wo, u, uc, 0.5, 0.8);
}

// openpbr_sample_reflection, with what the integrator asks of a bounce off a
// reflection (ReflectionSample's seen and glossy, the density by
// openpbr_seeing_share's thresholds) from the same evaluation.
fn openpbr_sample_seen(b: OpenPbr, wo: vec3<f32>, u: vec2<f32>, uc: f32, up_to: f32, passed_from: f32) -> ReflectionSample {
    var none: ReflectionSample;
    none.sample = no_sample();
    none.roughness = -1.0;
    if (b.count == 0u) {
        return none;
    }
    var cumulative = 0.0;
    var chosen = OPENPBR_LOBES;
    var lobe_uc = 0.5;
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let p = b.probabilities[slot];
        if (uc < cumulative + p || slot == b.last) {
            if (p > 0.0) {
                lobe_uc = clamp((uc - cumulative) / p, 0.0, 0.99999988079071044921875);
            }
            chosen = slot;
            break;
        }
        cumulative += p;
    }
    if (chosen == OPENPBR_LOBES) {
        return none;
    }
    let roughness = openpbr_reflection_roughness(b, chosen);
    let s = openpbr_lobe_draw(b, chosen, wo, u, lobe_uc);
    if (!s.ok) {
        return none;
    }
    let probability = max(b.probabilities[chosen], 1e-6);
    if (s.is_delta) {
        let through = openpbr_attenuation(b, openpbr_depth(chosen), wo, s.wi);
        let f = s.f * (b.scales[chosen] * (b.weights[chosen] * through / probability));
        return ReflectionSample(LobeSample(true, s.wi, f, 1.0, true, s.is_transmission, s.eta), roughness, 0.0, f, chosen);
    }
    // The whole stack's value and density in the direction drawn, in one
    // pass, and the drawn lobe's own, which keep its draw or not as its own
    // sample would (a density above nought; a transmission's value not
    // black).
    var total = 0.0;
    var seen = 0.0;
    var value = StackValue(vec3<f32>(0.0), vec3<f32>(0.0));
    var own_pdf = 0.0;
    var own_value = vec3<f32>(0.0);
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let density = openpbr_lobe_pdf(b, slot, wo, s.wi);
        let raw = openpbr_lobe_eval(b, slot, wo, s.wi);
        if (slot == chosen) {
            own_pdf = density;
            own_value = raw;
        }
        let p = b.probabilities[slot];
        if (p > 0.0) {
            total += p * density;
            let reflection = openpbr_reflection_roughness(b, slot);
            if (reflection >= 0.0) {
                let kept = openpbr_seeing_share(reflection, up_to, passed_from);
                if (kept > 0.0) {
                    seen += p * kept * density;
                }
            }
        }
        let through = openpbr_attenuation(b, openpbr_depth(slot), wo, s.wi);
        let term = raw * (b.scales[slot] * (b.weights[slot] * through));
        value.total += term;
        if (slot != LOBE_DIFFUSE) {
            value.glossy += term;
        }
    }
    if (own_pdf <= 0.0 || (chosen == LOBE_TRANSMISSION && all(own_value == vec3<f32>(0.0)))) {
        return none;
    }
    if (total <= 0.0) {
        return none;
    }
    if (all(value.total == vec3<f32>(0.0))) {
        return none;
    }
    return ReflectionSample(LobeSample(true, s.wi, value.total, total, false, s.is_transmission, s.eta), roughness, seen, value.glossy, chosen);
}

fn openpbr_sample(b: OpenPbr, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    return openpbr_sample_reflection(b, wo, u, uc).sample;
}

fn openpbr_albedo(b: OpenPbr, wo: vec3<f32>) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let through = openpbr_attenuation(b, openpbr_depth(slot), wo, wo);
        total += openpbr_lobe_albedo(b, slot, wo) * (b.scales[slot] * (b.weights[slot] * through));
    }
    return clamp(total, vec3<f32>(0.0), vec3<f32>(1.0));
}

struct AimChoice {
    ok: bool,
    probabilities: array<f32, 6>,
};

// aim_choosing: each lobe by its albedo towards `wo` through the layers
// over it, with the same floors; declined seen from behind, with one lobe,
// or when the estimates come to nothing usable.
fn openpbr_aim_choosing(b: OpenPbr, wo: vec3<f32>) -> AimChoice {
    var choice: AimChoice;
    if (wo.z <= 0.0 || b.count < 2u) {
        return choice;
    }
    var total = 0.0;
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        let through = openpbr_attenuation(b, openpbr_depth(slot), wo, wo);
        let seen = spectrum_luminance(openpbr_lobe_albedo(b, slot, wo));
        let least = select(0.0, 0.05, slot == LOBE_COAT || slot == LOBE_SPECULAR);
        let p = max(max(seen, least) * through * spectrum_luminance(b.scales[slot]) * b.weights[slot], 1e-4);
        choice.probabilities[slot] = p;
        total += p;
    }
    if (total > 0.0 && openpbr_finite(total)) {
        for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
            choice.probabilities[slot] /= total;
        }
        choice.ok = true;
    }
    return choice;
}

// OpenPbrBsdf::aim: choose the lobes by what each sends back towards `wo`
// rather than face on (docs/07 R1); call it once, before the stack's pdf
// and sample, with the `wo` they will be given. Declined, the face-on
// choice after all.
fn openpbr_aim(b_in: OpenPbr, wo: vec3<f32>) -> OpenPbr {
    var b = b_in;
    let choice = openpbr_aim_choosing(b, wo);
    if (choice.ok) {
        b.probabilities = choice.probabilities;
    } else if (!b.chosen) {
        b.probabilities = openpbr_face_on(b);
    }
    b.chosen = true;
    return b;
}
