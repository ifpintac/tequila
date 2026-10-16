//! Light path expressions (`docs/01-RENDERER-CORE.md` §5.5, §12).
//!
//! §12 is explicit about why this exists: the render passes are "implemented
//! via light path expressions, so they are consistent by construction rather
//! than separately hand-computed". That consistency is the whole point and it
//! is not a nicety — M4's exit criterion is that Beauty equals the sum of its
//! components to floating-point tolerance, and a set of passes each computed
//! its own way will never satisfy that. Here Beauty is `C.*`, Diffuse is
//! `C<RD>.*`, and a contribution is added to both by the same match against the
//! same path.
//!
//! # Notation
//!
//! Heckbert's, as §12's table uses it. A path is written from the camera:
//!
//! - `C` — the camera. Every path starts with one.
//! - `L` — a light. Every path that contributes ends with one. `L0` to `L7`
//!   match only a light on that light layer (§5.5), `LE` only the
//!   environment — sky, image and pins together — and `LM` only a surface
//!   whose material glows.
//! - `<XY>` — a scattering event, where `X` is `R` (reflection) or `T`
//!   (transmission) and `Y` is `D` (diffuse), `S` (specular/delta) or `G`
//!   (glossy). Either may be `.` for "any".
//! - `[DSG]` — an event whose scattering kind is one of these.
//! - `.` — any event at all.
//! - `*` and `+` — the usual repetition, postfix.
//!
//! # Why this is matched incrementally
//!
//! The obvious implementation records a path's event string and matches it when
//! the path ends. That costs a full match per contribution per AOV, and a
//! 1080p render at 128 samples makes something like a billion of them.
//!
//! So the matcher instead carries the set of NFA states each expression could
//! be in, advances every one of them by a single event as the path grows, and
//! asks at a contribution whether appending `L` would reach an accepting state.
//! One machine word per expression, one step per bounce, and the answer at a
//! contribution is a mask test. See [`LpeMatcher`].

use std::fmt;

/// How light left a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Reflect,
    Transmit,
}

/// What kind of scattering it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scatter {
    Diffuse,
    /// A delta lobe: a mirror, or smooth glass.
    Specular,
    Glossy,
}

/// Which light a path ended at, for §5.5's light layers: a layer number
/// from 0 to 7, [`ENVIRONMENT`], or [`EMISSIVE`].
pub type LightGroup = u8;

/// The environment: image, sky and pins.
pub const ENVIRONMENT: LightGroup = 8;
/// A surface whose own material emits.
pub const EMISSIVE: LightGroup = 9;
/// Light layers run from 0 to this.
pub const LAST_LAYER: LightGroup = 7;

/// One event along a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Camera,
    Scatter(Transport, Scatter),
    Light(LightGroup),
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Camera => write!(f, "C"),
            Event::Light(ENVIRONMENT) => write!(f, "LE"),
            Event::Light(EMISSIVE) => write!(f, "LM"),
            Event::Light(layer) => write!(f, "L{layer}"),
            Event::Scatter(transport, scatter) => write!(
                f,
                "<{}{}>",
                match transport {
                    Transport::Reflect => 'R',
                    Transport::Transmit => 'T',
                },
                match scatter {
                    Scatter::Diffuse => 'D',
                    Scatter::Specular => 'S',
                    Scatter::Glossy => 'G',
                }
            ),
        }
    }
}

/// What a pattern's single-event atom accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Atom {
    Camera,
    /// A light, of one group or of any.
    Light(Option<LightGroup>),
    /// A scattering event. `None` in either field means "any".
    Scatter {
        transport: Option<Transport>,
        /// A set of accepted kinds, as a bitmask over `Scatter`.
        kinds: u8,
    },
    /// Any event whatsoever, including `C` and `L`.
    Any,
}

const DIFFUSE: u8 = 1;
const SPECULAR: u8 = 2;
const GLOSSY: u8 = 4;
const ALL_KINDS: u8 = DIFFUSE | SPECULAR | GLOSSY;

