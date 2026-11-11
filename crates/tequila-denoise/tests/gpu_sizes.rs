//! The graphics card's denoiser keeps its filters for the sizes a view goes
//! back and forth between, rather than committing them again at each:
//! committing took 110 to 330 ms on an M3 Max at a 1448x814 view, against
//! 72 to 84 ms to denoise a frame. Skipped without a graphics card's
//! denoiser.

use tequila_denoise::{DenoiseSettings, GpuDenoiser, Quality};

#[test]
fn a_views_sizes_stay_committed() {
    let gpu = match GpuDenoiser::shared() {
        Ok(gpu) => gpu,
        Err(why) => {
            eprintln!("skipped: {why}");
            return;
        }
    };
    let denoise = |width: usize, height: usize, quality: Quality| {
        let n = width * height * 3;
        let image = gpu.denoise(width, height, &vec![0.5; n], &vec![0.5; n], &vec![0.0; n], DenoiseSettings { quality, strength: 1.0, prefilter_guides: true }).expect("denoised");
        assert_eq!(image.data.len(), n);
    };
    gpu.release();
    denoise(400, 300, Quality::Balanced);
    denoise(100, 75, Quality::Fast);
    denoise(200, 150, Quality::Fast);
    assert_eq!(gpu.committed(), 3, "the full size, its quarter and its half");
    denoise(400, 300, Quality::Balanced);
    assert_eq!(gpu.committed(), 3, "the full size again, from what was kept");
    denoise(380, 300, Quality::Balanced);
    assert_eq!(gpu.committed(), 3, "a new size near the full one takes its place");
    denoise(380, 300, Quality::Fast);
    assert_eq!(gpu.committed(), 3, "a size at another quality is made again, not kept twice");
    gpu.release();
    assert_eq!(gpu.committed(), 0);
}
