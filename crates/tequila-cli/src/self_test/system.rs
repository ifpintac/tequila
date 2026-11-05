//! What the report says about the computer: the operating system and its
//! version, the processor and its cores, the memory, each graphics card with
//! its maker, memory and driver, and which of Cycles' devices Tequila finds.
//!
//! Nothing that names the person or the machine. The sources read here hold
//! no user or computer name, path, serial number or network address: the
//! system's own version, the processor's name, DXGI's list of graphics
//! adapters and the display drivers' registry entries on Windows, `sysctl`
//! and some of `system_profiler`'s fields on a Mac (only the cards' own: it
//! also lists the screens, with their serial numbers), `/proc` and `/sys` on
//! Linux. What comes from elsewhere - Cycles' messages, which name the
//! folders it looked in - is scrubbed before it is reported
//! ([`super::scrub`]).


/// The computer, as the report describes it.
#[derive(Clone, Debug)]
pub struct Facts {
    /// "Windows 11 24H2 (10.0.26200.7019)", "macOS 15.1 (24B83)".
    pub os: String,
    pub arch: String,
    pub cpu: String,
    /// "8 cores", or on Apple silicon "4 performance + 4 efficiency cores".
    pub cores: Option<String>,
    pub threads: usize,
    pub memory: Option<u64>,
    pub gpus: Vec<Gpu>,
    /// Cycles' devices, or why there is no Cycles.
    pub cycles: Result<Vec<Device>, String>,
}

impl Default for Facts {
    fn default() -> Facts {
        Facts { os: String::new(), arch: String::new(), cpu: String::new(), cores: None, threads: 1, memory: None, gpus: Vec::new(), cycles: Ok(Vec::new()) }
    }
}

/// A graphics card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    /// Its own memory, in bytes; `None` where it shares the system's.
    pub memory: Option<u64>,
    pub driver: Option<String>,
}

/// A device Cycles can render on.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    /// CPU, CUDA, OPTIX, HIP, METAL, ONEAPI.
    pub kind: String,
    pub name: String,
    /// What a session is opened on to render on this device alone. Never
    /// reported: it is an internal name (bus numbers and the like).
    pub id: String,
}

/// Everything, looked up now. Cycles is loaded to list its devices, as a
/// render would load it - unless `cycles` is false, when listing them is
/// what stopped the test the last time.
pub fn gather_with(cycles: bool) -> Facts {
    let (cpu, cores) = platform::cpu();
    Facts {
        os: platform::os(),
        arch: std::env::consts::ARCH.to_string(),
        cpu: cpu.unwrap_or_else(|| "unknown processor".into()),
        cores,
        threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        memory: platform::memory(),
        gpus: platform::gpus(),
        cycles: match cycles {
            true => tequila_render_cycles::devices().map(|list| list.into_iter().map(|(kind, name, id)| Device { kind, name, id }).collect()),
            false => Err("not listed: listing Cycles' devices stopped the test".into()),
        },
    }
}

/// A graphics card maker, by its PCI vendor number.
pub fn vendor_name(id: u32) -> String {
    match id {
        0x10de => "NVIDIA".into(),
        0x1002 | 0x1022 => "AMD".into(),
        0x8086 => "Intel".into(),
        0x106b => "Apple".into(),
        0x5143 => "Qualcomm".into(),
        0x1414 => "Microsoft".into(),
        other => format!("vendor 0x{other:04x}"),
    }
}

/// The version NVIDIA's own software shows, from the Windows driver's:
/// 32.0.15.6636 is 566.36 - the last five digits of the last two parts.
pub fn nvidia_version(windows: &str) -> Option<String> {
    let parts: Vec<&str> = windows.split('.').collect();
    let [.., third, fourth] = parts[..] else { return None };
    let digits = format!("{}{:0>4}", third.trim(), fourth.trim());
    if digits.len() < 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let last = &digits[digits.len() - 5..];
    Some(format!("{}.{}", &last[..3], &last[3..]))
}

#[cfg(windows)]
mod platform {
    //! Windows: the version from the kernel, the processor and the display
    //! drivers from the registry, the graphics adapters from DXGI - present
    //! ones only, with their own memory and their driver's version.

