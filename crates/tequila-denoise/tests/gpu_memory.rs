//! What the graphics card's denoiser keeps of the computer's memory on a
//! Mac, where the graphics card has none of its own. A test alone in its own
//! process, because the footprint read here is the whole process's: the
//! crate's other tests, run beside it, would be counted with it.

#[cfg(target_os = "macos")]
mod mac {
    use std::time::{Duration, Instant};
    use tequila_denoise::{DenoiseSettings, GpuDenoiser, Quality};

    /// `rusage_info_v0`, from `<sys/resource.h>`.
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfo {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        proc_start_abstime: u64,
        proc_exit_abstime: u64,
    }

    extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfo) -> i32;
    }

    /// The process's memory in megabytes, as Activity Monitor counts it.
    fn footprint_mb() -> f64 {
        let mut info = RusageInfo::default();
        // SAFETY: flavor 0 (`RUSAGE_INFO_V0`) fills a `rusage_info_v0`,
        // which this is, for this process.
        let failed = unsafe { proc_pid_rusage(std::process::id() as i32, 0, &mut info) };
        assert_eq!(failed, 0, "proc_pid_rusage failed");
        info.phys_footprint as f64 / (1024.0 * 1024.0)
    }

    /// The Rendered view of a large window, 2560x1440 pixels on a Retina
    /// display: the frames that settle after a move (Fast, the guides
    /// filtered), then the converging ones (Balanced, the guides as
    /// rendered). Left to itself, OIDN's Metal device kept 7.9 GB of the
    /// computer's memory for them, for as long as the view showed; held to
    /// its limits, 1.9 GB. Let go, nearly all of it comes back. Skipped
    /// without a Metal device.
    #[test]
    fn the_viewport_denoiser_holds_little_and_gives_it_back() {
        let gpu = match GpuDenoiser::shared() {
            Ok(gpu) => gpu,
            Err(why) => {
                eprintln!("skipped: {why}");
                return;
            }
        };
        let (width, height) = (2560, 1440);
        let (mut color, mut albedo, mut normal) = (Vec::new(), Vec::new(), Vec::new());
        for y in 0..height {
            for x in 0..width {
                let grain = ((x * 7 + y * 13) % 17) as f32 / 17.0;
                let a = if (x / 2 + y / 2) % 2 == 0 { 0.2 } else { 0.7 };
                color.extend_from_slice(&[a * (0.2 + 1.6 * grain); 3]);
                albedo.extend_from_slice(&[a * (0.9 + 0.2 * grain); 3]);
                normal.extend_from_slice(&[0.1 * (grain - 0.5), 0.0, 1.0]);
            }
        }
        let before = footprint_mb();
        for (quality, prefilter_guides) in [(Quality::Fast, true), (Quality::Balanced, false)] {
            let settings = DenoiseSettings { quality, strength: 1.0, prefilter_guides };
            for _ in 0..2 {
                let out = gpu.denoise(width, height, &color, &albedo, &normal, settings).expect("denoised");
                assert_eq!(out.data.len(), color.len());
            }
        }
        let held = footprint_mb() - before;
        assert!(held < 3500.0, "the viewport's denoiser held {held:.0} MB for a {width}x{height} view");

        gpu.release();
        // Metal frees it a moment later, not at once.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let kept = footprint_mb() - before;
            if kept < 300.0 {
                break;
            }
            assert!(Instant::now() < deadline, "{kept:.0} MB still held 10 s after the denoiser let go (held {held:.0} MB)");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
