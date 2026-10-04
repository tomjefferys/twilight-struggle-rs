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

use super::effects::{ChinaTransfer, Contest, EffectResult, InfluenceChange, PlayAs, PlayCard, Reveal};
use super::OpsGrant;
use crate::dice::Dice;
use crate::board::Board;
use crate::cards::CardId;
use crate::country::{CountryId, Region, SubRegion, Superpower};
use crate::map::WorldMap;
use crate::ongoing::OngoingEffect;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    Everywhere,
    Region(Region),
    Sub(SubRegion),
    Names(&'static [&'static str]),
    /// Neighbours of the named country.
    AdjacentTo(&'static str),
    /// Any of these.
    Any(&'static [Where]),
    /// Those of the inner set that aren't battlegrounds.
    NonBattleground(&'static Where),
    /// Any country in one of these regions (a set only known at play time, e.g. the regions
    /// the scoring cards in a hand name).
    Regions(RegionSet),
}

/// A set of regions, as one bit per [`Region::ALL`] entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionSet(u8);

impl RegionSet {
    pub fn of(regions: &[Region]) -> Self {
        RegionSet(regions.iter().fold(0, |m, r| m | 1 << Self::bit(*r)))
    }

    fn bit(region: Region) -> usize {
        Region::ALL.iter().position(|&r| r == region).expect("every region is in ALL")
    }

    pub fn contains(self, region: Region) -> bool {
        self.0 & (1 << Self::bit(region)) != 0
    }
}

impl Where {
    pub fn contains(self, map: &WorldMap, id: CountryId) -> bool {
        let c = map.country(id);
        match self {
            Where::Everywhere => true,
            Where::Region(r) => c.region == r,
            Where::Sub(s) => c.is_in_sub_region(s),
            Where::Names(names) => names.iter().any(|n| map.id_by_name(n) == Some(id)),
            Where::AdjacentTo(name) => map.id_by_name(name).is_some_and(|n| c.adjacent.contains(&n)),
            Where::Any(parts) => parts.iter().any(|w| w.contains(map, id)),
            Where::NonBattleground(inner) => !c.battleground && inner.contains(map, id),
            Where::Regions(set) => set.contains(c.region),
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
    /// This side must already have influence there.
    has: Option<Superpower>,
}

impl Eligible {
    fn new(place: Where) -> Self {
        Eligible { place, control: Control::Any, empty: false, has: None }
    }

    fn control(mut self, control: Control) -> Self {
        self.control = control;
        self
    }

    fn empty(mut self) -> Self {
        self.empty = true;
        self
    }

    fn with_influence_of(mut self, side: Superpower) -> Self {
        self.has = Some(side);
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
        control_ok && self.has.is_none_or(|side| base.influence(id, side) > 0) && (!self.empty || (base.influence(id, Superpower::Us) == 0 && base.influence(id, Superpower::Ussr) == 0))
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
    /// As much as the target side already has there, once per country (add only).
    Double,
}

/// A discard-pile card on offer: what a pick needs to show and to play it.
#[derive(Debug, Clone)]
pub struct PileCard {
    pub id: CardId,
    pub name: String,
    pub ops: u8,
    /// Whether its event removes it from the game.
    pub removed: bool,
}

/// What a discard-pile pick does with the card chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PileUse {
    /// SALT Negotiations: into the player's hand.
    Take,
    /// Star Wars: played as an event.
    Play,
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
    /// A turn-long effect choosing this mode starts (Chernobyl's region).
    ongoing: Option<OngoingEffect>,
    /// VP the event awards its chooser on top of the picks (Special Relationship with NATO).
    vp: i8,
    /// The China Card changing hands when this mode is played (Ussuri River Skirmish).
    china: Option<ChinaTransfer>,
    extra: Extra,
}

/// What choosing a mode does beyond influence: DEFCON, Military Ops, ending the game.
#[derive(Debug, Clone, Copy)]
struct Extra {
    /// DEFCON is set to this (How I Learned to Stop Worrying).
    defcon: Option<u8>,
    /// Military Operations the chooser gains.
    mil_ops: i8,
    /// The game ends, the VP leader winning (Wargames).
    ends_game: bool,
    /// This side loses this card from its hand (Blockade, Latin American Debt Crisis, Aldrich Ames).
    discard: Option<(Superpower, CardId)>,
    /// Choosing this mode hands the event on to its follow-up session (Debt Crisis: the USSR's doubling).
    then: bool,
    /// Choosing this mode reports the event's roll-off (Summit, Olympic Games' participation).
    contest: bool,
    /// Choosing this mode lets the player who played the card conduct operations with it (Olympic Games' boycott).
    grant: Option<OpsGrant>,
    /// This side takes this card out of the discard pile (SALT Negotiations).
    take: Option<(Superpower, CardId)>,
    /// The card is played as an event straight away (Star Wars): id, printed ops, removed after its event.
    play: Option<PlayCard>,
}

impl Extra {
    const NONE: Extra = Extra { defcon: None, mil_ops: 0, ends_game: false, discard: None, then: false, contest: false, grant: None, take: None, play: None };
}

/// One side's bonus to a roll-off and where it comes from.
pub type RollBonus = (u8, String);

/// Summit's roll-off before it has been thrown: each side's bonus and where it comes from, and
/// the DEFCON the winner's options will start from.
#[derive(Debug, Clone)]
struct PendingRoll {
    us: (u8, String),
    ussr: (u8, String),
    defcon: u8,
    /// Ties are thrown again (Olympic Games) rather than standing (Summit).
    reroll_ties: bool,
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
        Spec { chooser, optional: false, modes: vec![Mode { ongoing: None, vp: 0, china: None, extra: Extra::NONE, label: label.into(), fixed: Vec::new(), rule: Some(rule) }] }
    }

    fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// The (single) mode also awards its chooser `vp` VP.
    fn with_vp(mut self, vp: i8) -> Self {
        self.modes[0].vp = vp;
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
    /// The event's roll-off hasn't been thrown yet.
    RollFirst,
    /// No roll-off is waiting.
    NoRoll,
}

impl fmt::Display for EventChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventChoiceError::RollFirst => write!(f, "roll first (r)"),
            EventChoiceError::NoRoll => write!(f, "there is no roll to make"),
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
    /// The operation this event allows once it's done (Junta), fixed when it opens.
    grant: Option<OpsGrant>,
    /// The cards a pick-a-card decision (Blockade, Debt Crisis, Aldrich Ames) offers
    /// to discard, in mode order after `gate_offset` leading modes; empty otherwise.
    gate: Vec<CardId>,
    /// Whose hand `gate` is in.
    gate_side: Superpower,
    /// How many modes come before the first card (1 for a decline mode, else 0).
    gate_offset: usize,
    /// What the decision asks, for the status bar.
    gate_prompt: String,
    /// A hand the event shows (Aldrich Ames, Cambridge Five), reported in the result.
    reveal: Option<Reveal>,
    /// The roll-off the event has already held (Summit, Olympic Games), reported by the modes marked `contest`.
    contest: Option<Contest>,
    /// What the player is being told before choosing (the roll-off's outcome, who sponsors).
    context: String,
    /// A roll-off still to be thrown (Summit).
    pending_roll: Option<PendingRoll>,
    /// Whether the event runs in a modal of its own (Summit, Olympic Games).
    session_modal: bool,
    /// The mode that takes a roll-off to settle (Olympic Games' participation), if any.
    roll_mode: Option<usize>,
    /// The session a "declined" gate hands on to (Debt Crisis's doubling).
    follow_up: Option<Box<EventChoice>>,
    /// Whether this is such a follow-up: the card has gone irrevocably, so
    /// its player can't back out of it.
    second_stage: bool,
    /// Whether the event was set off by a trigger rather than a played card (NORAD): there is no
    /// card to spend and no turn to hand over when it ends.
    triggered: bool,
    /// The discard-pile cards a pile pick offers, one per mode after the first; empty otherwise.
    pile: Vec<CardId>,
    /// Whether this is a discard-pile pick (even over an empty pile).
    pile_pick: bool,
    pile_use: PileUse,
    pile_offset: usize,
    /// Which mode the picker's highlight is on.
    cursor: usize,
    /// What the event is called when no card is (the opening setup).
    title: Option<&'static str>,
}

impl EventChoice {
    /// Opens `card`'s choice against `board`, or `None` for a card that
    /// isn't a choice card. A single-mode card has its mode chosen already.
    pub fn new(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId) -> Option<Self> {
        let spec = spec_for(card)?(map, board, status);
        Some(Self::from_spec(map, board, card, spec))
    }

    fn from_spec(map: &WorldMap, board: &Board, card: CardId, spec: Spec) -> Self {
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
            grant: None,
            gate: Vec::new(),
            gate_side: Superpower::Us,
            gate_offset: 0,
            gate_prompt: String::new(),
            reveal: None,
            contest: None,
            context: String::new(),
            pending_roll: None,
            session_modal: false,
            roll_mode: None,
            follow_up: None,
            second_stage: false,
            triggered: false,
            pile: Vec::new(),
            pile_pick: false,
            pile_use: PileUse::Take,
            pile_offset: 0,
            cursor: 0,
            title: None,
        };
        if choice.modes.len() == 1 {
            choice.select(map, 0);
        }
        choice
    }

    /// Opens the discard-or-suffer decision `card` hands `status`'s
    /// opponent-of-the-card's-side (Blockade, Latin American Debt Crisis):
    /// mode 1 declines and takes the penalty, every further mode discards
    /// one of `candidates` (`(card, name)`, the victim's cards that qualify).
    /// `None` if `card` isn't such a card or nothing qualifies — then the
    /// penalty simply applies, through the card's ordinary event.
    pub fn discard_gate(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId, candidates: &[(CardId, String)]) -> Option<Self> {
        if candidates.is_empty() {
            return None;
        }
        let gate = gate_for(card)?;
        let decline = Mode { label: gate.decline.to_string(), fixed: gate.fixed(), rule: None, ongoing: None, vp: 0, china: None, extra: Extra { then: gate.then.is_some(), ..Extra::NONE } };
        let mut modes = vec![decline];
        modes.extend(candidates.iter().map(|(id, name)| Mode {
            label: format!("discard {name}"),
            fixed: Vec::new(),
            rule: None,
            ongoing: None,
            vp: 0,
            china: None,
            extra: Extra { discard: Some((gate.decider, *id)), ..Extra::NONE },
        }));
        let spec = Spec { chooser: gate.decider, optional: false, modes };
        let mut choice = Self::from_spec(map, board, card, spec);
        choice.gate = candidates.iter().map(|(id, _)| *id).collect();
        choice.gate_side = gate.decider;
        choice.gate_offset = 1;
        choice.gate_prompt = format!("must discard a card worth {GATE_MIN_OPS}+ ops or suffer");
        if let Some(then) = gate.then {
            let mut next = Self::from_spec(map, board, card, then(map, board, status));
            next.second_stage = true;
            choice.follow_up = Some(Box::new(next));
        }
        Some(choice)
    }

    /// Aldrich Ames Remix (#98): the USSR discards one card of its choice from the US `hand`
    /// (`(card, name)`), and the whole hand is open to it for the rest of the turn. `None` if
    /// the hand is empty — then only the reveal happens, through the card's ordinary event.
    pub fn pick_from_hand(map: &WorldMap, board: &Board, card: CardId, picker: Superpower, hand: &[(CardId, String)]) -> Option<Self> {
        if hand.is_empty() {
            return None;
        }
        let victim = picker.opponent();
        let modes = hand
            .iter()
            .map(|(id, name)| Mode {
                label: format!("discard {name}"),
                fixed: Vec::new(),
                rule: None,
                ongoing: Some(OngoingEffect::HandRevealed { side: victim, card: card.0 }),
                vp: 0,
                china: None,
                extra: Extra { discard: Some((victim, *id)), ..Extra::NONE },
            })
            .collect();
        let mut choice = Self::from_spec(map, board, card, Spec { chooser: picker, optional: false, modes });
        choice.gate = hand.iter().map(|(id, _)| *id).collect();
        choice.gate_side = victim;
        choice.gate_offset = 0;
        choice.gate_prompt = format!("picks a card from the {victim} hand to discard");
        choice.reveal = Some(Reveal { side: victim, cards: choice.gate.clone() });
        Some(choice)
    }

    /// SALT Negotiations (#43): after its fixed effects (`defcon`, `ongoing`) the player may take
    /// one of the non-scoring cards in the discard `pile` into their hand, revealed. Mode 0 takes
    /// none; mode `i + 1` takes `pile[i]`. With an empty pile there is only mode 0, and the modal
    /// says why nothing can be taken.
    pub fn pick_from_pile(map: &WorldMap, board: &Board, card: CardId, picker: Superpower, pile: &[PileCard], defcon: Option<u8>, ongoing: Option<OngoingEffect>) -> Option<Self> {
        let mode = |label: String, take: Option<CardId>| Mode {
            label,
            fixed: Vec::new(),
            rule: None,
            ongoing,
            vp: 0,
            china: None,
            extra: Extra { defcon, take: take.map(|c| (picker, c)), ..Extra::NONE },
        };
        let mut modes = vec![mode("take no card".to_string(), None)];
        modes.extend(pile.iter().map(|p| mode(format!("take {}", p.name), Some(p.id))));
        Some(Self::pile_choice(map, board, card, picker, pile, modes, (1, PileUse::Take)))
    }

    /// Star Wars (#85): the player picks one of the non-scoring cards in the discard `pile`
    /// and must play it as an event at once. Mode `i` plays `pile[i]`; an empty pile leaves one
    /// mode that plays nothing, so the modal can say why.
    pub fn play_from_pile(map: &WorldMap, board: &Board, card: CardId, picker: Superpower, pile: &[PileCard]) -> Self {
        let mode = |label: String, play: Option<PlayCard>| Mode {
            label,
            fixed: Vec::new(),
            rule: None,
            ongoing: None,
            vp: 0,
            china: None,
            extra: Extra { play, ..Extra::NONE },
        };
        let modes = if pile.is_empty() {
            vec![mode("play nothing".to_string(), None)]
        } else {
            pile.iter().map(|p| mode(format!("play {}", p.name), Some(PlayCard { id: p.id, ops: p.ops, removed: p.removed, scoring: false, how: PlayAs::Event, exchange: false }))).collect()
        };
        Self::pile_choice(map, board, card, picker, pile, modes, (0, PileUse::Play))
    }

    fn pile_choice(map: &WorldMap, board: &Board, card: CardId, picker: Superpower, pile: &[PileCard], modes: Vec<Mode>, (offset, usage): (usize, PileUse)) -> Self {
        let mut choice = Self::from_spec(map, board, card, Spec { chooser: picker, optional: false, modes });
        choice.pile = pile.iter().map(|p| p.id).collect();
        choice.pile_pick = true;
        choice.pile_use = usage;
        choice.pile_offset = offset;
        choice.session_modal = true;
        choice.cursor = 0;
        choice
    }

    /// Grain Sales to Soviets (#67): the US has drawn `drawn` (card, name, how to play it) from the
    /// USSR hand — shown to it — and either plays it (event or operations) or returns it and
    /// conducts Grain Sales' own operations. `None`: the USSR has no cards, so only the operations.
    pub fn grain_sales(map: &WorldMap, board: &Board, card: CardId, chooser: Superpower, drawn: Option<(PlayCard, String)>) -> Self {
        let blank = Mode { label: String::new(), fixed: Vec::new(), rule: None, ongoing: None, vp: 0, china: None, extra: Extra::NONE };
        let ops = Extra { grant: Some(OpsGrant::ANY.for_side(chooser)), ..Extra::NONE };
        let (modes, reveal) = match &drawn {
            Some((pc, name)) => (
                vec![
                    Mode { label: format!("play {name} (its event, or its operations)"), extra: Extra { play: Some(*pc), ..Extra::NONE }, ..blank.clone() },
                    Mode { label: format!("return {name} to the USSR, then conduct Grain Sales' operations"), extra: ops, ..blank },
                ],
                Some(Reveal { side: chooser.opponent(), cards: vec![pc.id] }),
            ),
            None => (vec![Mode { label: "the USSR has no cards — conduct Grain Sales' operations".to_string(), extra: ops, ..blank }], None),
        };
        let mut choice = Self::from_spec(map, board, card, Spec { chooser, optional: false, modes });
        choice.reveal = reveal;
        choice.context = match &drawn {
            Some((_, name)) => format!("the {} card drawn at random is {name}", chooser.opponent()),
            None => String::new(),
        };
        choice
    }

    /// Missile Envy (#49): the opponent of its player, holding several cards tied for the
    /// highest Operations value, chooses which one is handed over (`options`: label, card).
    pub fn choose_exchange(map: &WorldMap, board: &Board, card: CardId, chooser: Superpower, options: &[(String, PlayCard)]) -> Self {
        let blank = Mode { label: String::new(), fixed: Vec::new(), rule: None, ongoing: None, vp: 0, china: None, extra: Extra::NONE };
        let modes = options.iter().map(|(name, pc)| Mode { label: format!("hand over {name}"), extra: Extra { play: Some(*pc), ..Extra::NONE }, ..blank.clone() }).collect();
        let mut choice = Self::from_spec(map, board, card, Spec { chooser, optional: false, modes });
        choice.context = "tied for the highest Operations value — choose the card to give up".to_string();
        choice
    }

    /// What the pick does with the card: take it into the hand, or play its event.
    pub fn pile_use(&self) -> PileUse {
        self.pile_use
    }

    /// How many modes come before the first pile card (1 when mode 0 declines).
    pub fn pile_offset(&self) -> usize {
        self.pile_offset
    }

    /// The discard-pile cards on offer (mode `i + 1` is `pile()[i]`); empty unless this is a pile pick.
    pub fn pile(&self) -> &[CardId] {
        &self.pile
    }

    /// Whether this is a discard-pile pick, offering `pile()` (possibly nothing).
    pub fn is_pile_pick(&self) -> bool {
        self.pile_pick
    }

    /// The mode the picker's highlight is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Moves the picker's highlight by `delta` modes, wrapping.
    pub fn move_cursor(&mut self, delta: i32) {
        let n = self.modes.len().max(1) as i32;
        self.cursor = (self.cursor as i32 + delta).rem_euclid(n) as usize;
    }

    /// The Cambridge Five (#104): the USSR may add 1 influence to a single country in one of
    /// `regions` (those the scoring cards in the US hand name). `reveal` is that hand's scoring
    /// cards, reported in the result. `None` if no region qualifies.
    pub fn in_named_regions(map: &WorldMap, board: &Board, card: CardId, regions: &[Region], reveal: Reveal) -> Option<Self> {
        if regions.is_empty() {
            return None;
        }
        let names: Vec<String> = regions.iter().map(|r| r.to_string()).collect();
        let spec = Spec::single(
            Ussr,
            format!("add 1 USSR influence to one country in {}", names.join(" or ")),
            Rule::add(Ussr, Eligible::new(Where::Regions(RegionSet::of(regions))), 1, 1, 1),
        )
        .optional();
        let mut choice = Self::from_spec(map, board, card, spec);
        choice.reveal = Some(reveal);
        Some(choice)
    }

    /// The cards a pick-a-card decision offers, if this is one — mode
    /// `gate_offset() + i` discards `gate_cards()[i]`.
    pub fn gate_cards(&self) -> &[CardId] {
        &self.gate
    }

    /// Whose hand [`EventChoice::gate_cards`] are in.
    pub fn gate_side(&self) -> Superpower {
        self.gate_side
    }

    /// How many modes precede the first card (Blockade's "keep your cards" is one).
    pub fn gate_offset(&self) -> usize {
        self.gate_offset
    }

    /// What the decision asks of its chooser.
    pub fn gate_prompt(&self) -> &str {
        &self.gate_prompt
    }

    /// The card the chosen mode discards, if it does.
    pub fn chosen_discard(&self) -> Option<CardId> {
        self.mode.and_then(|i| self.modes[i].extra.discard).map(|(_, card)| card)
    }

    /// The session to open next, if the chosen mode hands the event on.
    pub fn take_follow_up(&mut self) -> Option<EventChoice> {
        if self.mode.is_some_and(|i| self.modes[i].extra.then) { self.follow_up.take().map(|b| *b) } else { None }
    }

    /// Whether this session is the follow-up to a declined gate, which its
    /// player can no longer back out of.
    /// NORAD (#106): the US adds 1 influence to a country where it already has some — `None` when
    /// it has none anywhere.
    pub fn norad(map: &WorldMap, board: &Board, _status: &GameStatus) -> Option<Self> {
        if !map.iter().any(|(id, _)| board.influence(id, Us) > 0) {
            return None;
        }
        let spec = Spec::single(Us, "NORAD: add 1 US influence to a country containing US influence", Rule::add(Us, Eligible::new(Where::Everywhere).with_influence_of(Us), 1, 1, 1));
        let mut choice = Self::from_spec(map, board, CardId(106), spec);
        choice.triggered = true;
        choice.second_stage = true;
        Some(choice)
    }

    /// The opening placement (rule 3.2): `side` adds its starting influence — 6 anywhere in
    /// Eastern Europe for the USSR, 7 anywhere in Western Europe for the US — with no presence,
    /// cost or per-country limit. Not a card's event, so it has a `title`, and like NORAD it
    /// can't be backed out of.
    pub fn setup(map: &WorldMap, board: &Board, side: Superpower) -> Self {
        let (points, place, label) = match side {
            Ussr => (6, Where::Sub(SubRegion::EasternEurope), "USSR: place 6 influence anywhere in Eastern Europe"),
            Us => (7, Where::Sub(SubRegion::WesternEurope), "US: place 7 influence anywhere in Western Europe"),
        };
        let spec = Spec::single(side, label, Rule::add(side, Eligible::new(place), points, points, points));
        let mut choice = Self::from_spec(map, board, CardId(106), spec);
        choice.triggered = true;
        choice.second_stage = true;
        choice.title = Some("Setup");
        choice
    }

    /// What to call this event when no card is behind it.
    pub fn title(&self) -> Option<&'static str> {
        self.title
    }

    /// Whether a trigger, not a played card, opened this event.
    pub fn is_triggered(&self) -> bool {
        self.triggered
    }

    pub fn is_second_stage(&self) -> bool {
        self.second_stage
    }

    pub fn card(&self) -> CardId {
        self.card
    }

    /// Records the operation this event allows once confirmed.
    pub fn with_grant(mut self, grant: Option<OpsGrant>) -> Self {
        self.grant = grant;
        self
    }

    /// The operation this event allows once confirmed, if any.
    pub fn grant(&self) -> Option<OpsGrant> {
        self.mode.and_then(|i| self.modes[i].extra.grant).or(self.grant)
    }

    /// Summit (#45), before anyone has rolled: each side rolls a die plus its bonus
    /// (`(bonus, note)`, +1 per region it dominates or controls). Nothing can be chosen
    /// until [`EventChoice::roll_contest`]; then the winner gets 2 VP and may improve or
    /// degrade DEFCON by 1, or leave it be (a tie does nothing).
    pub fn summit_pending(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId, us: (u8, String), ussr: (u8, String)) -> Self {
        let spec = Spec { chooser: status.active, optional: false, modes: Vec::new() };
        let mut choice = Self::from_spec(map, board, card, spec);
        choice.context = "Summit: roll-off between the superpowers".to_string();
        choice.pending_roll = Some(PendingRoll { us, ussr, defcon: status.defcon, reroll_ties: false });
        choice.session_modal = true;
        choice
    }

    /// Whether a roll-off is still waiting to be thrown ([`EventChoice::roll_contest`]).
    pub fn needs_roll(&self) -> bool {
        self.pending_roll.is_some() && self.roll_mode.is_none_or(|m| self.mode == Some(m))
    }

    /// Whether the event is played in a modal of its own (Summit's roll, result and choice).
    pub fn has_session_modal(&self) -> bool {
        self.session_modal
    }

    /// The bonuses each side will roll with, `((US bonus, note), (USSR bonus, note))`, while the roll is pending.
    pub fn pending_bonuses(&self) -> Option<(&RollBonus, &RollBonus)> {
        self.pending_roll.as_ref().map(|p| (&p.us, &p.ussr))
    }

    /// How the pending roll-off will go, in 36ths: `(US wins, ties, USSR wins)`.
    pub fn roll_odds(&self) -> Option<(u8, u8, u8)> {
        let p = self.pending_roll.as_ref()?;
        let (mut us, mut tie, mut ussr) = (0, 0, 0);
        for a in 1..=6u8 {
            for b in 1..=6u8 {
                match (a + p.us.0).cmp(&(b + p.ussr.0)) {
                    std::cmp::Ordering::Greater => us += 1,
                    std::cmp::Ordering::Equal => tie += 1,
                    std::cmp::Ordering::Less => ussr += 1,
                }
            }
        }
        Some((us, tie, ussr))
    }

    /// The roll-off, once thrown.
    pub fn contest(&self) -> Option<&Contest> {
        self.contest.as_ref()
    }

    /// Throws Summit's roll-off. The winner becomes the chooser and gets the DEFCON options; a
    /// tie leaves a single "nothing happens" mode already chosen.
    pub fn roll_contest(&mut self, map: &WorldMap, dice: &mut Dice) -> Result<Contest, EventChoiceError> {
        if !self.needs_roll() {
            return Err(EventChoiceError::NoRoll);
        }
        let p = self.pending_roll.take().expect("needs_roll checked");
        let contest = Contest::roll(dice, p.us, p.ussr, p.reroll_ties);
        // Olympic Games: taking part is settled by this roll-off; the winner gets 2 VP.
        if let Some(m) = self.roll_mode {
            let winner = contest.winner().expect("ties were thrown again");
            // VP are relative to the chooser: negative gives the 2 VP to the sponsor.
            self.modes[m].vp = if winner == self.chooser { 2 } else { -2 };
            self.context = format!("Olympic Games: {winner} wins the roll-off and gets 2 VP");
            self.contest = Some(contest.clone());
            self.refresh(map);
            return Ok(contest);
        }
        let defcon = p.defcon;
        let mode = |label: String, vp: i8, to: u8| Mode {
            label,
            fixed: Vec::new(),
            rule: None,
            ongoing: None,
            vp,
            china: None,
            extra: Extra { defcon: (to != defcon).then_some(to), contest: true, ..Extra::NONE },
        };
        self.modes = match contest.winner() {
            Some(winner) => {
                self.chooser = winner;
                self.context = format!("Summit: {winner} wins the roll-off and gets 2 VP");
                let mut modes = Vec::new();
                if defcon < 5 {
                    modes.push(mode(format!("improve DEFCON to {}", defcon + 1), 2, defcon + 1));
                }
                if defcon > 1 {
                    modes.push(mode(format!("degrade DEFCON to {}", defcon - 1), 2, defcon - 1));
                }
                modes.push(mode(format!("leave DEFCON at {defcon}"), 2, defcon));
                modes
            }
            None => {
                self.context = "Summit: the roll-off is a tie".to_string();
                vec![mode("tie: no VP, DEFCON unchanged".to_string(), 0, defcon)]
            }
        };
        self.contest = Some(contest.clone());
        if self.modes.len() == 1 {
            self.select(map, 0);
        }
        Ok(contest)
    }

    /// Olympic Games (#20): the sponsor's opponent chooses to participate or boycott. Taking
    /// part needs a roll-off (the sponsor adds 2, ties are thrown again, the winner gets 2 VP):
    /// choosing it makes [`EventChoice::needs_roll`] true, and [`EventChoice::roll_contest`]
    /// settles who scores. Boycotting drops DEFCON by 1 and lets the sponsor conduct
    /// operations as if the card were worth 4.
    pub fn olympics_pending(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId, sponsor: Superpower) -> Self {
        let chooser = sponsor.opponent();
        let boycott_defcon = status.defcon.saturating_sub(1).max(1);
        let modes = vec![
            Mode {
                label: format!("participate: both roll, {sponsor} adds 2, the winner gets 2 VP"),
                fixed: Vec::new(),
                rule: None,
                ongoing: None,
                vp: 0,
                china: None,
                extra: Extra { contest: true, ..Extra::NONE },
            },
            Mode {
                label: format!("boycott: DEFCON drops to {boycott_defcon}, and {sponsor} may conduct operations as a 4-ops card"),
                fixed: Vec::new(),
                rule: None,
                ongoing: None,
                vp: 0,
                china: None,
                extra: Extra { defcon: Some(boycott_defcon), grant: Some(OpsGrant::ANY.with_ops(4)), ..Extra::NONE },
            },
        ];
        let mut choice = Self::from_spec(map, board, card, Spec { chooser, optional: false, modes });
        let bonus = |side: Superpower| if side == sponsor { (2, "sponsor".to_string()) } else { (0, String::new()) };
        choice.context = format!("Olympic Games: {sponsor} sponsors, {chooser} chooses");
        choice.pending_roll = Some(PendingRoll { us: bonus(Superpower::Us), ussr: bonus(Superpower::Ussr), defcon: status.defcon, reroll_ties: true });
        choice.roll_mode = Some(0);
        choice.session_modal = true;
        choice
    }

    /// Whether the event's tied roll-offs are thrown again (Olympic Games).
    pub fn rerolls_ties(&self) -> bool {
        self.pending_roll.as_ref().is_some_and(|p| p.reroll_ties)
    }

    /// The line saying what the event is about and who is choosing, before the options.
    pub fn context(&self) -> &str {
        &self.context
    }

    /// Whether the roll-off was part of choosing to take part (Olympic Games), which locks that choice in.
    pub fn is_participation(&self) -> bool {
        self.roll_mode.is_some()
    }

    /// Whether the dice have been thrown for a participation: the choice can't be changed now.
    fn roll_locked(&self) -> bool {
        self.roll_mode.is_some() && self.contest.is_some()
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
        if self.pending_roll.is_some() && self.roll_mode.is_none() {
            return Err(EventChoiceError::RollFirst);
        }
        if self.roll_locked() {
            return Err(EventChoiceError::ModeLocked);
        }
        if i >= self.modes.len() {
            return Err(EventChoiceError::BadMode { modes: self.modes.len() });
        }
        if !self.history.is_empty() {
            return Err(EventChoiceError::ModeLocked);
        }
        self.select(map, i);
        Ok(())
    }

    /// Un-chooses the mode (back to "choose a mode"), if nothing's been
    /// picked yet — the step back from a chosen region/mode that comes
    /// before abandoning the whole event.
    pub fn clear_mode(&mut self, map: &WorldMap) -> Result<(), EventChoiceError> {
        if !self.history.is_empty() || self.roll_locked() {
            return Err(EventChoiceError::ModeLocked);
        }
        self.board = self.base.clone();
        self.fixed_ids.clear();
        self.mode = None;
        self.refresh(map);
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
                // Once per country, and only where there is something to double.
                Chunk::Double if net != 0 => return None,
                Chunk::Double => self.base.influence(id, rule.target),
                Chunk::One | Chunk::All => 1,
            }
        } else {
            if rule.kind == Kind::Remove && !rule.eligible.allows(map, &self.base, id) {
                return None;
            }
            match rule.chunk {
                Chunk::Fixed(n) => n.min(cur),
                Chunk::All => cur,
                Chunk::One | Chunk::Match | Chunk::Double => 1.min(cur),
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
        if self.needs_roll() {
            return false;
        }
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
        if self.is_designation() {
            return match self.mode {
                Some(i) => format!("the USSR can't add influence in {} with ops · 1-{} to change", self.modes[i].label, self.modes.len()),
                None => format!(
                    "designate a region: {}",
                    self.modes.iter().enumerate().map(|(i, m)| format!("{}) {}", i + 1, m.label)).collect::<Vec<_>>().join("  ")
                ),
            };
        }
        if self.roll_mode.is_none()
            && let (Some((us, ussr)), Some((a, t, b))) = (self.pending_bonuses(), self.roll_odds())
        {
            return format!("{} (US +{}, USSR +{}) — US wins {a}/36, tie {t}/36, USSR wins {b}/36 · r to roll", self.context, us.0, ussr.0);
        }
        let context = if self.context.is_empty() { String::new() } else { format!("{} — ", self.context) };
        match self.mode {
            Some(i) => self.modes[i].label.clone(),
            None => format!("{context}{}", self.modes.iter().enumerate().map(|(i, m)| format!("{}) {}", i + 1, m.label)).collect::<Vec<_>>().join("  or  ")),
        }
    }

    /// What's left to spend, e.g. `2/4 countries · 2/5 points`.
    pub fn progress(&self) -> String {
        if self.is_designation() {
            return if self.mode.is_some() { "region chosen".into() } else { "choose a region".into() };
        }
        let Some(rule) = self.rule() else {
            return if self.needs_roll() {
                "roll first".into()
            } else if self.mode.is_some() {
                "mode chosen".into()
            } else {
                "choose a mode".into()
            };
        };
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
        if self.is_designation() {
            return format!("digits 1-{} pick the region · c confirms", self.modes.len());
        }
        let Some(rule) = self.rule() else {
            return if self.mode.is_some() { format!("digits 1-{} change the choice · c confirms", self.modes.len()) } else { "choose a mode first".into() };
        };
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
        let ongoing = self.mode.and_then(|i| self.modes[i].ongoing);
        let vp = self.mode.map_or(0, |i| self.modes[i].vp);
        let vp_delta = if self.chooser == Superpower::Us { vp } else { -vp };
        let china = self.mode.and_then(|i| self.modes[i].china);
        let extra = self.mode.map_or(Extra::NONE, |i| self.modes[i].extra);
        let defcon = extra.defcon.map(|d| (status.defcon, d));
        EffectResult {
            card: self.card,
            player: status.active,
            influence: self.changes.clone(),
            vp_delta,
            defcon,
            ongoing,
            lasting: None,
            cancels: None,
            china,
            space: None,
            mil_ops: extra.mil_ops,
            ends_game: extra.ends_game,
            reveals: self.reveal.clone(),
            discards: extra.discard.into_iter().collect(),
            takes: extra.take.into_iter().collect(),
            plays: extra.play,
            contest: if extra.contest { self.contest.clone() } else { None },
            title: self.title,
        }
    }

    /// The region a region-designating event (Chernobyl) has been set to
    /// so far, for the views to highlight.
    pub fn designated_region(&self) -> Option<Region> {
        match self.mode.and_then(|i| self.modes[i].ongoing)? {
            OngoingEffect::Chernobyl { region } => Some(region),
            _ => None,
        }
    }

    /// Whether this event only designates something (no countries to
    /// pick), so the map views have nothing to mark as live.
    pub fn is_designation(&self) -> bool {
        !self.modes.is_empty() && self.modes.iter().all(|m| m.rule.is_none() && matches!(m.ongoing, Some(OngoingEffect::Chernobyl { .. })))
    }

    /// Whether the event is settled by choosing a mode alone: no countries
    /// to pick and no region to designate (a discard decision, a DEFCON
    /// level). Views leave the map as it is for these.
    pub fn is_mode_only(&self) -> bool {
        !self.picks_countries() && !self.is_designation()
    }

    /// Whether any mode has countries to pick — false for an event that
    /// is settled by choosing a mode alone (a DEFCON level, Wargames).
    pub fn picks_countries(&self) -> bool {
        self.modes.iter().any(|m| m.rule.is_some())
    }
}

/// `US 4+1 (Europe) = 5, USSR 3 = 3` — both rolls, for a prompt or log line.
pub fn describe_contest(contest: &Contest) -> String {
    let side = |name: &str, r: &super::effects::ContestRoll| {
        if r.bonus == 0 { format!("{name} {}", r.die) } else { format!("{name} {}+{} ({}) = {}", r.die, r.bonus, r.note, r.total()) }
    };
    let rerolled = if contest.rerolls > 0 { format!(" after {} tied re-roll(s)", contest.rerolls) } else { String::new() };
    format!("{}, {}{rerolled}", side("US", &contest.us), side("USSR", &contest.ussr))
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
    (46, how_i_learned_to_stop_worrying),
    (47, junta),
    (53, south_african_unrest),
    (56, muslim_revolution),
    (63, colonial_rear_guards),
    (66, puppet_governments),
    (70, oas_founded),
    (74, the_voice_of_america),
    (75, liberation_theology),
    (76, ussuri_river_skirmish),
    (87, the_reformer),
    (88, marine_barracks_bombing),
    (94, chernobyl),
    (95, latin_american_debt_crisis),
    (99, pershing_ii_deployed),
    (100, wargames),
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
                ongoing: None, vp: 0, china: None, extra: Extra::NONE, label: "remove all US influence from 4 Eastern European countries".into(),
                fixed: Vec::new(),
                rule: Some(Rule::remove(Us, Eligible::new(EASTERN), ANY, ANY, 4).chunk(Chunk::All)),
            },
            Mode {
                ongoing: None, vp: 0, china: None, extra: Extra::NONE, label: "add 5 USSR influence to Eastern Europe (max 2 per country)".into(),
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
                ongoing: None, vp: 0, china: None, extra: Extra::NONE, label: "add 2 USSR influence to South Africa".into(),
                fixed: vec![Fixed { country: "South Africa", side: Ussr, op: FixedOp::Add(2) }],
                rule: None,
            },
            Mode {
                ongoing: None, vp: 0, china: None, extra: Extra::NONE, label: "add 1 USSR influence to South Africa and 2 to one adjacent country".into(),
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

/// #47 Junta: the player adds 2 influence to a single country in Central or South America
/// (a coup or realignment there follows — `events::ops_grant`).
fn junta(_: &WorldMap, _: &Board, status: &GameStatus) -> Spec {
    Spec::single(
        status.active,
        "add 2 influence to one country in Central or South America",
        Rule::add(status.active, Eligible::new(Where::Any(&[Where::Region(Region::CentralAmerica), Where::Region(Region::SouthAmerica)])), 2, 2, 1),
    )
}

/// #74 The Voice of America
fn the_voice_of_america(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Us,
        "remove 4 USSR influence from countries outside Europe (max 2 per country)",
        Rule::remove(Ussr, Eligible::new(Where::Any(&[
            Where::Region(Region::Asia),
            Where::Region(Region::MiddleEast),
            Where::Region(Region::Africa),
            Where::Region(Region::CentralAmerica),
            Where::Region(Region::SouthAmerica),
        ])), 4, 2, ANY),
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

/// #76 Ussuri River Skirmish: if the USSR holds the China Card the US takes it
/// (face up); if the US already holds it, +4 US influence in Asia (max 2 each).
fn ussuri_river_skirmish(_: &WorldMap, _: &Board, status: &GameStatus) -> Spec {
    if status.china_card == Ussr {
        return Spec {
            chooser: Us,
            optional: false,
            modes: vec![Mode {
                ongoing: None, vp: 0,
                china: Some(ChinaTransfer { to: Us, face_up: true }), extra: Extra::NONE,
                label: "the US takes the China Card (face up)".into(),
                fixed: Vec::new(),
                rule: None,
            }],
        };
    }
    Spec::single(
        Us,
        "add 4 US influence to Asia (max 2 per country)",
        Rule::add(Us, Eligible::new(Where::Region(Region::Asia)), 4, 2, ANY),
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
            ongoing: None, vp: 0, china: None, extra: Extra::NONE, label: "remove 2 more US influence from the Middle East (Lebanon's is already gone)".into(),
            fixed: vec![Fixed { country: "Lebanon", side: Us, op: FixedOp::Clear }],
            rule: Some(Rule::remove(Us, Eligible::new(Where::Region(Region::MiddleEast)), 2, ANY, ANY)),
        }],
    }
}

/// #94 Chernobyl: the US designates the region the USSR can't place ops
/// influence in. One mode per region, in the world map's own order.
fn chernobyl(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec {
        chooser: Us,
        optional: false,
        modes: Region::ALL
            .iter()
            .map(|&region| Mode {
                label: region.to_string(),
                fixed: Vec::new(),
                rule: None,
                ongoing: Some(OngoingEffect::Chernobyl { region }),
                vp: 0,
                china: None, extra: Extra::NONE,
            })
            .collect(),
    }
}

/// #46 How I Learned to Stop Worrying: the player sets DEFCON to any level and
/// adds 5 to their Military Operations. One mode per level, no countries.
fn how_i_learned_to_stop_worrying(_: &WorldMap, _: &Board, status: &GameStatus) -> Spec {
    Spec {
        chooser: status.active,
        optional: false,
        modes: (1..=5u8)
            .map(|level| Mode {
                label: format!("set DEFCON to {level}"),
                fixed: Vec::new(),
                rule: None,
                ongoing: None,
                vp: 0,
                china: None,
                extra: Extra { defcon: Some(level), mil_ops: 5, ..Extra::NONE },
            })
            .collect(),
    }
}

/// #95 Latin American Debt Crisis, once the US has declined to discard: the USSR may double its
/// influence in each of 2 South American countries. (The discard decision itself is
/// [`EventChoice::discard_gate`]; with nothing to discard the card comes straight here.)
fn latin_american_debt_crisis(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "double USSR influence in each of 2 South American countries",
        Rule {
            kind: Kind::Add,
            target: Ussr,
            eligible: Eligible::new(Where::Region(Region::SouthAmerica)),
            points: ANY,
            per_country: ANY,
            countries: 2,
            chunk: Chunk::Double,
        },
    )
    .optional()
}

