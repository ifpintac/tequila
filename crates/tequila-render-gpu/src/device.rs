//! Choosing a graphics card, and saying why one was refused (`docs/07` R2.1).

use std::fmt::Write as _;

/// What a device has to offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// Rendering: ray queries through acceleration structures.
    RayQueries,
    /// Compute alone: the differential tests, which can run on a software
    /// Vulkan driver (Mesa's lavapipe in CI) that has no ray queries.
    Compute,
}

/// A wgpu device and its queue, with what was chosen and why.
pub struct GpuDevice {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub needs: Needs,
    /// Set when the device is lost: the system reset the card (Windows does
    /// when one piece of work runs past two seconds), or its driver failed.
    /// Nothing it hands back after that can be trusted.
    lost: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The backends Tequila renders through (`docs/07` §3): no DX12, no GL.
pub const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN.union(wgpu::Backends::METAL);

impl GpuDevice {
    /// The best card that has what `needs` asks: a discrete one before an
    /// integrated one, either before a software one. The error lists every
    /// adapter seen and why it was passed over.
    pub fn new(needs: Needs) -> Result<GpuDevice, String> {
        GpuDevice::on(needs, None)
    }

    /// The card a person chose (`tequila_render_core::devices`): the one
    /// named `wanted.0` - the `wanted.1`th of that name, from 0, where
    /// there are several - if it has what `needs` asks; with `None`, the
    /// best, as [`GpuDevice::new`] chooses. The error says why the card
    /// named cannot render, or lists every adapter seen and why it was
    /// passed over.
    pub fn on(needs: Needs, wanted: Option<(&str, usize)>) -> Result<GpuDevice, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: BACKENDS,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapters = pollster::block_on(instance.enumerate_adapters(BACKENDS));
        let mut refused = String::new();
        let mut best: Option<(u32, wgpu::Adapter)> = None;
        let mut named = 0;
        for adapter in adapters {
            let info = adapter.get_info();
            if let Some((name, ordinal)) = wanted {
                if !info.name.trim().eq_ignore_ascii_case(name.trim()) {
                    continue;
                }
                named += 1;
                if named - 1 != ordinal {
                    continue;
                }
            }
            if let Some(reason) = refusal(&adapter, needs) {
                let _ = writeln!(refused, "  {} ({:?}): {reason}", info.name, info.backend);
                continue;
            }
            let rank = match info.device_type {
                wgpu::DeviceType::DiscreteGpu => 3,
                wgpu::DeviceType::IntegratedGpu => 2,
                wgpu::DeviceType::VirtualGpu | wgpu::DeviceType::Other => 1,
                wgpu::DeviceType::Cpu => 0,
            };
            if best.as_ref().is_none_or(|(r, _)| rank > *r) {
                best = Some((rank, adapter));
            }
        }
        let Some((_, adapter)) = best else {
            return Err(match wanted {
                Some((name, _)) if refused.is_empty() => format!("{name} is not a Vulkan or Metal graphics card here"),
                Some((name, _)) => format!("{name} cannot render here:\n{refused}"),
                None if refused.is_empty() => "no Vulkan or Metal graphics card was found".to_string(),
                None => format!("no graphics card can render here:\n{refused}"),
            });
        };
        let info = adapter.get_info();
        let supported = adapter.limits();
        let mut limits = wgpu::Limits::default();
        // The defaults are 256 and 128 MiB, less than a big scene's geometry.
        limits.max_buffer_size = supported.max_buffer_size;
        limits.max_storage_buffer_binding_size = supported.max_storage_buffer_binding_size;
        limits.max_storage_buffers_per_shader_stage = supported.max_storage_buffers_per_shader_stage;
        limits.max_compute_workgroups_per_dimension = supported.max_compute_workgroups_per_dimension;
        let mut features = wgpu::Features::empty();
        // Where the driver keeps a pipeline cache wgpu can hand back
        // (Vulkan), the kernels it builds are kept between runs
        // (crate::variants::KernelCache).
        if adapter.features().contains(wgpu::Features::PIPELINE_CACHE) {
            features |= wgpu::Features::PIPELINE_CACHE;
        }
        let mut experimental = wgpu::ExperimentalFeatures::disabled();
        if needs == Needs::RayQueries {
            features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY;
            // The acceleration-structure limits default to zero.
            limits.max_blas_primitive_count = supported.max_blas_primitive_count;
            limits.max_blas_geometry_count = supported.max_blas_geometry_count;
            limits.max_tlas_instance_count = supported.max_tlas_instance_count;
            limits.max_acceleration_structures_per_shader_stage = supported.max_acceleration_structures_per_shader_stage;
            limits.max_buffers_and_acceleration_structures_per_shader_stage =
                supported.max_buffers_and_acceleration_structures_per_shader_stage;
            // SAFETY: ray queries are experimental in wgpu 30; docs/07 chose
            // them, and the differential tests are what stand behind them.
            experimental = unsafe { wgpu::ExperimentalFeatures::enabled() };
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("tequila"),
            required_features: features,
            required_limits: limits,
            experimental_features: experimental,
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| format!("{} refused a device: {e}", info.name))?;
        let lost = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = lost.clone();
        device.set_device_lost_callback(move |reason, message| {
            if reason != wgpu::DeviceLostReason::Destroyed {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                eprintln!("  the graphics card was lost ({reason:?}): {message}");
            }
        });
        Ok(GpuDevice { adapter, device, queue, info, needs, lost })
    }

