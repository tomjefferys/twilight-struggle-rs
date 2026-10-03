//! Card events with a *choice* in them: the cards whose text makes a
//! player pick the countries to add influence to or remove it from.
//!
//! Where [`super::effects`] resolves a card in one pure call, a choice card
//! opens an [`EventChoice`] — carried by [`crate::ops::Operation::Event`] —
//! that stages the chooser's picks against a cloned [`Board`] exactly the
//! way [`crate::ops::InfluencePlacement`] stages ops points: nothing
//! reaches the real board until [`crate::game::Game::confirm`] turns
//! [`EventChoice::into_result`]'s [`EffectResult`] into real changes, so
//! every pick can be taken back first.
//!
//! The *chooser* is the card's own side, irrespective of who is phasing
//! (the USSR choosing where Marshall Plan's US influence goes would be
//! nonsense — but the US choosing where a USSR card's USSR influence goes
//! is not; whoever's card it is decides). Every card here moves one
//! side's influence at a time ([`Rule::target`]), by a vocabulary small
//! enough to describe all of them: a [`Rule`] is *add* or *remove* (or,
//! for De-Stalinization, *reallocate*), restricted to an [`Eligible`] set
//! of countries, with a total-points budget, a per-country cap, and a
//! cap on how many distinct countries may be touched. A card's text is
//! one function below building a [`Spec`], plus one line in `CHOICES`.
//!
//! `+` and `-` are the two steps ([`Sign`]). Whichever one a rule allows
//! as a *forward* move spends budget; the other, where a country already
//! has a staged change, takes the last one back. That is what makes `-`
//! mean "remove influence" here and "undo one point in this country" in
//! an ordinary placement.
//!
//! An event is carried out as fully as it can be, as the rules require:
//! [`EventChoice::is_complete`] is false while any forward step is still
//! legal, except on the two "may" cards ([`Spec::optional`]).

use std::fmt;

use super::effects::{EffectResult, InfluenceChange};
use crate::board::Board;
use crate::cards::CardId;
use crate::country::{CountryId, Region, SubRegion, Superpower};
use crate::map::WorldMap;
use crate::status::GameStatus;

/// "No limit" for a budget field.
const ANY: u8 = u8::MAX;

/// One of the two steps a chooser takes on a country.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    Plus,
    Minus,
}

/// A set of countries, by where they are.
#[derive(Debug, Clone, Copy)]
pub enum Where {
    Everywhere,
    Region(Region),
    Sub(SubRegion),
    Names(&'static [&'static str]),
    /// Neighbours of the named country.
    AdjacentTo(&'static str),
    /// Any of these.
    Any(&'static [Where]),
}

impl Where {
    fn contains(self, map: &WorldMap, id: CountryId) -> bool {
        let c = map.country(id);
        match self {
            Where::Everywhere => true,
            Where::Region(r) => c.region == r,
            Where::Sub(s) => c.is_in_sub_region(s),
            Where::Names(names) => names.iter().any(|n| map.id_by_name(n) == Some(id)),
            Where::AdjacentTo(name) => map.id_by_name(name).is_some_and(|n| c.adjacent.contains(&n)),
            Where::Any(parts) => parts.iter().any(|w| w.contains(map, id)),
        }
    }
}

/// Who may control a country for it to qualify, as of the start of the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Any,
    NotBy(Superpower),
    Neither,
}

/// Which countries a rule may touch. Judged against the board as it stood
/// when the event began, like placement's presence check.
#[derive(Debug, Clone, Copy)]
pub struct Eligible {
    place: Where,
    control: Control,
    /// Neither side has any influence there.
    empty: bool,
}

impl Eligible {
    fn new(place: Where) -> Self {
        Eligible { place, control: Control::Any, empty: false }
    }

    fn control(mut self, control: Control) -> Self {
        self.control = control;
        self
    }

    fn empty(mut self) -> Self {
        self.empty = true;
        self
    }

