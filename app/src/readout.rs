//! The statistics overlay's speed and memory (`docs/04` §2.1): how fast the
//! window drew while the view last moved, the slowest of those frames, and
//! how much memory Tequila holds, as the computer's own monitor counts it.
//!
//! Read from the system directly, as `tequila-cli`'s self-test reads its
//! facts: `proc_pid_rusage` on a Mac, `K32GetProcessMemoryInfo` on Windows,
//! `/proc` on Linux. No crate for three calls.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How often the memory is read again: often enough to watch it move, and
/// not a system call every frame.
const MEMORY_EVERY: Duration = Duration::from_millis(500);

/// A move that follows the last by more than this is a new one.
const PAUSE: Duration = Duration::from_millis(250);
/// A frame starts this long, at most, before its move is reported.
const SLACK: Duration = Duration::from_millis(5);

/// The overlay's running measurements, kept by the window.
#[derive(Default)]
pub struct Readout {
    /// When each of the frames of the last second started.
    frames: VecDeque<Instant>,
    /// When the view last moved: dragged, scrolled, or the camera changed.
    moved: Option<Instant>,
    /// When the move it is part of began: the first after a pause.
    moving_since: Option<Instant>,
    memory: Option<u64>,
    memory_read: Option<Instant>,
    installed: Option<Option<u64>>,
    processor: Option<String>,
}

impl Readout {
    /// A frame started at `now`.
    pub fn frame(&mut self, now: Instant) {
        self.frames.push_back(now);
        while self.frames.front().is_some_and(|&t| now.duration_since(t) > Duration::from_secs(1)) {
            self.frames.pop_front();
        }
    }

    /// The view moved at `now`: called at every frame of a move.
    pub fn moved(&mut self, now: Instant) {
        if self.moved.is_none_or(|at| now.saturating_duration_since(at) > PAUSE) {
            self.moving_since = Some(now);
        }
        self.moved = Some(now);
    }

    /// The frames of the move in hand, or of the one that ended within the
    /// last second: those from when it began, or a second before it last
    /// moved, to when it last moved. The still frames either side are not
    /// its frames: a still window draws a few times a second, and counted
    /// with a move's they made the first moments of every orbit read
    /// "13 fps, slowest 185 ms" on a view drawing 120 a second.
    fn moving_frames(&self) -> Vec<Instant> {
        let (Some(&last), Some(moved), Some(since)) = (self.frames.back(), self.moved, self.moving_since) else {
            return Vec::new();
        };
        if last.saturating_duration_since(moved) > Duration::from_secs(1) {
            return Vec::new();
        }
        let from = since.max(moved.checked_sub(Duration::from_secs(1)).unwrap_or(since));
        // The frame a move is reported in started just before the report.
        let to = moved + SLACK;
        self.frames.iter().copied().filter(|&t| t + SLACK >= from && t <= to).collect()
    }

    /// How many frames a second the window drew while the view moved - the
    /// speed an orbit or a drag feels - or `None` while it is still, or
    /// before a move has frames enough to tell.
    pub fn frames_per_second(&self) -> Option<usize> {
        let frames = self.moving_frames();
        let span = frames.last()?.saturating_duration_since(*frames.first()?).as_secs_f64();
        (frames.len() >= 3 && span > 0.0).then(|| ((frames.len() - 1) as f64 / span).round() as usize)
    }

    /// The longest gap between two frames of the move: a hitch shows here
    /// when the frame rate hides it.
    pub fn slowest_frame(&self) -> Option<Duration> {
        let frames = self.moving_frames();
        frames.iter().zip(frames.iter().skip(1)).map(|(a, b)| b.duration_since(*a)).max()
    }

    /// Tequila's memory in bytes, read again every half second.
    pub fn memory(&mut self, now: Instant) -> Option<u64> {
        if self.memory_read.is_none_or(|at| now.duration_since(at) >= MEMORY_EVERY) {
            self.memory = process_memory();
            self.memory_read = Some(now);
        }
        self.memory
    }

