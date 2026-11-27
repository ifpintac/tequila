//! The devices a Cycles session renders on, from the person's choice
//! (`tequila_render_core::devices`): the graphics cards of one kind - OptiX,
//! CUDA, HIP or Metal - and the processor beside them for a final render.

use tequila_render_core::devices::{Catalogue, Use, CYCLES_CARD_KINDS, PROCESSOR};

/// What a session is opened on for a choice, and what it renders on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The device for [`crate::Session::new`] and the rest: "CPU", a kind,
    /// an id, or several of them, a line each.
    pub device: String,
    /// The devices it renders on, by key, in the list's order.
    pub keys: Vec<String>,
    /// The kind its cards render through ("OPTIX", "CUDA"), if it has any.
    pub kind: Option<String>,
    /// What was chosen and is left out, a sentence each.
    pub notes: Vec<String>,
}

impl Plan {
    /// The processor alone.
    pub fn processor() -> Plan {
        Plan { device: "CPU".to_string(), keys: vec![PROCESSOR.to_string()], kind: None, notes: Vec::new() }
    }

    /// Whether a graphics card renders.
    pub fn on_card(&self) -> bool {
        self.keys.iter().any(|k| k != PROCESSOR)
    }
}

/// Whether the installed library renders on several devices named a line
/// each, which it says by having `fsc_session_devices`. One from before
/// takes a kind (every card of it) or one device's id.
pub fn has_device_lists() -> bool {
    crate::ffi::api().is_ok_and(|api| api.fsc_session_devices.is_some())
}

/// A kind as people know it.
pub fn kind_label(kind: &str) -> &str {
    match kind {
        "OPTIX" => "OptiX",
        "CUDA" => "CUDA",
        "HIP" => "HIP",
        "METAL" => "Metal",
        "ONEAPI" => "oneAPI",
        other => other,
    }
}

/// The session for the devices `chosen` (keys of `catalogue`, as
/// [`Catalogue::resolve`] gives them) for `purpose`, from a library that
/// takes several devices when `lists`.
///
/// Cycles renders on several cards of one kind together: the best kind all
/// the chosen cards have - OptiX where every one has it, else CUDA - or,
/// where they have none in common, the first card's, the others left out. A
/// Rendered view leaves the processor out beside a card, as Cycles does for
/// Blender's viewport. Every card of the kind - the one card, on most
/// computers - is named by the kind, which any library takes, and one card
/// of several by its id; a library without lists
/// renders on the first chosen card alone where the choice needs one.
pub fn plan(catalogue: &Catalogue, chosen: &[String], purpose: Use, lists: bool) -> Plan {
    let mut notes = Vec::new();
    let mut processor = chosen.iter().any(|k| k == PROCESSOR);
    let mut cards: Vec<&tequila_render_core::devices::Card> = Vec::new();
    for key in chosen.iter().filter(|k| *k != PROCESSOR) {
        match catalogue.card(key) {
            Some(card) if card.cycles_renders() => cards.push(card),
            Some(card) => notes.push(format!("Cycles can't render on {}, so it sits out", card.short_name())),
            None => {}
        }
    }
    if cards.is_empty() {
        if !processor {
            notes.push("No chosen graphics card is one Cycles can render on, so it renders on the processor".to_string());
        }
        return Plan { notes, ..Plan::processor() };
    }
    let kind = CYCLES_CARD_KINDS
        .into_iter()
        .find(|kind| cards.iter().all(|c| c.cycles_device(kind).is_some()))
        .or_else(|| cards[0].best_cycles_kind())
        .expect("a card Cycles renders on has a kind");
    let first = cards[0].short_name();
    cards.retain(|card| {
        let joins = card.cycles_device(kind).is_some();
        if !joins {
            notes.push(format!("{} can't render with {} beside {first}, so it sits out", card.short_name(), kind_label(kind)));
        }
        joins
    });
    if processor && purpose == Use::View {
        processor = false;
        notes.push("The processor sits out the Rendered view beside a graphics card, as in Blender: it would hold the card back".to_string());
    }
    let every = catalogue.cards.iter().filter(|c| c.cycles_device(kind).is_some()).count();
    let ids: Vec<String> = cards.iter().map(|c| c.cycles_device(kind).expect("kept for it").id.clone()).collect();
    let device = match (cards.len(), processor) {
        // As a computer with one card always rendered: by the kind.
        (n, false) if n == every => kind.to_string(),
        (1, false) => ids[0].clone(),
        _ if lists => std::iter::once("CPU".to_string()).filter(|_| processor).chain(ids.iter().cloned()).collect::<Vec<_>>().join("\n"),
        (n, true) if n == every => {
            processor = false;
            notes.push("The Cycles installed here can't add the processor to the graphics cards, so they render without it".to_string());
            kind.to_string()
        }
        _ => {
            processor = false;
            notes.push(format!("The Cycles installed here renders on one chosen card at a time, so it uses {first}"));
            cards.truncate(1);
            ids[0].clone()
        }
    };
    let keys = std::iter::once(PROCESSOR.to_string()).filter(|_| processor).chain(cards.iter().map(|c| c.key.clone())).collect();
    Plan { device, keys, kind: Some(kind.to_string()), notes }
}