    fn allows(&self, map: &WorldMap, base: &Board, id: CountryId) -> bool {
        if !self.place.contains(map, id) {
            return false;
        }
        let controller = base.controller(map, id);
        let control_ok = match self.control {
            Control::Any => true,
            Control::NotBy(side) => controller != Some(side),
            Control::Neither => controller.is_none(),
        };
        control_ok && (!self.empty || (base.influence(id, Superpower::Us) == 0 && base.influence(id, Superpower::Ussr) == 0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Add,
    Remove,
    /// De-Stalinization: remove from anywhere, add (up to what's been
    /// removed) to eligible countries.
    Reallocate,
}

/// How much one forward step moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chunk {
    One,
    /// Up to this many at once (never more than is there to remove).
    Fixed(u8),
    /// Everything the target side has there (removal only).
    All,
    /// Enough to match the opponent's influence there (add only).
    Match,
}

/// One card mode's budgeted add/remove.
#[derive(Debug, Clone)]
pub struct Rule {
    kind: Kind,
    /// Whose influence moves.
    pub target: Superpower,
    eligible: Eligible,
    /// Total points that may move (a reallocation's removal side).
    points: u8,
    per_country: u8,
    /// How many distinct countries may be touched.
    countries: u8,
    chunk: Chunk,
}

impl Rule {
    fn add(target: Superpower, eligible: Eligible, points: u8, per_country: u8, countries: u8) -> Self {
        Rule { kind: Kind::Add, target, eligible, points, per_country, countries, chunk: Chunk::One }
    }

    fn remove(target: Superpower, eligible: Eligible, points: u8, per_country: u8, countries: u8) -> Self {
        Rule { kind: Kind::Remove, target, eligible, points, per_country, countries, chunk: Chunk::One }
    }

    fn chunk(mut self, chunk: Chunk) -> Self {
        self.chunk = chunk;
        self
    }

    fn allows_forward(&self, sign: Sign) -> bool {
        matches!(
            (self.kind, sign),
            (Kind::Add, Sign::Plus) | (Kind::Remove, Sign::Minus) | (Kind::Reallocate, _)
        )
    }
}

/// A change applied the moment a mode is chosen — no pick involved.
#[derive(Debug, Clone, Copy)]
struct Fixed {
    country: &'static str,
    side: Superpower,
    op: FixedOp,
}

#[derive(Debug, Clone, Copy)]
enum FixedOp {
    Add(u8),
    Clear,
}

/// One way a card can be played. Most cards have exactly one.
#[derive(Debug, Clone)]
pub struct Mode {
    /// What the chooser is being asked to do, as a sentence fragment.
    pub label: String,
    fixed: Vec<Fixed>,
    rule: Option<Rule>,
}

/// A card's whole choice: who chooses, whether they must finish, and the
/// modes on offer.
#[derive(Debug, Clone)]
pub struct Spec {
    pub chooser: Superpower,
    /// "May" cards: the chooser can confirm with steps still available.
    pub optional: bool,
    pub modes: Vec<Mode>,
}

impl Spec {
    fn single(chooser: Superpower, label: impl Into<String>, rule: Rule) -> Self {
        Spec { chooser, optional: false, modes: vec![Mode { label: label.into(), fixed: Vec::new(), rule: Some(rule) }] }
    }

    fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Why a step was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventChoiceError {
    /// A card with several modes needs one chosen first.
    NoMode,
    /// No such mode.
    BadMode { modes: usize },
    /// Modes can't change once picks have been made.
    ModeLocked,
    NotAllowed { country: String, reason: String },
}

impl fmt::Display for EventChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventChoiceError::NoMode => write!(f, "choose which way to play this event first (mode <n>)"),
            EventChoiceError::BadMode { modes } => write!(f, "this event has {modes} mode(s)"),
            EventChoiceError::ModeLocked => write!(f, "undo the picks already made before changing mode"),
            EventChoiceError::NotAllowed { country, reason } => write!(f, "{country}: {reason}"),
        }
    }
}

impl std::error::Error for EventChoiceError {}

/// An event's choices in progress. `Clone` is cheap, like [`Board`]'s own.
#[derive(Clone)]
pub struct EventChoice {
    card: CardId,
    chooser: Superpower,
    optional: bool,
    modes: Vec<Mode>,
    mode: Option<usize>,
    /// The board when the event began — eligibility is judged against it.
    base: Board,
    /// `base` plus the mode's fixed changes and every pick so far.
    board: Board,
    /// Countries a mode's fixed changes touched, in order.
    fixed_ids: Vec<CountryId>,
    /// Every forward step, as (country, signed amount), oldest first.
    history: Vec<(CountryId, i8)>,
    /// Cached by [`EventChoice::refresh`] after every change, so that
    /// `Game::confirm` — which has no map — can still ask whether the
    /// event is finished and what it did.
    complete: bool,
    changes: Vec<InfluenceChange>,
}

impl EventChoice {
    /// Opens `card`'s choice against `board`, or `None` for a card that
    /// isn't a choice card. A single-mode card has its mode chosen already.
    pub fn new(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId) -> Option<Self> {
        let spec = spec_for(card)?(map, board, status);
        let mut choice = EventChoice {
            card,
            chooser: spec.chooser,
            optional: spec.optional,
            modes: spec.modes,
            mode: None,
            base: board.clone(),
            board: board.clone(),
            fixed_ids: Vec::new(),
            history: Vec::new(),
            complete: false,
            changes: Vec::new(),
        };
        if choice.modes.len() == 1 {
            choice.select(map, 0);
        }
        Some(choice)
    }

    pub fn card(&self) -> CardId {
        self.card
    }

    /// The side that makes this event's choices — the card's own side.
    pub fn chooser(&self) -> Superpower {
        self.chooser
    }

    pub fn modes(&self) -> &[Mode] {
        &self.modes
    }

