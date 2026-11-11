//! The names things go by elsewhere.
//!
//! Someone who has built materials in 3ds Max and V-Ray for ten years
//! types what their hands know: VRayDirt, Falloff, UVW Map, VRayMtl. The
//! search finds what Tequila has for each under its own name, and says
//! which name found it ("Occlusion › VRayDirt"), so the next time they know
//! what it is called here.
//!
//! These are names only, used to point (`TRADEMARKS.md` says whose they
//! are). What a thing does is Tequila's own, and where
//! there was nothing to point a name at, the thing was built first (the
//! setups in `browser`, the groups in `tequila_materials::groups`).

use tequila_materials::material_type::MaterialType;

/// A node or pattern, by its label in the Add menu, and the other names
/// that find it.
pub const NODES: &[(&str, &[&str])] = &[
    ("Projection", &["UVW Map", "VRayTriplanarTex", "Triplanar", "Box mapping", "BlendedBoxMap", "Planar mapping", "Cylindrical mapping", "Spherical mapping", "Real-world scale"]),
    ("Mapping", &["UVW Xform", "Tiling and offset", "Coordinates rollout", "Rotate texture"]),
    ("Texture Coordinate", &["VRaySamplerInfoTex", "UVW", "Object XYZ", "Explicit map channel"]),
    ("Geometry", &["VRaySamplerInfoTex", "World XYZ", "Surface normal"]),
    ("Layer Weight", &["VRayFalloff", "Perpendicular / Parallel", "Towards / Away"]),
    ("Fresnel", &["VRayFresnel", "VRayFakeFresnelTex", "Fresnel falloff"]),
    ("Object Info", &["VRayUserColor", "VRayUserScalar", "User attribute", "Object ID", "Node handle"]),
    ("Light Path", &["VRayOverrideMtl", "Ray switch", "Visible to camera"]),
    ("Noise Texture", &["VRayNoiseTex", "BerconNoise", "Perlin", "Fractal", "Turbulence", "Clouds", "Smoke"]),
    ("Voronoi Texture", &["Worley", "Cells", "Crackle", "Chips"]),
    ("Wave Texture", &["Waves", "Stripes", "Rings", "Bands"]),
    ("Brick Texture", &["Tiles", "Bricks", "Running bond", "Stack bond", "VRayBerconTile"]),
    ("Checker Texture", &["Checker", "Chequer"]),
    ("Gradient Texture", &["Gradient", "Gradient Ramp", "BerconGradient", "Linear gradient", "Radial gradient"]),
    ("Gabor Texture", &["Brushed streaks", "Anisotropic grain"]),
    ("Color Ramp", &["Gradient Ramp", "VRayGradient", "Ramp", "Output curve"]),
    ("Mix Color", &["Mix map", "RGB Multiply", "VRayCompTex", "Mask map", "RGB Tint", "Blend modes", "Overlay", "Screen"]),
    ("Hue / Saturation / Value", &["Color Correction", "HSV", "Hue shift", "Saturation", "Tint"]),
    ("Brightness / Contrast", &["Output", "Levels", "Brightness"]),
    ("Gamma", &["Output gamma", "Gamma correction"]),
    ("Invert Color", &["Output invert", "Negative"]),
    ("RGB", &["VRayColor", "Colour swatch", "Solid color"]),
    ("Blackbody", &["Color temperature", "Colour temperature", "VRayColor temperature", "White balance"]),
    ("Value", &["Float", "Constant", "Scalar"]),
    ("Math", &["Arithmetic", "VRayFloat"]),
    ("Map Range", &["Remap", "Fit range", "Output levels"]),
    ("Bump", &["VRayColor2Bump", "VRayBumpMtl", "Height to bump", "Bump map"]),
    ("Curvature", &["VRayCurvature", "Edge mask", "Convexity"]),
    ("Occlusion", &["VRayDirt", "Dirt", "Ambient occlusion", "AO"]),
    ("Distance", &["VRayDistanceTex", "Proximity", "Contact mask"]),
    ("Vertex Color", &["VertexColor", "Vertex paint"]),
    ("Mix Shader", &["VRayBlendMtl", "Blend material", "Shellac"]),
    ("Add Shader", &["Shellac", "Additive"]),
    ("Fuzz", &["VRayFur", "Fur", "Hair and fur", "Grass", "Pile"]),
    ("Bubbles", &["Carbonation", "Foam", "Fizz"]),
    ("Marble", &["Perlin Marble", "Veins"]),
    ("Wood", &["BerconWood", "Wood grain"]),
    ("Wood (Advanced)", &["Advanced Wood", "BerconWood"]),
    ("Spots", &["Speckle", "Splat", "Freckles"]),
    ("Granite", &["Stone", "Speckled stone"]),
    ("Leather", &["Grain leather", "Hide"]),
    ("Weave", &["Fabric", "Carbon fibre", "Carbon fiber", "Twill", "Cloth weave"]),
    ("Perforated mesh", &["Grille", "Speaker mesh", "Perforation", "Punched metal"]),
    ("Scratches", &["Micro scratches", "Swirl marks", "Imperfections"]),
    ("Brushed", &["Brushed metal", "Hairline", "Satin finish"]),
    ("Brushed (Radial)", &["Spun metal", "Circular brushed", "Lathe marks"]),
    ("Cellular", &["Cellular map", "Hammered", "Pebbles"]),
    ("Camouflage", &["Camo"]),
    ("Contour", &["Topographic", "Contour lines"]),
];