    /// A line for logs: which card, through what, with which driver.
    pub fn describe(&self) -> String {
        format!("{} ({:?}, {} {})", self.info.name, self.info.backend, self.info.driver, self.info.driver_info)
    }

    /// Wait for everything submitted so far.
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Whether the device has been lost (see [`GpuDevice::lost`]'s field).
    pub fn is_lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// A graphics card as wgpu sees it, for a list of devices: its name, the
/// backend it is reached through, and why it cannot render, if it cannot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub backend: String,
    pub refused: Option<String>,
}

/// Every card wgpu finds through Tequila's backends, software ones left
/// out: nobody chooses those to render on.
pub fn adapters(needs: Needs) -> Vec<Adapter> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: BACKENDS,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    pollster::block_on(instance.enumerate_adapters(BACKENDS))
        .into_iter()
        .filter(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu)
        .map(|adapter| {
            let info = adapter.get_info();
            let backend = match info.backend {
                wgpu::Backend::Vulkan => "Vulkan".to_string(),
                wgpu::Backend::Metal => "Metal".to_string(),
                other => format!("{other:?}"),
            };
            Adapter { name: info.name.trim().to_string(), backend, refused: refusal(&adapter, needs) }
        })
        .collect()
}

/// Why `adapter` cannot do what `needs` asks, if it cannot.
fn refusal(adapter: &wgpu::Adapter, needs: Needs) -> Option<String> {
    let features = adapter.features();
    let downlevel = adapter.get_downlevel_capabilities();
    if !downlevel.flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
        return Some("no compute shaders".to_string());
    }
    if needs == Needs::RayQueries && !features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        return Some("no ray queries (its driver may be too old)".to_string());
    }
    None
}

/// Whether a test that needs a card skips when none is found or fails:
/// `TEQUILA_REQUIRE_GPU_TESTS=1` (set in CI, where Mesa's software Vulkan is
/// installed) makes a missing compute adapter a failure, not a skip
/// (`docs/07` R2.4). The software driver has no ray queries, so a test
/// that needs them fails for want of them only under
/// `TEQUILA_REQUIRE_RAY_QUERIES=1`, for a machine with a card.
pub fn for_test(needs: Needs) -> Option<GpuDevice> {
    let required = match needs {
        Needs::Compute => "TEQUILA_REQUIRE_GPU_TESTS",
        Needs::RayQueries => "TEQUILA_REQUIRE_RAY_QUERIES",
    };
    match GpuDevice::new(needs) {
        Ok(device) => Some(device),
        Err(e) if std::env::var(required).is_ok_and(|v| v == "1") => {
            panic!("{required} is set and {e}")
        }
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card a person chose that is not here is said by its name, never
    /// swapped for another without a word; software adapters are not
    /// offered to choose.
    #[test]
    fn a_card_that_is_not_here_is_said_by_name() {
        let error = GpuDevice::on(Needs::Compute, Some(("No Such Graphics Card 9000", 0))).err().expect("no such card");
        assert!(error.starts_with("No Such Graphics Card 9000 "), "{error}");
        assert!(adapters(Needs::Compute).iter().all(|a| !a.name.is_empty()));
    }
}