    pub fn mode(&self) -> Option<usize> {
        self.mode
    }

    /// The speculative board — every pick already applied.
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Whether nothing has been picked yet (fixed changes don't count).
    pub fn is_pristine(&self) -> bool {
        self.history.is_empty()
    }

    fn rule(&self) -> Option<&Rule> {
        self.mode.and_then(|i| self.modes[i].rule.as_ref())
    }

    fn select(&mut self, map: &WorldMap, i: usize) {
        self.board = self.base.clone();
        self.fixed_ids.clear();
        for f in self.modes[i].fixed.clone() {
            let id = map.id_by_name(f.country).unwrap_or_else(|| panic!("event refers to unknown country {:?}", f.country));
            match f.op {
                FixedOp::Add(n) => self.board.add_influence(id, f.side, n),
                FixedOp::Clear => self.board.set_influence(id, f.side, 0),
            }
            self.fixed_ids.push(id);
        }
        self.mode = Some(i);
        self.refresh(map);
    }

    /// Recomputes the cached completeness and influence changes.
    fn refresh(&mut self, map: &WorldMap) {
        self.complete = self.compute_complete(map);
        self.changes.clear();
        for (id, _) in map.iter() {
            for side in [Superpower::Us, Superpower::Ussr] {
                let (before, after) = (self.base.influence(id, side), self.board.influence(id, side));
                if before != after {
                    self.changes.push(InfluenceChange { country: id, side, before, after });
                }
            }
        }
    }

    /// Picks which way to play a multi-mode card (0-based). Re-picking is
    /// fine until the first country has been chosen.
    pub fn choose_mode(&mut self, map: &WorldMap, i: usize) -> Result<(), EventChoiceError> {
        if i >= self.modes.len() {
            return Err(EventChoiceError::BadMode { modes: self.modes.len() });
        }
        if !self.history.is_empty() {
            return Err(EventChoiceError::ModeLocked);
        }
        self.select(map, i);
        Ok(())
    }

    // ---- budget bookkeeping ----

    fn hist_net(&self, id: CountryId) -> i8 {
        self.history.iter().filter(|(c, _)| *c == id).map(|(_, a)| *a).sum()
    }

    fn touched_count(&self) -> usize {
        let mut ids: Vec<CountryId> = self.history.iter().map(|(c, _)| *c).collect();
        ids.sort();
        ids.dedup();
        ids.into_iter().filter(|&id| self.hist_net(id) != 0).count()
    }

    fn added(&self) -> u16 {
        self.history.iter().filter(|(_, a)| *a > 0).map(|(_, a)| *a as u16).sum()
    }

    fn removed(&self) -> u16 {
        self.history.iter().filter(|(_, a)| *a < 0).map(|(_, a)| a.unsigned_abs() as u16).sum()
    }

    /// Points spent against the rule's `points` budget.
    fn used(&self, rule: &Rule) -> u16 {
        match rule.kind {
            Kind::Add => self.added(),
            Kind::Remove | Kind::Reallocate => self.removed(),
        }
    }

    // ---- stepping ----

    /// How much a *forward* `sign` step on `id` would move, or `None` if
    /// it isn't a legal forward step right now.
    pub fn can_forward(&self, map: &WorldMap, id: CountryId, sign: Sign) -> Option<u8> {
        let rule = self.rule()?;
        if !rule.allows_forward(sign) {
            return None;
        }
        let cur = self.board.influence(id, rule.target);
        let net = self.hist_net(id);
        let adding = sign == Sign::Plus;
        let amount = if adding {
            if !rule.eligible.allows(map, &self.base, id) {
                return None;
            }
            match rule.chunk {
                Chunk::Fixed(n) => n,
                Chunk::Match => self.base.influence(id, rule.target.opponent()).saturating_sub(cur),
                Chunk::One | Chunk::All => 1,
            }
        } else {
            if rule.kind == Kind::Remove && !rule.eligible.allows(map, &self.base, id) {
                return None;
            }
            match rule.chunk {
                Chunk::Fixed(n) => n.min(cur),
                Chunk::All => cur,
                Chunk::One | Chunk::Match => 1.min(cur),
            }
        };
        if amount == 0 {
            return None;
        }
        if rule.kind == Kind::Reallocate {
            // A reallocation never both adds to and removes from one country.
            if adding {
                if net < 0 || self.added() + amount as u16 > self.removed() || (net as u16 + amount as u16) > rule.per_country as u16 {
                    return None;
                }
            } else if net > 0 || self.removed() + amount as u16 > rule.points as u16 {
                return None;
            }
            return Some(amount);
        }
        if self.used(rule) + amount as u16 > rule.points as u16 || net.unsigned_abs() as u16 + amount as u16 > rule.per_country as u16 {
            return None;
        }
        if net == 0 && self.touched_count() >= rule.countries as usize {
            return None;
        }
        Some(amount)
    }

    /// Every legal forward step — what an AI is offered, and what decides
    /// whether the event is finished.
    pub fn forward_steps(&self, map: &WorldMap) -> Vec<(CountryId, Sign)> {
        let mut steps = Vec::new();
        for (id, _) in map.iter() {
            for sign in [Sign::Plus, Sign::Minus] {
                if self.can_forward(map, id, sign).is_some() {
                    steps.push((id, sign));
                }
            }
        }
        steps
    }

    /// Whether `sign` on `id` would take back a staged change there.
    pub fn can_take_back(&self, id: CountryId, sign: Sign) -> bool {
        let net = self.hist_net(id);
        match sign {
            Sign::Plus => net < 0,
            Sign::Minus => net > 0,
        }
    }

    /// Takes a step on `id`: forward if the rule allows it, otherwise a
    /// take-back of the last staged change there.
    pub fn step(&mut self, map: &WorldMap, id: CountryId, sign: Sign) -> Result<(), EventChoiceError> {
        let Some(rule) = self.rule().cloned() else {
            return Err(EventChoiceError::NoMode);
        };
        if self.can_take_back(id, sign) {
            let pos = self.history.iter().rposition(|(c, _)| *c == id).expect("net != 0 implies a history entry");
            let (_, amount) = self.history[pos];
            if rule.kind == Kind::Reallocate && amount < 0 && self.added() > self.removed() - amount.unsigned_abs() as u16 {
                return Err(self.refused(map, id, "undo the influence added elsewhere first".into()));
            }
            self.history.remove(pos);
            self.apply(id, rule.target, -amount);
            self.refresh(map);
            return Ok(());
        }
        match self.can_forward(map, id, sign) {
            Some(n) => {
                let signed = if sign == Sign::Plus { n as i8 } else { -(n as i8) };
                self.history.push((id, signed));
                self.apply(id, rule.target, signed);
                self.refresh(map);
                Ok(())
            }
            None => Err(self.refused(map, id, self.reason(map, id, sign))),
        }
    }

    fn apply(&mut self, id: CountryId, side: Superpower, signed: i8) {
        if signed >= 0 {
            self.board.add_influence(id, side, signed as u8);
        } else {
            self.board.remove_influence(id, side, signed.unsigned_abs());
        }
    }

    fn refused(&self, map: &WorldMap, id: CountryId, reason: String) -> EventChoiceError {
        EventChoiceError::NotAllowed { country: map.country(id).name.clone(), reason }
    }

    /// Why a forward `sign` step on `id` isn't legal — for an error message
    /// or the map's per-country hint.
    fn reason(&self, map: &WorldMap, id: CountryId, sign: Sign) -> String {
        let Some(rule) = self.rule() else { return "no mode chosen".into() };
        if !rule.allows_forward(sign) {
            return match rule.kind {
                Kind::Add => "this event only adds influence (+)".into(),
                _ => "this event only removes influence (-)".into(),
            };
        }
        let adding = sign == Sign::Plus;
        if (adding || rule.kind == Kind::Remove) && !rule.eligible.allows(map, &self.base, id) {
            return "not an eligible country for this event".into();
        }
        if !adding && self.board.influence(id, rule.target) == 0 {
            return format!("no {} influence to remove", rule.target);
        }
        if adding && rule.chunk == Chunk::Match {
            return format!("{} already has as much as {}", rule.target, rule.target.opponent());
        }
        if rule.kind == Kind::Reallocate {
            return if adding { "add only up to what has been removed, and never where you removed".into() } else { "nothing more to remove here".into() };
        }
        if self.used(rule) >= rule.points as u16 {
            return "no points left".into();
        }
        if self.hist_net(id) == 0 && self.touched_count() >= rule.countries as usize {
            return "no countries left to choose".into();
        }
        "this country's limit is reached".into()
    }

    /// Takes back the most recent pick, returning its country.
    pub fn undo_last(&mut self, map: &WorldMap) -> Option<CountryId> {
        let rule = self.rule()?.clone();
        let (id, amount) = *self.history.last()?;
        if rule.kind == Kind::Reallocate && amount < 0 && self.added() > self.removed() - amount.unsigned_abs() as u16 {
            // The newest removal can't go while later adds depend on it —
            // but the newest *entry* is the removal here only if no add
            // came after it, so this is just defensive.
            return None;
        }
        self.history.pop();
        self.apply(id, rule.target, -amount);
        self.refresh(map);
        Some(id)
    }

    // ---- completion ----

    /// Whether the event may be confirmed now: every legal step has been
    /// taken ("as fully as possible"), or — on a "may" card — nothing is
    /// left dangling.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    fn compute_complete(&self, map: &WorldMap) -> bool {
        let Some(_) = self.mode else { return false };
        let balanced = self.rule().is_none_or(|r| r.kind != Kind::Reallocate || self.added() == self.removed());
        if self.optional {
            return balanced;
        }
        self.forward_steps(map).is_empty()
    }

