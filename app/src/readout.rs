//! The statistics overlay's speed and memory (`docs/04` §2.1): how many
//! frames the window drew in the last second, the slowest of them, and how
//! much memory Tequila holds, as the computer's own monitor counts it.
//!
//! Read from the system directly, as `tequila-cli`'s self-test reads its
//! facts: `proc_pid_rusage` on a Mac, `K32GetProcessMemoryInfo` on Windows,
//! `/proc` on Linux. No crate for three calls.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How often the memory is read again: often enough to watch it move, and
/// not a system call every frame.
const MEMORY_EVERY: Duration = Duration::from_millis(500);

/// The overlay's running measurements, kept by the window.
#[derive(Default)]
pub struct Readout {
    /// When each of the frames of the last second started.
    frames: VecDeque<Instant>,
    /// When the view last moved: dragged, scrolled, or the camera changed.
    moved: Option<Instant>,
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

    /// The view moved at `now`.
    pub fn moved(&mut self, now: Instant) {
        self.moved = Some(now);
    }

    /// The frames drawn in the last second, while the view has moved in it
    /// - the speed an orbit or a drag feels - or `None` while it is still.
    /// A still window draws only when something asks, a few times a second,
    /// and "7 fps" then read as a slow computer when it was an idle one.
    pub fn frames_per_second(&self) -> Option<usize> {
        let last = *self.frames.back()?;
        self.moved.filter(|&at| last.saturating_duration_since(at) <= Duration::from_secs(1)).map(|_| self.frames.len())
    }

    /// The longest gap between two frames in the last second: a hitch shows
    /// here when the frame rate hides it.
    pub fn slowest_frame(&self) -> Option<Duration> {
        self.frames.iter().zip(self.frames.iter().skip(1)).map(|(a, b)| b.duration_since(*a)).max()
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

    /// The frame rate is the frames of the last second; the slowest frame
    /// is the longest gap between two of them.
    #[test]
    fn the_frame_rate_counts_the_last_second() {
        let mut readout = Readout::default();
        let start = Instant::now();
        for i in 0..120u64 {
            readout.frame(start + Duration::from_millis(i * 10));
        }
        assert_eq!(readout.frames_per_second(), None, "nothing has moved: idle");
        readout.moved(start + Duration::from_millis(1000));
        assert_eq!(readout.frames_per_second(), Some(101), "the frames of the last second only");
        readout.frame(start + Duration::from_millis(1290 + 100));
        assert_eq!(readout.slowest_frame(), Some(Duration::from_millis(200)), "the hitch shows");
        readout.frame(start + Duration::from_millis(2100));
        assert_eq!(readout.frames_per_second(), None, "still again after a second");
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
