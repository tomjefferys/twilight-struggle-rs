//! Turn structure: alternating US/USSR action rounds, enforced. `Game`
//! owns the four things that have to move in lockstep — the status
//! (turn/action round/active side), the board, whichever card is in play,
//! and whichever [`Operation`] is open — so "an operation spends a played
//! card's ops, and closing it passes the turn" lives in exactly one place,
//! rather than being duplicated between the REPL and interactive mode.
//!
//! This module is deliberately the only place that mutates
//! [`GameStatus::active`], `turn`, or `action_round` — see [`Game::advance`].
//! It is also, by design, the whole surface a future AI opponent needs:
//! nothing here touches a terminal, and `Game` is cheap to clone (per
//! [`Board`]'s own design note) so a lookahead search can try a line of
//! play on a copy without touching the real game.
//!
//! Rule 6.x's one-card-one-action shape means a turn spends exactly one
//! card on exactly one operation: [`Game::play_card`] takes a card from the
//! active side's hand (nothing about its text or event — just its ops
//! value), then [`Game::begin`] opens an operation with that many ops, and
//! [`Game::confirm`]/[`Game::cancel`] — the only two ways to close it —
//! both discard the card and hand the turn to the other side. Ending a turn
//! with ops unspent is allowed (a confirm needn't spend everything,
//! matching the ops modules' own behaviour) and simply forfeits them;
//! [`Game::pass`] is refused with a card in play — there's nothing to pass
//! on once a card's been committed to the turn, so [`Game::return_card`] is
//! the way out of a card played by mistake.
//!
//! [`Game::play_event`] is the other thing a played card can fund, sitting
//! alongside `begin` rather than inside it: it resolves `card`'s own text
//! (via [`crate::events::resolve`]) instead of opening an [`Operation`].
//! A scoring card can *only* go this way — it has no ops to spend, so
//! `begin` refuses it — which is why `play_card` no longer refuses a
//! scoring card itself; only `begin` does, now that there's something
//! else to do with one. Nothing here yet lets a card be played for ops
//! *and* its event in either order (the dual-use rule for an opponent's
//! card) — `PlayedCard` has no "which half is still open" state because
//! every event implemented so far (scoring cards) has nothing to combine
//! with: a future stage that adds an ops-and-event card will need to grow
//! that, not reshape it.
//!
//! A scoring event can end the game outright — Europe Scoring's Control
//! tier, or VP simply reaching ±20 — which [`Game::winner`] reports.
//! Once set, it never clears and every forward-moving method
//! ([`Game::play_card`], [`Game::begin`], [`Game::pass`]) refuses with
//! [`GameError::GameOver`].

use std::fmt;

use crate::board::Board;
use crate::cards::{CardCatalog, CardId, CardPhase, Hands, CHINA_CARD};
use crate::country::{CountryId, Region, Superpower};
use crate::dice::Dice;
use crate::cards::CardSide;
use crate::events::choice::{EventChoiceError, PileCard, Sign};
use crate::events::EventChoice;
use crate::events::{self, scoring, war, EffectResult, EventOutcome, OpsGrant, PlayAs, PlayCard, WarResult};
use crate::log::{CoupAftermath, Event, GameLog, LogEntry, TurnEndReport};
use crate::map::WorldMap;
use crate::ongoing::{LastingEffect, TurnEffects};
use crate::ops::{
    CoupError, InfluencePlacement, Operation, PlacementError, RealignError, Realignment, RollResult,
};
use crate::ops::{self, Coup, CoupResult};
use crate::scenario::Scenario;
use crate::space::{self, SpaceError, SpaceResult};
use crate::status::{hand_size_for_turn, rounds_for_turn, GameStatus, TURN_RANGE};

/// Why the game ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictoryReason {
    /// The VP track hit +20 (US) or -20 (USSR).
    Vp,
    /// Europe Scoring's Control tier (rule 10.1) — the one region card
    /// whose Control is an outright win rather than a VP value.
    EuropeControl,
    /// DEFCON reached 1 (rule 5.1): the side that played the event
    /// degrading it loses, so `Victory::side` is its opponent.
    Defcon,
    /// Wargames: the player ended the game, the VP leader winning.
    Wargames,
    /// A coup while Cuban Missile Crisis was in force: the coup-maker's opponent wins.
    CubanMissileCrisis,
    /// A side still held a scoring card when the turn ended (rule 3.2.1): its opponent wins.
    HeldScoringCard,
    /// Turn 10 ended and final scoring left one side ahead on VP — or level, a draw.
    FinalScoring,
}

/// What a side whose action rounds are escape attempts (Bear Trap, Quagmire) has to do with this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    /// Discard one of these (Operations cards worth 2+) and roll 1-4 to escape.
    Escape(Vec<CardId>),
    /// No such card: play the scoring cards held, one per round.
    PlayScoring,
    /// Nothing to discard and nothing to play: the round is skipped (`pass`).
    Skip,
}

/// One escape attempt: the card discarded, the die, and whether it sprang the trap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrapResult {
    /// Bear Trap or Quagmire.
    pub trap: CardId,
    pub side: Superpower,
    pub discarded: CardId,
    pub roll: u8,
    pub escaped: bool,
}

/// The standard deck, parsed once — for rules (the trap's "2+ Operations card") that only have a
/// card id to go on.
fn standard_cards() -> &'static CardCatalog {
    static CARDS: std::sync::OnceLock<CardCatalog> = std::sync::OnceLock::new();
    CARDS.get_or_init(|| CardCatalog::standard().expect("the standard deck loads"))
}

/// The game is over: who won (`None` for a draw), and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Victory {
    pub side: Option<Superpower>,
    pub reason: VictoryReason,
}

/// Which kind of operation [`Game::begin`] should open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Influence,
    Realign,
    Coup,
}

/// What [`Game::roll`] resolved — a realignment roll or a coup's one
/// attempt, whichever kind of operation was open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollOutcome {
    Realign(RollResult),
    Coup(CoupResult),
    /// A chosen-target war card's roll — which also closed the event.
    War(WarResult),
}