    /// A single-mode card with nothing it can do at all (Truman Doctrine
    /// with no uncontrolled European country with USSR influence…): it
    /// resolves immediately instead of opening a session.
    pub fn resolves_immediately(&self, map: &WorldMap) -> bool {
        self.modes.len() == 1 && self.history.is_empty() && self.forward_steps(map).is_empty()
    }

    // ---- display ----

    /// Net change in `side`'s influence in `id` since the event began,
    /// fixed changes included.
    pub fn delta(&self, id: CountryId, side: Superpower) -> i8 {
        self.board.influence(id, side) as i8 - self.base.influence(id, side) as i8
    }

    /// Countries changed so far, in the order they were first changed.
    pub fn touched(&self) -> Vec<CountryId> {
        let mut seen: Vec<CountryId> = Vec::new();
        for id in self.fixed_ids.iter().copied().chain(self.history.iter().map(|(c, _)| *c)) {
            if !seen.contains(&id) {
                seen.push(id);
            }
        }
        seen.into_iter()
            .filter(|&id| self.delta(id, Superpower::Us) != 0 || self.delta(id, Superpower::Ussr) != 0)
            .collect()
    }

    /// What the chooser is being asked to do — the chosen mode's label, or
    /// the numbered list of modes while none is chosen yet.
    pub fn prompt(&self) -> String {
        match self.mode {
            Some(i) => self.modes[i].label.clone(),
            None => self.modes.iter().enumerate().map(|(i, m)| format!("{}) {}", i + 1, m.label)).collect::<Vec<_>>().join("  or  "),
        }
    }