/// A kind of material, and the other names that find it.
pub const MATERIALS: &[(MaterialType, &[&str])] = &[
    (MaterialType::Generic, &["VRayMtl", "Physical Material", "Standard material", "Principled", "PBR", "Arch & Design"]),
    (MaterialType::Advanced, &["VRayMtl", "Standard Surface", "OpenPBR"]),
    (MaterialType::MetallicPaint, &["VRayCarPaintMtl", "Car paint", "VRayFlakesMtl", "VRayStochasticFlakesMtl", "Flakes", "Glitter"]),
    (MaterialType::Paint, &["Lacquer", "Gloss paint", "Clear coat"]),
    (MaterialType::Emissive, &["VRayLightMtl", "Light material", "Self-illumination", "Emission", "Glow"]),
    (MaterialType::Toon, &["VRayToonMtl", "Ink 'n Paint", "Cel shading"]),
    (MaterialType::Translucent, &["VRayFastSSS2", "SSS", "Subsurface", "Skin", "VRayALSurfaceMtl", "Wax", "VRay2SidedMtl"]),
    (MaterialType::TranslucentMedium, &["VRayFastSSS2", "Subsurface volume", "Milk", "Jade"]),
    (MaterialType::ScatteringMedium, &["VRayEnvironmentFog", "Fog", "Smoke", "Volume", "VRayVolumeGrid"]),
    (MaterialType::Glass, &["Refraction", "Thin glass", "Window glass"]),
    (MaterialType::GlassSolid, &["Refraction", "Solid glass", "Crystal"]),
    (MaterialType::Liquid, &["Water", "Drink", "Beverage"]),
    (MaterialType::Gem, &["Diamond", "Dispersion", "Gemstone"]),
    (MaterialType::Dielectric, &["Refraction", "IOR", "Abbe"]),
    (MaterialType::Metal, &["Chrome", "Steel", "Aluminium", "Gold", "Complex IOR"]),
    (MaterialType::Anisotropic, &["Brushed metal", "Anisotropy"]),
    (MaterialType::Plastic, &["ABS", "Polycarbonate", "Glossy plastic"]),
    (MaterialType::PlasticCloudy, &["Frosted plastic", "Polypropylene", "Milky plastic"]),
    (MaterialType::PlasticTransparent, &["Clear plastic", "Acrylic", "PMMA", "PET"]),
    (MaterialType::Diffuse, &["Matte", "Lambert", "Clay"]),
    (MaterialType::Flat, &["Unlit", "Constant colour", "Self-illuminated flat"]),
    (MaterialType::Velvet, &["Sheen", "Fabric", "Microfibre"]),
    (MaterialType::RealCloth, &["Cloth", "Fabric", "Woven", "Textile"]),
    (MaterialType::ThinFilm, &["Soap bubble", "Iridescent", "Oil film", "Coating"]),
    (MaterialType::MultiLayerOptics, &["Lens coating", "Anti-reflective", "Dichroic"]),
    (MaterialType::Measured, &["MERL", "Measured BRDF", "Scanned"]),
    (MaterialType::Ground, &["VRayMtlWrapper", "Matte/Shadow", "Shadow catcher", "Matte object"]),
    (MaterialType::Wireframe, &["VRayEdgesTex", "Edges", "Wire"]),
    (MaterialType::XRay, &["Ghost", "See-through"]),
    (MaterialType::Cutaway, &["VRayClipper", "Section", "Clipping"]),
    (MaterialType::AreaLight, &["VRayLight", "Plane light", "Softbox", "VRaySoftbox"]),
    (MaterialType::PointLight, &["Omni", "Sphere light", "Bulb"]),
    (MaterialType::Spotlight, &["Spot", "Target spot"]),
    (MaterialType::IesLight, &["VRayIES", "Photometric", "IES profile"]),
];

/// The other names a kind of material goes by.
pub fn of_material(kind: MaterialType) -> &'static [&'static str] {
    MATERIALS.iter().find(|(its, _)| *its == kind).map_or(&[], |(_, names)| names)
}
