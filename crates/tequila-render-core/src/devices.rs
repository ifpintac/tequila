//! Which processor and graphics cards render (`docs/04` §12.2, Rendering).
//!
//! The computer's devices as one list - the processor and each graphics
//! card, with its memory and what each engine reaches it through - and the
//! person's choice from it, made separately for the Rendered view and for
//! final renders. Where they have chosen nothing, or what they chose is no
//! longer in the computer, the choice is made for them ([`Catalogue::default_keys`]).
//!
//! Engine-neutral, and nothing here asks the system anything: the
//! application and the command line gather the facts and hand them to
//! [`Catalogue::assemble`]. What an engine makes of a choice is its own
//! crate's: Cycles' devices in `tequila-render-cycles` (`devices::plan`),
//! the graphics-card renderer's adapter in `tequila-render-gpu`. A choice is
//! remembered per computer, by the application, never in a document, which
//! travels between computers.

/// The processor's key in a choice.
pub const PROCESSOR: &str = "cpu";

/// What a card key starts with.
const CARD: &str = "gpu:";

/// The kinds of graphics card Cycles renders on, best first: OptiX on an
/// NVIDIA card's ray-tracing cores, CUDA on any NVIDIA card, HIP on an AMD
/// one, Metal on a Mac.
pub const CYCLES_CARD_KINDS: [&str; 4] = ["OPTIX", "CUDA", "HIP", "METAL"];

/// What a choice of devices is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Use {
    /// The Rendered view.
    View,
    /// Final renders: the Render window, the queue, scripts.
    Final,
}

/// The processor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Processor {
    pub name: String,
    pub threads: usize,
}

/// How Cycles reaches a card: its kind (OPTIX, CUDA, HIP, METAL) and the id
/// a session is opened on to render on that card alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CyclesDevice {
    pub kind: String,
    pub id: String,
}

/// A graphics card.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// What a choice remembers it by: `gpu:` and its name, and `#2`, `#3`
    /// after the second and third of the same name.
    pub key: String,
    pub name: String,
    /// Its own memory, in bytes; `None` where it is not known, or where it
    /// shares the computer's ([`Card::shared_memory`]).
    pub memory: Option<u64>,
    /// It shares the computer's memory, as Apple's graphics do.
    pub shared_memory: bool,
    /// How Cycles reaches it, best first; none where Cycles cannot render
    /// on it.
    pub cycles: Vec<CyclesDevice>,
    /// What Tequila's own graphics-card renderer reaches it through
    /// ("Vulkan", "Metal"), where that was asked; empty in the application,
    /// which does not render on the card itself yet (`docs/07` R4).
    pub gpu_backends: Vec<String>,
}

impl Card {
    /// The Cycles device of `kind` for this card.
    pub fn cycles_device(&self, kind: &str) -> Option<&CyclesDevice> {
        self.cycles.iter().find(|d| d.kind == kind)
    }

    /// Whether Cycles' graphics-card engine can render on it.
    pub fn cycles_renders(&self) -> bool {
        self.best_cycles_kind().is_some()
    }

    /// The best of the kinds Cycles reaches it through.
    pub fn best_cycles_kind(&self) -> Option<&'static str> {
        CYCLES_CARD_KINDS.into_iter().find(|kind| self.cycles_device(kind).is_some())
    }

    /// Its name without its maker's, which is in the list's other words:
    /// "GeForce RTX 3060".
    pub fn short_name(&self) -> String {
        short(&self.name)
    }
}

/// A card as the system lists it, before Cycles' devices are matched to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemCard {
    pub name: String,
    pub memory: Option<u64>,
    pub shared_memory: bool,
}

/// Every device here that could render.
#[derive(Clone, Debug, PartialEq)]
pub struct Catalogue {
    pub processor: Processor,
    /// Those Cycles renders on first, the most memory first among them;
    /// the rest as the system listed them.
    pub cards: Vec<Card>,
}

/// A card's name without the maker's: "NVIDIA GeForce RTX 3060" is
/// "GeForce RTX 3060".
pub fn short(name: &str) -> String {
    name.trim().trim_start_matches("NVIDIA ").trim_start_matches("AMD ").to_string()
}