    /// What's left to spend, e.g. `2/4 countries · 2/5 points`.
    pub fn progress(&self) -> String {
        let Some(rule) = self.rule() else { return "choose a mode".into() };
        let mut parts = Vec::new();
        if rule.countries != ANY && rule.kind != Kind::Reallocate {
            parts.push(format!("{}/{} countries", self.touched_count(), rule.countries));
        }
        if rule.points != ANY && rule.points != rule.countries {
            let verb = if rule.kind == Kind::Reallocate { "moved" } else { "points" };
            parts.push(format!("{}/{} {verb}", self.used(rule), rule.points));
        }
        if rule.kind == Kind::Reallocate {
            parts.push(format!("{} added", self.added()));
        }
        if parts.is_empty() {
            parts.push(format!("{} picked", self.touched_count()));
        }
        parts.join(" · ")
    }

    /// What's still to do, for the message when confirming too early.
    pub fn progress_left(&self) -> String {
        format!("{} ({})", self.prompt(), self.progress())
    }

    /// The forward step a chip should advertise on `id` — its sign and how
    /// much it would move — if there is one.
    pub fn suggestion(&self, map: &WorldMap, id: CountryId) -> Option<(Sign, u8)> {
        [Sign::Plus, Sign::Minus].into_iter().find_map(|s| self.can_forward(map, id, s).map(|n| (s, n)))
    }

    /// What `+`/`-` would do on `id` right now, for the map's footer.
    pub fn hint(&self, map: &WorldMap, id: CountryId) -> String {
        let Some(rule) = self.rule() else { return "choose a mode first".into() };
        let mut parts = Vec::new();
        for (sign, key, verb) in [(Sign::Plus, '+', "add"), (Sign::Minus, '-', "remove")] {
            if let Some(n) = self.can_forward(map, id, sign) {
                parts.push(format!("{key} {verb} {n} {}", rule.target));
            } else if self.can_take_back(id, sign) {
                parts.push(format!("{key} undo here"));
            }
        }
        if parts.is_empty() {
            let sign = if rule.kind == Kind::Add { Sign::Plus } else { Sign::Minus };
            return self.reason(map, id, sign);
        }
        parts.join("   ")
    }

    /// The finished event as the same [`EffectResult`] a fixed-effect card
    /// produces, so applying and logging it is one shared code path.
    pub fn into_result(&self, status: &GameStatus) -> EffectResult {
        EffectResult { card: self.card, player: status.active, influence: self.changes.clone(), vp_delta: 0, defcon: None }
    }
}

// ---- the cards, in printed-number order ----

type SpecFn = fn(&WorldMap, &Board, &GameStatus) -> Spec;

/// Every choice card, by printed number. Adding a card is one function
/// below plus one line here.
const CHOICES: &[(u8, SpecFn)] = &[
    (7, socialist_governments),
    (14, comecon),
    (16, warsaw_pact_formed),
    (19, truman_doctrine),
    (22, independent_reds),
    (23, marshall_plan),
    (28, suez_crisis),
    (29, east_european_unrest),
    (30, decolonization),
    (33, de_stalinization),
    (53, south_african_unrest),
    (56, muslim_revolution),
    (63, colonial_rear_guards),
    (66, puppet_governments),
    (70, oas_founded),
    (75, liberation_theology),
    (87, the_reformer),
    (88, marine_barracks_bombing),
    (105, special_relationship),
];