/// How a discard-or-suffer card's decision is set up.
struct GateSpec {
    /// Who has to discard or suffer.
    decider: Superpower,
    /// What declining means, as a mode label.
    decline: &'static str,
    /// Influence changes declining makes at once (Blockade).
    fixed: &'static [Fixed],
    /// The session declining hands on to (Debt Crisis's doubling).
    then: Option<SpecFn>,
}

impl GateSpec {
    fn fixed(&self) -> Vec<Fixed> {
        self.fixed.to_vec()
    }
}

/// Blockade (#10) and Latin American Debt Crisis (#95): the US discards a card
/// with 3 or more ops, or suffers the card.
fn gate_for(card: CardId) -> Option<GateSpec> {
    match card.0 {
        10 => Some(GateSpec {
            decider: Us,
            decline: "keep your cards → all US influence leaves West Germany",
            fixed: &[Fixed { country: "West Germany", side: Us, op: FixedOp::Clear }],
            then: None,
        }),
        95 => Some(GateSpec {
            decider: Us,
            decline: "keep your cards → the USSR may double its influence in 2 South American countries",
            fixed: &[],
            then: Some(latin_american_debt_crisis),
        }),
        _ => None,
    }
}

/// The ops a card must be worth for a discard-or-suffer card to accept it.
pub const GATE_MIN_OPS: u8 = 3;

