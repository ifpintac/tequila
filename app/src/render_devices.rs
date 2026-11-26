//! Which processor and graphics cards render (`docs/04` §12.2, Rendering):
//! the computer's devices, found once, and what the person's choices for
//! the Rendered view and for final renders (Preferences › Rendering, kept in
//! `preferences.txt` as `rendering.view_devices` and
//! `rendering.final_devices`) make each engine render on.
//!
//! The list and the rules are `tequila_render_core::devices`'; what Cycles
//! makes of a choice is `tequila_render_cycles::devices::plan`. Tequila's own
//! engine renders on the processor in the application: its graphics-card
//! renderer is not in it yet (`docs/07` R4), and when it is, it reads the
//! same choices.
//!
//! The graphics cards' memory comes from the system, as the self-test reads
//! it (`tequila-cli`'s `self_test/system.rs`): DXGI on Windows, `nvidia-smi`
//! on Linux. A Mac's graphics share its memory, and Cycles names them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tequila_render_core::devices::{Catalogue, Choice, Processor, SystemCard, Use, PROCESSOR};
use tequila_render_cycles::devices::Plan;
use tequila_scene::render::RenderEngine;

static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
static LOOKING: std::sync::Once = std::sync::Once::new();

/// The devices here, if they have been found: the first call starts
/// looking, on a thread of its own - listing Cycles' devices loads it and
/// wakes every graphics driver, a second or two the window should not wait.
pub fn catalogue_now() -> Option<&'static Catalogue> {
    if CATALOGUE.get().is_none() {
        LOOKING.call_once(|| {
            let _ = std::thread::Builder::new().name("devices".into()).spawn(|| {
                catalogue();
            });
        });
    }
    CATALOGUE.get()
}

/// The devices here, found now if they have not been: for the render
/// threads, which are about to load Cycles anyway.
pub fn catalogue() -> &'static Catalogue {
    CATALOGUE.get_or_init(gather)
}

fn gather() -> Catalogue {
    let name = crate::readout::Readout::default().processor().to_string();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let cycles = tequila_render_cycles::devices().unwrap_or_default();
    Catalogue::assemble(Processor { name, threads }, system_cards(), &cycles)
}

/// The Cycles session for `engine` and `choice`: Cycles on the processor
/// renders there whatever is chosen; on the graphics card, on the devices
/// chosen for `purpose`.
pub fn cycles_plan(engine: RenderEngine, choice: &Choice, purpose: Use) -> Plan {
    match engine {
        RenderEngine::CyclesGpu => plan_in(catalogue(), choice, purpose, tequila_render_cycles::devices::has_device_lists()),
        _ => Plan::processor(),
    }
}

/// [`cycles_plan`] on `catalogue`, from a library that takes several
/// devices when `lists`, with what the choice named that is gone said
/// first.
pub fn plan_in(catalogue: &Catalogue, choice: &Choice, purpose: Use, lists: bool) -> Plan {
    let resolved = catalogue.resolve(choice);
    let mut plan = tequila_render_cycles::devices::plan(catalogue, &resolved.keys, purpose, lists);
    let gone: Vec<String> = resolved.missing.iter().map(|name| format!("{name} was chosen, and isn't in this computer now")).collect();
    plan.notes.splice(0..0, gone);
    plan
}

/// What a plan renders on in a few words, with the kind its cards render
/// through: "GeForce RTX 2060 (OptiX)", "processor".
pub fn plan_label(catalogue: &Catalogue, plan: &Plan) -> String {
    let summary = match catalogue.summary(&plan.keys) {
        alone if alone == "processor" => "the processor".to_string(),
        summary => summary,
    };
    match &plan.kind {
        Some(kind) => format!("{summary} ({})", tequila_render_cycles::devices::kind_label(kind)),
        None => summary,
    }
}