    /// The computer's memory in bytes, read once.
    pub fn installed(&mut self) -> Option<u64> {
        *self.installed.get_or_insert_with(installed_memory)
    }

    /// The processor's name ("Apple M3 Max"), read once.
    pub fn processor(&mut self) -> &str {
        self.processor.get_or_insert_with(|| processor_name().unwrap_or_else(|| "processor".to_string()))
    }
}

/// `bytes` for the overlay: "640 MB", "6.5 GB".
pub fn size(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let gb = bytes as f64 / GB;
    if gb < 1.0 {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if gb < 10.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{gb:.0} GB")
    }
}

/// A graphics card's name from OpenGL's renderer string, without the bus
/// and instruction sets NVIDIA's driver adds: "NVIDIA GeForce RTX
/// 2060/PCIe/SSE2" is "NVIDIA GeForce RTX 2060".
pub fn card_name(renderer: &str) -> String {
    renderer.split('/').next().unwrap_or(renderer).trim().to_string()
}

// ---- The system's own counts ------------------------------------------------

/// The memory Tequila holds. On a Mac its footprint, which is what
/// Activity Monitor's Memory column shows: what is in use, compressed or
/// swapped out alike. On Windows its private bytes (Task Manager's Commit
/// size); on Linux its resident set.
#[cfg(target_os = "macos")]
pub fn process_memory() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct RusageInfoV0 {
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
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfoV0) -> i32;
    }
    let mut info = RusageInfoV0::default();
    // SAFETY: flavour 0 (`RUSAGE_INFO_V0`) fills a `rusage_info_v0`, which
    // this is laid out as, for this process.
    let failed = unsafe { proc_pid_rusage(std::process::id() as i32, 0, &mut info) };
    (failed == 0).then_some(info.phys_footprint)
}

#[cfg(windows)]
pub fn process_memory() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCountersEx, size: u32) -> i32;
    }
    let mut counters = ProcessMemoryCountersEx { cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32, ..Default::default() };
    // SAFETY: a PROCESS_MEMORY_COUNTERS_EX with its size set, for this
    // process's pseudo-handle, which needs no closing.
    let size = counters.cb;
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (ok != 0).then_some(counters.private_usage as u64)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn process_memory() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    kilobytes(&status, "VmRSS:")
}

#[cfg(target_os = "macos")]
fn installed_memory() -> Option<u64> {
    let mut bytes = 0u64;
    sysctl(c"hw.memsize", &mut bytes).then_some(bytes)
}

#[cfg(windows)]
fn installed_memory() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx { length: std::mem::size_of::<MemoryStatusEx>() as u32, ..Default::default() };
    // SAFETY: a MEMORYSTATUSEX with its length set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status.total_physical)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn installed_memory() -> Option<u64> {
    kilobytes(&std::fs::read_to_string("/proc/meminfo").ok()?, "MemTotal:")
}

#[cfg(not(any(target_os = "macos", windows)))]
fn kilobytes(text: &str, field: &str) -> Option<u64> {
    let kb: u64 = text.lines().find_map(|l| l.strip_prefix(field))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some(kb * 1024)
}

#[cfg(target_os = "macos")]
fn processor_name() -> Option<String> {
    let mut buffer = [0u8; 128];
    let mut length = buffer.len();
    // SAFETY: a buffer and its length, which sysctl fills and shortens.
    let failed = unsafe { sysctlbyname(c"machdep.cpu.brand_string".as_ptr(), buffer.as_mut_ptr().cast(), &mut length, std::ptr::null(), 0) };
    if failed != 0 {
        return None;
    }
    let text = String::from_utf8_lossy(&buffer[..length]).trim_end_matches('\0').trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(target_os = "macos")]
extern "C" {
    fn sysctlbyname(name: *const std::ffi::c_char, old: *mut std::ffi::c_void, old_len: *mut usize, new: *const std::ffi::c_void, new_len: usize) -> i32;
}