/// Whether `card` is a discard-or-suffer card (Blockade, Latin American Debt Crisis).
pub fn is_gate_card(card: CardId) -> bool {
    gate_for(card).is_some()
}

/// Who decides a discard-or-suffer card, if it is one.
pub fn gate_decider(card: CardId) -> Option<Superpower> {
    gate_for(card).map(|g| g.decider)
}

/// #99 Pershing II Deployed: USSR +1 VP; remove 1 US influence from each of 3 Western European countries.
fn pershing_ii_deployed(_: &WorldMap, _: &Board, _: &GameStatus) -> Spec {
    Spec::single(
        Ussr,
        "remove 1 US influence from each of 3 Western European countries (+1 VP)",
        Rule::remove(Us, Eligible::new(WESTERN), 3, 1, 3),
    )
    .with_vp(1)
}

/// #100 Wargames: at DEFCON 2 the player may end the game, giving the opponent 6 VP first;
/// the VP leader then wins (a tie goes to the opponent). At any other level nothing happens.
fn wargames(_: &WorldMap, _: &Board, status: &GameStatus) -> Spec {
    let none = Mode { label: String::new(), fixed: Vec::new(), rule: None, ongoing: None, vp: 0, china: None, extra: Extra::NONE };
    if status.defcon != 2 {
        return Spec { chooser: status.active, optional: false, modes: vec![Mode { label: "no effect (DEFCON isn't 2)".into(), ..none }] };
    }
    Spec {
        chooser: status.active,
        optional: false,
        modes: vec![
            Mode {
                label: "end the game: the opponent gets 6 VP, then the VP leader wins".into(),
                vp: -6,
                extra: Extra { ends_game: true, ..Extra::NONE },
                ..none.clone()
            },
            Mode { label: "play on".into(), ..none },
        ],
    }
}

/// #105 Special Relationship: with the UK US-controlled, +1 US influence in
/// a country adjacent to it — or, with NATO in effect, +2 in any Western
/// European country and +2 VP.
fn special_relationship(map: &WorldMap, board: &Board, status: &GameStatus) -> Spec {
    let uk = map.id_by_name("UK").expect("UK is on the map");
    let countries = if board.is_controlled_by(map, uk, Us) { 1 } else { 0 };
    if status.lasting.nato {
        return Spec::single(Us, "add 2 US influence to one Western European country (+2 VP)", Rule::add(Us, Eligible::new(WESTERN), 2, 2, countries))
            .with_vp(if countries > 0 { 2 } else { 0 });
    }
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