/// What a Cycles session says it renders on, in the list's words: its
/// devices' keys where the list has them. Through CUDA where OptiX would
/// not start, the card is the same.
pub fn session_label(catalogue: &Catalogue, devices: &[(String, String, String)]) -> Option<String> {
    let mut keys = Vec::new();
    for (kind, _, id) in devices {
        if kind == "CPU" {
            keys.push(PROCESSOR.to_string());
            continue;
        }
        let card = catalogue.cards.iter().find(|card| card.cycles.iter().any(|d| d.id == *id))?;
        keys.push(card.key.clone());
    }
    let order = catalogue.keys();
    keys.sort_by_key(|k| order.iter().position(|o| o == k));
    keys.dedup();
    (!keys.is_empty()).then(|| catalogue.summary(&keys))
}

/// The status bar's word for a Rendered view: on what it is path traced,
/// as the render thread said it (`Orchestrator::view_devices`): "Path
/// traced on GeForce RTX 2060", "Path traced on the processor".
pub fn path_traced(engine: RenderEngine, on: &str) -> String {
    match (engine, on) {
        (RenderEngine::Tequila, _) | (_, "processor") => "Path traced on the processor".to_string(),
        (_, "") => "Path traced".to_string(),
        (_, on) => format!("Path traced on {on}"),
    }
}

/// Set from Render › Devices…, taken by the window, which opens
/// Preferences on its Rendering page.
static OPEN_PREFERENCES: AtomicBool = AtomicBool::new(false);

/// Ask for Preferences › Rendering to open.
pub fn ask_for_preferences() {
    OPEN_PREFERENCES.store(true, Ordering::Release);
}

/// Whether Preferences › Rendering was asked for since this was last asked.
pub fn preferences_asked() -> bool {
    OPEN_PREFERENCES.swap(false, Ordering::AcqRel)
}

/// The final renders' choice, as Preferences has it, for the Render
/// window, which is not handed the preferences: set at every frame, as
/// `cycles_view::set_tuning` is.
static FINAL_CHOICE: std::sync::Mutex<Choice> = std::sync::Mutex::new(Choice::Auto);

/// Say what final renders are to render on.
pub fn set_final_choice(choice: &Choice) {
    let mut kept = FINAL_CHOICE.lock().unwrap_or_else(|e| e.into_inner());
    if *kept != *choice {
        *kept = choice.clone();
    }
}

/// The Render window's line under the engine: where the final render goes,
/// and what of the choice is left out, with a button to the choice.
pub fn render_window_line(ui: &mut egui::Ui, settings: &tequila_scene::render::RenderSettings) {
    let choice = FINAL_CHOICE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let choice = &choice;
    let (engine, _) = crate::batch::engine_for(settings, !settings.layers.is_empty());
    let (said, notes) = match engine {
        RenderEngine::Tequila => ("Renders on the processor.".to_string(), Vec::new()),
        RenderEngine::CyclesCpu => ("Renders on the processor.".to_string(), Vec::new()),
        RenderEngine::CyclesGpu => match catalogue_now() {
            Some(catalogue) => {
                let plan = plan_in(catalogue, choice, Use::Final, tequila_render_cycles::devices::has_device_lists());
                (format!("Renders on {}.", plan_label(catalogue, &plan)), plan.notes)
            }
            None => {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                ("Looking for this computer's devices…".to_string(), Vec::new())
            }
        },
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new(said).small().color(crate::theme::dim()));
        let response = ui.add(egui::Label::new(egui::RichText::new("Devices…").small().color(crate::theme::p().accent_text)).sense(egui::Sense::click()));
        if response.on_hover_text("Choose the processor and graphics cards that render, in Preferences › Rendering").clicked() {
            ask_for_preferences();
        }
    });
    for note in notes {
        ui.label(egui::RichText::new(format!("{note}.")).small().color(crate::theme::dim()));
    }
}

/// A change made in the Preferences grid.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Set(Use, Choice),
    Reset(Use),
}