impl Atom {
    fn accepts(&self, event: Event) -> bool {
        match (self, event) {
            (Atom::Any, _) => true,
            (Atom::Camera, Event::Camera) => true,
            (Atom::Light(want), Event::Light(group)) => want.is_none_or(|w| w == group),
            (Atom::Scatter { transport, kinds }, Event::Scatter(t, s)) => {
                if let Some(want) = transport {
                    if *want != t {
                        return false;
                    }
                }
                let bit = match s {
                    Scatter::Diffuse => DIFFUSE,
                    Scatter::Specular => SPECULAR,
                    Scatter::Glossy => GLOSSY,
                };
                kinds & bit != 0
            }
            _ => false,
        }
    }
}

/// The most states one expression may compile to.
///
/// Sixty-four so the state set is a single `u64` — which is what makes stepping
/// every expression at every bounce cheap enough to leave switched on. No
/// expression in §12's table needs more than a dozen.
pub const MAX_STATES: usize = 64;

/// A compiled light path expression.
///
/// A Thompson NFA: each state consumes one event and moves to a set of others,
/// with the epsilon closures already folded in so stepping is a table lookup
/// and an OR.
#[derive(Clone, Debug)]
pub struct Lpe {
    source: String,
    /// For each state, the atom it consumes and the closure of states reachable
    /// after consuming it.
    transitions: Vec<(Atom, u64)>,
    /// The states the machine starts in.
    start: u64,
    /// The states that accept.
    accepting: u64,
}

impl Lpe {
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Compile an expression.
    pub fn parse(source: &str) -> Result<Lpe, String> {
        let mut builder = Builder {
            fragments: Vec::new(),
        };
        let mut parser = Parser {
            chars: source.chars().peekable(),
            source,
        };
        let fragment = parser.parse_sequence(&mut builder, None)?;
        if parser.chars.peek().is_some() {
            return Err(format!("`{source}`: unexpected trailing input"));
        }

        // Fold the epsilon closures away. A fragment is a set of entry states
        // and a map from state to its exits; the closure is already maintained
        // by the builder, so all that is left is to name the pieces.
        if builder.fragments.len() > MAX_STATES {
            return Err(format!(
                "`{source}` needs {} states and the matcher holds {MAX_STATES}",
                builder.fragments.len()
            ));
        }
        Ok(Lpe {
            source: source.to_string(),
            transitions: builder.fragments,
            start: fragment.entry,
            accepting: fragment.accept,
        })
    }

    /// Consume one event.
    ///
    /// Returns the states that *did* the consuming and the states ready for the
    /// next event. Both are needed, and conflating them is the mistake this
    /// signature exists to prevent: acceptance is a property of the state that
    /// consumed the final event, not of where the machine ended up afterwards.
    /// A machine that has just consumed its last atom has nowhere left to go,
    /// so testing the *next* set against the accepting set finds nothing and
    /// every expression matches nothing at all.
    #[inline]
    fn step(&self, states: u64, event: Event) -> (u64, u64) {
        let mut consumed = 0u64;
        let mut next = 0u64;
        let mut remaining = states;
        while remaining != 0 {
            let index = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let (atom, exits) = &self.transitions[index];
            if atom.accepts(event) {
                consumed |= 1 << index;
                next |= exits;
            }
        }
        (consumed, next)
    }

    /// Whether the expression matches a whole path.
    ///
    /// The straightforward form, for tests and for anything not on the hot
    /// path. Rendering uses [`LpeMatcher`], which is this one step at a time.
    pub fn matches(&self, path: &[Event]) -> bool {
        let mut states = self.start;
        let mut consumed = 0u64;
        for event in path {
            let (just_consumed, next) = self.step(states, *event);
            if just_consumed == 0 {
                return false;
            }
            consumed = just_consumed;
            states = next;
        }
        consumed & self.accepting != 0
    }
}

/// A fragment under construction: the states it can be entered in, and the
/// states that accept if the fragment ends the expression.
#[derive(Clone, Copy, Debug)]
struct Fragment {
    entry: u64,
    accept: u64,
    /// True when the fragment matches the empty string, so a sequence before it
    /// keeps its own accepting states.
    nullable: bool,
}