#[cfg(target_os = "macos")]
fn sysctl(name: &std::ffi::CStr, value: &mut u64) -> bool {
    let mut length = std::mem::size_of::<u64>();
    // SAFETY: a u64 and its size, for a value sysctl gives as one.
    unsafe { sysctlbyname(name.as_ptr(), (value as *mut u64).cast(), &mut length, std::ptr::null(), 0) == 0 }
}

#[cfg(windows)]
fn processor_name() -> Option<String> {
    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut std::ffi::c_void, size: *mut u32) -> i32;
    }
    // HKEY_LOCAL_MACHINE, sign-extended as Windows defines it; RRF_RT_REG_SZ.
    const LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const STRING: u32 = 0x0000_0002;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (subkey, value) = (wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"), wide("ProcessorNameString"));
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: a string value read into a buffer of the size given.
    let status = unsafe {
        RegGetValueW(LOCAL_MACHINE, subkey.as_ptr(), value.as_ptr(), STRING, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
    };
    if status != 0 {
        return None;
    }
    let length = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..length]).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn processor_name() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let name = info.lines().find_map(|l| l.strip_prefix("model name"))?.trim_start_matches([' ', '\t', ':']).trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame rate is that of the frames drawn while the view moved,
    /// and the slowest frame the longest gap between two of them: the
    /// still frames before a move and after it are not counted.
    #[test]
    fn the_frame_rate_is_the_moving_frames_alone() {
        let mut readout = Readout::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // Still: a frame every 180 ms.
        for i in 0..6u64 {
            readout.frame(at(i * 180));
        }
        assert_eq!(readout.frames_per_second(), None, "nothing has moved: idle");
        // An orbit begins: a frame every 10 ms, each reporting the move.
        let mut now = 1000;
        for _ in 0..12 {
            readout.frame(at(now));
            readout.moved(at(now + 1));
            now += 10;
        }
        // A tenth of a second in. It read 17 fps, slowest 180 ms.
        assert_eq!(readout.frames_per_second(), Some(100), "the idle frames before it were counted");
        assert_eq!(readout.slowest_frame(), Some(Duration::from_millis(10)));
        // On for two seconds, with one hitch.
        for i in 0..200 {
            now += if i == 150 { 60 } else { 10 };
            readout.frame(at(now));
            readout.moved(at(now + 1));
        }
        assert_eq!(readout.frames_per_second(), Some(95), "the last second of the move");
        assert_eq!(readout.slowest_frame(), Some(Duration::from_millis(60)), "the hitch shows");
        // Stopped: the reading stays the move's while still frames follow,
        // and goes after a second.
        readout.frame(at(now + 300));
        readout.frame(at(now + 600));
        assert_eq!(readout.frames_per_second(), Some(100), "the still frames after the move were counted with its last ones");
        readout.frame(at(now + 1200));
        assert_eq!(readout.frames_per_second(), None, "still again after a second");
        // A second move is measured from its own start.
        let mut now = now + 2000;
        for _ in 0..5 {
            readout.frame(at(now));
            readout.moved(at(now + 1));
            now += 20;
        }
        assert_eq!(readout.frames_per_second(), Some(50));
    }

    /// Tequila's own memory and the computer's are read, and the first is
    /// the smaller.
    #[test]
    fn memory_is_read_from_the_system() {
        let mut readout = Readout::default();
        let held = readout.memory(Instant::now()).expect("this process's memory");
        let installed = readout.installed().expect("the computer's memory");
        assert!(held > 1024 * 1024 && held < installed, "{held} of {installed}");
        assert!(!readout.processor().is_empty());
    }

    #[test]
    fn sizes_and_card_names_read_plainly() {
        assert_eq!(size(640 * 1024 * 1024), "640 MB");
        assert_eq!(size(6 * 1024 * 1024 * 1024 + 512 * 1024 * 1024), "6.5 GB");
        assert_eq!(size(48 * 1024 * 1024 * 1024), "48 GB");
        assert_eq!(card_name("NVIDIA GeForce RTX 2060/PCIe/SSE2"), "NVIDIA GeForce RTX 2060");
        assert_eq!(card_name("Apple M3 Max"), "Apple M3 Max");
    }
}