/// Preferences › Rendering's devices: a row for the processor and each
/// graphics card, a box for each use, and under them what each engine
/// renders on.
pub fn preferences_grid(ui: &mut egui::Ui, view: &Choice, finals: &Choice) -> Vec<Change> {
    let mut changes = Vec::new();
    let Some(catalogue) = catalogue_now() else {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.spinner();
            crate::theme::note(ui, "Looking for this computer's processor and graphics cards…");
        });
        return changes;
    };
    let lists = tequila_render_cycles::devices::has_device_lists();
    let uses = [(Use::View, view, "Rendered view"), (Use::Final, finals, "Final renders")];
    let resolved: Vec<_> = uses.iter().map(|(_, choice, _)| catalogue.resolve(choice)).collect();
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        egui::Grid::new("render-devices").num_columns(3).spacing(egui::vec2(18.0, 6.0)).show(ui, |ui| {
            ui.label("");
            for (i, (purpose, choice, title)) in uses.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(*title).strong());
                    if **choice != Choice::Auto {
                        let (mark, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        let response = ui.interact(mark, ui.id().with(("reset-devices", i)), egui::Sense::click());
                        let p = crate::theme::p();
                        crate::icons::paint(ui.painter(), mark.shrink(2.0), crate::icons::Icon::History, if response.hovered() { p.text } else { p.accent_text });
                        let default = catalogue.summary(&catalogue.default_keys());
                        if response.on_hover_text(format!("Back to the default: {default}, chosen by Tequila")).clicked() {
                            changes.push(Change::Reset(*purpose));
                        }
                    }
                });
            }
            ui.end_row();
            for key in catalogue.keys() {
                let card = catalogue.card(&key);
                let renders = card.is_none_or(|c| c.cycles_renders());
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let name = match card {
                        Some(card) => card.name.clone(),
                        None => catalogue.processor.name.clone(),
                    };
                    let ink = if renders { crate::theme::p().text } else { crate::theme::dim() };
                    ui.label(egui::RichText::new(name).color(ink));
                    ui.label(egui::RichText::new(detail(catalogue, &key)).small().color(crate::theme::dim()));
                });
                for (i, (purpose, choice, _)) in uses.iter().enumerate() {
                    let chosen = resolved[i].keys.contains(&key);
                    let mut on = chosen && renders;
                    let last = chosen && resolved[i].keys.len() == 1;
                    let response = ui.add_enabled(renders && !last, crate::field::Checkbox::new(&mut on, ""));
                    let response = if !renders {
                        response.on_disabled_hover_text("Cycles can't render on this card")
                    } else if last {
                        response.on_disabled_hover_text("At least one device renders: turn another on first")
                    } else {
                        response.on_hover_text(match purpose {
                            Use::View => "Render the Rendered view on this device",
                            Use::Final => "Render final renders on this device",
                        })
                    };
                    if response.changed() {
                        changes.push(Change::Set(*purpose, catalogue.toggled(choice, &key, on)));
                    }
                }
                ui.end_row();
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.vertical(|ui| notes(ui, catalogue, &uses, &resolved, lists));
    });
    changes
}

/// Under the grid: what Cycles renders each use on, and what of each
/// choice is left out.
fn notes(ui: &mut egui::Ui, catalogue: &Catalogue, uses: &[(Use, &Choice, &str)], resolved: &[tequila_render_core::devices::Resolved], lists: bool) {
    for (i, (purpose, choice, title)) in uses.iter().enumerate() {
        let plan = plan_in(catalogue, choice, *purpose, lists);
        let mut line = format!("{title}: Cycles on the graphics card renders on {}", plan_label(catalogue, &plan));
        if resolved[i].defaulted {
            line.push_str(", as nothing chosen is here");
        }
        crate::theme::note(ui, format!("{line}."));
        for note in &plan.notes {
            crate::theme::note(ui, format!("{note}."));
        }
    }
    crate::theme::note(
        ui,
        "Cycles on the processor renders on the processor alone, and so does Tequila's own engine: its graphics-card renderer is not in the app yet.",
    );
}