struct Builder {
    fragments: Vec<(Atom, u64)>,
}

impl Builder {
    /// A fragment for one atom: a single state, with no exits yet.
    fn atom(&mut self, atom: Atom) -> Fragment {
        let index = self.fragments.len();
        self.fragments.push((atom, 0));
        Fragment {
            entry: 1 << index,
            accept: 1 << index,
            nullable: false,
        }
    }

    /// `a` then `b`.
    fn concat(&mut self, a: Fragment, b: Fragment) -> Fragment {
        // Everything that accepted `a` now continues into `b`.
        self.add_exits(a.accept, b.entry);
        Fragment {
            entry: if a.nullable { a.entry | b.entry } else { a.entry },
            accept: if b.nullable { a.accept | b.accept } else { b.accept },
            nullable: a.nullable && b.nullable,
        }
    }

    /// `a*`: zero or more.
    fn star(&mut self, a: Fragment) -> Fragment {
        self.add_exits(a.accept, a.entry);
        Fragment {
            entry: a.entry,
            accept: a.accept,
            nullable: true,
        }
    }

    /// `a+`: one or more.
    fn plus(&mut self, a: Fragment) -> Fragment {
        self.add_exits(a.accept, a.entry);
        Fragment {
            entry: a.entry,
            accept: a.accept,
            nullable: a.nullable,
        }
    }

    fn add_exits(&mut self, from: u64, to: u64) {
        let mut remaining = from;
        while remaining != 0 {
            let index = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            self.fragments[index].1 |= to;
        }
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    source: &'a str,
}

impl Parser<'_> {
    /// A sequence of repeated atoms, up to `stop`.
    fn parse_sequence(
        &mut self,
        builder: &mut Builder,
        stop: Option<char>,
    ) -> Result<Fragment, String> {
        let mut result: Option<Fragment> = None;
        loop {
            if let Some(stop) = stop {
                if self.chars.peek() == Some(&stop) {
                    break;
                }
            }
            let Some(&c) = self.chars.peek() else { break };
            if c == ')' && stop != Some(')') {
                break;
            }
            let atom = self.parse_repeat(builder)?;
            result = Some(match result {
                Some(previous) => builder.concat(previous, atom),
                None => atom,
            });
        }
        result.ok_or_else(|| format!("`{}`: an empty expression", self.source))
    }

    /// One atom, with any `*` or `+` after it.
    fn parse_repeat(&mut self, builder: &mut Builder) -> Result<Fragment, String> {
        let mut fragment = self.parse_atom(builder)?;
        loop {
            match self.chars.peek() {
                Some('*') => {
                    self.chars.next();
                    fragment = builder.star(fragment);
                }
                Some('+') => {
                    self.chars.next();
                    fragment = builder.plus(fragment);
                }
                _ => break,
            }
        }
        Ok(fragment)
    }

