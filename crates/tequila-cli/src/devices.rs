//! `--devices` and `--device`: the devices here, and which render
//! (`tequila_render_core::devices`, as Preferences › Rendering chooses them
//! in the application).
//!
//! `--device` takes what it always took - CPU, GPU, a Cycles kind (OPTIX,
//! CUDA, HIP, METAL) or one of Cycles' ids - or the devices `--devices`
//! lists, between commas: `cpu`, `gpu:2` (the second card listed), `gpu:`
//! and a card's name, or the name alone. Without it, a project's Cycles on
//! the graphics card renders on what Preferences › Rendering chose for
//! final renders on this computer, as the application would.

use tequila_render_core::devices::{Catalogue, Choice, Processor, SystemCard, Use, PROCESSOR};

/// The devices here: the self-test's facts (the processor, each card with
/// its memory, Cycles' devices), and what Tequila's own graphics-card
/// renderer reaches each card through.
pub fn catalogue() -> Catalogue {
    with_cards_renderer(true)
}

/// [`catalogue`], with what the graphics-card renderer reaches each card
/// through when `own` (which wakes the Vulkan or Metal driver).
pub fn with_cards_renderer(own: bool) -> Catalogue {
    let facts = crate::self_test::system::gather_with(true);
    let system = facts
        .gpus
        .iter()
        // Linux names a card without NVIDIA's tool by its device number,
        // which no other list does: Cycles names it.
        .filter(|gpu| !gpu.name.contains("graphics (device 0x"))
        .map(|gpu| SystemCard { name: gpu.name.clone(), memory: gpu.memory, shared_memory: gpu.memory.is_none() && gpu.vendor == "Apple" })
        .collect();
    let cycles: Vec<(String, String, String)> = facts.cycles.unwrap_or_default().into_iter().map(|d| (d.kind, d.name, d.id)).collect();
    let mut catalogue = Catalogue::assemble(Processor { name: facts.cpu, threads: facts.threads }, system, &cycles);
    if !own {
        return catalogue;
    }
    let adapters: Vec<(String, String)> = tequila_render_gpu::device::adapters(tequila_render_gpu::Needs::RayQueries)
        .into_iter()
        .filter(|a| a.refused.is_none())
        .map(|a| (a.name, a.backend))
        .collect();
    catalogue.add_gpu_backends(&adapters);
    catalogue
}

/// `--devices`: the list, each with the name `--device` takes, and what
/// final renders render on here.
pub fn list() -> Result<(), String> {
    let catalogue = catalogue();
    print!("{}", listing(&catalogue, &saved_final_choice(), tequila_render_cycles::devices::has_device_lists()));
    Ok(())
}

fn listing(catalogue: &Catalogue, saved: &Choice, lists: bool) -> String {
    let mut out = String::from("Devices here (for --device):\n");
    out.push_str(&format!("  cpu      {} · {} threads\n", catalogue.processor.name, catalogue.processor.threads));
    for (i, card) in catalogue.cards.iter().enumerate() {
        let memory = match (card.memory, card.shared_memory) {
            (_, true) => "shared memory".to_string(),
            (Some(bytes), _) => format!("{} GB", (bytes as f64 / (1u64 << 30) as f64).round()),
            (None, _) => "memory not known".to_string(),
        };
        let cycles: Vec<&str> = card.cycles.iter().map(|d| tequila_render_cycles::devices::kind_label(&d.kind)).collect();
        let cycles = if cycles.is_empty() { "Cycles can't render on it".to_string() } else { format!("Cycles: {}", cycles.join(", ")) };
        let own = if card.gpu_backends.is_empty() { String::new() } else { format!(" · Tequila (experimental): {}", card.gpu_backends.join(", ")) };
        out.push_str(&format!("  gpu:{:<4} {} · {memory} · {cycles}{own}\n", i + 1, card.name));
    }
    let plan = plan(catalogue, saved, lists);
    out.push_str(&format!(
        "Final renders with Cycles on the graphics card: {} ({}).\n",
        catalogue.summary(&plan.keys),
        if *saved == Choice::Auto { "chosen by Tequila" } else { "as Preferences › Rendering chose" }
    ));
    for note in plan.notes {
        out.push_str(&format!("  {note}.\n"));
    }
    out
}