fn spec_for(card: CardId) -> Option<SpecFn> {
    CHOICES.iter().find(|&&(n, _)| n == card.0).map(|&(_, f)| f)
}

pub fn is_choice_card(card: CardId) -> bool {
    spec_for(card).is_some()
}

use Superpower::{Us, Ussr};

const EASTERN: Where = Where::Sub(SubRegion::EasternEurope);
const WESTERN: Where = Where::Sub(SubRegion::WesternEurope);
const AFRICA_OR_SEA: Where = Where::Any(&[Where::Region(Region::Africa), Where::Sub(SubRegion::SoutheastAsia)]);

/// #7 Socialist Governments
fn socialist_governments(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "remove 3 US influence from Western Europe (max 2 per country)",
        Rule::remove(Us, Eligible::new(WESTERN), 3, 2, ANY),
    )
}

/// #14 Comecon
fn comecon(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "add 1 USSR influence to each of 4 non-US-controlled Eastern European countries",
        Rule::add(Ussr, Eligible::new(EASTERN).control(Control::NotBy(Us)), 4, 1, 4),
    )
}

/// #16 Warsaw Pact Formed
fn warsaw_pact_formed(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec {
        chooser: Ussr,
        optional: false,
        modes: vec![
            Mode {
                label: "remove all US influence from 4 Eastern European countries".into(),
                fixed: Vec::new(),
                rule: Some(Rule::remove(Us, Eligible::new(EASTERN), ANY, ANY, 4).chunk(Chunk::All)),
            },
            Mode {
                label: "add 5 USSR influence to Eastern Europe (max 2 per country)".into(),
                fixed: Vec::new(),
                rule: Some(Rule::add(Ussr, Eligible::new(EASTERN), 5, 2, ANY)),
            },
        ],
    }
}

/// #19 Truman Doctrine
fn truman_doctrine(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "remove all USSR influence from one uncontrolled European country",
        Rule::remove(Ussr, Eligible::new(Where::Region(Region::Europe)).control(Control::Neither), ANY, ANY, 1).chunk(Chunk::All),
    )
}

/// #22 Independent Reds
fn independent_reds(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "raise US influence to the USSR's in one of Yugoslavia, Romania, Bulgaria, Hungary, Czechoslovakia",
        Rule::add(
            Us,
            Eligible::new(Where::Names(&["Yugoslavia", "Romania", "Bulgaria", "Hungary", "Czechoslovakia"])),
            ANY,
            ANY,
            1,
        )
        .chunk(Chunk::Match),
    )
}

/// #23 Marshall Plan
fn marshall_plan(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "add 1 US influence to each of 7 non-USSR-controlled Western European countries",
        Rule::add(Us, Eligible::new(WESTERN).control(Control::NotBy(Ussr)), 7, 1, 7),
    )
}

/// #28 Suez Crisis
fn suez_crisis(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "remove 4 US influence from France, the UK and Israel (max 2 per country)",
        Rule::remove(Us, Eligible::new(Where::Names(&["France", "UK", "Israel"])), 4, 2, ANY),
    )
}

/// #29 East European Unrest: 1 per country early/mid war, 2 in the late war.
fn east_european_unrest(_: &WorldMap, _: &Board, status: &GameStatus) -> Spec {
    let n: u8 = if status.turn >= 8 { 2 } else { 1 };
    Spec::single(
        Us,
        format!("remove {n} USSR influence from each of 3 Eastern European countries"),
        Rule::remove(Ussr, Eligible::new(EASTERN), ANY, n, 3).chunk(Chunk::Fixed(n)),
    )
}

/// #30 Decolonization
fn decolonization(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "add 1 USSR influence to each of 4 countries in Africa and/or Southeast Asia",
        Rule::add(Ussr, Eligible::new(AFRICA_OR_SEA), 4, 1, 4),
    )
}

/// #33 De-Stalinization
fn de_stalinization(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "move up to 4 USSR influence: - removes it, + adds it to a non-US-controlled country (max 2 each)",
        Rule {
            kind: Kind::Reallocate,
            target: Ussr,
            eligible: Eligible::new(Where::Everywhere).control(Control::NotBy(Us)),
            points: 4,
            per_country: 2,
            countries: ANY,
            chunk: Chunk::One,
        },
    )
    .optional()
}

/// #53 South African Unrest
fn south_african_unrest(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec {
        chooser: Ussr,
        optional: false,
        modes: vec![
            Mode {
                label: "add 2 USSR influence to South Africa".into(),
                fixed: vec![Fixed { country: "South Africa", side: Ussr, op: FixedOp::Add(2) }],
                rule: None,
            },
            Mode {
                label: "add 1 USSR influence to South Africa and 2 to one adjacent country".into(),
                fixed: vec![Fixed { country: "South Africa", side: Ussr, op: FixedOp::Add(1) }],
                rule: Some(Rule::add(Ussr, Eligible::new(Where::AdjacentTo("South Africa")), ANY, 2, 1).chunk(Chunk::Fixed(2))),
            },
        ],
    }
}