/// A device's second line: the processor's threads; a card's memory and
/// what Cycles renders on it through.
fn detail(catalogue: &Catalogue, key: &str) -> String {
    let Some(card) = catalogue.card(key) else {
        return format!("Processor · {} threads", catalogue.processor.threads);
    };
    let memory = match (card.memory, card.shared_memory) {
        (_, true) => "shares the computer's memory".to_string(),
        (Some(bytes), _) => memory_size(bytes),
        (None, _) => "memory not known".to_string(),
    };
    let kinds: Vec<&str> = card.cycles.iter().map(|d| tequila_render_cycles::devices::kind_label(&d.kind)).collect();
    if kinds.is_empty() {
        format!("{memory} · Cycles can't render on it")
    } else {
        format!("{memory} · {}", kinds.join(", "))
    }
}

/// A card's memory as its maker sells it: "12 GB". The system counts a
/// little less than that (a 6 GB card's 5.8).
fn memory_size(bytes: u64) -> String {
    let gb = bytes as f64 / (1u64 << 30) as f64;
    if gb < 1.0 {
        format!("{:.0} MB", bytes as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} GB", gb.round())
    }
}

/// The graphics cards the system lists, with their own memory.
#[cfg(windows)]
fn system_cards() -> Vec<SystemCard> {
    windows::cards()
}

/// An NVIDIA card's name and memory from its driver's tool; another's
/// memory is not known, and Cycles names it.
#[cfg(all(unix, not(target_os = "macos")))]
fn system_cards() -> Vec<SystemCard> {
    let Ok(out) = std::process::Command::new("nvidia-smi").args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (name, memory) = line.split_once(',')?;
            Some(SystemCard { name: name.trim().to_string(), memory: memory.trim().parse::<u64>().ok().map(|mb| mb << 20), shared_memory: false })
        })
        .collect()
}

/// A Mac's graphics share its memory, and Cycles names them.
#[cfg(target_os = "macos")]
fn system_cards() -> Vec<SystemCard> {
    Vec::new()
}

#[cfg(windows)]
mod windows {
    //! DXGI's adapters, as the self-test reads them: present ones only,
    //! not the Basic Render Driver nor a remote session's.

    use super::SystemCard;
    use std::ffi::c_void;

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

    #[link(name = "dxgi")]
    extern "system" {
        fn CreateDXGIFactory1(riid: *const Guid, factory: *mut *mut c_void) -> i32;
    }