/// What `--device` asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
    /// What it always took, passed on as it is: CPU, GPU, a Cycles kind or
    /// id.
    Named(String),
    /// Devices from the list.
    Chosen(Choice),
}

impl Asked {
    /// `--device`'s value, against the list here.
    pub fn parse(text: &str, catalogue: impl FnOnce() -> Catalogue) -> Result<(Asked, Option<Catalogue>), String> {
        let text = text.trim();
        let kinds = ["CPU", "GPU", "OPTIX", "CUDA", "HIP", "METAL", "ONEAPI"];
        // A Cycles id is its kind, an underscore and the card's name and bus.
        let id = |kind: &str| text.get(..kind.len() + 1).is_some_and(|head| head.eq_ignore_ascii_case(&format!("{kind}_")));
        if kinds.iter().any(|k| k.eq_ignore_ascii_case(text) || id(k)) {
            return Ok((Asked::Named(text.to_string()), None));
        }
        let catalogue = catalogue();
        let mut keys = Vec::new();
        for part in text.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
            let key = key_for(&catalogue, part).ok_or_else(|| format!("--device: there is no {part:?} here; --devices lists what there is"))?;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        if keys.is_empty() {
            return Err("--device names no device; --devices lists them".into());
        }
        Ok((Asked::Chosen(Choice::Chosen(keys)), Some(catalogue)))
    }
}

/// A device's key from what a person typed: `cpu`, `gpu:2`, `gpu:` and a
/// name, or a name.
fn key_for(catalogue: &Catalogue, part: &str) -> Option<String> {
    if part.eq_ignore_ascii_case(PROCESSOR) || part.eq_ignore_ascii_case("processor") {
        return Some(PROCESSOR.to_string());
    }
    let named = match part.get(..4) {
        Some(head) if head.eq_ignore_ascii_case("gpu:") => part[4..].trim(),
        _ => part,
    };
    if let Ok(n) = named.parse::<usize>() {
        return catalogue.cards.get(n.checked_sub(1)?).map(|c| c.key.clone());
    }
    catalogue
        .cards
        .iter()
        .find(|c| c.key.get(4..).is_some_and(|k| k.eq_ignore_ascii_case(named)))
        .or_else(|| catalogue.cards.iter().find(|c| c.name.eq_ignore_ascii_case(named)))
        .map(|c| c.key.clone())
}

/// Cycles' session for `choice` on final renders.
pub fn plan(catalogue: &Catalogue, choice: &Choice, lists: bool) -> tequila_render_cycles::devices::Plan {
    let resolved = catalogue.resolve(choice);
    let mut plan = tequila_render_cycles::devices::plan(catalogue, &resolved.keys, Use::Final, lists);
    let gone: Vec<String> = resolved.missing.iter().map(|name| format!("{name} was chosen, and isn't in this computer now")).collect();
    plan.notes.splice(0..0, gone);
    plan
}

/// The card Tequila's own graphics-card renderer draws on for `choice`:
/// the first card in it, by name and which of that name; `None` where it
/// names none. It draws on one card at a time (`docs/07` R5.4).
pub fn card_for(catalogue: &Catalogue, choice: &Choice) -> Option<(String, usize)> {
    let Choice::Chosen(keys) = choice else { return None };
    let key = keys.iter().find(|k| *k != PROCESSOR)?;
    let card = catalogue.card(key)?;
    let ordinal = catalogue.cards.iter().take_while(|c| c.key != card.key).filter(|c| c.name == card.name).count();
    Some((card.name.clone(), ordinal))
}

/// What Preferences › Rendering chose for final renders on this computer,
/// read from the application's `preferences.txt`: the default where it
/// chose nothing.
pub fn saved_final_choice() -> Choice {
    let file = tequila_format::library::settings_directory().join("preferences.txt");
    std::fs::read_to_string(file).map(|text| final_choice_in(&text)).unwrap_or_default()
}