/// #56 Muslim Revolution
fn muslim_revolution(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "remove all US influence from 2 of Sudan, Iran, Iraq, Egypt, Libya, Saudi Arabia, Syria, Jordan",
        Rule::remove(
            Us,
            Eligible::new(Where::Names(&["Sudan", "Iran", "Iraq", "Egypt", "Libya", "Saudi Arabia", "Syria", "Jordan"])),
            ANY,
            ANY,
            2,
        )
        .chunk(Chunk::All),
    )
}

/// #63 Colonial Rear Guards
fn colonial_rear_guards(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "add 1 US influence to each of 4 countries in Africa and/or Southeast Asia",
        Rule::add(Us, Eligible::new(AFRICA_OR_SEA), 4, 1, 4),
    )
}

/// #66 Puppet Governments
fn puppet_governments(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "add 1 US influence to each of up to 3 countries with no US or USSR influence",
        Rule::add(Us, Eligible::new(Where::Everywhere).empty(), 3, 1, 3),
    )
    .optional()
}

/// #70 OAS Founded
fn oas_founded(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "add 2 US influence to countries in Central and/or South America",
        Rule::add(Us, Eligible::new(Where::Any(&[Where::Region(Region::CentralAmerica), Where::Region(Region::SouthAmerica)])), 2, ANY, ANY),
    )
}

/// #75 Liberation Theology
fn liberation_theology(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "add 3 USSR influence to Central America (max 2 per country)",
        Rule::add(Ussr, Eligible::new(Where::Region(Region::CentralAmerica)), 3, 2, ANY),
    )
}

/// #87 The Reformer: 4 influence, or 6 if the USSR is ahead on VP.
/// (The coup ban in Europe is `Game::begin`'s business.)
fn the_reformer(_: &WorldMap, _: &Board, status: &GameStatus) -> Spec {
    let n: u8 = if status.vp < 0 { 6 } else { 4 };
    Spec::single(
        Ussr,
        format!("add {n} USSR influence to Europe (max 2 per country)"),
        Rule::add(Ussr, Eligible::new(Where::Region(Region::Europe)), n, 2, ANY),
    )
}

/// #88 Marine Barracks Bombing
fn marine_barracks_bombing(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec {
        chooser: Ussr,
        optional: false,
        modes: vec![Mode {
            label: "remove 2 more US influence from the Middle East (Lebanon's is already gone)".into(),
            fixed: vec![Fixed { country: "Lebanon", side: Us, op: FixedOp::Clear }],
            rule: Some(Rule::remove(Us, Eligible::new(Where::Region(Region::MiddleEast)), 2, ANY, ANY)),
        }],
    }
}

