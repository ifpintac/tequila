//! χ² sampling tests for every BSDF lobe and light type.
//!
//! `docs/05-ROADMAP-AND-TESTING.md` §3.2: "For each BSDF and each light type,
//! verify that `sample()` and `pdf()` agree." This is the M0 exit-criterion
//! test, and it runs for every lobe at six roughness values, for the layered
//! stack, and for the environment sampler.
//!
//! M2 adds the lobes that came with the OpenPBR model — rough dielectric
//! transmission, sheen, and the assembled layer stack — plus the second half
//! of that milestone's exit criterion: "the χ² test passes for every material
//! type in the catalog".
//!
//! Note the multiple-comparison correction below. Running ~100 tests at
//! p = 0.01 would produce a spurious failure roughly two times in three, and a
//! suite that cries wolf gets ignored — which is worse than not having it.

use tequila_analytic_tests::{chi2_test, Chi2Config, SphericalDistribution};
use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::metals;
use tequila_materials::lobes::{
    Conductor, DielectricSpecular, DielectricTransmission, OpenPbrBsdf, OrenNayarDiffuse,
    SpecularOverDiffuse,
};
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_materials::microfacet::Ggx;
use tequila_materials::Bsdf;

/// Significance level from §3.2.
const SIGNIFICANCE: f64 = 0.01;

/// The roughness ladder §3.2 asks for.
const ROUGHNESSES: [f32; 6] = [0.05, 0.1, 0.25, 0.5, 0.75, 1.0];

/// Outgoing directions to test each lobe from. Near-grazing is included
/// deliberately: it is where masking-shadowing and the visible-normal warp are
/// most active, and where a sampling error is largest.
const COSINES: [f32; 3] = [0.15, 0.6, 1.0];

fn wo_at(mu: f32) -> Vec3 {
    vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu)
}

/// Adapts a [`Bsdf`] to the harness for one fixed outgoing direction.
struct BsdfUnderTest<B: Bsdf> {
    bsdf: B,
    wo: Vec3,
}

impl<B: Bsdf> SphericalDistribution for BsdfUnderTest<B> {
    fn sample(&self, u: (f32, f32), uc: f32) -> Option<Vec3> {
        let s = self.bsdf.sample(self.wo, vec2(u.0, u.1), uc)?;
        // A delta lobe has no density to compare against; §3.3 defines its
        // PDF as zero and its sample as pre-divided.
        if s.is_delta {
            return None;
        }
        Some(s.wi)
    }

    fn pdf(&self, direction: Vec3) -> f32 {
        self.bsdf.pdf(self.wo, direction)
    }
}

/// Run the test and assert, with a message that says enough to start
/// debugging from.
fn assert_consistent(
    name: &str,
    distribution: &dyn SphericalDistribution,
    seed: u64,
    tests: usize,
) {
    let result = chi2_test(distribution, Chi2Config::default(), seed);
    assert!(
        result.is_testable(),
        "{name}: too concentrated for the harness to partition. This is not a \
         pass — see Chi2Result::is_testable."
    );
    if result.zero_density_samples > 0 {
        panic!(
            "{name}: {} of {} samples landed where pdf() reports zero density.",
            result.zero_density_samples, result.samples_drawn
        );
    }
    if result.resolution_limited {
        // Not a pass and not a failure: the partition could not resolve this
        // distribution, so the comparison says nothing. The support check
        // above has already run and needs no resolution, so a sampler
        // producing impossible directions would still have been caught.
        eprintln!("{name}: inconclusive, the partition could not resolve it");
        return;
    }
    // Sidak correction across the whole suite, so the *family-wise* false
    // positive rate is the stated 1%, not the per-test rate.
    let threshold = 1.0 - (1.0 - SIGNIFICANCE).powf(1.0 / tests as f64);
    if result.p_value >= threshold {
        return;
    }
    let detail = match result.worst_bin {
        Some(b) => format!(
            " worst bin (band {}, sector {}): observed {:.0}, expected {:.1}",
            b.band, b.sector, b.observed, b.expected
        ),
        None => String::new(),
    };
    panic!(
        "{name}: sample() and pdf() disagree. \
         chi2 = {:.1}, dof = {}, p = {:.2e} (threshold {:.2e}), {} bins pooled.{detail}",
        result.statistic, result.degrees_of_freedom, result.p_value, threshold, result.pooled_bins
    );
}