/// A card's name as Cycles describes it, without the " (Display)" it adds
/// to a card that drives a screen and has no compute preemption.
fn plain(description: &str) -> &str {
    let description = description.trim();
    description.strip_suffix("(Display)").map_or(description, str::trim_end)
}

/// The name a key remembers: "gpu:NVIDIA GeForce RTX 3060#2" is "NVIDIA
/// GeForce RTX 3060"; the processor's is "the processor".
pub fn name_in(key: &str) -> String {
    match key.strip_prefix(CARD) {
        Some(card) => match card.rsplit_once('#') {
            Some((name, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => name.to_string(),
            _ => card.to_string(),
        },
        None if key == PROCESSOR => "the processor".to_string(),
        None => key.to_string(),
    }
}

impl Catalogue {
    /// The list, from the processor, the cards the system lists, and the
    /// devices Cycles found as `fsc_devices` lists them: (kind,
    /// description, id). A Cycles device goes to the card of its name - the
    /// first of that name it is not on yet, so two cards of one model each
    /// get their own - or, where the system did not list one (a driver it
    /// does not know), to a card of its own with its memory unknown.
    pub fn assemble(processor: Processor, system: Vec<SystemCard>, cycles: &[(String, String, String)]) -> Catalogue {
        let mut cards: Vec<Card> = system
            .into_iter()
            .filter(|card| !card.name.trim().is_empty())
            .map(|card| Card {
                key: String::new(),
                name: card.name.trim().to_string(),
                memory: card.memory.filter(|_| !card.shared_memory),
                shared_memory: card.shared_memory,
                cycles: Vec::new(),
                gpu_backends: Vec::new(),
            })
            .collect();
        for (kind, description, id) in cycles {
            let kind = kind.trim().to_ascii_uppercase();
            if kind == "CPU" || kind.is_empty() {
                continue;
            }
            let name = plain(description);
            let device = CyclesDevice { kind: kind.clone(), id: id.trim().to_string() };
            match cards.iter_mut().find(|c| c.name.eq_ignore_ascii_case(name) && c.cycles_device(&kind).is_none()) {
                Some(card) => card.cycles.push(device),
                None => cards.push(Card {
                    key: String::new(),
                    name: name.to_string(),
                    memory: None,
                    // Apple's graphics are listed by Cycles alone, and share
                    // the computer's memory.
                    shared_memory: name.starts_with("Apple "),
                    cycles: vec![device],
                    gpu_backends: Vec::new(),
                }),
            }
        }
        for card in &mut cards {
            card.cycles.sort_by_key(|d| CYCLES_CARD_KINDS.iter().position(|k| *k == d.kind).unwrap_or(CYCLES_CARD_KINDS.len()));
        }
        // Stable: two cards of one model keep the system's order.
        cards.sort_by_key(|card| (!card.cycles_renders(), std::cmp::Reverse(card.memory.unwrap_or(0))));
        let mut catalogue = Catalogue { processor, cards };
        catalogue.name_keys();
        catalogue
    }

    /// The cards' keys: by name, numbered from the second of a name.
    fn name_keys(&mut self) {
        for i in 0..self.cards.len() {
            let before = self.cards[..i].iter().filter(|c| c.name == self.cards[i].name).count();
            let name = self.cards[i].name.clone();
            self.cards[i].key = if before == 0 { format!("{CARD}{name}") } else { format!("{CARD}{name}#{}", before + 1) };
        }
    }

    /// Add what Tequila's own graphics-card renderer reaches each card
    /// through: (adapter name, backend), as wgpu lists them. An adapter
    /// goes to the card of its name, as Cycles' devices do; one the list
    /// has no card for (a card only Vulkan sees) gets a card of its own.
    pub fn add_gpu_backends(&mut self, adapters: &[(String, String)]) {
        let mut taken: Vec<(usize, String)> = Vec::new();
        for (name, backend) in adapters {
            let name = name.trim();
            let found = (0..self.cards.len()).find(|&i| self.cards[i].name.eq_ignore_ascii_case(name) && !taken.contains(&(i, backend.clone())));
            let index = match found {
                Some(i) => i,
                None => {
                    self.cards.push(Card {
                        key: String::new(),
                        name: name.to_string(),
                        memory: None,
                        shared_memory: name.starts_with("Apple "),
                        cycles: Vec::new(),
                        gpu_backends: Vec::new(),
                    });
                    self.cards.len() - 1
                }
            };
            taken.push((index, backend.clone()));
            if !self.cards[index].gpu_backends.contains(backend) {
                self.cards[index].gpu_backends.push(backend.clone());
            }
        }
        self.name_keys();
    }

    /// The card with this key.
    pub fn card(&self, key: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.key == key)
    }

    /// Every key, the processor's first, in the list's order.
    pub fn keys(&self) -> Vec<String> {
        std::iter::once(PROCESSOR.to_string()).chain(self.cards.iter().map(|c| c.key.clone())).collect()
    }

    /// What is chosen when the person has chosen nothing.
    ///
    /// The graphics cards of the best kind Cycles reaches - as rendering on
    /// "the graphics card" always took every one of them - but only those
    /// of the biggest one's class: a card with less than half the memory of
    /// the biggest is left out. Cycles renders a scene on several cards
    /// only when it fits on each, so a small card beside a big one limits
    /// what the big one can render, and with every card restarting at each
    /// change of the Rendered view it holds the big one back there too.
    /// Cards of one model, or near enough, all render, as before. Where the
    /// memory is not known, every card of the kind does, as before.
    ///
    /// Not the processor beside a card: next to a capable card it adds
    /// little, takes every core the rest of the computer wants, and the
    /// card waits for its slower samples. The processor alone where there
    /// is no card Cycles can use.
    pub fn default_keys(&self) -> Vec<String> {
        let Some(kind) = CYCLES_CARD_KINDS.into_iter().find(|kind| self.cards.iter().any(|c| c.cycles_device(kind).is_some())) else {
            return vec![PROCESSOR.to_string()];
        };
        let candidates: Vec<&Card> = self.cards.iter().filter(|c| c.cycles_device(kind).is_some()).collect();
        let largest = candidates.iter().filter_map(|c| c.memory).max();
        candidates
            .into_iter()
            .filter(|c| match (c.memory, largest) {
                (Some(memory), Some(largest)) => memory.saturating_mul(2) >= largest,
                _ => true,
            })
            .map(|c| c.key.clone())
            .collect()
    }

    /// The devices `choice` stands for here: what it names that is here,
    /// in the list's order; a card it names that is gone is found by its
    /// name where another of that model is here and not chosen. If nothing
    /// it names is here, or it names nothing, the default
    /// ([`Catalogue::default_keys`]) - so at least one device always
    /// renders.
    pub fn resolve(&self, choice: &Choice) -> Resolved {
        let names = match choice {
            Choice::Auto => return Resolved { keys: self.default_keys(), missing: Vec::new(), defaulted: false },
            Choice::Chosen(keys) => keys,
        };
        let mut keys: Vec<String> = Vec::new();
        let mut missing = Vec::new();
        for key in names {
            let here = self.keys().into_iter().find(|k| k == key && !keys.contains(k)).or_else(|| {
                key.starts_with(CARD)
                    .then(|| name_in(key))
                    .and_then(|name| self.cards.iter().find(|c| c.name == name && !keys.contains(&c.key) && !names.contains(&c.key)))
                    .map(|c| c.key.clone())
            });
            match here {
                Some(key) => keys.push(key),
                None if keys.iter().any(|k| k == key) => {}
                None => missing.push(name_in(key)),
            }
        }
        if keys.is_empty() {
            return Resolved { keys: self.default_keys(), missing, defaulted: true };
        }
        let order = self.keys();
        keys.sort_by_key(|k| order.iter().position(|o| o == k));
        Resolved { keys, missing, defaulted: false }
    }

    /// `choice` with one device turned on or off. From the default, the
    /// default's devices are the start; a choice that comes out the same as
    /// the default is the default again, so it keeps following the
    /// computer. The last device on cannot be turned off: the same choice
    /// comes back.
    pub fn toggled(&self, choice: &Choice, key: &str, on: bool) -> Choice {
        let mut keys = self.resolve(choice).keys;
        if on {
            if !keys.iter().any(|k| k == key) {
                keys.push(key.to_string());
            }
        } else {
            if keys.len() == 1 && keys[0] == key {
                return choice.clone();
            }
            keys.retain(|k| k != key);
        }
        let order = self.keys();
        keys.sort_by_key(|k| order.iter().position(|o| o == k));
        if keys == self.default_keys() {
            Choice::Auto
        } else {
            Choice::Chosen(keys)
        }
    }

    /// A device's name, short: "Processor", "GeForce RTX 3060".
    pub fn short_name(&self, key: &str) -> String {
        match self.card(key) {
            Some(card) => card.short_name(),
            None if key == PROCESSOR => "Processor".to_string(),
            None => short(&name_in(key)),
        }
    }

    /// Devices in a few words, for the status bar and the render window:
    /// "GeForce RTX 3060", "GeForce RTX 3060 + processor", "2 graphics
    /// cards", "processor".
    pub fn summary(&self, keys: &[String]) -> String {
        let processor = keys.iter().any(|k| k == PROCESSOR);
        let cards: Vec<&String> = keys.iter().filter(|k| *k != PROCESSOR).collect();
        match (cards.as_slice(), processor) {
            ([], _) => "processor".to_string(),
            ([one], false) => self.short_name(one),
            ([one], true) => format!("{} + processor", self.short_name(one)),
            (many, false) => format!("{} graphics cards", many.len()),
            (many, true) => format!("{} graphics cards + processor", many.len()),
        }
    }
}

/// Which devices the person chose for one use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Choice {
    /// None: Tequila chooses ([`Catalogue::default_keys`]).
    #[default]
    Auto,
    /// These keys ([`PROCESSOR`], [`Card::key`]), in the list's order.
    Chosen(Vec<String>),
}