    use super::{nvidia_version, vendor_name, Gpu};
    use std::ffi::c_void;

    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }

    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }

    /// SYSTEM_LOGICAL_PROCESSOR_INFORMATION: a mask, what it describes, and
    /// a union of sixteen bytes.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProcessorInfo {
        mask: usize,
        relationship: u32,
        rest: [u64; 2],
    }

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[repr(C)]
    struct AdapterDesc1 {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        sub_sys_id: u32,
        revision: u32,
        dedicated_video_memory: usize,
        dedicated_system_memory: usize,
        shared_system_memory: usize,
        luid_low: u32,
        luid_high: i32,
        flags: u32,
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn GetLogicalProcessorInformation(buffer: *mut ProcessorInfo, length: *mut u32) -> i32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(key: isize, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
    }

    #[link(name = "dxgi")]
    extern "system" {
        fn CreateDXGIFactory1(riid: *const Guid, factory: *mut *mut c_void) -> i32;
    }

    /// HKEY_LOCAL_MACHINE, sign-extended as Windows defines it.
    const LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const ANY_TYPE: u32 = 0x0000_ffff;
    const RELATION_PROCESSOR_CORE: u32 = 0;
    const ADAPTER_SOFTWARE: u32 = 2;
    const IID_FACTORY1: Guid = Guid { data1: 0x770a_ae78, data2: 0xf26f, data3: 0x4dba, data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87] };
    const IID_DEVICE: Guid = Guid { data1: 0x54ec_77fa, data2: 0x1377, data3: 0x44e6, data4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c] };
    /// The display adapters' class, where each driver keeps its entry.
    const DISPLAY_CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

    enum Stored {
        Text(String),
        Number(u64),
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn read(subkey: &str, value: &str) -> Option<Stored> {
        let (subkey, value) = (wide(subkey), wide(value));
        let (mut kind, mut size) = (0u32, 0u32);
        // SAFETY: a size query with no buffer, then a read into one that size.
        unsafe {
            if RegGetValueW(LOCAL_MACHINE, subkey.as_ptr(), value.as_ptr(), ANY_TYPE, &mut kind, std::ptr::null_mut(), &mut size) != 0 || size == 0 {
                return None;
            }
            let mut data = vec![0u8; size as usize + 2];
            let mut size = data.len() as u32;
            if RegGetValueW(LOCAL_MACHINE, subkey.as_ptr(), value.as_ptr(), ANY_TYPE, &mut kind, data.as_mut_ptr().cast(), &mut size) != 0 {
                return None;
            }
            data.truncate(size as usize);
            let number = |bytes: &[u8]| match bytes.len() {
                4 => Some(u32::from_le_bytes(bytes.try_into().ok()?) as u64),
                8 => Some(u64::from_le_bytes(bytes.try_into().ok()?)),
                _ => None,
            };
            match kind {
                // REG_SZ, REG_EXPAND_SZ, REG_MULTI_SZ (its first string).
                1 | 2 | 7 => {
                    let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
                    Some(Stored::Text(String::from_utf16_lossy(&units).trim().to_string()))
                }
                // REG_BINARY, REG_DWORD, REG_QWORD.
                3 | 4 | 11 => number(&data).map(Stored::Number),
                _ => None,
            }
        }
    }

    fn text(subkey: &str, value: &str) -> Option<String> {
        match read(subkey, value)? {
            Stored::Text(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    fn number(subkey: &str, value: &str) -> Option<u64> {
        match read(subkey, value)? {
            Stored::Number(n) => Some(n),
            _ => None,
        }
    }

    pub fn os() -> String {
        let mut info = OsVersionInfo { size: std::mem::size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, service_pack: [0; 128] };
        // SAFETY: the structure is the size it says.
        if unsafe { RtlGetVersion(&mut info) } != 0 {
            return "Windows".into();
        }
        let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        // The product name says Windows 10 on Windows 11 too, so the build
        // decides - except on a server, which names itself.
        let server = text(key, "ProductName").filter(|p| p.contains("Server"));
        let name = match &server {
            Some(product) => product.as_str(),
            None if info.major == 10 && info.build >= 22000 => "Windows 11",
            None if info.major == 10 => "Windows 10",
            None => "Windows",
        };
        // The release (24H2) and the update's revision, as winver shows them.
        let release = text(key, "DisplayVersion").map(|r| format!(" {r}")).unwrap_or_default();
        let revision = number(key, "UBR").map(|r| format!(".{r}")).unwrap_or_default();
        format!("{name}{release} ({}.{}.{}{revision})", info.major, info.minor, info.build)
    }

    pub fn cpu() -> (Option<String>, Option<String>) {
        let name = text(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString");
        let mut length = 0u32;
        // SAFETY: a size query, then a read into a buffer of that many bytes.
        let cores = unsafe {
            GetLogicalProcessorInformation(std::ptr::null_mut(), &mut length);
            let count = length as usize / std::mem::size_of::<ProcessorInfo>();
            let mut buffer = vec![ProcessorInfo { mask: 0, relationship: 0, rest: [0; 2] }; count];
            (count > 0 && GetLogicalProcessorInformation(buffer.as_mut_ptr(), &mut length) != 0)
                .then(|| buffer.iter().filter(|p| p.relationship == RELATION_PROCESSOR_CORE).count())
                .filter(|&n| n > 0)
        };
        (name, cores.map(|n| format!("{n} cores")))
    }

    pub fn memory() -> Option<u64> {
        // SAFETY: zeroed is a valid MEMORYSTATUSEX once its length is set.
        unsafe {
            let mut status: MemoryStatus = std::mem::zeroed();
            status.length = std::mem::size_of::<MemoryStatus>() as u32;
            (GlobalMemoryStatusEx(&mut status) != 0).then_some(status.total_physical)
        }
    }

    /// A COM method from `object`'s table, by its place in it.
    unsafe fn method<T: Copy>(object: *mut c_void, index: usize) -> T {
        let table = *(object as *const *const usize);
        std::mem::transmute_copy(&*table.add(index))
    }

    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type EnumAdapters1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type CheckInterfaceSupport = unsafe extern "system" fn(*mut c_void, *const Guid, *mut i64) -> i32;
    type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> i32;

    pub fn gpus() -> Vec<Gpu> {
        let mut out = Vec::new();
        // SAFETY: DXGI's documented calls, each object released once. The
        // places in the tables are IDXGIFactory1's EnumAdapters1 (12) and
        // IDXGIAdapter1's CheckInterfaceSupport (9) and GetDesc1 (10), after
        // IUnknown's three and IDXGIObject's four.
        unsafe {
            let mut factory = std::ptr::null_mut();
            if CreateDXGIFactory1(&IID_FACTORY1, &mut factory) < 0 || factory.is_null() {
                return out;
            }
            let enumerate: EnumAdapters1 = method(factory, 12);
            for index in 0..16 {
                let mut adapter = std::ptr::null_mut();
                if enumerate(factory, index, &mut adapter) < 0 || adapter.is_null() {
                    break;
                }
                let describe: GetDesc1 = method(adapter, 10);
                let mut desc: AdapterDesc1 = std::mem::zeroed();
                // Not the Basic Render Driver, nor a remote session's.
                if describe(adapter, &mut desc) >= 0 && desc.flags & ADAPTER_SOFTWARE == 0 && desc.vendor_id != 0x1414 {
                    let end = desc.description.iter().position(|&u| u == 0).unwrap_or(desc.description.len());
                    let name = String::from_utf16_lossy(&desc.description[..end]).trim().to_string();
                    // The user-mode driver's version, which is what
                    // CheckInterfaceSupport hands back for IDXGIDevice.
                    let check: CheckInterfaceSupport = method(adapter, 9);
                    let mut version = 0i64;
                    let windows = (check(adapter, &IID_DEVICE, &mut version) >= 0).then(|| {
                        let v = version as u64;
                        format!("{}.{}.{}.{}", v >> 48, (v >> 32) & 0xffff, (v >> 16) & 0xffff, v & 0xffff)
                    });
                    let driver = match (desc.vendor_id, windows) {
                        (0x10de, Some(windows)) => Some(match nvidia_version(&windows) {
                            Some(nvidia) => format!("{nvidia} ({windows})"),
                            None => windows,
                        }),
                        // AMD's own software numbers its releases (24.10.1),
                        // which is what people know theirs by.
                        (0x1002, Some(windows)) => Some(match radeon_software(&name) {
                            Some(adrenalin) => format!("Adrenalin {adrenalin} ({windows})"),
                            None => windows,
                        }),
                        (_, windows) => windows,
                    };
                    let memory = (desc.dedicated_video_memory > 0).then_some(desc.dedicated_video_memory as u64);
                    out.push(Gpu { name, vendor: vendor_name(desc.vendor_id), memory, driver });
                }
                let release: Release = method(adapter, 2);
                release(adapter);
            }
            let release: Release = method(factory, 2);
            release(factory);
        }
        out
    }

    /// The AMD software release an adapter's driver came with, from the
    /// display driver's entry of that name.
    fn radeon_software(name: &str) -> Option<String> {
        (0..32).map(|i| format!(r"{DISPLAY_CLASS}\{i:04}")).find_map(|key| {
            let described = text(&key, "DriverDesc")?;
            described.eq_ignore_ascii_case(name).then(|| text(&key, "RadeonSoftwareVersion")).flatten()
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    //! A Mac: `sysctl` for the processor and the memory, the system's
    //! version file, and `system_profiler` for the graphics - of which only
    //! the cards' own fields are read.

    use super::Gpu;
    use std::process::Command;

    fn sysctl(name: &str) -> Option<String> {
        let out = Command::new("/usr/sbin/sysctl").args(["-n", name]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    }

    pub fn os() -> String {
        let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").unwrap_or_default();
        let field = |key: &str| {
            plist
                .split(&format!("<key>{key}</key>"))
                .nth(1)
                .and_then(|rest| rest.split("<string>").nth(1))
                .and_then(|rest| rest.split("</string>").next())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        match (field("ProductVersion"), field("ProductBuildVersion")) {
            (Some(version), Some(build)) => format!("macOS {version} ({build})"),
            (Some(version), None) => format!("macOS {version}"),
            _ => "macOS".into(),
        }
    }

    pub fn cpu() -> (Option<String>, Option<String>) {
        let name = sysctl("machdep.cpu.brand_string");
        let count = |key: &str| sysctl(key).and_then(|s| s.parse::<usize>().ok());
        // Apple silicon has two kinds of core.
        let cores = match (count("hw.perflevel0.physicalcpu"), count("hw.perflevel1.physicalcpu")) {
            (Some(performance), Some(efficiency)) if efficiency > 0 => Some(format!("{performance} performance + {efficiency} efficiency cores")),
            _ => count("hw.physicalcpu").map(|n| format!("{n} cores")),
        };
        (name, cores)
    }

    pub fn memory() -> Option<u64> {
        sysctl("hw.memsize")?.parse().ok()
    }

    pub fn gpus() -> Vec<Gpu> {
        let Ok(out) = Command::new("/usr/sbin/system_profiler").args(["SPDisplaysDataType", "-json"]).output() else { return Vec::new() };
        let Ok(value) = tequila_format::json::parse(&out.stdout) else { return Vec::new() };
        let text = |item: &tequila_format::json::Value, key: &str| item.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        let mut gpus = Vec::new();
        for item in value.get("SPDisplaysDataType").map(|v| v.as_array()).unwrap_or(&[]) {
            let Some(model) = text(item, "sppci_model") else { continue };
            let vendor = text(item, "spdisplays_vendor").or_else(|| text(item, "sppci_vendor")).unwrap_or_default();
            // "sppci_vendor_Apple", "Intel", "AMD (0x1002)".
            let vendor = vendor.trim_start_matches("sppci_vendor_").split(" (").next().unwrap_or("").to_string();
            let cores = text(item, "sppci_cores").map(|c| format!(" ({c} cores)")).unwrap_or_default();
            // A card of its own lists its memory; Apple's graphics share
            // the system's.
            let memory = text(item, "spdisplays_vram").and_then(|v| parse_size(&v));
            let metal = text(item, "spdisplays_mtlgpufamilysupport").map(|m| m.trim_start_matches("spdisplays_").replace("metal", "Metal "));
            gpus.push(Gpu { name: format!("{model}{cores}"), vendor, memory, driver: metal });
        }
        gpus
    }

    /// "1536 MB", "8 GB".
    fn parse_size(text: &str) -> Option<u64> {
        let mut words = text.split_whitespace();
        let number: f64 = words.next()?.parse().ok()?;
        let unit = match words.next()?.to_ascii_uppercase().as_str() {
            "MB" => 1u64 << 20,
            "GB" => 1u64 << 30,
            _ => return None,
        };
        Some((number * unit as f64) as u64)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    //! Linux: `/etc/os-release`, `/proc` for the processor and memory, the
    //! DRM devices in `/sys` for the graphics cards, and `nvidia-smi` for an
    //! NVIDIA card's name, memory and driver.

    use super::{vendor_name, Gpu};

    fn read(path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    pub fn os() -> String {
        let release = read("/etc/os-release").and_then(|text| {
            text.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string())
        });
        let kernel = read("/proc/sys/kernel/osrelease").map(|k| format!(" · Linux {k}")).unwrap_or_default();
        format!("{}{kernel}", release.unwrap_or_else(|| "Linux".into()))
    }

    pub fn cpu() -> (Option<String>, Option<String>) {
        let info = read("/proc/cpuinfo").unwrap_or_default();
        let name = info.lines().find_map(|l| l.strip_prefix("model name")).and_then(|v| v.split_once(':')).map(|(_, v)| v.trim().to_string());
        // Cores are the distinct (package, core) pairs.
        let mut pairs = std::collections::BTreeSet::new();
        let (mut package, mut core) = (None, None);
        for line in info.lines().chain(std::iter::once("")) {
            if line.trim().is_empty() {
                if let (Some(p), Some(c)) = (package.take(), core.take()) {
                    pairs.insert((p, c));
                }
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                match key.trim() {
                    "physical id" => package = Some(value.trim().to_string()),
                    "core id" => core = Some(value.trim().to_string()),
                    _ => {}
                }
            }
        }
        (name, (!pairs.is_empty()).then(|| format!("{} cores", pairs.len())))
    }

    pub fn memory() -> Option<u64> {
        let info = read("/proc/meminfo")?;
        let kb: u64 = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kb * 1024)
    }

    pub fn gpus() -> Vec<Gpu> {
        let nvidia = nvidia_smi();
        let mut nvidia_seen = 0;
        let mut gpus = Vec::new();
        let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return gpus };
        let mut cards: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.strip_prefix("card").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
            .collect();
        cards.sort();
        for card in cards {
            let device = format!("/sys/class/drm/{card}/device");
            let hex = |file: &str| read(&format!("{device}/{file}")).and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
            let Some(vendor) = hex("vendor") else { continue };
            let driver = std::fs::read_link(format!("{device}/driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
            if vendor == 0x10de {
                if let Some(card) = nvidia.get(nvidia_seen) {
                    gpus.push(card.clone());
                    nvidia_seen += 1;
                    continue;
                }
            }
            let version = driver.as_ref().and_then(|d| read(&format!("/sys/module/{d}/version")));
            let memory = read(&format!("{device}/mem_info_vram_total")).and_then(|v| v.parse::<u64>().ok()).filter(|&b| b > 0);
            let name = format!("{} graphics (device 0x{:04x})", vendor_name(vendor), hex("device").unwrap_or(0));
            let driver = match (driver, version) {
                (Some(d), Some(v)) => Some(format!("{d} {v}")),
                (Some(d), None) => Some(d),
                _ => None,
            };
            gpus.push(Gpu { name, vendor: vendor_name(vendor), memory, driver });
        }
        gpus
    }

    fn nvidia_smi() -> Vec<Gpu> {
        let Ok(out) = std::process::Command::new("nvidia-smi").args(["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader,nounits"]).output() else {
            return Vec::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split(',').map(str::trim).collect();
                let [name, memory, driver] = fields[..] else { return None };
                Some(Gpu {
                    name: name.to_string(),
                    vendor: "NVIDIA".into(),
                    memory: memory.parse::<u64>().ok().map(|mb| mb << 20),
                    driver: Some(driver.to_string()),
                })
            })
            .collect()
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use super::Gpu;
    pub fn os() -> String {
        std::env::consts::OS.to_string()
    }
    pub fn cpu() -> (Option<String>, Option<String>) {
        (None, None)
    }
    pub fn memory() -> Option<u64> {
        None
    }
    pub fn gpus() -> Vec<Gpu> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_nvidia_driver_reads_as_nvidia_numbers_it() {
        assert_eq!(nvidia_version("32.0.15.6636").as_deref(), Some("566.36"));
        assert_eq!(nvidia_version("31.0.15.3623").as_deref(), Some("536.23"));
        assert_eq!(nvidia_version("27.21.14.5671").as_deref(), Some("456.71"));
        assert_eq!(nvidia_version("garbage"), None);
    }

    /// The facts are there, on this computer: a processor with threads,
    /// some memory, an operating system with a version.
    #[test]
    fn the_computer_is_described() {
        let facts = gather_with(true);
        assert!(facts.threads >= 1);
        assert!(!facts.os.is_empty() && facts.os.chars().any(|c| c.is_ascii_digit()), "{}", facts.os);
        assert!(facts.memory.is_some_and(|m| m > 256 << 20), "{:?}", facts.memory);
        assert!(facts.cpu != "unknown processor" || cfg!(target_arch = "aarch64"), "{}", facts.cpu);
        for gpu in &facts.gpus {
            assert!(!gpu.name.is_empty() && !gpu.vendor.is_empty(), "{gpu:?}");
        }
        println!("{facts:#?}");
    }
}