    fn parse_atom(&mut self, builder: &mut Builder) -> Result<Fragment, String> {
        let Some(c) = self.chars.next() else {
            return Err(format!("`{}`: the expression ends early", self.source));
        };
        match c {
            'C' => Ok(builder.atom(Atom::Camera)),
            'L' => {
                let group = match self.chars.peek() {
                    Some('E') => Some(ENVIRONMENT),
                    Some('M') => Some(EMISSIVE),
                    Some(d) if d.is_ascii_digit() => {
                        let layer = d.to_digit(10).unwrap() as LightGroup;
                        if layer > LAST_LAYER {
                            return Err(format!("`{}`: light layers run from 0 to {LAST_LAYER}", self.source));
                        }
                        Some(layer)
                    }
                    _ => None,
                };
                if group.is_some() {
                    self.chars.next();
                }
                Ok(builder.atom(Atom::Light(group)))
            }
            '.' => Ok(builder.atom(Atom::Any)),
            '(' => {
                let inner = self.parse_sequence(builder, Some(')'))?;
                if self.chars.next() != Some(')') {
                    return Err(format!("`{}`: an unclosed group", self.source));
                }
                Ok(inner)
            }
            '[' => {
                let mut kinds = 0u8;
                loop {
                    match self.chars.next() {
                        Some(']') => break,
                        Some('D') => kinds |= DIFFUSE,
                        Some('S') => kinds |= SPECULAR,
                        Some('G') => kinds |= GLOSSY,
                        Some(other) => {
                            return Err(format!(
                                "`{}`: `{other}` is not a scattering kind",
                                self.source
                            ))
                        }
                        None => return Err(format!("`{}`: an unclosed class", self.source)),
                    }
                }
                if kinds == 0 {
                    return Err(format!("`{}`: an empty class", self.source));
                }
                Ok(builder.atom(Atom::Scatter {
                    transport: None,
                    kinds,
                }))
            }
            '<' => {
                let transport = match self.chars.next() {
                    Some('R') => Some(Transport::Reflect),
                    Some('T') => Some(Transport::Transmit),
                    Some('.') => None,
                    other => {
                        return Err(format!(
                            "`{}`: `{other:?}` is not a transport",
                            self.source
                        ))
                    }
                };
                let kinds = match self.chars.next() {
                    Some('D') => DIFFUSE,
                    Some('S') => SPECULAR,
                    Some('G') => GLOSSY,
                    Some('.') => ALL_KINDS,
                    Some('[') => {
                        let mut kinds = 0u8;
                        loop {
                            match self.chars.next() {
                                Some(']') => break,
                                Some('D') => kinds |= DIFFUSE,
                                Some('S') => kinds |= SPECULAR,
                                Some('G') => kinds |= GLOSSY,
                                other => {
                                    return Err(format!(
                                        "`{}`: `{other:?}` is not a scattering kind",
                                        self.source
                                    ))
                                }
                            }
                        }
                        kinds
                    }
                    other => {
                        return Err(format!(
                            "`{}`: `{other:?}` is not a scattering kind",
                            self.source
                        ))
                    }
                };
                if self.chars.next() != Some('>') {
                    return Err(format!("`{}`: an unclosed event", self.source));
                }
                Ok(builder.atom(Atom::Scatter { transport, kinds }))
            }
            other => Err(format!(
                "`{}`: `{other}` is not a light path expression",
                self.source
            )),
        }
    }
}

/// Several expressions, stepped together as a path grows.
///
/// The state of every expression is one `u64`, so a path of a dozen bounces
/// through a dozen AOVs costs a dozen words of state and one table lookup per
/// bounce per AOV. That is what makes it affordable to route contributions as
/// they happen rather than to record a path and match it afterwards — and
/// recording it afterwards is not really an option, since a contribution is
/// made at every bounce and each one belongs to a *different* path.
#[derive(Clone, Debug, Default)]
pub struct LpeMatcher {
    expressions: Vec<Lpe>,
    /// Current state set per expression.
    states: Vec<u64>,
    /// State set per expression at each vertex, so the matcher can be rewound.
    /// Not needed yet, and deliberately absent: the integrator only ever
    /// extends a path.
    depth: usize,
}