    const IID_FACTORY1: Guid = Guid { data1: 0x770a_ae78, data2: 0xf26f, data3: 0x4dba, data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87] };
    const ADAPTER_SOFTWARE: u32 = 2;
    const MICROSOFT: u32 = 0x1414;

    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type EnumAdapters1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut AdapterDesc1) -> i32;

    /// A COM method from `object`'s table, by its place in it.
    unsafe fn method<T: Copy>(object: *mut c_void, index: usize) -> T {
        let table = *(object as *const *const usize);
        std::mem::transmute_copy(&*table.add(index))
    }

    pub fn cards() -> Vec<SystemCard> {
        let mut out = Vec::new();
        // SAFETY: DXGI's documented calls, each object released once. The
        // places in the tables are IDXGIFactory1's EnumAdapters1 (12) and
        // IDXGIAdapter1's GetDesc1 (10), after IUnknown's three and
        // IDXGIObject's four.
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
                if describe(adapter, &mut desc) >= 0 && desc.flags & ADAPTER_SOFTWARE == 0 && desc.vendor_id != MICROSOFT {
                    let end = desc.description.iter().position(|&u| u == 0).unwrap_or(desc.description.len());
                    let name = String::from_utf16_lossy(&desc.description[..end]).trim().to_string();
                    let memory = (desc.dedicated_video_memory > 0).then_some(desc.dedicated_video_memory as u64);
                    out.push(SystemCard { name, memory, shared_memory: false });
                }
                let release: Release = method(adapter, 2);
                release(adapter);
            }
            let release: Release = method(factory, 2);
            release(factory);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_render_core::devices::SystemCard;

    const GB: u64 = 1 << 30;

    /// The request's computer: a 4 GB card, a 12 GB one and the processor.
    fn mixed() -> Catalogue {
        let mut cycles = vec![("CPU".to_string(), "Processor".to_string(), "CPU".to_string())];
        for (kind, suffix) in [("CUDA", ""), ("OPTIX", "_OptiX")] {
            cycles.push((kind.into(), "NVIDIA GeForce GTX 1650".into(), format!("CUDA_NVIDIA GeForce GTX 1650_0000:01:00{suffix}")));
            cycles.push((kind.into(), "NVIDIA GeForce RTX 3060".into(), format!("CUDA_NVIDIA GeForce RTX 3060_0000:02:00{suffix}")));
        }
        Catalogue::assemble(
            Processor { name: "AMD Ryzen 9 5900X".into(), threads: 24 },
            vec![
                SystemCard { name: "NVIDIA GeForce GTX 1650".into(), memory: Some(4 * GB), shared_memory: false },
                SystemCard { name: "NVIDIA GeForce RTX 3060".into(), memory: Some(12 * GB), shared_memory: false },
            ],
            &cycles,
        )
    }

    /// The choice reaches Cycles: the default is the 12 GB card alone, for
    /// both uses; the processor alone is Cycles on the processor; a card
    /// that is gone is said, and what is here renders.
    #[test]
    fn the_choice_becomes_cycles_devices() {
        let list = mixed();
        let auto = plan_in(&list, &Choice::Auto, Use::View, true);
        assert_eq!(auto.device, "CUDA_NVIDIA GeForce RTX 3060_0000:02:00_OptiX");
        assert_eq!(plan_label(&list, &auto), "GeForce RTX 3060 (OptiX)");
        let cpu = plan_in(&list, &Choice::Chosen(vec!["cpu".into()]), Use::Final, true);
        assert_eq!(cpu.device, "CPU");
        assert_eq!(plan_label(&list, &cpu), "the processor");
        let gone = plan_in(&list, &Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 4090".into(), "gpu:NVIDIA GeForce GTX 1650".into()]), Use::Final, true);
        assert_eq!(gone.device, "CUDA_NVIDIA GeForce GTX 1650_0000:01:00_OptiX");
        assert!(gone.notes[0].contains("RTX 4090"), "{:?}", gone.notes);
        let both = plan_in(&list, &Choice::Chosen(vec!["cpu".into(), "gpu:NVIDIA GeForce RTX 3060".into(), "gpu:NVIDIA GeForce GTX 1650".into()]), Use::Final, true);
        assert_eq!(both.device, "CPU\nCUDA_NVIDIA GeForce RTX 3060_0000:02:00_OptiX\nCUDA_NVIDIA GeForce GTX 1650_0000:01:00_OptiX");
        assert_eq!(plan_label(&list, &both), "2 graphics cards + processor (OptiX)");
    }

    /// Cycles on the processor stays there whatever is chosen, and so does
    /// Tequila's own engine; neither needs the devices to have been found.
    #[test]
    fn the_processor_engines_ignore_the_choice() {
        let cards = Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 3060".into()]);
        assert_eq!(cycles_plan(RenderEngine::CyclesCpu, &cards, Use::Final), Plan::processor());
        assert_eq!(cycles_plan(RenderEngine::Tequila, &cards, Use::View), Plan::processor());
    }

    #[test]
    fn a_session_says_what_it_renders_on_in_the_lists_words() {
        let list = mixed();
        let session = [("OPTIX".to_string(), "NVIDIA GeForce RTX 3060".to_string(), "CUDA_NVIDIA GeForce RTX 3060_0000:02:00_OptiX".to_string())];
        assert_eq!(session_label(&list, &session).as_deref(), Some("GeForce RTX 3060"));
        let cuda = [
            ("CPU".to_string(), "Processor".to_string(), "CPU".to_string()),
            ("CUDA".to_string(), "NVIDIA GeForce RTX 3060".to_string(), "CUDA_NVIDIA GeForce RTX 3060_0000:02:00".to_string()),
        ];
        assert_eq!(session_label(&list, &cuda).as_deref(), Some("GeForce RTX 3060 + processor"));
        assert_eq!(session_label(&list, &[("CUDA".into(), "?".into(), "CUDA_unknown".into())]), None);
    }

    /// The choices reach the engines: the Rendered view's goes with the
    /// view's request, which restarts on a change; the final renders' with
    /// a queued render as it starts.
    #[test]
    fn the_choices_reach_the_view_and_the_queue() {
        use crate::preferences::Value;
        let mut app = crate::TequilaApp::for_tests();
        app.viewport_size = (320, 200);
        let cpu = Choice::Chosen(vec!["cpu".into()]);
        app.set_preference("rendering.view_devices", Value::Text("CPU".into())).unwrap();
        assert_eq!(app.prefs.view_devices, cpu);
        assert_eq!(app.orchestrator.request().map(|r| r.devices), Some(cpu.clone()));
        let both = Choice::Chosen(vec!["cpu".into(), "gpu:NVIDIA GeForce RTX 3060".into()]);
        app.set_preference("rendering.final_devices", Value::Text("cpu; gpu:NVIDIA GeForce RTX 3060".into())).unwrap();
        let id = app.project.enqueue("Devices", 0, Default::default());
        assert_eq!(app.build_job(id).expect("a job").devices, both);
        // Nothing named is refused, and the choice stays.
        assert!(app.set_preference("rendering.final_devices", Value::Text(";".into())).is_err());
        assert!(app.set_preference("rendering.final_devices", Value::Text("the toaster".into())).is_err());
        assert_eq!(app.prefs.final_devices, both);
        // Back to Tequila's choice.
        app.reset_preferences(Some(crate::preferences::Page::Rendering)).unwrap();
        assert_eq!((app.prefs.view_devices.clone(), app.prefs.final_devices.clone()), (Choice::Auto, Choice::Auto));
    }

    /// Settings from before there was a choice keep the default; a choice
    /// is written and read back.
    #[test]
    fn settings_from_before_keep_the_default() {
        let mut app = crate::TequilaApp::for_tests();
        let (taken, wrong) = app.import_preferences("viewport.grid=on\nnavigation.invert_zoom=on\n");
        assert_eq!((taken, wrong.len()), (2, 0));
        assert_eq!((app.prefs.view_devices.clone(), app.prefs.final_devices.clone()), (Choice::Auto, Choice::Auto));
        let exported = app.exported_preferences();
        assert!(exported.contains("rendering.view_devices=auto\n") && exported.contains("rendering.final_devices=auto\n"), "{exported}");
        app.import_preferences("rendering.final_devices=gpu:NVIDIA GeForce RTX 3060#2\nrendering.view_devices=nonsense\n");
        assert_eq!(app.prefs.final_devices, Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 3060#2".into()]));
        assert_eq!(app.prefs.view_devices, Choice::Auto, "a line not understood keeps the default");
        assert!(app.exported_preferences().contains("rendering.final_devices=gpu:NVIDIA GeForce RTX 3060#2\n"));
    }

    #[test]
    fn the_status_bar_says_where_the_view_is_traced() {
        assert_eq!(path_traced(RenderEngine::CyclesGpu, "GeForce RTX 2060"), "Path traced on GeForce RTX 2060");
        assert_eq!(path_traced(RenderEngine::CyclesGpu, "processor"), "Path traced on the processor");
        assert_eq!(path_traced(RenderEngine::Tequila, ""), "Path traced on the processor");
        assert_eq!(path_traced(RenderEngine::CyclesCpu, ""), "Path traced");
    }

    #[test]
    fn a_card_is_described_by_its_memory_and_kinds() {
        let list = mixed();
        assert_eq!(detail(&list, "gpu:NVIDIA GeForce RTX 3060"), "12 GB · OptiX, CUDA");
        assert_eq!(detail(&list, "cpu"), "Processor · 24 threads");
        assert_eq!(memory_size(6_224_347_136), "6 GB");
        assert_eq!(memory_size(512 << 20), "512 MB");
    }
}