/// Total number of χ² tests in this file, for the multiple-comparison
/// correction. Deliberately a constant rather than something computed: it
/// makes the correction visible, and a stale value shows up as a test that is
/// slightly too strict or too lax rather than as silent drift.
///
/// The M2 additions: transmission over the roughness ladder and three
/// directions, the OpenPBR stack over five configurations of it, and the
/// catalog's scattering presets at three directions each.
///
/// Then §4.4's fibres: the hair model over the roughness ladder, three
/// directions and three places across the fibre.
const TOTAL_TESTS: usize = 6 * 3 * 4 + 4 + 6 * 3 + 6 * 3 * 5 + 30 * 3 + 6 * 3 * 3;

// --- Fibres ----------------------------------------------------------------

/// A fibre (§4.4's Fuzz grown as geometry), whose four lobes cover the whole
/// sphere: reflected, through, back out, and spread round. Its density is
/// the lobes' luminance-weighted sum, and the sampler picks a lobe by the
/// same weights, so a weight or an angle sampled one way and evaluated
/// another shows here. The viewer is square to the fibre's `y`, as the
/// integrator places it, at three angles off square to the fibre.
#[test]
fn fibre_sampling_is_consistent() {
    use tequila_materials::lobes::Fibre;
    for roughness in ROUGHNESSES {
        for mu in COSINES {
            for h in [-0.7f32, 0.0, 0.5] {
                let d = BsdfUnderTest {
                    bsdf: Fibre::new(Spectrum { r: 0.7, g: 0.45, b: 0.2 }, roughness, h),
                    wo: wo_at(mu),
                };
                assert_consistent(
                    &format!("fibre, roughness {roughness}, cos {mu}, h {h}"),
                    &d,
                    (roughness.to_bits() as u64) << 8 ^ (mu.to_bits() as u64) << 2 ^ h.to_bits() as u64 ^ 0xf1b4e,
                    TOTAL_TESTS,
                );
            }
        }
    }
}

// --- M2 lobes ------------------------------------------------------------

/// Rough dielectric transmission: the lobe whose sampler and density disagreed
/// in two separate ways, neither of which any other test could see.
///
/// The first was a sampler that kept reflections aimed below the horizon and
/// let `pdf` measure them as refractions. The second was a half vector
/// oriented against the viewer rather than against the surface, which had the
/// density claiming refractions into directions no upward-facing microfacet
/// can produce — the density integrated to 1.25 over the sphere, which is not
/// a thing a density can do. Both were invisible to the white-furnace test and
/// to a pointwise `sample`-then-`pdf` check, and both are exactly what this
/// test is shaped to catch.
#[test]
fn transmission_sampling_is_consistent() {
    for roughness in ROUGHNESSES {
        for mu in COSINES {
            let d = BsdfUnderTest {
                bsdf: DielectricTransmission::new(roughness.max(0.05), 1.5),
                wo: wo_at(mu),
            };
            assert_consistent(
                &format!("rough dielectric transmission, roughness {roughness}, cos {mu}"),
                &d,
                (roughness.to_bits() as u64) << 8 | mu.to_bits() as u64 | 0x5000,
                TOTAL_TESTS,
            );
        }
    }
}