/// `device` with OptiX swapped for CUDA, which runs on every NVIDIA card
/// Cycles supports, where OptiX will not start (it needs a recent driver):
/// the kind's name, and each OptiX id, which is its card's CUDA id with
/// `_OptiX` after it. `None` where there is no OptiX in it.
pub fn without_optix(device: &str) -> Option<String> {
    let mut changed = false;
    let lines: Vec<String> = device
        .lines()
        .map(|line| {
            if line.trim().eq_ignore_ascii_case("OPTIX") {
                changed = true;
                "CUDA".to_string()
            } else if let Some(cuda) = line.trim().strip_suffix("_OptiX") {
                changed = true;
                cuda.to_string()
            } else {
                line.to_string()
            }
        })
        .collect();
    changed.then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_render_core::devices::{Processor, SystemCard};

    const GB: u64 = 1 << 30;

    /// The request's computer: a 12 GB card, a 4 GB one and the processor,
    /// every NVIDIA card with CUDA and OptiX.
    fn mixed() -> Catalogue {
        let mut cycles = vec![("CPU".to_string(), "Processor".to_string(), "CPU".to_string())];
        for (kind, suffix) in [("CUDA", ""), ("OPTIX", "_OptiX")] {
            cycles.push((kind.into(), "NVIDIA GeForce RTX 3060".into(), format!("CUDA_NVIDIA GeForce RTX 3060_0000:01:00{suffix}")));
            cycles.push((kind.into(), "NVIDIA GeForce GTX 1650".into(), format!("CUDA_NVIDIA GeForce GTX 1650_0000:02:00{suffix}")));
        }
        Catalogue::assemble(
            Processor { name: "Processor".into(), threads: 16 },
            vec![
                SystemCard { name: "NVIDIA GeForce RTX 3060".into(), memory: Some(12 * GB), shared_memory: false },
                SystemCard { name: "NVIDIA GeForce GTX 1650".into(), memory: Some(4 * GB), shared_memory: false },
                SystemCard { name: "Intel(R) UHD Graphics 770".into(), memory: None, shared_memory: false },
            ],
            &cycles,
        )
    }

    const BIG: &str = "gpu:NVIDIA GeForce RTX 3060";
    const SMALL: &str = "gpu:NVIDIA GeForce GTX 1650";

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn one_card_is_its_own_id_and_every_card_the_kind() {
        let list = mixed();
        let one = plan(&list, &keys(&[BIG]), Use::Final, false);
        assert_eq!(one.device, "CUDA_NVIDIA GeForce RTX 3060_0000:01:00_OptiX");
        assert_eq!(one.keys, keys(&[BIG]));
        assert!(one.notes.is_empty() && one.on_card());
        // Both: the kind, which every library takes - and what "the
        // graphics card" always meant.
        let both = plan(&list, &keys(&[BIG, SMALL]), Use::View, false);
        assert_eq!(both.device, "OPTIX");
        assert_eq!(both.keys, keys(&[BIG, SMALL]));
    }

    /// The default on a computer with one card is the kind, as it always
    /// was: the same session, on any library.
    #[test]
    fn a_computer_with_one_card_renders_as_it_always_did() {
        let one = Catalogue::assemble(
            Processor { name: "Processor".into(), threads: 12 },
            vec![SystemCard { name: "NVIDIA GeForce RTX 2060".into(), memory: Some(6 * GB), shared_memory: false }],
            &[
                ("CUDA".into(), "NVIDIA GeForce RTX 2060".into(), "CUDA_NVIDIA GeForce RTX 2060_0000:01:00".into()),
                ("OPTIX".into(), "NVIDIA GeForce RTX 2060".into(), "CUDA_NVIDIA GeForce RTX 2060_0000:01:00_OptiX".into()),
            ],
        );
        for purpose in [Use::View, Use::Final] {
            for lists in [false, true] {
                let auto = plan(&one, &one.default_keys(), purpose, lists);
                assert_eq!((auto.device.as_str(), auto.kind.as_deref()), ("OPTIX", Some("OPTIX")));
            }
        }
    }

    #[test]
    fn the_processor_alone_is_cycles_on_the_processor() {
        let list = mixed();
        assert_eq!(plan(&list, &keys(&["cpu"]), Use::View, true), Plan::processor());
        assert_eq!(plan(&list, &keys(&["cpu"]), Use::Final, false).device, "CPU");
        // Only a card Cycles cannot use: the processor, said.
        let intel = plan(&list, &keys(&["gpu:Intel(R) UHD Graphics 770"]), Use::Final, true);
        assert_eq!(intel.device, "CPU");
        assert_eq!(intel.notes.len(), 2, "{:?}", intel.notes);
    }

    #[test]
    fn the_processor_joins_the_cards_for_a_final_render() {
        let list = mixed();
        let hybrid = plan(&list, &keys(&["cpu", BIG]), Use::Final, true);
        assert_eq!(hybrid.device, "CPU\nCUDA_NVIDIA GeForce RTX 3060_0000:01:00_OptiX");
        assert_eq!(hybrid.keys, keys(&["cpu", BIG]));
        // Not the Rendered view's: Cycles leaves it out there.
        let view = plan(&list, &keys(&["cpu", BIG]), Use::View, true);
        assert_eq!(view.device, "CUDA_NVIDIA GeForce RTX 3060_0000:01:00_OptiX");
        assert_eq!(view.keys, keys(&[BIG]));
        assert_eq!(view.notes.len(), 1);
        // An older library: the cards without it, said.
        let old = plan(&list, &keys(&["cpu", BIG, SMALL]), Use::Final, false);
        assert_eq!(old.device, "OPTIX");
        assert_eq!(old.keys, keys(&[BIG, SMALL]));
        assert_eq!(old.notes.len(), 1);
    }

    #[test]
    fn some_of_several_cards_are_named_a_line_each() {
        let mut list = mixed();
        // A third card, so that two of them are not every one.
        list.cards.push(tequila_render_core::devices::Card {
            key: "gpu:NVIDIA GeForce RTX 3060#2".into(),
            name: "NVIDIA GeForce RTX 3060".into(),
            memory: Some(12 * GB),
            shared_memory: false,
            cycles: vec![tequila_render_core::devices::CyclesDevice { kind: "OPTIX".into(), id: "CUDA_NVIDIA GeForce RTX 3060_0000:03:00_OptiX".into() }],
            gpu_backends: Vec::new(),
        });
        let two = plan(&list, &keys(&[BIG, "gpu:NVIDIA GeForce RTX 3060#2"]), Use::Final, true);
        assert_eq!(two.device, "CUDA_NVIDIA GeForce RTX 3060_0000:01:00_OptiX\nCUDA_NVIDIA GeForce RTX 3060_0000:03:00_OptiX");
        // A library without lists: the first alone, never a card turned off.
        let old = plan(&list, &keys(&[BIG, "gpu:NVIDIA GeForce RTX 3060#2"]), Use::Final, false);
        assert_eq!(old.device, "CUDA_NVIDIA GeForce RTX 3060_0000:01:00_OptiX");
        assert_eq!(old.keys, keys(&[BIG]));
        assert_eq!(old.notes.len(), 1);
    }

    #[test]
    fn cards_with_no_kind_in_common_keep_the_first() {
        let mut list = mixed();
        list.cards[1].cycles.retain(|d| d.kind == "CUDA");
        // An OptiX card and a CUDA-only one: CUDA for both.
        let both = plan(&list, &keys(&[BIG, SMALL]), Use::Final, true);
        assert_eq!(both.device, "CUDA");
        list.cards[1].cycles = vec![tequila_render_core::devices::CyclesDevice { kind: "HIP".into(), id: "HIP_x".into() }];
        let apart = plan(&list, &keys(&[BIG, SMALL]), Use::Final, true);
        // The only OptiX card left is every OptiX card: by the kind.
        assert_eq!(apart.device, "OPTIX");
        assert_eq!(apart.keys, keys(&[BIG]));
        assert_eq!(apart.notes.len(), 1);
    }

    #[test]
    fn optix_steps_down_to_cuda() {
        assert_eq!(without_optix("OPTIX").as_deref(), Some("CUDA"));
        assert_eq!(without_optix("CPU\nCUDA_A_0000:01:00_OptiX\nCUDA_B_0000:02:00_OptiX").as_deref(), Some("CPU\nCUDA_A_0000:01:00\nCUDA_B_0000:02:00"));
        assert_eq!(without_optix("CUDA"), None);
        assert_eq!(without_optix("CPU"), None);
    }
}