impl LpeMatcher {
    pub fn new(expressions: Vec<Lpe>) -> LpeMatcher {
        let states = expressions.iter().map(|e| e.start).collect();
        LpeMatcher {
            expressions,
            states,
            depth: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.expressions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.expressions.is_empty()
    }

    pub fn expressions(&self) -> &[Lpe] {
        &self.expressions
    }

    /// Start a new path. Every path starts at the camera, so that event is
    /// consumed here rather than left for the caller to remember.
    pub fn begin(&mut self) {
        for (state, expression) in self.states.iter_mut().zip(&self.expressions) {
            *state = expression.step(expression.start, Event::Camera).1;
        }
        self.depth = 0;
    }

    /// Extend the path by one scattering event.
    pub fn scatter(&mut self, transport: Transport, scatter: Scatter) {
        let event = Event::Scatter(transport, scatter);
        for (state, expression) in self.states.iter_mut().zip(&self.expressions) {
            *state = expression.step(*state, event).1;
        }
        self.depth += 1;
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Which expressions accept the path as it stands, ended at a light.
    ///
    /// Returns a bitmask over the expression list, which the caller uses to
    /// index its AOVs. A mask rather than a list because the answer is usually
    /// two or three of a dozen and allocating for that would cost more than the
    /// match.
    pub fn accepting_at_light(&self, group: LightGroup) -> u64 {
        let mut mask = 0u64;
        for (index, (state, expression)) in self.states.iter().zip(&self.expressions).enumerate() {
            let (consumed, _) = expression.step(*state, Event::Light(group));
            if consumed & expression.accepting != 0 {
                mask |= 1 << index;
            }
        }
        mask
    }

    /// A snapshot of the current states, so a path can be continued after a
    /// branch. The integrator takes one before next-event estimation, since the
    /// shadow ray's `L` must not disturb the path that continues past it.
    pub fn save(&self) -> Vec<u64> {
        self.states.clone()
    }

    pub fn restore(&mut self, saved: &[u64]) {
        self.states.copy_from_slice(saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(spec: &str) -> Vec<Event> {
        // A compact way to write test paths: `C`, `L`, and pairs like `RD`.
        let mut events = Vec::new();
        let mut chars = spec.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                'C' => events.push(Event::Camera),
                'L' => events.push(Event::Light(0)),
                'R' | 'T' => {
                    let transport = if c == 'R' {
                        Transport::Reflect
                    } else {
                        Transport::Transmit
                    };
                    let scatter = match chars.next() {
                        Some('D') => Scatter::Diffuse,
                        Some('S') => Scatter::Specular,
                        Some('G') => Scatter::Glossy,
                        other => panic!("bad scatter {other:?}"),
                    };
                    events.push(Event::Scatter(transport, scatter));
                }
                other => panic!("bad event {other}"),
            }
        }
        events
    }

    /// Every expression §12's table names has to compile and mean what the
    /// table says. A pass whose expression does not parse is a pass that is
    /// silently empty.
    #[test]
    fn the_documented_expressions_compile_and_match() {
        let cases: [(&str, &[&str], &[&str]); 6] = [
            // Beauty: everything.
            ("C.*L", &["CL", "CRDL", "CRSRDL"], &[]),
            // Diffuse: first bounce is a diffuse reflection.
            ("C<RD>.*L", &["CRDL", "CRDRSL"], &["CRSL", "CTSL"]),
            // Direct diffuse.
            ("C<RD>L", &["CRDL"], &["CRDRDL", "CRSL"]),
            // Reflection: first bounce is a specular reflection.
            ("C<RS>.*L", &["CRSL", "CRSRDL"], &["CRDL", "CTSL"]),
            // Refraction.
            ("C<TS>.*L", &["CTSL", "CTSRDL"], &["CRSL", "CRDL"]),
            // Caustics: diffuse first, then one or more specular or glossy.
            (
                "C<RD>[SG]+L",
                &["CRDRSL", "CRDRSTSL", "CRDRGL"],
                &["CRDL", "CRDRDL"],
            ),
        ];
        for (source, should, should_not) in cases {
            let lpe = Lpe::parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
            for p in should {
                assert!(lpe.matches(&path(p)), "`{source}` should match `{p}`");
            }
            for p in should_not {
                assert!(!lpe.matches(&path(p)), "`{source}` should not match `{p}`");
            }
        }
    }

    /// The property M4's exit criterion rests on: a set of expressions that
    /// partitions the paths sums to the whole. Checked here on the expressions
    /// rather than on an image, because if the partition is wrong no amount of
    /// sampling will make the sum come out.
    #[test]
    fn the_first_bounce_expressions_partition_every_path() {
        let parts = [
            Lpe::parse("C<RD>.*L").unwrap(),
            Lpe::parse("C<RS>.*L").unwrap(),
            Lpe::parse("C<RG>.*L").unwrap(),
            Lpe::parse("C<T.>.*L").unwrap(),
            // A path that reaches a light with no scattering at all — the
            // camera seeing a light directly.
            Lpe::parse("CL").unwrap(),
        ];
        let everything = Lpe::parse("C.*L").unwrap();

        for spec in [
            "CL", "CRDL", "CRSL", "CRGL", "CTSL", "CTDL", "CRDRSL", "CTSRDRGL", "CRGRDTSL",
        ] {
            let p = path(spec);
            let matched: Vec<usize> = parts
                .iter()
                .enumerate()
                .filter(|(_, lpe)| lpe.matches(&p))
                .map(|(i, _)| i)
                .collect();
            assert!(
                everything.matches(&p),
                "`{spec}` is not matched by the beauty expression"
            );
            assert_eq!(
                matched.len(),
                1,
                "`{spec}` matched {matched:?} of the component expressions rather \
                 than exactly one"
            );
        }
    }

    /// Stepping incrementally has to give the same answer as matching a whole
    /// path. It is the only part of this module the renderer actually uses, and
    /// the two agreeing is what lets the simple form stand as the reference.
    #[test]
    fn the_incremental_matcher_agrees_with_whole_path_matching() {
        let sources = [
            "C.*L",
            "C<RD>.*L",
            "C<RD>L",
            "C<RS>.*L",
            "C<TS>.*L",
            "C<RD>[SG]+L",
            "C.*[DS].*L",
        ];
        let expressions: Vec<Lpe> = sources.iter().map(|s| Lpe::parse(s).unwrap()).collect();
        let mut matcher = LpeMatcher::new(expressions.clone());

        for spec in [
            "CL", "CRDL", "CRSL", "CTSL", "CRDRDL", "CRDRSL", "CRSRDL", "CTSTSRDL", "CRGRGL",
        ] {
            let events = path(spec);
            matcher.begin();
            for event in &events[1..events.len() - 1] {
                let Event::Scatter(transport, scatter) = event else {
                    unreachable!("only scattering events are in the middle")
                };
                matcher.scatter(*transport, *scatter);
            }
            let mask = matcher.accepting_at_light(0);
            for (index, expression) in expressions.iter().enumerate() {
                let incremental = mask & (1 << index) != 0;
                let whole = expression.matches(&events);
                assert_eq!(
                    incremental, whole,
                    "`{}` on `{spec}`: incremental said {incremental}, whole said {whole}",
                    expression.source()
                );
            }
        }
    }

    /// Next-event estimation ends a path at a light *and then keeps going*.
    /// The matcher has to survive that, or every AOV after the first bounce is
    /// wrong — and wrong in a way that still produces a plausible image.
    #[test]
    fn a_light_probe_does_not_disturb_the_path() {
        let expressions = vec![Lpe::parse("C<RD>L").unwrap(), Lpe::parse("C.*L").unwrap()];
        let mut matcher = LpeMatcher::new(expressions);

        matcher.begin();
        matcher.scatter(Transport::Reflect, Scatter::Diffuse);
        let saved = matcher.save();
        // The direct expression accepts here.
        assert_eq!(matcher.accepting_at_light(0) & 1, 1);
        matcher.restore(&saved);

        // And after another bounce it no longer does, which it could not report
        // if the probe had advanced the path.
        matcher.scatter(Transport::Reflect, Scatter::Diffuse);
        assert_eq!(matcher.accepting_at_light(0) & 1, 0);
        assert_eq!(matcher.accepting_at_light(0) & 2, 2);
    }

    /// Malformed expressions are reported, not silently treated as matching
    /// nothing — an AOV that is quietly empty looks like a renderer bug.
    #[test]
    fn a_malformed_expression_is_reported() {
        for bad in ["", "C<R", "C<RX>", "C[", "C[]", "C)", "Cx", "C<RD"] {
            assert!(
                Lpe::parse(bad).is_err(),
                "`{bad}` should not have compiled"
            );
        }
    }

    /// Grouping, so an expression can repeat more than one event.
    #[test]
    fn groups_repeat_as_a_unit() {
        let lpe = Lpe::parse("C(<RD><RS>)+L").unwrap();
        assert!(lpe.matches(&path("CRDRSL")));
        assert!(lpe.matches(&path("CRDRSRDRSL")));
        assert!(!lpe.matches(&path("CRDL")));
        assert!(!lpe.matches(&path("CRDRSRDL")));
    }
}