/// Why a `Game` method was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameError {
    /// An operation is already open; finish or cancel it first.
    OperationOpen { verb: &'static str, remaining: u8, total: u8 },
    /// No operation is open, but this call needed one.
    NoOperation,
    /// An operation is open, but not the kind this call needed — `open`
    /// names the one that's actually open (its [`Operation::verb`]).
    WrongKind { open: &'static str },
    /// Nothing has been placed yet, so there's nothing to undo.
    NothingToUndo,
    /// The open operation has a resolved die roll (or coup attempt) that
    /// can't be taken back — `verb` names what was rolled, for the
    /// message ("realignment roll" or "coup").
    CannotUndo { verb: &'static str },
    /// [`Game::abandon`] refused a [`Realignment`](crate::ops::Realignment)
    /// or [`Coup`](crate::ops::Coup) that's already rolled a die or made
    /// its attempt — never an [`InfluencePlacement`](crate::ops::InfluencePlacement),
    /// which has no roll to reveal and can always be abandoned. Only
    /// [`Game::cancel`] can close it now, which keeps the roll and costs
    /// the turn the normal way.
    CannotAbandon { verb: &'static str, ops_spent: u8, ops_total: u8 },
    /// [`Game::begin`] needs a card in play to know how many ops to open
    /// with — [`Game::play_card`] first.
    NoCard,
    /// [`Game::play_card`] refused: a card is already in play, or
    /// [`Game::pass`] refused for the same reason — there's nothing to pass
    /// on once a card's been committed to the turn.
    CardInPlay { card: CardId },
    /// [`Game::play_card`] refused: `card` isn't in the active side's hand
    /// (or, for the China Card, the active side doesn't hold it).
    NotInHand,
    /// [`Game::begin`] refused: a scoring card has no ops to spend — it
    /// can only be played as an event, via [`Game::play_event`].
    ScoringCard,
    /// [`Game::play_card`] refused: the China Card is face down, so it
    /// can't be played for its ops this turn.
    ChinaCardFaceDown,
    /// [`Game::play_event`] refused: [`crate::events::is_implemented`]
    /// doesn't recognise this card yet.
    EventNotImplemented { card: CardId },
    /// [`Game::play_card`], [`Game::begin`], or [`Game::pass`] refused:
    /// [`Game::winner`] is already set, so there's nothing left to do.
    GameOver,
    /// [`Game::confirm`] refused an event: it must be carried out as fully
    /// as it can be, and `left` says what still can be.
    EventIncomplete { left: String },
    /// [`Game::cancel`] refused an event — its text has to be carried out;
    /// undo picks with `u`/`-`, or (for the side that played it)
    /// [`Game::abandon`] with nothing picked.
    CannotCancelEvent,
    /// [`Game::abandon`] refused an event: picks have been made, or it's
    /// the opponent's card being resolved by its owner.
    CannotAbandonEvent,
    /// [`Game::play_event`] refused: `by`'s event has already been played
    /// and bars this card's (Camp David Accords vs Arab-Israeli War).
    EventPrevented { card: CardId, by: CardId },
    /// The card's event needs one of `any_of`'s to have happened first.
    EventRequires { card: CardId, any_of: &'static [CardId] },
    /// The card in play has already had its event resolved and is only
    /// waiting for the operation that event allows — it can't be played as
    /// an event again, returned to the hand, or spent on a space attempt.
    EventPlayed { card: CardId },
    /// [`Game::play_event`] refused: the card's event draws on chance, so it needs
    /// [`Game::play_event_with`]'s dice.
    NeedsDice { card: CardId },
    /// A trap, a pending trigger or a crisis rule refused this: the message says what to do instead.
    Trap(String),
    /// [`Game::play_event`] refused: this card's event can't be played in the Late War.
    EventTooLate { card: CardId },
    /// [`Game::begin`] refused: the card's event only allows `allowed`.
    OpsNotGranted { allowed: String },
    /// [`Game::confirm`]/[`Game::cancel`] refused a war: it ends only by
    /// rolling on a target ([`Game::roll`]) or being abandoned.
    WarNotRolled,
    War(war::WarError),
    Event(EventChoiceError),
    Placement(PlacementError),
    Realign(RealignError),
    Coup(CoupError),
    Space(SpaceError),
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::OperationOpen { verb, remaining, total } => {
                write!(f, "a {verb} session is already open ({remaining} of {total} ops left) — confirm or cancel it first")
            }
            GameError::NoOperation => write!(f, "no operation session open"),
            GameError::WrongKind { open } => write!(f, "a {open} session is open"),
            GameError::NothingToUndo => write!(f, "nothing to undo"),
            GameError::CannotUndo { verb } => write!(f, "a resolved {verb} can't be taken back"),
            GameError::CannotAbandon { verb, ops_spent, ops_total } => {
                write!(f, "a {verb} session already has {ops_spent} of {ops_total} ops spent on a roll that can't be undone — cancel it instead")
            }
            GameError::NoCard => write!(f, "no card in play — play one first"),
            GameError::CardInPlay { card } => write!(f, "card #{card} is already in play — play an operation with it, or return it first"),
            GameError::NotInHand => write!(f, "that card isn't in your hand"),
            GameError::ScoringCard => write!(f, "a scoring card can only be played as an event"),
            GameError::ChinaCardFaceDown => write!(f, "the China Card is face down and can't be played yet"),
            GameError::EventNotImplemented { card } => write!(f, "card #{card}'s event isn't implemented yet"),
            GameError::GameOver => write!(f, "the game is already over"),
            GameError::EventIncomplete { left } => write!(f, "the event isn't finished yet — {left}"),
            GameError::CannotCancelEvent => write!(f, "an event can't be cancelled — finish it, or undo your picks"),
            GameError::CannotAbandonEvent => write!(f, "this event can't be abandoned now — undo your picks, or finish it"),
            GameError::EventPrevented { card, by } => write!(f, "card #{card}'s event can't be played: #{by}'s has already happened"),
            GameError::EventRequires { card, any_of } => {
                let names: Vec<String> = any_of.iter().map(|c| format!("#{c}")).collect();
                write!(f, "card #{card}'s event needs {}'s to have happened first", names.join(" or "))
            }
            GameError::EventPlayed { card } => write!(f, "card #{card}'s event has already been played — conduct its operation, or pass to skip it"),
            GameError::NeedsDice { card } => write!(f, "card #{card}'s event draws on chance — play it with dice"),
            GameError::Trap(text) => write!(f, "{text}"),
            GameError::EventTooLate { card } => write!(f, "card #{card}'s event can't be played in the Late War"),
            GameError::OpsNotGranted { allowed } => write!(f, "this card's event only allows {allowed}"),
            GameError::WarNotRolled => write!(f, "a war ends when it's rolled on a target (roll <country>), or abandoned"),
            GameError::War(e) => write!(f, "{e}"),
            GameError::Event(e) => write!(f, "{e}"),
            GameError::Placement(e) => write!(f, "{e}"),
            GameError::Realign(e) => write!(f, "{e}"),
            GameError::Coup(e) => write!(f, "{e}"),
            GameError::Space(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GameError {}

impl From<war::WarError> for GameError {
    fn from(e: war::WarError) -> Self {
        GameError::War(e)
    }
}

impl From<EventChoiceError> for GameError {
    fn from(e: EventChoiceError) -> Self {
        GameError::Event(e)
    }
}

impl From<PlacementError> for GameError {
    fn from(e: PlacementError) -> Self {
        GameError::Placement(e)
    }
}

impl From<RealignError> for GameError {
    fn from(e: RealignError) -> Self {
        GameError::Realign(e)
    }
}

impl From<SpaceError> for GameError {
    fn from(e: SpaceError) -> Self {
        GameError::Space(e)
    }
}

impl From<CoupError> for GameError {
    fn from(e: CoupError) -> Self {
        GameError::Coup(e)
    }
}

/// A card taken from the active side's hand via [`Game::play_card`], not
/// yet discarded — its ops value is what [`Game::begin`] opens the next
/// operation with. `hand_index` is where it came from, so
/// [`Game::return_card`] can put it back in the same spot; `None` for the
/// China Card, which never lives in a [`Hands`] list in the first place.
/// `logged` tracks whether [`Game::log_card_selected`] has already pushed
/// this card's [`Event::Selected`] entry — the log is a record of the
/// actual game, not of application-level steps, so playing a card alone
/// writes nothing; `logged` only flips to `true` once the selection is
/// irrevocable (see that function's own doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlayedCard {
    id: CardId,
    ops: u8,
    hand_index: Option<usize>,
    logged: bool,
    /// Whether this card is a scoring card — cached from the catalog at
    /// [`Game::play_card`] time so [`Game::begin`] can refuse it without
    /// needing a [`CardCatalog`] of its own.
    scoring: bool,
    /// Cached from the catalog the same way: whether playing this card's
    /// *event* removes it from the game (rule 4.4) instead of discarding it.
    removed_after_event: bool,
    /// Set once this card's event has resolved and allowed an operation
    /// with the card's own ops (ABM Treaty, KAL-007, …): the card stays in
    /// play, restricted to these kinds, until that operation closes or is
    /// skipped.
    ops_after_event: Option<OpsGrant>,
    /// The card arrived through another card's event (Star Wars, Five Year Plan): its event must
    /// be played now — no ops, no space attempt, no taking it back.
    forced_event: Option<(CardId, PlayAs)>,
}

/// The live game: status (including whose turn it is), the committed
/// board, whichever card is currently in play, and whichever operation the
/// active side currently has open.
///
/// `Clone` is cheap, per [`Board`]'s own design note, and exists for the
/// same reason: AI lookahead over a copy, without touching the real game.
#[derive(Clone)]
pub struct Game {
    status: GameStatus,
    board: Board,
    card: Option<PlayedCard>,
    op: Option<Operation>,
    log: GameLog,
    hands: Hands,
    winner: Option<Victory>,
    /// DEFCON when the current action round began, for NORAD's "moved to 2 during that round".
    round_defcon: u8,
    /// NORAD may be owed at the end of the last action round; [`Game::settle`] decides.
    norad_due: bool,
    phase: Phase,
    /// How far the end of the turn has got, for steps that wait on a player.
    turn_end: TurnEndProgress,
    /// The Eagle/Bear has Landed holder who has yet to decide whether to discard.
    held_discard: Option<Superpower>,
    headline: HeadlineState,
}

/// What [`Game::end_turn`] has already done, kept while it waits for the perk holder's decision.
#[derive(Debug, Clone, Copy, Default)]
struct TurnEndProgress {
    scored: bool,
    perk_done: bool,
    report: TurnEndReport,
}

/// The region cards final scoring plays, Europe first (its Control wins outright).
const FINAL_SCORING: [CardId; 6] = [CardId(2), CardId(1), CardId(3), CardId(37), CardId(79), CardId(81)];

/// Where in a turn the game is. Only `ActionRounds` lets a card be played; `TurnEnd` is the
/// moment after the last action round that [`Game::settle`] resolves — it needs the map, the
/// catalog and dice, which the rest of turn handling (`advance`) deliberately doesn't have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Each side chooses a headline card, then the two are revealed and played as events.
    Headline,
    ActionRounds,
    TurnEnd,
}

/// The headline phase's progress (rule 4.4).
#[derive(Debug, Clone, Copy, Default)]
struct HeadlineState {
    /// Each side's pick, `[US, USSR]`: the outer `None` until it has chosen, the inner `None`
    /// for a side that had no card to headline.
    picks: [Option<Option<CardId>>; 2],
    revealed: bool,
    /// Who resolves first and second, once revealed.
    order: [Option<Superpower>; 2],
    /// Defectors cancelled the USSR's headline event.
    cancelled: bool,
    /// How many of `order` have been dealt with.
    next: usize,
}

fn side_slot(side: Superpower) -> usize {
    match side {
        Superpower::Us => 0,
        Superpower::Ussr => 1,
    }
}

/// Who chooses their headline card first: the USSR, unless it holds the Man in Earth Orbit
/// perk, which makes the US show its choice first (the holder sees it before choosing).
pub fn headline_order(status: &GameStatus) -> [Superpower; 2] {
    if space::perk_holder(status, space::Perk::OpponentHeadlinesFirst) == Some(Superpower::Ussr) {
        [Superpower::Us, Superpower::Ussr]
    } else {
        [Superpower::Ussr, Superpower::Us]
    }
}

/// UN Intervention can't be played in the headline phase.
const UN_INTERVENTION: CardId = CardId(32);
/// Defectors: as a US headline it cancels the USSR's headline event.
const DEFECTORS: CardId = CardId(103);

impl Game {
    /// Starts from a scenario's status, board, and hands, with no card
    /// played, no operation open, no winner, and an empty history.
    pub fn from_scenario(scenario: &Scenario) -> Self {
        let mut status = scenario.status;
        // Round 0 is the headline phase: the first picker is the one to act.
        let phase = if status.in_headline() {
            status.active = headline_order(&status)[0];
            Phase::Headline
        } else {
            Phase::ActionRounds
        };
        Game {
            status,
            board: scenario.board.clone(),
            card: None,
            op: None,
            log: GameLog::new(),
            hands: scenario.hands.clone(),
            winner: None,
            round_defcon: scenario.status.defcon,
            norad_due: false,
            phase,
            turn_end: TurnEndProgress::default(),
            held_discard: None,
            headline: HeadlineState::default(),
        }
    }

    pub fn status(&self) -> &GameStatus {
        &self.status
    }

    /// The game's winner, if a scoring event has ended it (Europe
    /// Scoring's Control tier, or VP reaching ±20) — `None` while play
    /// continues. Never clears once set.
    pub fn winner(&self) -> Option<Victory> {
        self.winner
    }

    /// The game's history so far.
    pub fn log(&self) -> &GameLog {
        &self.log
    }

    /// Records a `set`/`add`/`remove`-style debug edit made through
    /// [`Game::board_mut`], which — unlike every other mutator here —
    /// bypasses the operation system entirely and so cannot be observed
    /// by `Game` itself. Callers should read the country's influence
    /// before and after their edit and report both.
    pub fn record_edit(&mut self, country: CountryId, side: Superpower, before: u8, after: u8) {
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: None,
            event: Event::Edit { country, side, before, after },
        });
    }

    /// Records a free-text annotation with no side and no board effect of
    /// its own — e.g. reloading a scenario, which resets the board out
    /// from under whatever history came before it.
    pub fn record_note(&mut self, text: impl Into<String>) {
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: None,
            event: Event::Note(text.into()),
        });
    }

    /// A copy for AI lookahead: same status, board, card in play, and open
    /// operation, but an empty history. `Game` is cheap to clone by design
    /// (see the module doc), and the log is the one field whose size isn't
    /// bounded — a search that clones many nodes should use this instead
    /// of [`Clone::clone`], so lookahead doesn't drag a growing `Vec`
    /// through every branch it never plays out.
    pub fn lookahead(&self) -> Game {
        Game {
            status: self.status,
            board: self.board.clone(),
            card: self.card,
            op: self.op.clone(),
            log: GameLog::new(),
            hands: self.hands.clone(),
            winner: self.winner,
            round_defcon: self.round_defcon,
            norad_due: self.norad_due,
            phase: self.phase,
            turn_end: self.turn_end,
            held_discard: self.held_discard,
            headline: self.headline,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// `side`'s held cards, in hand order. Doesn't include the China Card
    /// ([`crate::cards::CHINA_CARD`]) — see [`Hands`]'s own doc — nor
    /// whatever's currently [`Game::card_in_play`], which
    /// [`Game::play_card`] has already removed.
    pub fn hand(&self, side: Superpower) -> &[CardId] {
        self.hands.hand(side)
    }

    /// All the card piles — hands, deck, discard and removed — read-only, for a view of them.
    pub fn hands(&self) -> &Hands {
        &self.hands
    }

    /// Every card discarded so far — via `confirm`/`cancel` (the ops
    /// path) or [`Game::play_event`] on a card that isn't
    /// `removed_after_event`. Read-only, the same shape [`Game::hand`]
    /// already gives a current hand.
    pub fn discards(&self) -> &[CardId] {
        self.hands.discards()
    }

    /// Every card [`Game::play_event`] has removed from the game entirely
    /// (a `removed_after_event` card, e.g. Southeast Asia Scoring) —
    /// never reshuffled back into a deck, unlike [`Game::discards`].
    pub fn removed_from_game(&self) -> &[CardId] {
        self.hands.removed()
    }

    /// The committed board — never the speculative one. This is what a
    /// debug editor (`set`/`add`/`remove`) should read and write, and
    /// what a view should show when nothing is being previewed.
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Mutable access to the committed board, for `set`/`add`/`remove`
    /// style debug editing. Callers should refuse this while
    /// [`Game::operation`] is `Some` — the same guard the REPL already
    /// applies — so an open operation's legality checks can't be
    /// invalidated by a board shifting underneath them.
    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }

    /// Mutable access to the status (turn/AR/active side/DEFCON/VP/China
    /// Card/…), for debug-mode editing (`vp`/`defcon`/`turn`/`ar`/
    /// `active`/`china`). Bypasses `advance`/`apply_vp`/`set_winner`
    /// entirely, the same way [`Game::board_mut`] bypasses the ops
    /// system — a caller should refuse this while [`Game::operation`] is
    /// `Some` or a card is in play, and report the change itself via
    /// [`Game::record_edit`] or [`Game::record_note`], since `Game`
    /// cannot observe what's done with this any more than it can for
    /// `board_mut`.
    pub fn status_mut(&mut self) -> &mut GameStatus {
        &mut self.status
    }

    /// Mutable access to both sides' hands and the discard/removed
    /// piles, for debug-mode `give`/`discard`/`exile`
    /// ([`crate::cards::Hands::take`]/`push_to_hand`/`discard`/
    /// `remove_from_game`). Bypasses `play_card`/`confirm`/`cancel`
    /// entirely, so a caller reports what it did the same way
    /// [`Game::board_mut`]'s callers do.
    pub fn hands_mut(&mut self) -> &mut Hands {
        &mut self.hands
    }

    /// A snapshot of the live game — status, board, and hands — in the
    /// exact shape a [`Scenario`] holds, for
    /// [`crate::states::StateLibrary::save`] to write out. Deliberately
    /// drops the log, whichever card is in play, and any open
    /// operation: a saved *test state* is a bare position to jump back
    /// into, not an in-progress game's move history (see
    /// `StateLibrary`'s own doc for why those are kept as two different
    /// things). Refused by the caller (not here — `Game` has no concept
    /// of "refuse") while a card is in play, the same way `save` refuses
    /// in `main.rs`, so a snapshot never silently drops a half-played
    /// turn.
    pub fn snapshot(&self) -> Scenario {
        Scenario { status: self.status, board: self.board.clone(), hands: self.hands.clone() }
    }

    /// The board a view should read: the open operation's speculative one
    /// if it has one (a placement does; a realignment or coup doesn't,
    /// since their rolls already land on the real board), otherwise the
    /// committed board.
    pub fn view_board(&self) -> &Board {
        self.op.as_ref().and_then(Operation::board).unwrap_or(&self.board)
    }

    /// The side whose turn it is to act.
    pub fn active(&self) -> Superpower {
        self.status.active
    }

    /// Ops available to the active side right now: the open operation's
    /// own `remaining()` if one is open (so this and the operation never
    /// disagree mid-action), else the played card's own ops if one's in
    /// play but no operation's open yet, else 0 — there's no ops to spend
    /// with nothing played.
    pub fn ops_available(&self) -> u8 {
        self.op
            .as_ref()
            .map(Operation::remaining)
            .or(self.card.map(|c| self.status.effects.card_ops(c.ops_after_event.and_then(|g| g.ops).unwrap_or(c.ops), self.ops_side()).0))
            .unwrap_or(0)
    }

    /// The card currently in play, if any — taken from the active side's
    /// hand by [`Game::play_card`], not yet discarded by
    /// [`Game::confirm`]/[`Game::cancel`]. Its ops value is what
    /// [`Game::begin`] will open the next operation with.
    pub fn card_in_play(&self) -> Option<CardId> {
        self.card.map(|c| c.id)
    }

    /// [`Game::card_in_play`] plus the index it should be spliced back
    /// into the hand at for display — its original position before
    /// [`Game::play_card`] removed it (`PlayedCard::hand_index`), or
    /// `None` for the China Card, which was never in the hand list to
    /// begin with. `render::render_hand`'s caller uses this to keep the
    /// played card visible in its usual spot, prominently marked, rather
    /// than just naming it in the status bar — see that function's own
    /// doc.
    pub fn card_in_play_slot(&self) -> Option<(CardId, Option<usize>)> {
        self.card.map(|c| (c.id, c.hand_index))
    }

    /// The operations the card in play's already-resolved event still
    /// allows, if it has one pending — `None` otherwise.
    pub fn ops_after_event(&self) -> Option<OpsGrant> {
        self.card.and_then(|c| c.ops_after_event)
    }

    pub fn operation(&self) -> Option<&Operation> {
        self.op.as_ref()
    }

    /// Who has to act next: the chooser of an open event
    /// ([`Operation::Event`] — the card's own side, whoever is phasing),
    /// otherwise [`Game::active`]. This, not `active`, is what an AI or a
    /// status bar should read to know whose move it is.
    pub fn decider(&self) -> Superpower {
        match &self.op {
            Some(Operation::Event(e)) => e.chooser(),
            Some(op) => op.side(),
            None => self.ops_side(),
        }
    }

    /// The side that conducts the operations of the card in play: its player, except where its
    /// event hands them to the other side (Grain Sales to Soviets).
    pub fn ops_side(&self) -> Superpower {
        self.card.and_then(|c| c.ops_after_event).and_then(|g| g.side).unwrap_or(self.status.active)
    }

    /// The open operation, narrowed to an [`InfluencePlacement`] — `None`
    /// if no operation is open or a different kind is. Lets a caller keep
    /// reading placement-specific detail (`pending`, `board`, …) after a
    /// `place` call without re-deriving it from [`Game::operation`].
    pub fn placement(&self) -> Option<&InfluencePlacement> {
        match &self.op {
            Some(Operation::Influence(p)) => Some(p),
            _ => None,
        }
    }

    /// Takes `id` from the active side's hand, making it the
    /// [`Game::card_in_play`] whose ops value the next [`Game::begin`]
    /// spends, or whose event the next [`Game::play_event`] resolves —
    /// nothing about the card's text or event happens *here*, for either
    /// path; this only ever moves the card out of the hand. Refused if a
    /// card is already in play ([`GameError::CardInPlay`]), an operation
    /// is open (opening one implies a card was already played), the game
    /// is already over ([`GameError::GameOver`]), or it isn't actually
    /// available to the active side
    /// ([`GameError::NotInHand`]/[`GameError::ChinaCardFaceDown`]).
    /// Writes nothing to the log by itself: the log is a record of the
    /// actual game, not of application-level steps, and a card that's
    /// merely selected can still be taken back with no trace, via
    /// [`Game::return_card`] (or [`Game::abandon`] then `return_card`) —
    /// see [`Game::log_card_selected`] for where its entry really comes
    /// from.
    ///
    /// The China Card ([`CHINA_CARD`]) is the one exception to "in your
    /// hand": it never lives in a [`Hands`] list (see that type's own
    /// doc), so it's playable instead whenever `GameStatus::china_card`
    /// names the active side *and* `china_card_face_up` is true — a side
    /// that's just received it face down (see [`Game::confirm`]/`cancel`)
    /// can't play it again until the next turn flips it back up
    /// ([`Game::advance`]).
    pub fn play_card(&mut self, cards: &CardCatalog, id: CardId) -> Result<(), GameError> {
        if self.winner.is_some() {
            return Err(GameError::GameOver);
        }
        if let Some(card) = self.card {
            return Err(GameError::CardInPlay { card: card.id });
        }
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        self.trigger_guard()?;
        if let Some((side, forced)) = self.status.forced_play
            && side == self.status.active
            && id.0 != forced
        {
            return Err(GameError::Trap(format!("{} must be used for operations this action round", cards.card(CardId(forced)).name)));
        }
        if let Some((fx, trap)) = self.trap() {
            match trap {
                Trap::Escape(_) => return Err(GameError::Trap(format!("{} has {} trapped — discard an Operations card worth 2+ and roll to escape", fx.label(), self.status.active))),
                Trap::PlayScoring if !events::scoring::is_scoring_card(id) => {
                    return Err(GameError::Trap(format!("{} has {} trapped with no card to discard — only scoring cards can be played", fx.label(), self.status.active)))
                }
                Trap::PlayScoring | Trap::Skip => {}
            }
        }
        self.take_card(cards, id)
    }

    /// Takes `id` from the active side's hand (or the China Card, if theirs and face up) and
    /// makes it the card in play — the part of [`Game::play_card`] after its action-round
    /// guards, which the headline phase's own resolution shares.
    fn take_card(&mut self, cards: &CardCatalog, id: CardId) -> Result<(), GameError> {
        let side = self.status.active;
        let card = cards.card(id);
        let hand_index = if id == CHINA_CARD {
            if self.status.china_card != side {
                return Err(GameError::NotInHand);
            }
            if !self.status.china_card_face_up {
                return Err(GameError::ChinaCardFaceDown);
            }
            None
        } else {
            Some(self.hands.remove(side, id).ok_or(GameError::NotInHand)?)
        };
        self.card = Some(PlayedCard { id, ops: card.ops, hand_index, logged: false, scoring: card.scoring, removed_after_event: card.removed_after_event, ops_after_event: None, forced_event: self.status.forced_play.filter(|&(s, c)| s == side && c == id.0).map(|_| (id, PlayAs::Ops)) });
        Ok(())
    }

    /// Puts the card currently in play back in the active side's hand at
    /// the index it was taken from (a no-op position for the China Card,
    /// which was never removed from one) — the free undo for a mistaken
    /// [`Game::play_card`]. Refused while an operation is open (abandon it
    /// first via [`Game::abandon`]) or with no card in play.
    pub fn return_card(&mut self) -> Result<CardId, GameError> {
        self.headline_guard()?;
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        if let Some(card) = self.card.filter(|c| c.ops_after_event.is_some()) {
            return Err(GameError::EventPlayed { card: card.id });
        }
        if self.card.is_some_and(|c| c.forced_event.is_some_and(|(host, _)| host != c.id)) {
            return Err(GameError::Trap("this card came out of another event — it has to be played".into()));
        }
        let card = self.card.take().ok_or(GameError::NoCard)?;
        if let Some(index) = card.hand_index {
            self.hands.insert(self.status.active, index, card.id);
        }
        Ok(card.id)
    }

    /// Opens a new operation for the active side, spending the ops of
    /// whichever card is currently in play. This is the entire enforcement
    /// mechanism beyond the card itself: there is no argument through which
    /// a caller could name the wrong side or the wrong ops count. Refused
    /// if an operation is already open, if no card has been played yet
    /// ([`GameError::NoCard`] — [`Game::play_card`] first), or if the
    /// played card is a scoring card ([`GameError::ScoringCard`] — it has
    /// no ops to spend; [`Game::play_event`] is the only thing it can
    /// fund).
    pub fn begin(&mut self, kind: OperationKind) -> Result<(), GameError> {
        self.headline_guard()?;
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let card = self.card.ok_or(GameError::NoCard)?;
        if card.scoring {
            return Err(GameError::ScoringCard);
        }
        if matches!(card.forced_event, Some((_, PlayAs::Event))) {
            return Err(GameError::Trap("this card came out of another event — its event has to be played".into()));
        }
        if let Some(grant) = card.ops_after_event
            && !grant.allows(kind)
        {
            return Err(GameError::OpsNotGranted { allowed: grant.describe() });
        }
        let scope = card.ops_after_event.and_then(OpsGrant::target_scope);
        let side = self.ops_side();
        let effects = self.status.effects;
        // An event can make the card worth something else for its operation (Olympic Games' boycott: 4).
        let printed = card.ops_after_event.and_then(|g| g.ops).unwrap_or(card.ops);
        let (ops, _) = effects.card_ops(printed, side);
        let bonuses = effects.ops_bonuses(side, card.id);
        self.op = Some(match kind {
            OperationKind::Influence => Operation::Influence(
                InfluencePlacement::new(side, ops, &self.board)
                    .with_banned_region(effects.placement_banned(side))
                    .with_bonuses(bonuses),
            ),
            OperationKind::Realign => Operation::Realign(
                Realignment::new(side, ops, &self.board).with_banned_regions(ops::defcon_banned(self.status.defcon)).with_effects(effects).with_bonuses(bonuses).with_lasting(self.status.lasting).with_scope(scope),
            ),
            OperationKind::Coup => {
                // The Reformer (#87), once played, bars the USSR from coups
                // in Europe for the rest of the game.
                let mut banned = ops::defcon_banned(self.status.defcon);
                if side == Superpower::Ussr && self.hands.removed().contains(&CardId(87)) && !banned.contains(&Region::Europe) {
                    banned.push(Region::Europe);
                }
                Operation::Coup(Coup::new(side, ops, &self.board).with_banned_regions(banned).with_effects(effects).with_bonuses(bonuses).with_lasting(self.status.lasting).with_scope(scope))
            }
        });
        Ok(())
    }

    /// Places one point of influence in `id`, if an [`InfluencePlacement`]
    /// is open. Errors if a different kind of operation is open, none is,
    /// or the placement itself refuses (see [`InfluencePlacement::place`]).
    pub fn place(&mut self, map: &WorldMap, id: CountryId) -> Result<u8, GameError> {
        match &mut self.op {
            Some(Operation::Influence(p)) => Ok(p.place(map, id)?),
            Some(Operation::Event(e)) => Ok(e.step(map, id, Sign::Plus).map(|()| 0)?),
            Some(op) => Err(GameError::WrongKind { open: op.verb() }),
            None => Err(GameError::NoOperation),
        }
    }

    /// The `-` key: takes back one pending point in `id` of an influence
    /// placement (refunding its cost), or — in an event — removes
    /// influence there if the event allows it, else takes back a staged
    /// add. See [`EventChoice::step`](crate::ops::EventChoice::step).
    pub fn unplace(&mut self, map: &WorldMap, id: CountryId) -> Result<(), GameError> {
        match &mut self.op {
            Some(Operation::Influence(p)) => p.unplace(id).map(|_| ()).ok_or(GameError::NothingToUndo),
            Some(Operation::Event(e)) => Ok(e.step(map, id, Sign::Minus)?),
            Some(op) => Err(GameError::WrongKind { open: op.verb() }),
            None => Err(GameError::NoOperation),
        }
    }

    /// The non-scoring cards in the discard pile, for a pick. With `playable`, only those whose
    /// event can be played now (implemented, and not prevented).
    fn pile_cards(&self, cards: &CardCatalog, playable: bool) -> Vec<PileCard> {
        self.hands
            .discards()
            .iter()
            .filter(|&&c| !cards.card(c).scoring)
            .filter(|&&c| !playable || (events::is_implemented(c) && events::blocked_at(c, self.hands.removed(), self.status.turn).is_none()))
            .map(|&c| PileCard { id: c, name: cards.card(c).name.clone(), ops: cards.card(c).ops, removed: cards.card(c).removed_after_event })
            .collect()
    }

    /// Whether the card in play must have its event played (it came out of another card's event).
    pub fn forced_event(&self) -> bool {
        self.forced_by().is_some()
    }

    /// The card whose event put the card in play there (Five Year Plan, Star Wars), if it did.
    pub fn forced_by(&self) -> Option<CardId> {
        self.card.and_then(|c| c.forced_event).map(|(host, _)| host)
    }

    /// How the card put into play by another event has to be played.
    pub fn forced_how(&self) -> Option<PlayAs> {
        self.card.and_then(|c| c.forced_event).map(|(_, how)| how)
    }

    /// Moves the highlight of an open discard-pile pick.
    pub fn move_event_cursor(&mut self, delta: i32) {
        if let Some(Operation::Event(e)) = &mut self.op {
            e.move_cursor(delta);
        }
    }

    /// Chooses the mode the open event's highlight is on (a discard-pile pick).
    pub fn choose_event_cursor(&mut self, map: &WorldMap) -> Result<(), GameError> {
        match &mut self.op {
            Some(Operation::Event(e)) => {
                let at = e.cursor();
                Ok(e.choose_mode(map, at)?)
            }
            Some(op) => Err(GameError::WrongKind { open: op.verb() }),
            None => Err(GameError::NoOperation),
        }
    }

    /// Chooses which way to play an open multi-mode event (0-based).
    pub fn choose_mode(&mut self, map: &WorldMap, mode: usize) -> Result<(), GameError> {
        match &mut self.op {
            Some(Operation::Event(e)) => Ok(e.choose_mode(map, mode)?),
            Some(op) => Err(GameError::WrongKind { open: op.verb() }),
            None => Err(GameError::NoOperation),
        }
    }

    /// Resolves one realignment roll, or a coup's one attempt — whichever
    /// kind of operation is open — writing straight to the real board,
    /// immediately and irreversibly, the way a die roll can't be taken
    /// back. Doesn't advance the turn; only [`Game::confirm`],
    /// [`Game::cancel`], and [`Game::pass`] do that.
    ///
    /// The *first* roll of either kind is also what makes the card that
    /// funded it irrevocable — [`Game::abandon`] refuses a realignment or
    /// coup from here on — so this is where [`Game::log_card_selected`]
    /// finally writes the card's own entry, immediately before the roll's
    /// own.
    pub fn roll(&mut self, map: &WorldMap, id: CountryId, dice: &mut Dice) -> Result<RollOutcome, GameError> {
        // The operation's own side: usually the phasing player, but Grain Sales gives the US the ops.
        let side = self.op.as_ref().map_or(self.status.active, Operation::side);
        if let Some(Operation::War(w)) = &self.op {
            if !w.is_legal_target(map, id) {
                return Err(w.resolve(map, &self.board, id, 0).expect_err("an illegal target is refused").into());
            }
            let result = w.resolve(map, &self.board, id, dice.roll())?;
            self.op = None;
            self.finish_war(result.clone());
            return Ok(RollOutcome::War(result));
        }
        let outcome = match &mut self.op {
            Some(Operation::Realign(r)) => RollOutcome::Realign(r.roll(map, &mut self.board, id, dice)?),
            Some(Operation::Coup(c)) => RollOutcome::Coup(c.attempt(map, &mut self.board, id, dice)?),
            Some(op) => return Err(GameError::WrongKind { open: op.verb() }),
            None => return Err(GameError::NoOperation),
        };
        self.log_card_selected();
        let event = match outcome {
            RollOutcome::Realign(result) => Event::Realign(result),
            RollOutcome::Coup(result) => Event::Coup(result),
            RollOutcome::War(_) => unreachable!("a war returned above"),
        };
        self.log.push(LogEntry { turn: self.status.turn, action_round: self.status.action_round, side: Some(side), event });
        if let RollOutcome::Coup(result) = &outcome {
            self.coup_aftermath(map, side, result);
        }
        Ok(outcome)
    }

    /// The aftermath of the coup just resolved, if it set anything off —
    /// read from the log, where it sits right after the coup's own entry
    /// (and before a game-over entry it may have caused).
    pub fn last_coup_aftermath(&self) -> Option<CoupAftermath> {
        self.log.entries().iter().rev().take(2).find_map(|e| match e.event {
            Event::CoupAftermath(a) => Some(a),
            _ => None,
        })
    }

    /// What a coup attempt on `target` sets off beyond its own result
    /// (rule 6.3.4 and the turn-long events): a battleground coup degrades
    /// DEFCON by 1 — reaching 1 loses the game for the phasing side —
    /// unless Nuclear Subs spares a US one, and Yuri and Samantha pays the
    /// USSR 1 VP per US coup. DEFCON is applied before VP, so a DEFCON loss
    /// outranks any VP. Logged as one [`Event::CoupAftermath`] after the
    /// coup's own entry, or not at all if nothing happened.
    fn coup_aftermath(&mut self, map: &WorldMap, side: Superpower, result: &CoupResult) {
        let target = result.target;
        let mut aftermath = CoupAftermath::default();
        // Every coup is a Military Operation (rule 6.3.4): its ops go on the coup-maker's track.
        let track = match side {
            Superpower::Us => &mut self.status.military_ops_us,
            Superpower::Ussr => &mut self.status.military_ops_ussr,
        };
        let before = *track;
        *track = (before + result.ops as i8).clamp(0, war::MIL_OPS_MAX);
        aftermath.mil_ops = Some((before, *track));
        if map.country(target).battleground {
            if self.status.effects.spares_defcon(side) {
                aftermath.defcon_spared = true;
            } else {
                let before = self.status.defcon;
                self.apply_defcon(before.saturating_sub(1));
                aftermath.defcon = Some((before, self.status.defcon));
            }
        }
        if let Some((_, beneficiary, n)) = self.status.effects.coup_vp(side) {
            let delta = if beneficiary == Superpower::Us { n as i8 } else { -(n as i8) };
            self.apply_vp(delta);
            aftermath.vp = Some((delta, self.status.vp));
        }
        if self.status.effects.coup_forbidden(side) {
            self.set_winner(side.opponent(), VictoryReason::CubanMissileCrisis);
        }
        if !aftermath.is_empty() {
            self.log.push(LogEntry {
                turn: self.status.turn,
                action_round: self.status.action_round,
                side: Some(side),
                event: Event::CoupAftermath(aftermath),
            });
        }
        self.log_game_over();
    }

    /// Throws the open event's roll-off — Summit's, once its player has seen the odds. The
    /// winner becomes the one who decides (see [`Game::decider`]); nothing is applied until
    /// [`Game::confirm`].
    pub fn roll_contest(&mut self, map: &WorldMap, dice: &mut Dice) -> Result<events::Contest, GameError> {
        match &mut self.op {
            Some(Operation::Event(e)) => Ok(e.roll_contest(map, dice)?),
            Some(op) => Err(GameError::WrongKind { open: op.verb() }),
            None => Err(GameError::NoOperation),
        }
    }

    /// Takes back the single most recently placed influence point,
    /// refunding its ops — only meaningful for a placement. Refused (with
    /// no state changed) for a realignment or coup, whose rolls already
    /// resolved, and when no operation is open.
    pub fn undo(&mut self, map: &WorldMap) -> Result<CountryId, GameError> {
        match &mut self.op {
            Some(Operation::Influence(p)) => p.undo_last(map).ok_or(GameError::NothingToUndo),
            Some(Operation::Event(e)) => e.undo_last(map).ok_or(GameError::NothingToUndo),
            Some(Operation::Realign(_)) => Err(GameError::CannotUndo { verb: "realignment roll" }),
            Some(Operation::Coup(_)) => Err(GameError::CannotUndo { verb: "coup" }),
            Some(Operation::War(_)) => Err(GameError::CannotUndo { verb: "war" }),
            None => Err(GameError::NoOperation),
        }
    }

    /// Closes the open operation, committing a placement's pending points
    /// to the board (a realignment's or coup's rolls are already there),
    /// and hands the turn to the other side. Ending a turn with ops
    /// unspent is allowed — it simply forfeits them, the same way
    /// [`InfluencePlacement::commit`] never required spending everything.
    ///
    /// Returns the closed operation so the caller can still build its own
    /// summary from it (`pending_countries()`, `history()`, `result()`,
    /// …) — its `board()` now equals the committed board, not a
    /// speculative one, since the operation is over.
    pub fn confirm(&mut self) -> Result<Operation, GameError> {
        if let Some(Operation::Event(e)) = &self.op {
            if !e.is_complete() {
                return Err(GameError::EventIncomplete { left: e.progress_left() });
            }
            let mut op = self.op.take().expect("checked Some above");
            let Operation::Event(e) = &mut op else { unreachable!() };
            // A declined gate hands the card on to its follow-up session.
            if let Some(next) = e.take_follow_up() {
                self.op = Some(Operation::Event(Box::new(next)));
                return Ok(op);
            }
            let result = e.into_result(&self.status);
            if e.is_triggered() {
                self.finish_triggered(result);
                return Ok(op);
            }
            self.finish_effect(result, e.grant());
            return Ok(op);
        }
        if matches!(self.op, Some(Operation::War(_))) {
            return Err(GameError::WarNotRolled);
        }
        let op = self.op.take().ok_or(GameError::NoOperation)?;
        if let Operation::Influence(p) = &op {
            self.board = p.board().clone();
        }
        self.log_close(&op, true);
        self.finish_operation(&op);
        Ok(op)
    }

    /// What follows a closed operation: Che's second coup if the first
    /// removed US influence, else the card is spent and the turn handed over.
    fn finish_operation(&mut self, op: &Operation) {
        if self.winner.is_none()
            && let Some(next) = self.follow_up_grant(op)
        {
            if let Some(card) = self.card.as_mut() {
                card.ops_after_event = Some(next);
            }
            return;
        }
        self.discard_played_card();
        self.advance();
    }

    /// The grant for a second coup, if the card's event allows one and the
    /// coup just closed removed any of the opponent's influence — against a
    /// different country than the first.
    fn follow_up_grant(&self, op: &Operation) -> Option<OpsGrant> {
        let grant = self.card?.ops_after_event.filter(|g| g.follow_up)?;
        let Operation::Coup(coup) = op else { return None };
        let result = coup.result().filter(|r| r.removed > 0)?;
        Some(OpsGrant { follow_up: false, exclude: Some(result.target), ..grant })
    }

    /// Closes the open operation without committing a placement's pending
    /// points (a realignment's or coup's rolls, already on the board,
    /// stay there — there's nothing to discard), and hands the turn to
    /// the other side.
    pub fn cancel(&mut self) -> Result<Operation, GameError> {
        if matches!(self.op, Some(Operation::Event(_))) {
            return Err(GameError::CannotCancelEvent);
        }
        if matches!(self.op, Some(Operation::War(_))) {
            return Err(GameError::WarNotRolled);
        }
        let op = self.op.take().ok_or(GameError::NoOperation)?;
        self.log_close(&op, false);
        self.finish_operation(&op);
        Ok(op)
    }

    /// Closes the open operation without costing the turn — the free undo
    /// for opening the wrong kind by mistake, as long as nothing
    /// *irreversible* has happened. What counts as irreversible differs by
    /// kind: an [`InfluencePlacement`] can always be abandoned, however
    /// many points are already pending — nothing about placement is
    /// hidden or committed until [`Game::confirm`] runs, so every pending
    /// point is simply discarded and every op it cost refunded, the same
    /// board effect [`Game::cancel`] has on a placement, just without the
    /// turn cost. A [`Realignment`] or [`Coup`], by contrast, can only be
    /// abandoned *before* its first roll or attempt: a die roll writes
    /// straight to the real board the instant it happens (see each
    /// module's own doc) and reveals a result that can't be un-rolled, so
    /// once `ops_spent() > 0` for either of those two kinds, `cancel` is
    /// the only way to close it — refused with [`GameError::CannotAbandon`].
    /// Leaves no trace in the log either way: as far as the history is
    /// concerned, an abandoned operation never happened.
    pub fn abandon(&mut self) -> Result<Operation, GameError> {
        if self.phase == Phase::Headline && self.op.is_some() {
            return Err(GameError::Trap("a headline event has to be played out — it can't be backed out of".into()));
        }
        match &self.op {
            None => Err(GameError::NoOperation),
            // Placement never rolls dice, so there's nothing it could
            // reveal — always safe to discard, no matter how many points
            // are pending.
            Some(Operation::Influence(_)) => Ok(self.op.take().expect("checked Some above")),
            // A war rolls nothing until a target is chosen, so it can always be backed out of.
            Some(Operation::War(_)) => Ok(self.op.take().expect("checked Some above")),
            // An event can be backed out of only by the side that chose to
            // play it, and only before anything has been picked.
            Some(Operation::Event(e)) => {
                if e.is_pristine() && e.chooser() == self.status.active && !e.is_second_stage() && e.contest().is_none() {
                    Ok(self.op.take().expect("checked Some above"))
                } else {
                    Err(GameError::CannotAbandonEvent)
                }
            }
            Some(op) => {
                let ops_spent = op.ops_spent();
                if ops_spent > 0 {
                    return Err(GameError::CannotAbandon { verb: op.verb(), ops_spent, ops_total: op.ops_total() });
                }
                Ok(self.op.take().expect("checked Some above"))
            }
        }
    }

    /// The step back *before* [`Game::abandon`]: with a multi-mode event
    /// open and a mode chosen but nothing picked, un-chooses the mode
    /// (Chernobyl's region) and returns `true`, leaving the event open
    /// awaiting a choice. `false` — and nothing changed — otherwise, so a
    /// caller falls through to `abandon`. Like `abandon`, only the side
    /// that played the card may back out.
    pub fn clear_event_mode(&mut self, map: &WorldMap) -> bool {
        let active = self.status.active;
        match &mut self.op {
            // The victim of a discard-or-suffer card may change their mind too, though they aren't the phasing side.
            Some(Operation::Event(e))
                if e.modes().len() > 1 && e.mode().is_some() && e.is_pristine() && (e.chooser() == active || !e.gate_cards().is_empty() || e.has_session_modal()) =>
            {
                e.clear_mode(map).is_ok()
            }
            _ => false,
        }
    }

    /// Spends the card in play on a space race attempt (rule 6.4) instead
    /// of an operation or its event: rolls the die against the next box
    /// and, on a roll at or under its number, moves the marker there and
    /// pays the box's VP. Either way the card is discarded — its event
    /// never happens, so Flower Power and the like don't see it — and the
    /// turn is handed over. Refused for anything [`space::check`] refuses
    /// (a scoring card, the China Card, too few ops, no attempts left),
    /// with no card in play, while an operation is open, or once the game
    /// is over. The ops compared are the card's *effective* ops, so
    /// Containment and Red Scare count.
    pub fn space(&mut self, dice: &mut Dice) -> Result<SpaceResult, GameError> {
        self.space_check()?;
        let card = self.card.expect("space_check found a card in play");
        let side = self.status.active;
        let result = space::resolve(&self.status, side, card.id, dice.roll());
        self.log_card_selected();
        match side {
            Superpower::Us => self.status.space_attempts_us += 1,
            Superpower::Ussr => self.status.space_attempts_ussr += 1,
        }
        if result.success {
            space::set_position(&mut self.status, side, result.to());
            self.apply_vp(result.vp_delta);
        }
        let vp_after = self.status.vp;
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(side),
            event: Event::Space { result, vp_after },
        });
        self.log_game_over();
        self.card = None;
        if self.status.forced_play.is_some_and(|(_, c)| c == card.id.0) {
            self.status.forced_play = None;
        }
        self.hands.discard(card.id);
        if self.winner.is_none() {
            self.advance();
        }
        Ok(result)
    }

    /// Whether [`Game::space`] would be accepted for the card in play right
    /// now — what [`Game::legal_actions`] and the key hints ask.
    pub fn can_space(&self) -> bool {
        self.space_check().is_ok()
    }

    /// [`space::check`] for the card in play, or why not.
    pub fn space_check(&self) -> Result<(), GameError> {
        if self.winner.is_some() {
            return Err(GameError::GameOver);
        }
        self.headline_guard()?;
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let card = self.card.ok_or(GameError::NoCard)?;
        if card.ops_after_event.is_some() {
            return Err(GameError::EventPlayed { card: card.id });
        }
        if matches!(card.forced_event, Some((_, PlayAs::Event))) {
            return Err(GameError::Trap("this card came out of another event — its event has to be played".into()));
        }
        let side = self.status.active;
        let ops = self.status.effects.card_ops(card.ops, side).0;
        Ok(space::check(&self.status, side, card.id == CHINA_CARD, card.scoring, ops)?)
    }

    /// What the active side's trap (Bear Trap for the USSR, Quagmire for the US) asks of this action
    /// round, if one is in force and nothing has been played yet.
    pub fn trap(&self) -> Option<(LastingEffect, Trap)> {
        if self.winner.is_some() || self.card.is_some() || self.op.is_some() {
            return None;
        }
        let side = self.status.active;
        let effect = self.status.lasting.trap_on(side)?;
        let cards = standard_cards();
        let hand = self.hands.hand(side);
        let discardable: Vec<CardId> = hand.iter().copied().filter(|&c| !cards.card(c).scoring && cards.card(c).ops >= 2).collect();
        Some((
            effect,
            if !discardable.is_empty() {
                Trap::Escape(discardable)
            } else if hand.iter().any(|&c| cards.card(c).scoring) {
                Trap::PlayScoring
            } else {
                Trap::Skip
            },
        ))
    }

    /// The trapped side's whole action round: discards `discard`, rolls a die, and escapes (the
    /// trap card ends) on 1-4. Either way the round is spent.
    pub fn escape_trap(&mut self, dice: &mut Dice, discard: CardId) -> Result<TrapResult, GameError> {
        self.trigger_guard()?;
        let Some((effect, trap)) = self.trap() else {
            return Err(GameError::Trap("no trap is holding this action round".into()));
        };
        let Trap::Escape(candidates) = trap else {
            return Err(GameError::Trap(format!("{} has nothing to discard — play your scoring cards or pass", self.status.active)));
        };
        if !candidates.contains(&discard) {
            return Err(GameError::Trap("discard an Operations card worth 2 or more".into()));
        }
        let side = self.status.active;
        self.hands.remove(side, discard);
        self.hands.discard(discard);
        let roll = dice.roll();
        let escaped = roll <= 4;
        if escaped {
            self.status.lasting.cancel(effect);
        }
        let result = TrapResult { trap: effect.card(), side, discarded: discard, roll, escaped };
        self.log.push(LogEntry { turn: self.status.turn, action_round: self.status.action_round, side: Some(side), event: Event::Trap(result) });
        self.advance();
        Ok(result)
    }

    /// Cuban Missile Crisis's way out, any time between operations: the threatened side removes 2
    /// of its own influence from Cuba (USSR) or West Germany/Turkey (US) and the event ends.
    pub fn defuse_crisis(&mut self, map: &WorldMap, country: CountryId) -> Result<(), GameError> {
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let Some(by) = self.status.effects.cuban_missile_crisis else {
            return Err(GameError::Trap("Cuban Missile Crisis isn't in force".into()));
        };
        let side = by.opponent();
        let name = map.country(country).name.as_str();
        let (ok, wanted) = match side {
            Superpower::Ussr => (name == "Cuba", "Cuba"),
            Superpower::Us => (name == "West Germany" || name == "Turkey", "West Germany or Turkey"),
        };
        if !ok {
            return Err(GameError::Trap(format!("{side} can defuse the crisis only by removing 2 influence from {wanted}")));
        }
        let have = self.board.influence(country, side);
        if have < 2 {
            return Err(GameError::Trap(format!("{side} needs 2 influence in {name} to defuse the crisis (has {have})")));
        }
        self.board.set_influence(country, side, have - 2);
        self.status.effects.cuban_missile_crisis = None;
        self.log.push(LogEntry { turn: self.status.turn, action_round: self.status.action_round, side: Some(side), event: Event::Defused { side, country } });
        Ok(())
    }

    /// Refuses a move while NORAD's trigger is waiting for [`Game::settle`].
    fn trigger_guard(&self) -> Result<(), GameError> {
        if self.norad_due {
            return Err(GameError::Trap("NORAD's trigger has to be settled first".into()));
        }
        if self.held_discard.is_some() {
            return Err(GameError::Trap("Eagle/Bear has Landed: discard a card, or keep them all, first".into()));
        }
        if self.phase == Phase::TurnEnd {
            return Err(GameError::Trap("the turn is over — settle its end first".into()));
        }
        if self.phase == Phase::Headline {
            return Err(GameError::Trap("this is the headline phase — choose your headline card (headline <card>)".into()));
        }
        Ok(())
    }

    /// Whether [`Game::settle`] has something to do: NORAD's trigger, or the end of the turn
    /// (unless an event it opened — NORAD's — is still waiting on its player).
    pub fn settlement_due(&self) -> bool {
        self.norad_due
            || (self.phase == Phase::TurnEnd && self.op.is_none() && self.winner.is_none() && self.held_discard.is_none())
            || (self.phase == Phase::Headline
                && self.op.is_none()
                && self.card.is_none()
                && self.winner.is_none()
                && (self.headline.revealed || self.headline_candidates(self.status.active).is_empty()))
    }

    /// The Eagle/Bear has Landed holder, while the end of the turn waits on whether they
    /// discard a held card ([`Game::discard_held`]).
    pub fn awaiting_discard(&self) -> Option<Superpower> {
        self.held_discard.filter(|_| self.winner.is_none())
    }

    /// Answers the Eagle/Bear has Landed perk at the end of the turn: discard `card` from the
    /// holder's hand, or `None` to keep every card. The turn's end then carries on at the next
    /// [`Game::settle`].
    pub fn discard_held(&mut self, card: Option<CardId>) -> Result<(), GameError> {
        let side = self.awaiting_discard().ok_or(GameError::Trap("nothing is waiting for a discard".into()))?;
        if let Some(card) = card {
            if self.hands.remove(side, card).is_none() {
                return Err(GameError::NotInHand);
            }
            self.hands.discard(card);
        }
        self.held_discard = None;
        self.log.push(LogEntry { turn: self.status.turn, action_round: self.status.action_round, side: Some(side), event: Event::HeldDiscard { card } });
        Ok(())
    }

    /// Settles what the last action round set off that needs the map: NORAD's +1 US influence
    /// (opened as an event for the US, if it holds Canada and has influence to add to). Callers
    /// run it after anything that can end an action round.
    pub fn settle(&mut self, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) {
        // Each step can lead straight to the next (the end of a turn into the headline phase).
        for _ in 0..8 {
            if self.winner.is_some() || self.op.is_some() || !self.settlement_due() {
                return;
            }
            if std::mem::take(&mut self.norad_due) {
                let canada_held = map.id_by_name("Canada").is_some_and(|c| self.board.is_controlled_by(map, c, Superpower::Us));
                if canada_held && let Some(e) = EventChoice::norad(map, &self.board, &self.status) {
                    self.op = Some(Operation::Event(Box::new(e)));
                    return;
                }
            } else if self.phase == Phase::TurnEnd {
                self.end_turn(map, cards, dice);
            } else if self.phase == Phase::Headline {
                self.headline_step(map, cards, dice);
            }
        }
    }

    /// Refuses what only an action round allows (operations, space attempts, taking a card
    /// back) during the headline phase, where a card is played for its event alone.
    fn headline_guard(&self) -> Result<(), GameError> {
        if self.phase == Phase::Headline {
            return Err(GameError::Trap("headline cards are played for their events only".into()));
        }
        Ok(())
    }

    /// Whether the side to act is choosing its headline card right now.
    pub fn picking_headline(&self) -> bool {
        self.phase == Phase::Headline
            && !self.headline.revealed
            && self.op.is_none()
            && self.card.is_none()
            && self.winner.is_none()
            && !self.headline_candidates(self.status.active).is_empty()
    }

    /// The cards `side` may headline: anything in its hand except UN Intervention (the China
    /// Card is never in a hand, so can't be one).
    pub fn headline_candidates(&self, side: Superpower) -> Vec<CardId> {
        self.hands.hand(side).iter().copied().filter(|&c| c != UN_INTERVENTION).collect()
    }

    /// What the side now choosing may see of the other's headline: the Man in Earth Orbit holder
    /// picks second and sees the opponent's card.
    pub fn headline_pick_seen(&self) -> Option<(Superpower, CardId)> {
        let me = self.status.active;
        if self.phase != Phase::Headline || self.headline.revealed || space::perk_holder(&self.status, space::Perk::OpponentHeadlinesFirst) != Some(me) {
            return None;
        }
        self.headline.picks[side_slot(me.opponent())].flatten().map(|c| (me.opponent(), c))
    }

    /// Whether the other side has already chosen its headline card (not saying which).
    pub fn headline_other_has_chosen(&self) -> bool {
        self.phase == Phase::Headline && !self.headline.revealed && self.headline.picks[side_slot(self.status.active.opponent())].is_some()
    }

    /// Chooses `card` from the active side's hand as its headline for the turn (rule 4.4). The
    /// USSR chooses first and the US second, the first choice hidden until both are in — or
    /// the other way round while the USSR holds the Man in Earth Orbit perk. Once both have
    /// chosen the cards are revealed and played by [`Game::settle`].
    pub fn headline(&mut self, cards: &CardCatalog, card: CardId) -> Result<(), GameError> {
        if self.winner.is_some() {
            return Err(GameError::GameOver);
        }
        if self.phase != Phase::Headline || self.headline.revealed {
            return Err(GameError::Trap("it isn't time to choose a headline card".into()));
        }
        if self.card.is_some() || self.op.is_some() {
            return Err(GameError::Trap("a headline event is still being played".into()));
        }
        let side = self.status.active;
        if card == UN_INTERVENTION && self.hands.hand(side).contains(&card) {
            return Err(GameError::Trap("UN Intervention can't be played in the headline phase".into()));
        }
        if !self.hands.hand(side).contains(&card) {
            return Err(GameError::NotInHand);
        }
        self.record_headline_pick(cards, side, Some(card));
        Ok(())
    }

    fn record_headline_pick(&mut self, cards: &CardCatalog, side: Superpower, pick: Option<CardId>) {
        self.headline.picks[side_slot(side)] = Some(pick);
        let (Some(us), Some(ussr)) = (self.headline.picks[0], self.headline.picks[1]) else {
            self.status.active = side.opponent();
            return;
        };
        // Both are in: reveal. The higher Operations value resolves first, the US on a tie;
        // Defectors always goes first, to cancel the USSR's card.
        let ops = |c: Option<CardId>| c.map_or(-1, |c| cards.card(c).ops as i8);
        let cancelled = us == Some(DEFECTORS) && ussr.is_some();
        let us_first = cancelled || ops(us) >= ops(ussr);
        let order = if us_first { [Superpower::Us, Superpower::Ussr] } else { [Superpower::Ussr, Superpower::Us] };
        self.headline.revealed = true;
        self.headline.cancelled = cancelled;
        self.headline.order = order.map(Some);
        self.headline.next = 0;
        let first = (us.is_some() || ussr.is_some()).then_some(order[0]);
        self.log.push(LogEntry { turn: self.status.turn, action_round: 0, side: None, event: Event::Headline { ussr, us, first, cancelled } });
    }

    /// One step of the headline phase: a side with nothing to headline passes it up; once both
    /// have chosen, the cards are played as events in order — each by its own side, each
    /// through the ordinary event path (choices, wars and all), whose end leaves the next
    /// step to the following call. A card whose event can't be played is simply discarded.
    fn headline_step(&mut self, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) {
        if !self.headline.revealed {
            let side = self.status.active;
            if self.headline_candidates(side).is_empty() {
                self.record_headline_pick(cards, side, None);
            }
            return;
        }
        while self.winner.is_none() && self.op.is_none() && self.card.is_none() {
            let Some(side) = self.headline.order.get(self.headline.next).copied().flatten() else {
                self.finish_headline();
                return;
            };
            self.headline.next += 1;
            self.status.active = side;
            let Some(card) = self.headline.picks[side_slot(side)].flatten() else { continue };
            if self.headline.cancelled && side == Superpower::Ussr {
                if self.hands.remove(side, card).is_some() {
                    self.hands.discard(card);
                }
                continue;
            }
            if self.take_card(cards, card).is_err() {
                continue;
            }
            if self.play_event_with(map, cards, dice).is_err() {
                // Not implemented, prevented, too late…: the card is spent with no effect.
                self.discard_played_card();
            }
        }
    }

    /// Both headline events are done: the first action round begins.
    fn finish_headline(&mut self) {
        self.headline = HeadlineState::default();
        self.phase = Phase::ActionRounds;
        self.status.action_round = 1;
        self.status.active = Superpower::Ussr;
        self.round_defcon = self.status.defcon;
    }

    /// The end of a turn (rule 4.5), in order: Military Operations against DEFCON, a held scoring
    /// card, the China Card and every "remainder of the turn" effect, then — after turn 10, the
    /// end of the game — the next turn: DEFCON improves by 1, Mid/Late War cards join the deck
    /// when their turn comes, and both hands are dealt back up.
    fn end_turn(&mut self, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) {
        let (turn, round) = (self.status.turn, self.status.action_round);
        if !self.turn_end.scored {
            let mut report = TurnEndReport { mil_ops: (self.status.military_ops_us, self.status.military_ops_ussr), defcon: self.status.defcon, ..Default::default() };
            // Each side short of DEFCON hands the opponent the shortfall; both short nets out.
            let short = |ops: i8| (self.status.defcon as i8 - ops).max(0);
            let swing = short(report.mil_ops.1) - short(report.mil_ops.0);
            self.status.military_ops_us = 0;
            self.status.military_ops_ussr = 0;
            self.apply_vp(swing);
            report.vp_delta = swing;
            report.vp_after = self.status.vp;
            // A scoring card still held loses the game (the USSR is checked first if both hold one).
            for side in [Superpower::Ussr, Superpower::Us] {
                if self.hands.hand(side).iter().any(|&c| cards.card(c).scoring) {
                    self.set_winner(side.opponent(), VictoryReason::HeldScoringCard);
                }
            }
            self.turn_end = TurnEndProgress { scored: true, perk_done: false, report };
        }
        // Eagle/Bear has Landed: its holder may discard a held card — a decision to wait for.
        if self.winner.is_none() && !self.turn_end.perk_done {
            self.turn_end.perk_done = true;
            if let Some(holder) = space::perk_holder(&self.status, space::Perk::DiscardHeld)
                && !self.hands.hand(holder).is_empty()
            {
                self.held_discard = Some(holder);
                self.status.active = holder;
                return;
            }
        }
        let mut report = std::mem::take(&mut self.turn_end).report;
        self.status.china_card_face_up = true;
        self.status.space_attempts_us = 0;
        self.status.space_attempts_ussr = 0;
        self.status.effects = TurnEffects::default();
        if self.winner.is_none() && turn >= *TURN_RANGE.end() {
            self.push_turn_end(turn, round, report);
            self.finish_game(map);
            return;
        }
        if self.winner.is_none() {
            self.start_next_turn(cards, dice, &mut report);
        }
        self.push_turn_end(turn, round, report);
        self.log_game_over();
    }

    /// Logged against the turn that ended, whatever the status says by now.
    fn push_turn_end(&mut self, turn: u8, action_round: u8, report: TurnEndReport) {
        self.log.push(LogEntry { turn, action_round, side: None, event: Event::TurnEnd(report) });
    }

    /// Turn marker, DEFCON, deck additions and the deal for the turn after the one that just ended.
    fn start_next_turn(&mut self, cards: &CardCatalog, dice: &mut Dice, report: &mut TurnEndReport) {
        self.status.turn += 1;
        let turn = self.status.turn;
        self.status.action_rounds_per_turn = rounds_for_turn(turn);
        // Round 0 is the headline phase; its first chooser is the one to act.
        self.status.action_round = 0;
        self.status.active = headline_order(&self.status)[0];
        self.phase = Phase::Headline;
        self.headline = HeadlineState::default();
        if self.status.defcon < 5 {
            report.defcon_change = Some((self.status.defcon, self.status.defcon + 1));
            self.status.defcon += 1;
        }
        self.round_defcon = self.status.defcon;
        let joining = match turn {
            4 => Some(CardPhase::Mid),
            8 => Some(CardPhase::Late),
            _ => None,
        };
        if let Some(era) = joining {
            let unseen: Vec<CardId> = cards
                .ids()
                .filter(|&id| id != CHINA_CARD && cards.card(id).phase == era && !self.hands.contains(id))
                .collect();
            report.added = unseen.len() as u8;
            self.hands.shuffle_into_deck(unseen, dice);
        }
        let target = hand_size_for_turn(turn);
        let discards_before = self.hands.discards().len();
        loop {
            let mut dealt_any = false;
            for side in [Superpower::Ussr, Superpower::Us] {
                if self.hands.hand(side).len() < target
                    && let Some(card) = self.hands.draw(dice)
                {
                    self.hands.push_to_hand(side, card);
                    match side {
                        Superpower::Us => report.dealt.0 += 1,
                        Superpower::Ussr => report.dealt.1 += 1,
                    }
                    dealt_any = true;
                }
            }
            if !dealt_any {
                break;
            }
        }
        report.reshuffled = discards_before > 0 && self.hands.discards().is_empty();
    }

    /// After turn 10 (rule 10.2): every region is scored as if its card had been played —
    /// Europe first, since its Control still wins outright — then whoever holds the China
    /// Card gets 1 VP. Unless that ended the game early (the track reaching ±20), the side
    /// ahead on VP wins and level is a draw.
    fn finish_game(&mut self, map: &WorldMap) {
        let mut results = Vec::new();
        for card in FINAL_SCORING {
            if self.winner.is_some() {
                break;
            }
            let result = scoring::resolve(map, &self.board, &self.status.lasting, card).expect("every region card resolves");
            self.apply_vp(result.vp_delta);
            if let Some(side) = result.automatic_victory {
                self.set_winner(side, VictoryReason::EuropeControl);
            }
            results.push((result, self.status.vp));
        }
        let china = self.winner.is_none().then_some(self.status.china_card);
        if let Some(holder) = china {
            self.apply_vp(if holder == Superpower::Us { 1 } else { -1 });
        }
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: None,
            event: Event::FinalScoring { results, china, vp_after: self.status.vp },
        });
        if self.winner.is_none() {
            let side = match self.status.vp.signum() {
                1 => Some(Superpower::Us),
                -1 => Some(Superpower::Ussr),
                _ => None,
            };
            self.winner = Some(Victory { side, reason: VictoryReason::FinalScoring });
        }
        self.log_game_over();
    }

    /// Forfeits the active side's turn without opening an operation.
    /// Refused if one is already open — cancel it first — if a card is
    /// in play (once a card's been taken from the hand, there's nothing
    /// left to "pass" on, so [`Game::return_card`] is the way out instead),
    /// or if the game is already over ([`GameError::GameOver`]).
    pub fn pass(&mut self) -> Result<(), GameError> {
        if self.winner.is_some() {
            return Err(GameError::GameOver);
        }
        // While the Eagle/Bear has Landed holder is deciding, passing keeps every card.
        if self.awaiting_discard().is_some() {
            return self.discard_held(None);
        }
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        self.trigger_guard()?;
        if let Some((side, forced)) = self.status.forced_play
            && side == self.status.active
            && self.card.is_none()
            && self.hands.hand(side).iter().any(|c| c.0 == forced)
        {
            return Err(GameError::Trap(format!("card #{forced} has to be used for operations this action round — play it")));
        }
        if let Some((fx, trap)) = self.trap()
            && trap != Trap::Skip
        {
            return Err(GameError::Trap(format!("{} has {} trapped — escape it, or play your scoring cards, before passing", fx.label(), self.status.active)));
        }
        if let Some(card) = self.card {
            // After its event, a card that allowed an operation can be
            // passed on: the operation is simply skipped.
            if card.ops_after_event.is_none() {
                return Err(GameError::CardInPlay { card: card.id });
            }
        }
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(self.status.active),
            event: Event::Pass,
        });
        self.discard_played_card();
        self.advance();
        Ok(())
    }

    /// Resolves the event text of whichever card is currently in play —
    /// the other thing a played card can fund, alongside [`Game::begin`].
    /// Only scoring cards and the fixed-effect cards are implemented so far
    /// ([`crate::events::is_implemented`]); refused otherwise with
    /// [`GameError::EventNotImplemented`]. Either side may play any card's
    /// event, its opponent's included — the event does what the card says,
    /// whoever plays it. Also refused with no card in
    /// play ([`GameError::NoCard`]), while an operation is open
    /// ([`GameError::OperationOpen`] — nothing currently implemented
    /// needs this, since every scoring card has 0 ops and so can never
    /// get an operation open in the first place, but a later ops-and-event
    /// card will), or once the game is already over
    /// ([`GameError::GameOver`]).
    ///
    /// Applies the event's own VP swing (clamped to ±20) and sets
    /// [`Game::winner`] if that reaches it or the event is an outright
    /// win (Europe Scoring's Control tier) — in either case, nothing
    /// resets `active`/`turn`/`action_round` any further, so a human or
    /// AI reading [`Game::winner`] after this call sees the game exactly
    /// as it ended. Otherwise discards the card (or, for a
    /// `removed_after_event` card, removes it from the game entirely —
    /// see [`crate::cards::Hands::remove_from_game`]) and hands the turn
    /// to the other side, same as [`Game::confirm`]/[`Game::cancel`].
    pub fn play_event(&mut self, map: &WorldMap, cards: &CardCatalog) -> Result<EventOutcome, GameError> {
        if let Some(card) = self.card
            && events::needs_dice(card.id)
        {
            return Err(GameError::NeedsDice { card: card.id });
        }
        self.play_event_with(map, cards, &mut Dice::from_seed(0))
    }

    /// [`Game::play_event`] for every card, including the ones whose event draws on
    /// chance (Terrorism's random discard). Draws nothing for any other card.
    pub fn play_event_with(&mut self, map: &WorldMap, cards: &CardCatalog, dice: &mut Dice) -> Result<EventOutcome, GameError> {
        if self.winner.is_some() {
            return Err(GameError::GameOver);
        }
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let card = self.card.ok_or(GameError::NoCard)?;
        if card.ops_after_event.is_some() {
            return Err(GameError::EventPlayed { card: card.id });
        }
        if matches!(card.forced_event, Some((_, PlayAs::Ops))) {
            return Err(GameError::Trap("this card has to be used for operations, not its event".into()));
        }
        if !events::is_implemented(card.id) {
            return Err(GameError::EventNotImplemented { card: card.id });
        }
        match events::blocked_at(card.id, self.hands.removed(), self.status.turn) {
            Some(events::Blocked::Prevented { by }) => return Err(GameError::EventPrevented { card: card.id, by }),
            Some(events::Blocked::Requires { any_of }) => return Err(GameError::EventRequires { card: card.id, any_of }),
            Some(events::Blocked::LateWar) => return Err(GameError::EventTooLate { card: card.id }),
            None => {}
        }

        // Olympic Games: the sponsor's opponent chooses whether to take part; taking part
        // is settled by a roll-off thrown with `Game::roll_contest`.
        if card.id == CardId(20) {
            let choice = EventChoice::olympics_pending(map, &self.board, &self.status, card.id, self.status.active);
            let chooser = choice.chooser();
            self.op = Some(Operation::Event(Box::new(choice)));
            return Ok(EventOutcome::Pending { card: card.id, chooser });
        }

        // Summit: each side rolls (`Game::roll_contest`), +1 for every region it dominates
        // or controls; the winner gets 2 VP and picks the DEFCON change. A tie changes nothing.
        if card.id == CardId(45) {
            let tiers = events::scoring::region_tiers(map, &self.board, &self.status.lasting);
            let held = |ours: fn(&(Region, events::scoring::Tier, events::scoring::Tier)) -> events::scoring::Tier| -> (u8, String) {
                let regions: Vec<String> =
                    tiers.iter().filter(|t| matches!(ours(t), events::scoring::Tier::Domination | events::scoring::Tier::Control)).map(|t| t.0.to_string()).collect();
                (regions.len() as u8, regions.join(", "))
            };
            let choice = EventChoice::summit_pending(map, &self.board, &self.status, card.id, held(|t| t.1), held(|t| t.2));
            self.op = Some(Operation::Event(Box::new(choice)));
            return Ok(EventOutcome::Pending { card: card.id, chooser: self.status.active });
        }

        // Grain Sales to Soviets: the US draws a USSR card at random, then plays it or returns it.
        if card.id == CardId(67) {
            let hand = self.hands.hand(Superpower::Ussr).to_vec();
            let drawn = (!hand.is_empty()).then(|| {
                let pick = hand[dice.index(hand.len())];
                let c = cards.card(pick);
                (PlayCard { id: pick, ops: c.ops, removed: c.removed_after_event, scoring: c.scoring, how: PlayAs::Either, exchange: false }, c.name.clone())
            });
            let choice = EventChoice::grain_sales(map, &self.board, card.id, Superpower::Us, drawn);
            self.op = Some(Operation::Event(Box::new(choice)));
            return Ok(EventOutcome::Pending { card: card.id, chooser: Superpower::Us });
        }

        // Missile Envy: swap for the opponent's highest-Operations card (they pick among ties).
        if card.id == CardId(49) {
            let player = self.status.active;
            let opponent = player.opponent();
            let best = self.hands.hand(opponent).iter().filter(|&&c| !cards.card(c).scoring).map(|&c| cards.card(c).ops).max();
            if let Some(best) = best {
                let options: Vec<(String, PlayCard)> = self
                    .hands
                    .hand(opponent)
                    .iter()
                    .filter(|&&c| !cards.card(c).scoring && cards.card(c).ops == best)
                    .map(|&c| {
                        let taken = cards.card(c);
                        // Its event occurs at once if it is the player's own or neutral and can be played;
                        // an opponent's event is not played — only its Operations value is used.
                        let fires = taken.side != crate::cards::side_of(opponent)
                            && events::is_implemented(c)
                            && events::blocked_at(c, self.hands.removed(), self.status.turn).is_none();
                        (taken.name.clone(), PlayCard { id: c, ops: taken.ops, removed: taken.removed_after_event, scoring: false, how: if fires { PlayAs::Event } else { PlayAs::Ops }, exchange: true })
                    })
                    .collect();
                if options.len() == 1 {
                    let mut result = EffectResult::blank(card.id, player);
                    result.plays = Some(options[0].1);
                    let outcome = EventOutcome::Effect(result.clone());
                    self.finish_effect(result, None);
                    return Ok(outcome);
                }
                let choice = EventChoice::choose_exchange(map, &self.board, card.id, opponent, &options);
                self.op = Some(Operation::Event(Box::new(choice)));
                return Ok(EventOutcome::Pending { card: card.id, chooser: opponent });
            }
        }

        // Five Year Plan: the USSR discards a random card; a US event fires at once (the card
        // then waits in play, its event to be played — see `PlayedCard::forced_event`).
        if card.id == CardId(5) {
            let hand = self.hands.hand(Superpower::Ussr).to_vec();
            if !hand.is_empty() {
                let pick = hand[dice.index(hand.len())];
                let picked = cards.card(pick);
                let fires = picked.side == CardSide::Us
                    && !picked.scoring
                    && events::is_implemented(pick)
                    && events::blocked_at(pick, self.hands.removed(), self.status.turn).is_none();
                let mut result = EffectResult::blank(card.id, self.status.active);
                result.discards.push((Superpower::Ussr, pick));
                if fires {
                    result.plays = Some(PlayCard { id: pick, ops: picked.ops, removed: picked.removed_after_event, scoring: false, how: PlayAs::Event, exchange: false });
                }
                let outcome = EventOutcome::Effect(result.clone());
                self.finish_effect(result, None);
                return Ok(outcome);
            }
        }

        // Star Wars: with the US ahead in the space race, it picks a discarded card and plays its event.
        if card.id == CardId(85) && self.status.space_race_us > self.status.space_race_ussr {
            let pile = self.pile_cards(cards, true);
            let choice = EventChoice::play_from_pile(map, &self.board, card.id, Superpower::Us, &pile);
            self.op = Some(Operation::Event(Box::new(choice)));
            return Ok(EventOutcome::Pending { card: card.id, chooser: Superpower::Us });
        }

        // SALT Negotiations: DEFCON +2 and the coup penalty, then the player may take a
        // non-scoring card from the discard pile (an empty pile still opens the pick, to say so).
        if card.id == CardId(43) {
            let pile = self.pile_cards(cards, false);
            let player = self.status.active;
            let defcon = Some((self.status.defcon + 2).min(5));
            // Even an empty pile opens the pick, so the player sees why nothing can be taken.
            if let Some(pick) = EventChoice::pick_from_pile(map, &self.board, card.id, player, &pile, defcon, Some(crate::ongoing::OngoingEffect::Salt)) {
                self.op = Some(Operation::Event(Box::new(pick)));
                return Ok(EventOutcome::Pending { card: card.id, chooser: player });
            }
        }

        // Aldrich Ames Remix: the USSR picks a card out of the (revealed) US hand to
        // discard. With nothing there, only the reveal happens, through the ordinary event.
        if card.id == CardId(98) {
            let us_hand: Vec<(CardId, String)> = self.hands.hand(Superpower::Us).iter().map(|&c| (c, cards.card(c).name.clone())).collect();
            if let Some(pick) = EventChoice::pick_from_hand(map, &self.board, card.id, Superpower::Ussr, &us_hand) {
                self.op = Some(Operation::Event(Box::new(pick)));
                return Ok(EventOutcome::Pending { card: card.id, chooser: Superpower::Ussr });
            }
        }

        // The Cambridge Five: the USSR may add 1 influence in a region named by a scoring
        // card in the US hand (Southeast Asia isn't a region, so its card names none).
        if card.id == CardId(104) {
            let scoring: Vec<CardId> = self.hands.hand(Superpower::Us).iter().copied().filter(|&c| cards.card(c).scoring).collect();
            let mut regions: Vec<Region> = scoring.iter().filter_map(|&c| events::scoring::region_of(c)).collect();
            regions.dedup();
            let reveal = events::Reveal { side: Superpower::Us, cards: scoring };
            if let Some(choice) = EventChoice::in_named_regions(map, &self.board, card.id, &regions, reveal) {
                self.op = Some(Operation::Event(Box::new(choice)));
                return Ok(EventOutcome::Pending { card: card.id, chooser: Superpower::Ussr });
            }
        }

        // Blockade and Debt Crisis first ask the US to discard a 3+ ops card
        // or suffer: it decides, whoever played the card. With nothing to
        // discard the penalty simply applies, through the card's own event.
        if let Some(decider) = events::choice::gate_decider(card.id) {
            let candidates: Vec<(CardId, String)> = self
                .hands
                .hand(decider)
                .iter()
                .filter(|&&c| cards.card(c).ops >= events::choice::GATE_MIN_OPS)
                .map(|&c| (c, cards.card(c).name.clone()))
                .collect();
            if let Some(gate) = EventChoice::discard_gate(map, &self.board, &self.status, card.id, &candidates) {
                self.op = Some(Operation::Event(Box::new(gate)));
                return Ok(EventOutcome::Pending { card: card.id, chooser: decider });
            }
        }

        // A war opens a session; `Game::roll` on a target (the only one, for
        // Korean War and Arab-Israeli War) resolves it.
        if war::is_war_card(card.id) {
            let player = self.status.active;
            self.op = Some(Operation::War(war::War::new(card.id, player).with_lasting(map, &self.board, &self.status.lasting)));
            return Ok(EventOutcome::Pending { card: card.id, chooser: player });
        }

        // A choice card doesn't resolve in one call: it opens a session
        // its chooser (the card's own side) works through, and only
        // `confirm` applies it — unless there is nothing it can do at
        // all, in which case it resolves on the spot.
        if let Some(choice) = EventChoice::new(map, &self.board, &self.status, card.id) {
            let choice = choice.with_grant(events::ops_grant(map, &self.board, self.hands.removed(), card.id, self.status.active));
            if choice.resolves_immediately(map) {
                let result = choice.into_result(&self.status);
                self.finish_effect(result.clone(), choice.grant());
                return Ok(EventOutcome::Effect(result));
            }
            let chooser = choice.chooser();
            self.op = Some(Operation::Event(Box::new(choice)));
            return Ok(EventOutcome::Pending { card: card.id, chooser });
        }

        let mut outcome = events::resolve(map, &self.board, &self.status, card.id).expect("is_implemented checked above");
        // Fill in what depends on the hands now, so the result handed back to the caller
        // (and its modal) carries it as well as the log.
        if let EventOutcome::Effect(result) = &mut outcome {
            if let Some(reveal) = &mut result.reveals {
                // The Cambridge Five shows only the scoring cards.
                reveal.cards = self.hands.hand(reveal.side).iter().copied().filter(|&c| card.id != CardId(104) || cards.card(c).scoring).collect();
            }
            if card.id == CardId(92) {
                // Terrorism: the opponent loses cards at random (two for the US once #82 is out).
                let victim = self.status.active.opponent();
                let n = if victim == Superpower::Us && self.hands.removed().contains(&CardId(82)) { 2 } else { 1 };
                let mut hand = self.hands.hand(victim).to_vec();
                for _ in 0..n.min(hand.len()) {
                    let i = dice.index(hand.len());
                    result.discards.push((victim, hand.remove(i)));
                }
            }
        }

        match &outcome {
            EventOutcome::Scoring(result) => {
                self.log_card_selected();
                self.apply_vp(result.vp_delta);
                // Shuttle Diplomacy is spent by the scoring it modified.
                if result.modifiers.contains(&CardId(73)) {
                    self.status.lasting.cancel(LastingEffect::ShuttleDiplomacy);
                    self.hands.discard(CardId(73));
                }
                if let Some(side) = result.automatic_victory {
                    self.set_winner(side, VictoryReason::EuropeControl);
                }
                let vp_after = self.status.vp;
                self.log.push(LogEntry {
                    turn: self.status.turn,
                    action_round: self.status.action_round,
                    side: Some(self.status.active),
                    event: Event::Scored { result: result.clone(), vp_after },
                });
                self.log_game_over();
                self.discard_or_remove_event_card();
                if self.winner.is_none() {
                    self.advance();
                }
            }
            EventOutcome::Effect(result) => {
                let grant = events::ops_grant(map, &self.board, self.hands.removed(), card.id, self.status.active);
                self.finish_effect(result.clone(), grant);
            }
            EventOutcome::Pending { .. } => unreachable!("wars and choice cards are handled above"),
        }
        Ok(outcome)
    }

    /// Applies a finished fixed-effect or choice event — influence, then
    /// DEFCON, then VP — logs it, discards (or removes) the card, and
    /// hands the turn over unless that ended the game. The one path both
    /// [`Game::play_event`] (fixed effects) and [`Game::confirm`] (a
    /// finished choice) use.
    fn finish_effect(&mut self, result: EffectResult, grant: Option<OpsGrant>) {
        let plays = result.plays;
        let host = result.card;
        self.apply_effect(result);
        // Another card's event puts a card into play (Star Wars, Five Year Plan, Grain Sales,
        // Missile Envy): this card is spent and the one it names takes its place, the turn
        // not yet over.
        if let (None, Some(p)) = (self.winner, plays)
            && self.hands.take(p.id)
        {
            if p.exchange {
                // Missile Envy changes hands; its new holder must use it for operations.
                let opponent = self.status.active.opponent();
                if let Some(me) = self.card.take() {
                    self.hands.push_to_hand(opponent, me.id);
                    self.status.forced_play = Some((opponent, me.id.0));
                }
            } else {
                self.discard_or_remove_event_card();
            }
            self.card = Some(PlayedCard { id: p.id, ops: p.ops, hand_index: None, logged: false, scoring: p.scoring, removed_after_event: p.removed, ops_after_event: None, forced_event: Some((host, p.how)) });
            return;
        }
        // A card whose event allows an operation stays in play for it.
        // (not a headline card: that is its event only)
        let grant = grant.filter(|_| self.phase != Phase::Headline);
        if let (None, Some(grant), Some(card)) = (self.winner, grant, self.card.as_mut()) {
            card.ops_after_event = Some(grant);
            return;
        }
        self.discard_or_remove_event_card();
        if self.winner.is_none() {
            self.advance();
        }
    }

    /// Applies and logs a resolved event's changes — everything but spending the card and handing
    /// the turn over.
    fn apply_effect(&mut self, result: EffectResult) {
        self.log_card_selected();
        for &(side, card) in &result.discards {
            if self.hands.remove(side, card).is_some() {
                self.hands.discard(card);
            }
        }
        for &(side, card) in &result.takes {
            if self.hands.take(card) {
                self.hands.push_to_hand(side, card);
            }
        }
        for change in &result.influence {
            self.board.set_influence(change.country, change.side, change.after);
        }
        // DEFCON before VP, so a DEFCON-1 loss outranks whatever
        // VP the same card also awards (`set_winner` keeps the first).
        if let Some((_, after)) = result.defcon {
            self.apply_defcon(after);
        }
        self.apply_vp(result.vp_delta);
        if result.mil_ops != 0 {
            let track = match result.player {
                Superpower::Us => &mut self.status.military_ops_us,
                Superpower::Ussr => &mut self.status.military_ops_ussr,
            };
            *track = (*track + result.mil_ops).clamp(0, war::MIL_OPS_MAX);
        }
        if result.ends_game && self.winner.is_none() {
            let side = match self.status.vp.signum() {
                1 => Superpower::Us,
                -1 => Superpower::Ussr,
                _ => result.player.opponent(),
            };
            self.set_winner(side, VictoryReason::Wargames);
        }
        if let Some(effect) = result.ongoing {
            self.status.effects.apply(effect);
        }
        if let Some(effect) = result.lasting {
            self.status.lasting.apply(effect);
        }
        if let Some(effect) = result.cancels {
            self.status.lasting.cancel(effect);
        }
        if let Some((side, _, to)) = result.space {
            space::set_position(&mut self.status, side, to);
        }
        if let Some(transfer) = result.china {
            self.status.china_card = transfer.to;
            self.status.china_card_face_up = transfer.face_up;
        }
        let vp_after = self.status.vp;
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(self.status.active),
            event: Event::EventResolved { result, vp_after },
        });
        self.log_game_over();
    }

    /// Applies the influence of an event nothing played (NORAD): no card to spend, no turn to hand over.
    fn finish_triggered(&mut self, result: EffectResult) {
        for change in &result.influence {
            self.board.set_influence(change.country, change.side, change.after);
        }
        let vp_after = self.status.vp;
        self.log.push(LogEntry { turn: self.status.turn, action_round: self.status.action_round, side: Some(result.player), event: Event::EventResolved { result, vp_after } });
    }

    /// Applies a resolved war — influence, VP, then the beneficiary's
    /// Military Operations — logs it, discards (or removes) the card, and
    /// hands the turn over unless that ended the game.
    fn finish_war(&mut self, result: WarResult) {
        self.log_card_selected();
        for change in &result.influence {
            self.board.set_influence(change.country, change.side, change.after);
        }
        self.apply_vp(result.vp_delta);
        let track = match result.side {
            Superpower::Us => &mut self.status.military_ops_us,
            Superpower::Ussr => &mut self.status.military_ops_ussr,
        };
        *track = (*track + result.mil_ops).clamp(0, war::MIL_OPS_MAX);
        let vp_after = self.status.vp;
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(self.status.active),
            event: Event::War { result, vp_after },
        });
        self.log_game_over();
        if let Some(card) = self.card.map(|c| c.id) {
            self.flower_power_check(card);
        }
        self.discard_or_remove_event_card();
        if self.winner.is_none() {
            self.advance();
        }
    }

    /// Pushes [`Event::GameOver`] if [`Game::winner`] is set.
    fn log_game_over(&mut self) {
        if let Some(victory) = self.winner {
            self.log.push(LogEntry {
                turn: self.status.turn,
                action_round: self.status.action_round,
                side: Some(self.status.active),
                event: Event::GameOver(victory),
            });
        }
    }

    /// Sets DEFCON to `level` (clamped to the track), ending the game
    /// against the phasing (active) side if it reaches 1 — rule 8.1.3: the
    /// phasing player is responsible for the marker reaching DEFCON 1, whoever's
    /// choice actually moved it, and loses.
    fn apply_defcon(&mut self, level: u8) {
        self.status.defcon = level.clamp(1, 5);
        if self.status.defcon == 1 {
            self.set_winner(self.status.active.opponent(), VictoryReason::Defcon);
        }
    }

    /// Adds `delta` to the VP track, clamped to ±20 (rule 5.5's cap —
    /// nothing above this reads a VP outside that range), and sets
    /// [`Game::winner`] if it lands exactly on either end.
    fn apply_vp(&mut self, delta: i8) {
        let new_vp = (self.status.vp as i16 + delta as i16).clamp(-20, 20) as i8;
        self.status.vp = new_vp;
        if new_vp >= 20 {
            self.set_winner(Superpower::Us, VictoryReason::Vp);
        } else if new_vp <= -20 {
            self.set_winner(Superpower::Ussr, VictoryReason::Vp);
        }
    }

    /// Sets [`Game::winner`] — a no-op if one's already set, so the
    /// *first* way the game ends is the one that sticks (relevant only
    /// to a contrived board where a VP cap and a Europe automatic
    /// victory would otherwise race on the same call).
    fn set_winner(&mut self, side: Superpower, reason: VictoryReason) {
        if self.winner.is_none() {
            self.winner = Some(Victory { side: Some(side), reason });
        }
    }

    /// Pushes [`Event::Selected`] for the card currently in play, if it
    /// hasn't been already (`PlayedCard::logged` — a no-op otherwise).
    /// This, not [`Game::play_card`], is where the entry really comes
    /// from: the log is a record of the actual game, not of
    /// application-level steps, so a card that's merely *selected* writes
    /// nothing — only once its selection can no longer be taken back does
    /// it become something that really happened. That point differs by
    /// what the card ends up funding: [`Game::roll`] calls this itself,
    /// right before a realignment's or coup's *first* roll (the instant
    /// [`Game::abandon`] stops being able to undo it); [`Game::log_close`]
    /// calls it for everything else — a placement, confirmed or cancelled
    /// with any (or no) points pending, or a realignment/coup that never
    /// rolled at all, discarding the card either way. If neither ever
    /// runs — the operation (or the bare card) is abandoned and
    /// [`Game::return_card`] puts it back in the hand — this never runs
    /// either, and the log shows no sign the card was ever picked up.
    fn log_card_selected(&mut self) {
        let Some(card) = &mut self.card else { return };
        if card.logged {
            return;
        }
        card.logged = true;
        let id = card.id;
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(self.status.active),
            event: Event::Selected { card: id },
        });
    }

    /// Pushes the log entry (entries, for a placement) for a closing
    /// operation — called from `confirm`/`cancel` *before* [`Game::advance`],
    /// so they carry the turn/AR the operation actually happened in, not
    /// the next one. Starts with [`Game::log_card_selected`], so a
    /// placement (or an un-rolled realignment/coup) gets its card's own
    /// entry here, right before everything else this closing operation
    /// writes — a realignment or coup that *did* roll already logged it,
    /// back at the first roll, so this is a no-op for those.
    fn log_close(&mut self, op: &Operation, committed: bool) {
        self.log_card_selected();
        let turn = self.status.turn;
        let action_round = self.status.action_round;
        let side = Some(op.side());
        let mut push = |event| self.log.push(LogEntry { turn, action_round, side, event });

        let (kind, rolls, ops_spent, ops_total) = match op {
            Operation::Influence(p) => {
                // The placement analogue of a resolved roll: its own entry,
                // pushed just before the `Closed` line below — mirroring
                // how a realignment's or coup's rolls already have their
                // own entries by the time `Closed` is pushed. Omitted
                // entirely if nothing was placed, the same way zero rolls
                // simply mean zero `Event::Realign` entries.
                let countries = p.pending_countries();
                if !countries.is_empty() {
                    push(Event::Placed { countries });
                }
                (OperationKind::Influence, 0, p.ops_spent(), p.ops_total())
            }
            Operation::Realign(r) => (OperationKind::Realign, r.history().len() as u8, r.ops_spent(), r.ops_total()),
            Operation::Coup(c) => (OperationKind::Coup, c.result().is_some() as u8, c.ops_spent(), c.ops_total()),
            Operation::Event(_) => unreachable!("an event closes through finish_effect, never log_close"),
            Operation::War(_) => unreachable!("a war closes through finish_war, never log_close"),
        };
        push(Event::Closed { kind, committed, rolls, ops_spent, ops_total });
    }

    /// Discards whichever card was in play — called from `confirm`/`cancel`
    /// only, the two places a played card is ever actually spent (`abandon`
    /// leaves it in play; `return_card` puts it straight back in the hand).
    /// The China Card is the one exception to "discard pile": instead of
    /// leaving play, it passes face down to the opponent (a simplified
    /// rule 6.1 — no Asia Scoring bonus modelled yet) and turns back face
    /// up once [`Game::advance`] rolls the turn over to a new one.
    fn discard_played_card(&mut self) {
        let Some(card) = self.card.take() else { return };
        if self.status.forced_play.is_some_and(|(_, c)| c == card.id.0) {
            self.status.forced_play = None;
        }
        if card.id == CHINA_CARD && self.status.active == Superpower::Us {
            // Formosan Resolution ends once the US plays the China Card.
            self.status.lasting.cancel(LastingEffect::Formosan);
        }
        self.flower_power_check(card.id);
        if card.id == CHINA_CARD {
            self.status.china_card = self.status.active.opponent();
            self.status.china_card_face_up = false;
        } else if card.ops_after_event.is_some() && card.removed_after_event {
            // Its event was played (before the operation), so it leaves the game.
            self.hands.remove_from_game(card.id);
        } else {
            self.hands.discard(card.id);
        }
    }

    /// [`Game::discard_played_card`]'s counterpart for
    /// [`Game::play_event`]: sends the card to the removed-from-play pile
    /// instead of the discard pile if its *event* is
    /// `removed_after_event` (rule 4.4) — never the China Card, since
    /// none of the cards [`events::is_implemented`] recognises is it.
    fn discard_or_remove_event_card(&mut self) {
        let Some(card) = self.card.take() else { return };
        if card.id == CardId(73) && self.status.lasting.shuttle_diplomacy {
            // Shuttle Diplomacy stays in effect — it's discarded by the scoring it modifies.
            return;
        }
        if card.removed_after_event {
            self.hands.remove_from_game(card.id);
        } else {
            self.hands.discard(card.id);
        }
    }

    /// Logs a lasting event's own payout of `vp` (USSR-favouring when
    /// negative, in [`GameStatus::vp`]'s convention) and applies it.
    fn trigger(&mut self, card: CardId, vp_delta: i8) {
        self.apply_vp(vp_delta);
        let vp_after = self.status.vp;
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(self.status.active),
            event: Event::Triggered { card, vp_delta, vp_after },
        });
        self.log_game_over();
    }

    /// Flower Power: the USSR gets 2 VP whenever the US spends a war card,
    /// for ops or for its event.
    fn flower_power_check(&mut self, card: CardId) {
        if self.status.lasting.flower_power && self.status.active == Superpower::Us && war::is_war_card(card) {
            self.trigger(CardId(59), -2);
        }
    }

    /// Hands the turn to the next action round slot: USSR to USA within
    /// the same round, or USA to the next round's first side that still
    /// has a round to play ([`GameStatus::rounds_for`] — North Sea Oil
    /// gives the US a round the USSR doesn't get, and the Space Station
    /// box does the same for its holder). When neither side has a round
    /// left the turn rolls over: `action_round` back to 1, the China Card
    /// face up again, every "remainder of the turn" effect and the space
    /// attempt counters cleared. The only writer of `active`/`turn`/
    /// `action_round`, called from `confirm`, `cancel`, `pass` and
    /// `space` — nowhere else.
    fn advance(&mut self) {
        // A headline event ends with no turn to hand over: `settle` carries on to the next one.
        if self.phase == Phase::Headline {
            return;
        }
        // NORAD: the round that just ended moved DEFCON to 2.
        if self.status.lasting.norad && self.status.defcon == 2 && self.round_defcon != 2 {
            self.norad_due = true;
        }
        self.advance_round();
        self.round_defcon = self.status.defcon;
    }

    fn advance_round(&mut self) {
        let round = self.status.action_round;
        match self.status.active {
            Superpower::Ussr => {
                if self.status.rounds_for(Superpower::Us) >= round {
                    self.status.active = Superpower::Us;
                } else {
                    self.begin_round(round + 1);
                }
            }
            Superpower::Us => {
                // We Will Bury You pays once the US has finished the round
                // it was owed in (`skip` rounds are let pass first).
                match self.status.lasting.we_will_bury_you {
                    Some(0) => {
                        self.status.lasting.cancel(LastingEffect::WeWillBuryYou { skip: 0 });
                        self.trigger(CardId(50), -3);
                        if self.winner.is_some() {
                            return;
                        }
                    }
                    Some(n) => self.status.lasting.we_will_bury_you = Some(n - 1),
                    None => {}
                }
                self.begin_round(round + 1);
            }
        }
    }

    /// Starts action round `round`: the USSR goes first if it has a round
    /// to play, else the US; if neither does, the turn rolls over.
    fn begin_round(&mut self, round: u8) {
        for side in [Superpower::Ussr, Superpower::Us] {
            if self.status.rounds_for(side) >= round {
                self.status.active = side;
                self.status.action_round = round;
                return;
            }
        }
        // Neither side has a round left: the turn is over. `active`/`action_round` stay where they
        // are until `settle` (which has the map, cards and dice) runs the end of the turn.
        self.phase = Phase::TurnEnd;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardCatalog;
    use crate::country::Superpower::{Us, Ussr};
    use crate::map::WorldMap;

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn cards() -> CardCatalog {
        CardCatalog::standard().unwrap()
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country named {name:?}"))
    }

    /// Runs the end of the turn, which `advance` leaves to `settle`.
    fn settle(game: &mut Game, map: &WorldMap, cards: &CardCatalog) {
        game.settle(map, cards, &mut Dice::from_seed(1));
    }

    /// Every test scenario deals the same four cards, so any test can play
    /// a card without having to spell out a hand of its own: USSR gets
    /// Socialist Governments (3 ops) and Fidel (2 ops); US gets Duck and
    /// Cover (3 ops) and Five Year Plan (3 ops).
    const HANDS_JSON: &str = r#""hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Socialist Governments","Fidel"]}"#;

    fn scenario(map: &WorldMap, cards: &CardCatalog) -> Scenario {
        let json = format!("{{{HANDS_JSON}}}");
        Scenario::from_json(map, cards, &json).unwrap()
    }

    /// A scenario with one country pre-seeded, so realignment/coup targets
    /// (which need opponent presence) have something to aim at — plus the
    /// same standard hands every other scenario here deals.
    fn scenario_with(map: &WorldMap, cards: &CardCatalog, country: &str, us: u8, ussr: u8) -> Scenario {
        let json = format!(r#"{{"influence":{{"{country}":[{us},{ussr}]}},{HANDS_JSON}}}"#);
        Scenario::from_json(map, cards, &json).unwrap()
    }

    /// A scenario with `status_json` (a `GameStatus`-shaped object body)
    /// overriding the defaults, plus the same standard hands.
    fn scenario_with_status(map: &WorldMap, cards: &CardCatalog, status_json: &str) -> Scenario {
        let json = format!(r#"{{"status":{status_json},{HANDS_JSON}}}"#);
        Scenario::from_json(map, cards, &json).unwrap()
    }

    /// A scenario like [`scenario`], but with `extra` prepended to
    /// `side`'s hand — for tests that need a scoring card (or any other
    /// third card) available without losing the two ordinary cards every
    /// other test here already relies on.
    fn scenario_with_extra_card(map: &WorldMap, cards: &CardCatalog, side: Superpower, extra: &str) -> Scenario {
        let (us_hand, ussr_hand) = match side {
            Superpower::Us => (format!(r#""{extra}","Duck and Cover","Five Year Plan""#), r#""Socialist Governments","Fidel""#.to_string()),
            Superpower::Ussr => (r#""Duck and Cover","Five Year Plan""#.to_string(), format!(r#""{extra}","Socialist Governments","Fidel""#)),
        };
        let json = format!(r#"{{"hands":{{"us":[{us_hand}],"ussr":[{ussr_hand}]}}}}"#);
        Scenario::from_json(map, cards, &json).unwrap()
    }

    /// Plays `name` (one of the cards [`scenario`]/[`scenario_with`] deal)
    /// for the active side, returning its id — the one step every test
    /// now needs before `begin` will open an operation.
    fn play(game: &mut Game, cards: &CardCatalog, name: &str) -> CardId {
        let id = cards.id_by_name(name).unwrap_or_else(|| panic!("no card named {name:?}"));
        game.play_card(cards, id).unwrap();
        id
    }

    #[test]
    fn ussr_acts_first_and_confirm_hands_over_to_us() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        assert_eq!(game.active(), Ussr);

        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        assert_eq!(game.operation().unwrap().side(), Ussr);
        game.confirm().unwrap();

        assert_eq!(game.active(), Us);
    }

    #[test]
    fn action_round_only_increments_after_us_acts() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let start_ar = game.status().action_round;

        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // USSR -> US
        assert_eq!(game.status().action_round, start_ar);

        play(&mut game, &cards, "Duck and Cover");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // US -> USSR
        assert_eq!(game.status().action_round, start_ar + 1);
    }

    #[test]
    fn action_round_rolls_the_turn_over_when_it_passes_the_limit() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario_with_status(
            &map,
            &cards,
            r#"{"turn":1,"action_round":2,"action_rounds_per_turn":2}"#,
        ));
        assert_eq!(game.status().active, Ussr);

        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // USSR -> US, still AR 2
        assert_eq!(game.status().turn, 1);
        assert_eq!(game.status().action_round, 2);

        play(&mut game, &cards, "Duck and Cover");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // US -> USSR: AR 2 was the last of the turn
        assert_eq!(game.phase(), Phase::TurnEnd, "the turn waits for settle, which has the map, cards and dice it needs");
        assert_eq!(game.status().turn, 1);
        settle(&mut game, &map, &cards);
        assert_eq!(game.status().turn, 2);
        assert_eq!((game.phase(), game.status().action_round), (Phase::Headline, 0), "the new turn opens with the headline phase");
    }

    #[test]
    fn begin_is_refused_while_an_operation_is_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        assert!(matches!(game.begin(OperationKind::Coup), Err(GameError::OperationOpen { .. })));
    }

    #[test]
    fn begin_is_refused_with_no_card_in_play() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        assert!(matches!(game.begin(OperationKind::Influence), Err(GameError::NoCard)));
    }

    #[test]
    fn every_operation_opens_with_the_active_side_and_the_played_cards_ops() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments"); // 3 ops
        game.begin(OperationKind::Realign).unwrap();
        let op = game.operation().unwrap();
        assert_eq!(op.side(), Ussr);
        assert_eq!(op.ops_total(), 3);
        assert_eq!(game.ops_available(), 3);
    }

    #[test]
    fn cancel_advances_the_turn_just_like_confirm() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.cancel().unwrap();
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn pass_advances_with_no_operation_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        game.pass().unwrap();
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn pass_is_refused_while_an_operation_is_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        assert!(matches!(game.pass(), Err(GameError::OperationOpen { .. })));
    }

    #[test]
    fn pass_is_refused_with_a_card_in_play_but_no_operation_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Fidel");
        assert!(matches!(game.pass(), Err(GameError::CardInPlay { .. })));
    }

    #[test]
    fn abandon_is_refused_with_no_operation_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        assert!(matches!(game.abandon(), Err(GameError::NoOperation)));
    }

    #[test]
    fn an_untouched_operation_can_be_abandoned_without_costing_the_turn() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let sg = play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.abandon().unwrap();
        assert_eq!(game.active(), Ussr, "abandoning before anything's spent shouldn't hand the turn over");
        assert!(game.operation().is_none());
        assert_eq!(game.card_in_play(), Some(sg), "the card stays in play — abandon only closes the operation");
    }

    #[test]
    fn a_placement_can_always_be_abandoned_even_with_points_pending() {
        // Placement never rolls a die, so nothing about it is hidden or
        // irreversible until `confirm` runs — unlike realignment/coup,
        // `abandon` never refuses it.
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland"); // borders the USSR itself
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments"); // 3 ops
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.place(&map, poland).unwrap();
        game.place(&map, poland).unwrap();

        game.abandon().unwrap();

        assert_eq!(game.active(), Ussr, "abandoning a placement shouldn't hand the turn over");
        assert!(game.operation().is_none());
        assert_eq!(game.board().influence(poland, Ussr), 0, "every pending point should be discarded, not committed");
        assert_eq!(game.ops_available(), 3, "every op it cost should be refunded, since the card stays in play");
    }

    #[test]
    fn undoing_every_placed_point_makes_the_operation_abandonable_too() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.undo(&map).unwrap();
        game.abandon().unwrap();
        assert_eq!(game.active(), Ussr);
    }

    #[test]
    fn abandon_is_refused_once_a_realignment_roll_has_been_made() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Realign).unwrap();
        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandon { ops_spent: 1, .. })));
    }

    #[test]
    fn abandon_is_refused_once_a_coup_has_been_attempted() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut game, &cards, "Fidel"); // 2 ops
        game.begin(OperationKind::Coup).unwrap();
        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandon { ops_total: 2, .. })));
    }

    #[test]
    fn abandoning_leaves_no_trace_in_the_log() {
        // Not even the card's own `Selected` entry: a coup that never
        // rolled, then abandoned, means the card's selection was never
        // irrevocable (`Game::log_card_selected` only runs at the first
        // roll or at `confirm`/`cancel`, neither of which happened here).
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        game.abandon().unwrap();
        assert!(game.log().is_empty(), "an abandoned operation should leave the log exactly as it was");
    }

    #[test]
    fn abandoning_a_placement_with_pending_points_leaves_no_trace_in_the_log_either() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.abandon().unwrap();
        assert!(game.log().is_empty(), "as far as the history's concerned, an abandoned placement never happened either");
    }

    #[test]
    fn returning_a_card_that_never_funded_anything_leaves_no_trace_in_the_log() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Fidel");
        game.return_card().unwrap();
        assert!(game.log().is_empty(), "a card selected and returned without ever being played shouldn't appear in the log");
    }

    #[test]
    fn a_card_is_logged_the_instant_its_first_roll_makes_it_irrevocable() {
        let map = map();
        let cards = cards();
        // Not a battleground, so the aftermath is only the coup's Military Ops (no DEFCON drop).
        let poland = id(&map, "Czechoslovakia");
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Czechoslovakia", 1, 0));
        let fidel = play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        assert!(game.log().is_empty(), "playing a card and opening an operation shouldn't log anything yet");

        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 3, "the roll's first action should log the card, the roll itself, then its Military Ops");
        assert!(matches!(entries[0].event, Event::Selected { card } if card == fidel));
        assert!(matches!(entries[1].event, Event::Coup(_)));
        assert!(matches!(entries[2].event, Event::CoupAftermath(a) if a.mil_ops == Some((0, 2))));

        // A second roll on the same operation shouldn't log the card
        // again — it's `CoupError` territory (a coup only ever attempts
        // once), but a multi-roll realignment is the real-world case this
        // guards: `log_card_selected` must be idempotent per card.
    }

    #[test]
    fn an_un_rolled_realignment_that_closes_still_logs_the_card() {
        // No roll happened, so `Game::roll` never got the chance to log
        // the card — `log_close` is the fallback that catches a card
        // whose operation still closed (confirm or cancel) without ever
        // becoming irrevocable the other way.
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let sg = play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Realign).unwrap();
        game.cancel().unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].event, Event::Selected { card } if card == sg));
        assert!(matches!(entries[1].event, Event::Closed { kind: OperationKind::Realign, committed: false, .. }));
    }

    #[test]
    fn a_confirmed_placement_lands_on_the_board_but_a_cancelled_one_does_not() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");

        let mut confirmed = Game::from_scenario(&scenario(&map, &cards));
        play(&mut confirmed, &cards, "Socialist Governments");
        confirmed.begin(OperationKind::Influence).unwrap();
        confirmed.place(&map, poland).unwrap();
        confirmed.confirm().unwrap();
        assert_eq!(confirmed.board().influence(poland, Ussr), 1);

        let mut cancelled = Game::from_scenario(&scenario(&map, &cards));
        play(&mut cancelled, &cards, "Socialist Governments");
        cancelled.begin(OperationKind::Influence).unwrap();
        cancelled.place(&map, poland).unwrap();
        cancelled.cancel().unwrap();
        assert_eq!(cancelled.board().influence(poland, Ussr), 0);
    }

    #[test]
    fn place_is_refused_when_a_different_kind_of_operation_is_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Realign).unwrap();
        assert!(matches!(game.place(&map, id(&map, "Poland")), Err(GameError::WrongKind { .. })));
    }

    #[test]
    fn undo_is_refused_for_a_resolved_realignment_or_coup() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");

        let mut realigning = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut realigning, &cards, "Socialist Governments");
        realigning.begin(OperationKind::Realign).unwrap();
        let mut dice = Dice::from_seed(0);
        realigning.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(realigning.undo(&map), Err(GameError::CannotUndo { .. })));

        let mut couping = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut couping, &cards, "Fidel");
        couping.begin(OperationKind::Coup).unwrap();
        couping.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(couping.undo(&map), Err(GameError::CannotUndo { .. })));
    }

    #[test]
    fn roll_does_not_advance_the_turn_only_confirm_and_cancel_do() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();
        assert_eq!(game.active(), Ussr);
        game.confirm().unwrap();
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn a_scripted_sequence_produces_the_expected_log_entries_in_order() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let poland = id(&map, "Poland");

        play(&mut game, &cards, "Socialist Governments"); // USSR
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.place(&map, poland).unwrap();
        game.confirm().unwrap(); // -> US

        let mut dice = Dice::from_seed(0);
        play(&mut game, &cards, "Duck and Cover"); // US
        game.begin(OperationKind::Realign).unwrap();
        game.roll(&map, poland, &mut dice).unwrap();
        game.cancel().unwrap(); // -> USSR

        game.pass().unwrap(); // USSR -> US

        let events: Vec<_> = game.log().entries().iter().map(|e| &e.event).collect();
        assert!(matches!(events[0], Event::Selected { .. }));
        assert!(matches!(events[1], Event::Placed { .. }));
        assert!(matches!(events[2], Event::Closed { kind: OperationKind::Influence, committed: true, .. }));
        assert!(matches!(events[3], Event::Selected { .. }));
        assert!(matches!(events[4], Event::Realign(_)));
        assert!(matches!(events[5], Event::Closed { kind: OperationKind::Realign, committed: false, .. }));
        assert!(matches!(events[6], Event::Pass));
        assert_eq!(events.len(), 7);
    }

    #[test]
    fn a_confirm_entry_carries_the_turn_and_side_it_actually_happened_in() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario_with_status(
            &map,
            &cards,
            r#"{"turn":1,"action_round":2,"action_rounds_per_turn":2}"#,
        ));

        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // USSR -> US, rolls the turn over on the *next* US confirm

        // entries[0] is the card's own `Selected` entry, stamped the same
        // way at the same moment; entries[1] is `Closed` itself.
        let entry = &game.log().entries()[1];
        assert!(matches!(entry.event, Event::Closed { .. }));
        assert_eq!(entry.turn, 1);
        assert_eq!(entry.action_round, 2);
        assert_eq!(entry.side, Some(Ussr));
        // advance() has already run, so status has moved on from what the entry recorded.
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn a_cancelled_placement_gets_its_own_placed_line_and_a_cancel_line() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let sg = play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.cancel().unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 3);
        match &entries[0].event {
            Event::Selected { card } => assert_eq!(*card, sg),
            other => panic!("expected a Selected event naming the played card, got {other:?}"),
        }
        match &entries[1].event {
            Event::Placed { countries } => assert_eq!(countries, &vec![(poland, 1)]),
            other => panic!("expected a Placed event, got {other:?}"),
        }
        assert!(matches!(entries[2].event, Event::Closed { kind: OperationKind::Influence, committed: false, .. }));
        assert_eq!(game.board().influence(poland, Ussr), 0);
    }

    #[test]
    fn an_immediately_cancelled_placement_with_nothing_placed_has_no_placed_line() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        game.cancel().unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0].event, Event::Selected { .. }));
        assert!(matches!(
            entries[1].event,
            Event::Closed { kind: OperationKind::Influence, committed: false, .. }
        ));
    }

    #[test]
    fn a_realignment_roll_is_logged_before_the_operation_closes() {
        let map = map();
        let cards = cards();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        play(&mut game, &cards, "Socialist Governments");
        let mut dice = Dice::from_seed(0);
        game.begin(OperationKind::Realign).unwrap();
        game.roll(&map, poland, &mut dice).unwrap();

        assert_eq!(game.log().len(), 2);
        assert!(matches!(game.log().entries()[0].event, Event::Selected { .. }));
        assert!(matches!(game.log().entries()[1].event, Event::Realign(_)));

        game.confirm().unwrap();
        assert_eq!(game.log().len(), 3);
    }

    #[test]
    fn lookahead_clones_state_but_starts_with_an_empty_log() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        game.pass().unwrap();
        assert_eq!(game.log().len(), 1);

        let ahead = game.lookahead();
        assert!(ahead.log().is_empty());
        assert_eq!(ahead.status(), game.status());
        assert_eq!(ahead.active(), game.active());
    }

    #[test]
    fn lookahead_preserves_the_card_in_play() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        let ahead = game.lookahead();
        assert_eq!(ahead.card_in_play(), game.card_in_play());
    }

    #[test]
    fn playing_a_card_removes_it_from_hand_and_confirm_discards_it() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let sg = cards.id_by_name("Socialist Governments").unwrap();
        assert!(game.hand(Ussr).contains(&sg));

        game.play_card(&cards, sg).unwrap();
        assert!(!game.hand(Ussr).contains(&sg), "playing a card should remove it from the hand");
        assert_eq!(game.card_in_play(), Some(sg));

        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.card_in_play(), None, "confirm should discard the played card");
        assert!(!game.hand(Ussr).contains(&sg), "a discarded card shouldn't return to the hand");
    }

    #[test]
    fn play_card_is_refused_while_a_card_is_already_in_play() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        let fidel = cards.id_by_name("Fidel").unwrap();
        assert!(matches!(game.play_card(&cards, fidel), Err(GameError::CardInPlay { .. })));
    }

    #[test]
    fn a_scoring_card_can_be_played_but_not_spent_on_an_operation() {
        let map = map();
        let cards = cards();
        let scenario = scenario_with_extra_card(&map, &cards, Ussr, "Europe Scoring");
        let mut game = Game::from_scenario(&scenario);
        let scoring = cards.id_by_name("Europe Scoring").unwrap();
        game.play_card(&cards, scoring).unwrap();
        assert_eq!(game.card_in_play(), Some(scoring));
        assert!(matches!(game.begin(OperationKind::Influence), Err(GameError::ScoringCard)));
        // Still a free undo, same as any other played-but-unspent card.
        assert_eq!(game.return_card().unwrap(), scoring);
    }

    #[test]
    fn a_card_not_in_hand_cannot_be_played() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let not_dealt = cards.id_by_name("Blockade").unwrap();
        assert!(matches!(game.play_card(&cards, not_dealt), Err(GameError::NotInHand)));
    }

    #[test]
    fn return_card_puts_it_back_in_the_hand_at_its_original_index() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let fidel = cards.id_by_name("Fidel").unwrap();
        let before = game.hand(Ussr).to_vec();
        let fidel_index = before.iter().position(|&c| c == fidel).unwrap();

        game.play_card(&cards, fidel).unwrap();
        let returned = game.return_card().unwrap();

        assert_eq!(returned, fidel);
        assert_eq!(game.hand(Ussr), before.as_slice());
        assert_eq!(game.hand(Ussr).iter().position(|&c| c == fidel), Some(fidel_index));
    }

    #[test]
    fn return_card_is_refused_while_an_operation_is_open() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        play(&mut game, &cards, "Socialist Governments");
        game.begin(OperationKind::Influence).unwrap();
        assert!(matches!(game.return_card(), Err(GameError::OperationOpen { .. })));
    }

    #[test]
    fn return_card_is_refused_with_no_card_in_play() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        assert!(matches!(game.return_card(), Err(GameError::NoCard)));
    }

    #[test]
    fn the_china_card_can_be_played_for_its_ops_and_passes_face_down_after_confirm() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        assert_eq!(game.status().china_card, Ussr);
        assert!(game.status().china_card_face_up);

        game.play_card(&cards, CHINA_CARD).unwrap();
        assert_eq!(game.card_in_play(), Some(CHINA_CARD));
        game.begin(OperationKind::Influence).unwrap();
        assert_eq!(game.operation().unwrap().ops_total(), cards.card(CHINA_CARD).ops);
        game.confirm().unwrap();

        assert_eq!(game.status().china_card, Us, "the China Card should pass to the opponent");
        assert!(!game.status().china_card_face_up, "it should land face down");
    }

    #[test]
    fn a_face_down_china_card_cannot_be_played() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        game.play_card(&cards, CHINA_CARD).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // China Card -> US, face down
        assert_eq!(game.active(), Us);
        assert!(matches!(game.play_card(&cards, CHINA_CARD), Err(GameError::ChinaCardFaceDown)));
    }

    #[test]
    fn the_china_card_turns_face_up_again_once_the_turn_rolls_over() {
        let map = map();
        let cards = cards();
        let json = format!(
            r#"{{"status":{{"turn":1,"action_round":2,"action_rounds_per_turn":2,"active":"Us","china_card":"Us","china_card_face_up":false}},{HANDS_JSON}}}"#
        );
        let mut game = Game::from_scenario(&Scenario::from_json(&map, &cards, &json).unwrap());
        assert!(!game.status().china_card_face_up);

        game.pass().unwrap(); // Us -> Ussr, ends the turn
        settle(&mut game, &map, &cards);
        assert_eq!(game.status().turn, 2);
        assert!(game.status().china_card_face_up, "the turn rollover should flip it back up");
    }

    /// A scenario where the USSR controls every Europe battleground plus
    /// the UK — Europe Scoring's Control tier (rule 10.1) — with "Europe
    /// Scoring" in its hand, for the handful of tests below that need
    /// the game to actually end.
    fn europe_control_scenario(map: &WorldMap, cards: &CardCatalog) -> Scenario {
        let json = r#"{
            "hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Europe Scoring","Fidel"]},
            "influence":{
                "France":[0,10],"West Germany":[0,10],"East Germany":[0,10],
                "Poland":[0,10],"Italy":[0,10],"UK":[0,10]
            }
        }"#;
        Scenario::from_json(map, cards, json).unwrap()
    }

    #[test]
    fn a_scoring_card_event_applies_vp_and_discards_the_card() {
        let map = map();
        let cards = cards();
        let json = r#"{
            "hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Middle East Scoring","Fidel"]},
            "influence":{"Jordan":[0,5]}
        }"#;
        let scenario = Scenario::from_json(&map, &cards, json).unwrap();
        let mut game = Game::from_scenario(&scenario);
        let me_scoring = cards.id_by_name("Middle East Scoring").unwrap();

        game.play_card(&cards, me_scoring).unwrap();
        let outcome = game.play_event(&map, &cards).unwrap();
        match outcome {
            EventOutcome::Scoring(result) => assert_eq!(result.vp_delta, -3),
            EventOutcome::Effect(_) | EventOutcome::Pending { .. } => panic!("a scoring card should resolve as a scoring event"),
        }
        assert_eq!(game.status().vp, -3, "presence-only USSR should cost the US side 3 VP");
        assert_eq!(game.card_in_play(), None, "the event should discard the card that funded it");
        assert!(game.discards().contains(&me_scoring));
        assert_eq!(game.active(), Us, "closing the event should hand the turn over, same as confirm/cancel");
    }

    #[test]
    fn southeast_asia_scoring_removes_the_card_from_the_game_instead_of_discarding_it() {
        let map = map();
        let cards = cards();
        let json = r#"{"hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Southeast Asia Scoring","Fidel"]}}"#;
        let scenario = Scenario::from_json(&map, &cards, json).unwrap();
        let mut game = Game::from_scenario(&scenario);
        let se_asia = cards.id_by_name("Southeast Asia Scoring").unwrap();

        game.play_card(&cards, se_asia).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert!(game.removed_from_game().contains(&se_asia));
        assert!(!game.discards().contains(&se_asia));
    }

    #[test]
    fn play_event_refuses_a_card_whose_event_is_not_implemented_yet() {
        let map = map();
        let cards = cards();
        let mut game = Game::from_scenario(&scenario_with_extra_card(&map, &cards, Ussr, "UN Intervention"));
        let sg = play(&mut game, &cards, "UN Intervention");
        assert!(matches!(
            game.play_event(&map, &cards),
            Err(GameError::EventNotImplemented { card }) if card == sg
        ));
        // Refusing to resolve the event shouldn't have consumed it —
        // it's still exactly as playable for ops as before.
        game.begin(OperationKind::Influence).unwrap();
    }

    #[test]
    fn reaching_plus_20_vp_clamps_and_sets_the_us_as_winner() {
        let map = map();
        let cards = cards();
        let json = r#"{
            "status":{"vp":18},
            "hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Southeast Asia Scoring","Fidel"]},
            "influence":{
                "Thailand":[5,0],"Burma":[5,0],"Laos/Cambodia":[5,0],"Vietnam":[5,0],
                "Malaysia":[5,0],"Indonesia":[5,0],"Philippines":[5,0]
            }
        }"#;
        let scenario = Scenario::from_json(&map, &cards, json).unwrap();
        let mut game = Game::from_scenario(&scenario);
        let se_asia = cards.id_by_name("Southeast Asia Scoring").unwrap();

        game.play_card(&cards, se_asia).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().vp, 20, "VP should clamp at +20, not overshoot to 26");
        assert_eq!(game.winner(), Some(Victory { side: Some(Us), reason: VictoryReason::Vp }));
    }

    #[test]
    fn reaching_minus_20_vp_clamps_and_sets_the_ussr_as_winner() {
        let map = map();
        let cards = cards();
        let json = r#"{
            "status":{"vp":-18},
            "hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Southeast Asia Scoring","Fidel"]},
            "influence":{
                "Thailand":[0,5],"Burma":[0,5],"Laos/Cambodia":[0,5],"Vietnam":[0,5],
                "Malaysia":[0,5],"Indonesia":[0,5],"Philippines":[0,5]
            }
        }"#;
        let scenario = Scenario::from_json(&map, &cards, json).unwrap();
        let mut game = Game::from_scenario(&scenario);
        let se_asia = cards.id_by_name("Southeast Asia Scoring").unwrap();

        game.play_card(&cards, se_asia).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().vp, -20, "VP should clamp at -20, not overshoot to -26");
        assert_eq!(game.winner(), Some(Victory { side: Some(Ussr), reason: VictoryReason::Vp }));
    }

    #[test]
    fn europe_scoring_control_wins_the_game_outright() {
        let map = map();
        let cards = cards();
        let scenario = europe_control_scenario(&map, &cards);
        let mut game = Game::from_scenario(&scenario);
        let europe_scoring = cards.id_by_name("Europe Scoring").unwrap();

        game.play_card(&cards, europe_scoring).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.winner(), Some(Victory { side: Some(Ussr), reason: VictoryReason::EuropeControl }));
    }

    #[test]
    fn once_the_game_is_won_further_actions_are_refused_and_the_turn_does_not_advance() {
        let map = map();
        let cards = cards();
        let scenario = europe_control_scenario(&map, &cards);
        let mut game = Game::from_scenario(&scenario);
        let europe_scoring = cards.id_by_name("Europe Scoring").unwrap();
        game.play_card(&cards, europe_scoring).unwrap();
        game.play_event(&map, &cards).unwrap();
        let active_before = game.active();

        assert!(matches!(game.play_card(&cards, cards.id_by_name("Fidel").unwrap()), Err(GameError::GameOver)));
        assert!(matches!(game.pass(), Err(GameError::GameOver)));
        assert_eq!(game.active(), active_before, "the turn should not advance once the game is over");
    }

    #[test]
    fn the_log_order_is_selected_then_scored_then_gameover_when_the_event_wins() {
        let map = map();
        let cards = cards();
        let scenario = europe_control_scenario(&map, &cards);
        let mut game = Game::from_scenario(&scenario);
        let europe_scoring = cards.id_by_name("Europe Scoring").unwrap();
        game.play_card(&cards, europe_scoring).unwrap();
        game.play_event(&map, &cards).unwrap();

        let events: Vec<&Event> = game.log().entries().iter().map(|e| &e.event).collect();
        assert_eq!(events.len(), 3, "expected exactly Selected, Scored, GameOver: {events:?}");
        assert!(matches!(events[0], Event::Selected { .. }));
        assert!(matches!(events[1], Event::Scored { .. }));
        assert!(matches!(events[2], Event::GameOver(_)));
    }

    #[test]
    fn lookahead_carries_the_winner() {
        let map = map();
        let cards = cards();
        let scenario = europe_control_scenario(&map, &cards);
        let mut game = Game::from_scenario(&scenario);
        let europe_scoring = cards.id_by_name("Europe Scoring").unwrap();
        game.play_card(&cards, europe_scoring).unwrap();
        game.play_event(&map, &cards).unwrap();

        let ahead = game.lookahead();
        assert_eq!(ahead.winner(), game.winner());
    }
}