fn final_choice_in(text: &str) -> Choice {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .find(|(key, _)| key.trim() == "rendering.final_devices")
        .and_then(|(_, value)| Choice::parse(value).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn mixed() -> Catalogue {
        let mut cycles = vec![("CPU".to_string(), "Processor".to_string(), "CPU".to_string())];
        for (kind, suffix) in [("CUDA", ""), ("OPTIX", "_OptiX")] {
            cycles.push((kind.into(), "NVIDIA GeForce RTX 3060".into(), format!("CUDA_NVIDIA GeForce RTX 3060_0000:01:00{suffix}")));
            cycles.push((kind.into(), "NVIDIA GeForce GTX 1650".into(), format!("CUDA_NVIDIA GeForce GTX 1650_0000:02:00{suffix}")));
        }
        let mut list = Catalogue::assemble(
            Processor { name: "AMD Ryzen 9 5900X".into(), threads: 24 },
            vec![
                SystemCard { name: "NVIDIA GeForce GTX 1650".into(), memory: Some(4 * GB), shared_memory: false },
                SystemCard { name: "NVIDIA GeForce RTX 3060".into(), memory: Some(12 * GB), shared_memory: false },
            ],
            &cycles,
        );
        list.add_gpu_backends(&[("NVIDIA GeForce RTX 3060".into(), "Vulkan".into())]);
        list
    }

    #[test]
    fn the_device_flag_takes_what_it_took_and_the_lists_names() {
        let named = |text: &str| Asked::parse(text, mixed).unwrap().0;
        assert_eq!(named("CPU"), Asked::Named("CPU".into()));
        assert_eq!(named("gpu"), Asked::Named("gpu".into()));
        assert_eq!(named("OPTIX"), Asked::Named("OPTIX".into()));
        assert_eq!(named("CUDA_NVIDIA GeForce RTX 3060_0000:01:00"), Asked::Named("CUDA_NVIDIA GeForce RTX 3060_0000:01:00".into()));
        let chosen = |list: &[&str]| Asked::Chosen(Choice::Chosen(list.iter().map(|k| k.to_string()).collect()));
        assert_eq!(named("gpu:1"), chosen(&["gpu:NVIDIA GeForce RTX 3060"]), "the first listed is the biggest");
        assert_eq!(named("cpu, gpu:2"), chosen(&["cpu", "gpu:NVIDIA GeForce GTX 1650"]));
        assert_eq!(named("NVIDIA GeForce GTX 1650"), chosen(&["gpu:NVIDIA GeForce GTX 1650"]));
        assert_eq!(named("gpu:nvidia geforce rtx 3060"), chosen(&["gpu:NVIDIA GeForce RTX 3060"]));
        assert!(Asked::parse("gpu:9", mixed).is_err());
        assert!(Asked::parse("toaster", mixed).is_err());
    }

    #[test]
    fn the_choice_reaches_cycles_and_the_cards_own_renderer() {
        let list = mixed();
        let small = Choice::Chosen(vec!["gpu:NVIDIA GeForce GTX 1650".into()]);
        assert_eq!(plan(&list, &small, true).device, "CUDA_NVIDIA GeForce GTX 1650_0000:02:00_OptiX");
        assert_eq!(plan(&list, &Choice::Auto, false).device, "CUDA_NVIDIA GeForce RTX 3060_0000:01:00_OptiX");
        assert_eq!(card_for(&list, &small), Some(("NVIDIA GeForce GTX 1650".into(), 0)));
        assert_eq!(card_for(&list, &Choice::Chosen(vec!["cpu".into()])), None);
        let listed = listing(&list, &Choice::Auto, true);
        assert!(listed.contains("gpu:1    NVIDIA GeForce RTX 3060 · 12 GB · Cycles: OptiX, CUDA · Tequila (experimental): Vulkan"), "{listed}");
        assert!(listed.contains("Final renders with Cycles on the graphics card: GeForce RTX 3060 (chosen by Tequila)."), "{listed}");
    }

    #[test]
    fn the_saved_choice_is_read_from_the_applications_file() {
        assert_eq!(final_choice_in("viewport.grid=on\nrendering.final_devices=cpu; gpu:NVIDIA GeForce RTX 3060\n"), Choice::Chosen(vec!["cpu".into(), "gpu:NVIDIA GeForce RTX 3060".into()]));
        assert_eq!(final_choice_in("rendering.final_devices=auto\n"), Choice::Auto);
        assert_eq!(final_choice_in("rendering.final_devices=???\n"), Choice::Auto, "a line not understood keeps the default");
        assert_eq!(final_choice_in(""), Choice::Auto);
    }
}