impl Choice {
    /// From the settings file or a script: `auto`, or keys between
    /// semicolons, `cpu; gpu:NVIDIA GeForce RTX 3060`. Card names hold no
    /// semicolons. Nothing named is an error: one device at least renders.
    pub fn parse(text: &str) -> Result<Choice, String> {
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("auto") {
            return Ok(Choice::Auto);
        }
        let mut keys: Vec<String> = Vec::new();
        for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let key = if part.eq_ignore_ascii_case(PROCESSOR) {
                PROCESSOR.to_string()
            } else if part.len() > CARD.len() && part[..CARD.len()].eq_ignore_ascii_case(CARD) {
                format!("{CARD}{}", part[CARD.len()..].trim())
            } else {
                return Err(format!("`{part}` is not a device: devices are `cpu` or `gpu:` and a graphics card's name"));
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        if keys.is_empty() {
            return Err("at least one device renders: `auto`, `cpu`, or `gpu:` and a graphics card's name".to_string());
        }
        Ok(Choice::Chosen(keys))
    }

    /// As the settings file holds it.
    pub fn to_text(&self) -> String {
        match self {
            Choice::Auto => "auto".to_string(),
            Choice::Chosen(keys) => keys.join("; "),
        }
    }
}

/// What a choice stands for on this computer ([`Catalogue::resolve`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The devices, at least one, in the list's order.
    pub keys: Vec<String>,
    /// What it named that is not here, by name.
    pub missing: Vec<String>,
    /// Nothing it named is here, so the default renders.
    pub defaulted: bool,
}