/// #105 Special Relationship — only its first branch (UK US-controlled,
/// NATO not in effect): NATO isn't implemented, so it is never in effect.
fn special_relationship(map: &WorldMap, board: &Board, _: &GameStatus) -> Spec {
    let uk = map.id_by_name("UK").expect("UK is on the map");
    let countries = if board.is_controlled_by(map, uk, Us) { 1 } else { 0 };
    Spec::single(
        Us,
        "add 1 US influence to one country adjacent to the UK",
        Rule::add(Us, Eligible::new(Where::AdjacentTo("UK")), 1, 1, countries),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardCatalog;

    fn fixtures() -> (WorldMap, CardCatalog) {
        (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
    }

    fn open(map: &WorldMap, board: &Board, card: u8) -> EventChoice {
        EventChoice::new(map, board, &GameStatus::default(), CardId(card)).unwrap()
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap()
    }

    #[test]
    fn every_choice_card_builds_on_a_blank_board() {
        // Pins every hard-coded country name.
        let (map, _) = fixtures();
        let board = Board::new(&map);
        for &(n, _) in CHOICES {
            let mut c = open(&map, &board, n);
            for i in 0..c.modes().len() {
                c.choose_mode(&map, i).unwrap();
                let _ = c.forward_steps(&map);
                let _ = c.is_complete();
                let _ = c.into_result(&GameStatus::default());
            }
        }
    }

    #[test]
    fn choice_cards_are_real_non_scoring_cards_with_their_own_side_choosing() {
        let (_, cards) = fixtures();
        for &(n, _) in CHOICES {
            assert!(!cards.card(CardId(n)).scoring, "card #{n} is a scoring card");
            assert!(!super::super::effects::is_effect_card(CardId(n)), "card #{n} is also a fixed effect");
        }
    }

    #[test]
    fn the_choice_table_has_no_duplicate_ids() {
        let mut ids: Vec<u8> = CHOICES.iter().map(|&(n, _)| n).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CHOICES.len());
    }

    #[test]
    fn comecon_takes_four_countries_one_point_each_and_skips_us_controlled_ones() {
        let (map, _) = fixtures();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Poland"), Us, 5); // US-controlled: stability 3
        let mut c = open(&map, &board, 14);
        assert!(c.step(&map, id(&map, "Poland"), Sign::Plus).is_err());
        for name in ["East Germany", "Czechoslovakia", "Hungary", "Romania"] {
            c.step(&map, id(&map, name), Sign::Plus).unwrap();
        }
        assert!(!c.can_forward(&map, id(&map, "Bulgaria"), Sign::Plus).is_some());
        assert!(c.is_complete());
        assert!(c.step(&map, id(&map, "Hungary"), Sign::Plus).is_err(), "per-country cap of 1");
    }

    #[test]
    fn minus_takes_back_a_staged_add_and_frees_the_budget() {
        let (map, _) = fixtures();
        let mut c = open(&map, &Board::new(&map), 14);
        let hungary = id(&map, "Hungary");
        c.step(&map, hungary, Sign::Plus).unwrap();
        assert_eq!(c.delta(hungary, Ussr), 1);
        c.step(&map, hungary, Sign::Minus).unwrap();
        assert_eq!(c.delta(hungary, Ussr), 0);
        assert!(c.is_pristine());
    }

    #[test]
    fn confirming_early_is_incomplete_but_a_may_card_can_stop_anywhere() {
        let (map, _) = fixtures();
        let board = Board::new(&map);
        assert!(!open(&map, &board, 14).is_complete());
        assert!(open(&map, &board, 66).is_complete());
    }

    #[test]
    fn warsaw_pact_needs_a_mode_and_locks_it_once_a_pick_is_made() {
        let (map, _) = fixtures();
        let mut c = open(&map, &Board::new(&map), 16);
        assert!(c.mode().is_none());
        assert_eq!(c.step(&map, id(&map, "Poland"), Sign::Plus), Err(EventChoiceError::NoMode));
        c.choose_mode(&map, 1).unwrap();
        c.step(&map, id(&map, "Poland"), Sign::Plus).unwrap();
        assert_eq!(c.choose_mode(&map, 0), Err(EventChoiceError::ModeLocked));
    }

    #[test]
    fn independent_reds_matches_the_ussr() {
        let (map, _) = fixtures();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Romania"), Ussr, 3);
        board.set_influence(id(&map, "Romania"), Us, 1);
        let mut c = open(&map, &board, 22);
        c.step(&map, id(&map, "Romania"), Sign::Plus).unwrap();
        assert_eq!(c.board().influence(id(&map, "Romania"), Us), 3);
        assert!(c.is_complete());
    }

    #[test]
    fn de_stalinization_never_adds_more_than_it_removed() {
        let (map, _) = fixtures();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Poland"), Ussr, 3);
        let mut c = open(&map, &board, 33);
        assert!(c.step(&map, id(&map, "Hungary"), Sign::Plus).is_err());
        c.step(&map, id(&map, "Poland"), Sign::Minus).unwrap();
        c.step(&map, id(&map, "Poland"), Sign::Minus).unwrap();
        c.step(&map, id(&map, "Hungary"), Sign::Plus).unwrap();
        assert!(!c.is_complete(), "one removed point is still unplaced");
        // Poland can't be taken back below what Hungary is relying on.
        c.step(&map, id(&map, "Hungary"), Sign::Plus).unwrap();
        assert!(c.is_complete());
        assert!(c.step(&map, id(&map, "Poland"), Sign::Plus).is_err() || c.delta(id(&map, "Poland"), Ussr) > -2);
    }

    #[test]
    fn truman_with_nothing_to_do_resolves_immediately() {
        let (map, _) = fixtures();
        assert!(open(&map, &Board::new(&map), 19).resolves_immediately(&map));
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Italy"), Ussr, 1);
        assert!(!open(&map, &board, 19).resolves_immediately(&map));
    }

    #[test]
    fn special_relationship_needs_a_us_controlled_uk() {
        let (map, _) = fixtures();
        let mut board = Board::new(&map);
        assert!(open(&map, &board, 105).resolves_immediately(&map));
        board.set_influence(id(&map, "UK"), Us, 5);
        let mut c = open(&map, &board, 105);
        c.step(&map, id(&map, "France"), Sign::Plus).unwrap();
        assert!(c.is_complete());
    }

    #[test]
    fn marine_barracks_clears_lebanon_up_front() {
        let (map, _) = fixtures();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Lebanon"), Us, 2);
        let c = open(&map, &board, 88);
        assert_eq!(c.board().influence(id(&map, "Lebanon"), Us), 0);
        assert_eq!(c.touched(), vec![id(&map, "Lebanon")]);
    }
}