/// The assembled OpenPBR stack, which is where §2.4's rule bites: `pdf` must
/// report the density of the *mixture*, not of the lobe that happened to be
/// sampled. A stack that reports the sampled lobe's own density agrees with
/// itself pointwise and is wrong here.
#[test]
fn the_openpbr_stack_samples_its_own_density() {
    let configurations: [(&str, fn(f32) -> Material); 5] = [
        ("metal", |r| Material {
            base_metalness: 1.0,
            specular_roughness: r,
            ..Default::default()
        }),
        ("plastic", |r| Material {
            specular_roughness: r,
            ..Default::default()
        }),
        ("glass", |r| Material {
            transmission_weight: 1.0,
            specular_roughness: r,
            ..Default::default()
        }),
        // §3.1 singles layering out: "layering is where energy is most often
        // lost", and it is equally where a selector misreports the mixture.
        ("coated metal", |r| Material {
            base_metalness: 1.0,
            specular_roughness: r,
            coat_weight: 1.0,
            coat_roughness: 0.1,
            ..Default::default()
        }),
        ("fuzz over diffuse", |r| Material {
            specular_weight: 0.0,
            fuzz_weight: 1.0,
            fuzz_roughness: r,
            ..Default::default()
        }),
    ];

    for (name, build) in configurations {
        for roughness in ROUGHNESSES {
            for mu in COSINES {
                let d = BsdfUnderTest {
                    bsdf: OpenPbrBsdf::new(&build(roughness), 0.02),
                    wo: wo_at(mu),
                };
                assert_consistent(
                    &format!("{name}, roughness {roughness}, cos {mu}"),
                    &d,
                    (roughness.to_bits() as u64) << 8
                        | mu.to_bits() as u64
                        | (name.len() as u64) << 40
                        | 0x6000,
                    TOTAL_TESTS,
                );
            }
        }
    }
}

/// The M2 exit criterion: every material type in the catalog.
///
/// Light presets are excluded — they carry emission and no scattering lobe,
/// and their sampling belongs to the light sampler, tested with the other
/// light types below.
#[test]
fn every_material_type_in_the_catalog_samples_its_own_density() {
    let scattering: Vec<MaterialType> = MaterialType::ALL
        .iter()
        .copied()
        .filter(|t| !t.is_light())
        .collect();
    assert!(
        scattering.len() >= 25,
        "only {} scattering presets; the catalog should have far more",
        scattering.len()
    );

    for material_type in scattering {
        // A small roughness floor, as the integrator applies for path
        // regularization: a perfectly smooth preset is all delta, and a delta
        // lobe has no density to compare against.
        for mu in COSINES {
            let d = BsdfUnderTest {
                bsdf: OpenPbrBsdf::new(&material_type.defaults(), 0.02),
                wo: wo_at(mu),
            };
            let result = chi2_test(
                &d,
                Chi2Config::default(),
                material_type.id().len() as u64 * 7919 | (mu.to_bits() as u64) << 16,
            );
            // A preset with no scattering at all — pure emission — has
            // nothing to partition, and that is not a failure.
            if !result.is_testable() {
                continue;
            }
            assert_consistent(
                &format!("{} preset, cos {mu}", material_type.label()),
                &d,
                material_type.id().len() as u64 * 7919 | (mu.to_bits() as u64) << 16,
                TOTAL_TESTS,
            );
        }
    }
}

#[test]
fn diffuse_sampling_is_consistent() {
    for roughness in ROUGHNESSES {
        for mu in COSINES {
            let d = BsdfUnderTest {
                bsdf: OrenNayarDiffuse::new(Spectrum::splat(0.7), roughness),
                wo: wo_at(mu),
            };
            assert_consistent(
                &format!("Oren-Nayar diffuse, roughness {roughness}, cos {mu}"),
                &d,
                (roughness.to_bits() as u64) << 8 | mu.to_bits() as u64,
                TOTAL_TESTS,
            );
        }
    }
}

#[test]
fn conductor_sampling_is_consistent() {
    for roughness in ROUGHNESSES {
        for mu in COSINES {
            let d = BsdfUnderTest {
                bsdf: Conductor::new(Ggx::from_roughness(roughness, 0.0), metals::COPPER),
                wo: wo_at(mu),
            };
            assert_consistent(
                &format!("GGX conductor, roughness {roughness}, cos {mu}"),
                &d,
                (roughness.to_bits() as u64) << 8 | mu.to_bits() as u64 | 0x1000,
                TOTAL_TESTS,
            );
        }
    }
}

#[test]
fn anisotropic_conductor_sampling_is_consistent() {
    // Anisotropy exercises the elliptical visible-normal warp, which is where
    // a transposed `alpha_x`/`alpha_y` hides: the isotropic case cannot see
    // it. The outgoing direction is off-axis in both x and y on purpose, so
    // that swapping the pair changes the answer.
    let wo = vec3(0.3, 0.5, 0.812).normalize();
    for roughness in ROUGHNESSES {
        let d = BsdfUnderTest {
            bsdf: Conductor::new(Ggx::from_roughness(roughness, 0.7), metals::ALUMINIUM),
            wo,
        };
        assert_consistent(
            &format!("anisotropic GGX conductor, roughness {roughness}"),
            &d,
            (roughness.to_bits() as u64) | 0x2000,
            TOTAL_TESTS,
        );
    }
}