impl Resolved {
    /// Whether the processor is among them.
    pub fn processor(&self) -> bool {
        self.keys.iter().any(|k| k == PROCESSOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn cpu() -> Processor {
        Processor { name: "AMD Ryzen 9 5900X 12-Core Processor".into(), threads: 24 }
    }

    fn system(name: &str, gb: u64) -> SystemCard {
        SystemCard { name: name.into(), memory: Some(gb * GB), shared_memory: false }
    }

    /// Cycles' list as `fsc_devices` gives it, for NVIDIA cards: a CUDA
    /// device and an OptiX one each.
    fn nvidia(cards: &[(&str, u32)]) -> Vec<(String, String, String)> {
        let mut out = vec![("CPU".to_string(), cpu().name, "CPU".to_string())];
        for (name, bus) in cards {
            out.push(("CUDA".into(), name.to_string(), format!("CUDA_{name}_0000:{bus:02x}:00")));
        }
        for (name, bus) in cards {
            out.push(("OPTIX".into(), name.to_string(), format!("CUDA_{name}_0000:{bus:02x}:00_OptiX")));
        }
        out
    }

    #[test]
    fn cycles_devices_go_to_the_cards_of_their_names() {
        let list = Catalogue::assemble(
            cpu(),
            vec![system("NVIDIA GeForce GTX 1650", 4), system("Intel(R) UHD Graphics 770", 0), system("NVIDIA GeForce RTX 3060", 12)],
            &nvidia(&[("NVIDIA GeForce RTX 3060", 1), ("NVIDIA GeForce GTX 1650 (Display)", 2)]),
        );
        let names: Vec<&str> = list.cards.iter().map(|c| c.name.as_str()).collect();
        // Cycles' cards first, the most memory first; then the rest.
        assert_eq!(names, ["NVIDIA GeForce RTX 3060", "NVIDIA GeForce GTX 1650", "Intel(R) UHD Graphics 770"]);
        let big = &list.cards[0];
        assert_eq!(big.key, "gpu:NVIDIA GeForce RTX 3060");
        assert_eq!(big.memory, Some(12 * GB));
        assert_eq!(big.best_cycles_kind(), Some("OPTIX"), "OptiX before CUDA");
        assert_eq!(big.cycles_device("CUDA").unwrap().id, "CUDA_NVIDIA GeForce RTX 3060_0000:01:00");
        assert_eq!(list.cards[1].cycles.len(), 2, "\" (Display)\" is not part of the name");
        assert!(!list.cards[2].cycles_renders(), "Cycles does not render on Intel's graphics here");
        assert_eq!(list.keys(), ["cpu", "gpu:NVIDIA GeForce RTX 3060", "gpu:NVIDIA GeForce GTX 1650", "gpu:Intel(R) UHD Graphics 770"]);
    }

    #[test]
    fn two_cards_of_one_model_each_have_their_own_key_and_devices() {
        let list = Catalogue::assemble(
            cpu(),
            vec![system("NVIDIA GeForce RTX 4090", 24), system("NVIDIA GeForce RTX 4090", 24)],
            &nvidia(&[("NVIDIA GeForce RTX 4090", 1), ("NVIDIA GeForce RTX 4090", 2)]),
        );
        assert_eq!(list.cards[0].key, "gpu:NVIDIA GeForce RTX 4090");
        assert_eq!(list.cards[1].key, "gpu:NVIDIA GeForce RTX 4090#2");
        assert!(list.cards[0].cycles_device("OPTIX").unwrap().id.contains("0000:01:00"));
        assert!(list.cards[1].cycles_device("OPTIX").unwrap().id.contains("0000:02:00"));
        assert_eq!(name_in("gpu:NVIDIA GeForce RTX 4090#2"), "NVIDIA GeForce RTX 4090");
    }

    #[test]
    fn a_card_only_cycles_lists_is_still_listed() {
        // A Mac: the system's list is not read; Cycles names the graphics.
        let mac = Catalogue::assemble(Processor { name: "Apple M3 Max".into(), threads: 16 }, Vec::new(), &[
            ("CPU".into(), "Apple M3 Max".into(), "CPU".into()),
            ("METAL".into(), "Apple M3 Max (GPU - 40 cores)".into(), "METAL_Apple M3 Max (GPU - 40 cores)".into()),
        ]);
        assert_eq!(mac.cards.len(), 1);
        assert!(mac.cards[0].shared_memory && mac.cards[0].memory.is_none());
        assert_eq!(mac.default_keys(), ["gpu:Apple M3 Max (GPU - 40 cores)"]);
    }

    #[test]
    fn the_default_is_the_big_card_without_a_small_one_or_the_processor() {
        // One card: it, alone.
        let one = Catalogue::assemble(cpu(), vec![system("NVIDIA GeForce RTX 2060", 6)], &nvidia(&[("NVIDIA GeForce RTX 2060", 1)]));
        assert_eq!(one.default_keys(), ["gpu:NVIDIA GeForce RTX 2060"]);
        // The request's computer: a 4 GB card held a 12 GB one back.
        let mixed = Catalogue::assemble(
            cpu(),
            vec![system("NVIDIA GeForce GTX 1650", 4), system("NVIDIA GeForce RTX 3060", 12)],
            &nvidia(&[("NVIDIA GeForce GTX 1650", 1), ("NVIDIA GeForce RTX 3060", 2)]),
        );
        assert_eq!(mixed.default_keys(), ["gpu:NVIDIA GeForce RTX 3060"]);
        // Cards of a class render together, as before.
        let pair = Catalogue::assemble(
            cpu(),
            vec![system("NVIDIA GeForce RTX 3080", 10), system("NVIDIA GeForce RTX 3060", 12)],
            &nvidia(&[("NVIDIA GeForce RTX 3080", 1), ("NVIDIA GeForce RTX 3060", 2)]),
        );
        assert_eq!(pair.default_keys(), ["gpu:NVIDIA GeForce RTX 3060", "gpu:NVIDIA GeForce RTX 3080"]);
        // Memory unknown: every card of the kind, as before.
        let unknown = Catalogue::assemble(cpu(), Vec::new(), &nvidia(&[("NVIDIA GeForce GTX 1650", 1), ("NVIDIA GeForce RTX 3060", 2)]));
        assert_eq!(unknown.default_keys().len(), 2);
        // No card Cycles can use: the processor.
        let none = Catalogue::assemble(cpu(), vec![system("Intel(R) UHD Graphics 770", 0)], &[("CPU".into(), cpu().name, "CPU".into())]);
        assert_eq!(none.default_keys(), ["cpu"]);
        let bare = Catalogue::assemble(cpu(), Vec::new(), &[]);
        assert_eq!(bare.default_keys(), ["cpu"]);
    }

    #[test]
    fn the_default_keeps_to_the_best_kind() {
        // An NVIDIA card with OptiX and an older one with CUDA alone: every
        // card of the best kind, which "the graphics card" always took.
        let mut cycles = nvidia(&[("NVIDIA GeForce RTX 3060", 1)]);
        cycles.push(("CUDA".into(), "NVIDIA GeForce GTX 780".into(), "CUDA_NVIDIA GeForce GTX 780_0000:02:00".into()));
        cycles.push(("HIP".into(), "AMD Radeon RX 7900 XTX".into(), "HIP_AMD Radeon RX 7900 XTX_0000:03:00".into()));
        let list = Catalogue::assemble(cpu(), Vec::new(), &cycles);
        assert_eq!(list.default_keys(), ["gpu:NVIDIA GeForce RTX 3060"]);
        // AMD alone: HIP.
        let amd = Catalogue::assemble(cpu(), vec![system("AMD Radeon RX 7900 XTX", 24)], &[("HIP".into(), "AMD Radeon RX 7900 XTX".into(), "HIP_x".into())]);
        assert_eq!(amd.default_keys(), ["gpu:AMD Radeon RX 7900 XTX"]);
    }

    fn mixed() -> Catalogue {
        Catalogue::assemble(
            cpu(),
            vec![system("NVIDIA GeForce GTX 1650", 4), system("NVIDIA GeForce RTX 3060", 12)],
            &nvidia(&[("NVIDIA GeForce GTX 1650", 1), ("NVIDIA GeForce RTX 3060", 2)]),
        )
    }

    #[test]
    fn a_choice_reads_back_as_it_was_written() {
        let choice = Choice::Chosen(vec!["cpu".into(), "gpu:NVIDIA GeForce RTX 3060".into()]);
        assert_eq!(choice.to_text(), "cpu; gpu:NVIDIA GeForce RTX 3060");
        assert_eq!(Choice::parse(&choice.to_text()), Ok(choice));
        assert_eq!(Choice::parse("auto"), Ok(Choice::Auto));
        assert_eq!(Choice::parse(" "), Ok(Choice::Auto));
        assert_eq!(Choice::parse("CPU;GPU:NVIDIA GeForce RTX 3060;cpu"), Ok(Choice::Chosen(vec!["cpu".into(), "gpu:NVIDIA GeForce RTX 3060".into()])));
        assert!(Choice::parse("toaster").is_err());
        assert!(Choice::parse(";;").is_err(), "nothing named");
    }

    #[test]
    fn a_choice_resolves_to_what_is_here() {
        let list = mixed();
        let both = Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 3060".into(), "cpu".into()]);
        let resolved = list.resolve(&both);
        assert_eq!(resolved.keys, ["cpu", "gpu:NVIDIA GeForce RTX 3060"], "in the list's order");
        assert!(resolved.missing.is_empty() && !resolved.defaulted && resolved.processor());
        assert_eq!(list.resolve(&Choice::Auto).keys, list.default_keys());
    }

    #[test]
    fn a_card_that_is_gone_falls_back() {
        let list = mixed();
        // The 12 GB card was taken out; the 4 GB one stays chosen.
        let gone = Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 4090".into(), "gpu:NVIDIA GeForce GTX 1650".into()]);
        let resolved = list.resolve(&gone);
        assert_eq!(resolved.keys, ["gpu:NVIDIA GeForce GTX 1650"]);
        assert_eq!(resolved.missing, ["NVIDIA GeForce RTX 4090"]);
        // Nothing chosen is here: the default renders, and says so.
        let all_gone = list.resolve(&Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 4090".into()]));
        assert_eq!(all_gone.keys, list.default_keys());
        assert!(all_gone.defaulted);
        // The second of two identical cards was taken out: the first is
        // the one chosen, found by its name.
        let one = Catalogue::assemble(cpu(), vec![system("NVIDIA GeForce RTX 4090", 24)], &nvidia(&[("NVIDIA GeForce RTX 4090", 1)]));
        let second = one.resolve(&Choice::Chosen(vec!["gpu:NVIDIA GeForce RTX 4090#2".into()]));
        assert_eq!(second.keys, ["gpu:NVIDIA GeForce RTX 4090"]);
        assert!(second.missing.is_empty() && !second.defaulted);
    }

    #[test]
    fn at_least_one_device_is_always_on() {
        let list = mixed();
        let only = Choice::Chosen(vec!["cpu".into()]);
        assert_eq!(list.toggled(&only, "cpu", false), only, "the last device stays on");
        // From the default: its card is the start.
        let with_cpu = list.toggled(&Choice::Auto, "cpu", true);
        assert_eq!(with_cpu, Choice::Chosen(vec!["cpu".into(), "gpu:NVIDIA GeForce RTX 3060".into()]));
        let cpu_alone = list.toggled(&with_cpu, "gpu:NVIDIA GeForce RTX 3060", false);
        assert_eq!(cpu_alone, only);
        assert_eq!(list.toggled(&cpu_alone, "gpu:NVIDIA GeForce RTX 3060", false), only);
        // Back to the default's devices: the default again.
        assert_eq!(list.toggled(&with_cpu, "cpu", false), Choice::Auto);
        assert!(!list.resolve(&Choice::Chosen(Vec::new())).keys.is_empty());
    }

    #[test]
    fn devices_are_summed_up_in_a_few_words() {
        let list = mixed();
        let big = "gpu:NVIDIA GeForce RTX 3060".to_string();
        let small = "gpu:NVIDIA GeForce GTX 1650".to_string();
        assert_eq!(list.summary(&[big.clone()]), "GeForce RTX 3060");
        assert_eq!(list.summary(&["cpu".into(), big.clone()]), "GeForce RTX 3060 + processor");
        assert_eq!(list.summary(&[big.clone(), small.clone()]), "2 graphics cards");
        assert_eq!(list.summary(&["cpu".into(), big, small]), "2 graphics cards + processor");
        assert_eq!(list.summary(&["cpu".into()]), "processor");
    }

    #[test]
    fn the_graphics_card_renderers_adapters_join_their_cards() {
        let mut list = mixed();
        list.add_gpu_backends(&[
            ("NVIDIA GeForce RTX 3060".into(), "Vulkan".into()),
            ("NVIDIA GeForce GTX 1650".into(), "Vulkan".into()),
            ("llvmpipe (LLVM 15.0.7, 256 bits)".into(), "Vulkan".into()),
        ]);
        assert_eq!(list.cards[0].gpu_backends, ["Vulkan"]);
        assert_eq!(list.cards[1].gpu_backends, ["Vulkan"]);
        assert_eq!(list.cards.len(), 3, "an adapter with no card has one of its own");
        assert_eq!(list.cards[2].key, "gpu:llvmpipe (LLVM 15.0.7, 256 bits)");
    }
}