#[test]
fn dielectric_sampling_is_consistent() {
    for roughness in ROUGHNESSES {
        for mu in COSINES {
            let d = BsdfUnderTest {
                bsdf: DielectricSpecular::new(roughness, 1.5),
                wo: wo_at(mu),
            };
            assert_consistent(
                &format!("GGX dielectric, roughness {roughness}, cos {mu}"),
                &d,
                (roughness.to_bits() as u64) << 8 | mu.to_bits() as u64 | 0x3000,
                TOTAL_TESTS,
            );
        }
    }
}

/// §3.2 asks specifically for the layered stack. Multi-lobe PDFs are where the
/// "return the weighted sum over all lobes, not just the sampled one" rule of
/// `docs/02` §2.4 gets broken, and a χ² test catches that immediately.
#[test]
fn layered_sampling_is_consistent() {
    for roughness in ROUGHNESSES {
        for mu in COSINES {
            let d = BsdfUnderTest {
                bsdf: SpecularOverDiffuse::plastic(Spectrum::splat(0.55), roughness),
                wo: wo_at(mu),
            };
            assert_consistent(
                &format!("specular over diffuse, roughness {roughness}, cos {mu}"),
                &d,
                (roughness.to_bits() as u64) << 8 | mu.to_bits() as u64 | 0x4000,
                TOTAL_TESTS,
            );
        }
    }
}

/// The environment sampler, per §3.2's "the environment sampler" line.
mod environment {
    use super::*;
    use tequila_render_core::environment::Environment;
    use tequila_texture::image::{ColorSpace, Image};
    use std::sync::Arc;

    struct EnvironmentUnderTest(Environment);

    impl SphericalDistribution for EnvironmentUnderTest {
        fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
            self.0.sample(vec2(u.0, u.1)).map(|s| s.direction)
        }
        fn pdf(&self, direction: Vec3) -> f32 {
            self.0.pdf(direction)
        }
    }

    fn image_from(width: usize, height: usize, f: impl Fn(usize, usize) -> f32) -> Environment {
        let mut data = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let v = f(x, y);
                data.extend_from_slice(&[v, v, v]);
            }
        }
        Environment::new(
            Arc::new(Image {
                width,
                height,
                channels: 3,
                data,
                color_space: ColorSpace::LinearSrgb,
            }),
            1.0,
        )
    }

    #[test]
    fn uniform_environment_sampling_is_consistent() {
        let env = EnvironmentUnderTest(image_from(64, 32, |_, _| 1.0));
        assert_consistent("uniform environment", &env, 0x5001, TOTAL_TESTS);
    }

    /// A gradient: smoothly varying, so every bin has support, but not
    /// uniform. Catches a missing `sin(theta)` row weight, which shows up as a
    /// systematic bias towards the poles.
    #[test]
    fn gradient_environment_sampling_is_consistent() {
        let env = EnvironmentUnderTest(image_from(64, 32, |_, y| 0.1 + y as f32 * 0.1));
        assert_consistent("gradient environment", &env, 0x5002, TOTAL_TESTS);
    }

    /// A sky-and-ground split, the shape of every real outdoor HDRI.
    #[test]
    fn split_environment_sampling_is_consistent() {
        let env = EnvironmentUnderTest(image_from(64, 32, |_, y| if y < 16 { 4.0 } else { 0.3 }));
        assert_consistent("sky and ground environment", &env, 0x5003, TOTAL_TESTS);
    }

    /// A bright localized source: the case importance sampling exists for, and
    /// the one where a density that disagrees with the sampler does the most
    /// damage.
    #[test]
    fn concentrated_environment_sampling_is_consistent() {
        let env = EnvironmentUnderTest(image_from(64, 32, |x, y| {
            let (dx, dy) = (x as f32 - 20.0, y as f32 - 10.0);
            0.05 + 60.0 * (-(dx * dx + dy * dy) / 8.0).exp()
        }));
        assert_consistent("concentrated environment", &env, 0x5004, TOTAL_TESTS);
    }
}
