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
use crate::cards::{CardCatalog, CardId, Hands, CHINA_CARD};
use crate::country::{CountryId, Superpower};
use crate::dice::Dice;
use crate::events::{self, EventOutcome};
use crate::log::{Event, GameLog, LogEntry};
use crate::map::WorldMap;
use crate::ops::{
    CoupError, InfluencePlacement, Operation, PlacementError, RealignError, Realignment, RollResult,
};
use crate::ops::{Coup, CoupResult};
use crate::scenario::Scenario;
use crate::status::GameStatus;

/// Why the game ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictoryReason {
    /// The VP track hit +20 (US) or -20 (USSR).
    Vp,
    /// Europe Scoring's Control tier (rule 10.1) — the one region card
    /// whose Control is an outright win rather than a VP value.
    EuropeControl,
}

/// The game is over: who won, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Victory {
    pub side: Superpower,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollOutcome {
    Realign(RollResult),
    Coup(CoupResult),
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
    Placement(PlacementError),
    Realign(RealignError),
    Coup(CoupError),
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
            GameError::Placement(e) => write!(f, "{e}"),
            GameError::Realign(e) => write!(f, "{e}"),
            GameError::Coup(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for GameError {}

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
}

impl Game {
    /// Starts from a scenario's status, board, and hands, with no card
    /// played, no operation open, no winner, and an empty history.
    pub fn from_scenario(scenario: &Scenario) -> Self {
        Game {
            status: scenario.status,
            board: scenario.board.clone(),
            card: None,
            op: None,
            log: GameLog::new(),
            hands: scenario.hands.clone(),
            winner: None,
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
        }
    }

    /// `side`'s held cards, in hand order. Doesn't include the China Card
    /// ([`crate::cards::CHINA_CARD`]) — see [`Hands`]'s own doc — nor
    /// whatever's currently [`Game::card_in_play`], which
    /// [`Game::play_card`] has already removed.
    pub fn hand(&self, side: Superpower) -> &[CardId] {
        self.hands.hand(side)
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
        self.op.as_ref().map(Operation::remaining).or(self.card.map(|c| c.ops)).unwrap_or(0)
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

    pub fn operation(&self) -> Option<&Operation> {
        self.op.as_ref()
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
        self.card = Some(PlayedCard { id, ops: card.ops, hand_index, logged: false, scoring: card.scoring });
        Ok(())
    }

    /// Puts the card currently in play back in the active side's hand at
    /// the index it was taken from (a no-op position for the China Card,
    /// which was never removed from one) — the free undo for a mistaken
    /// [`Game::play_card`]. Refused while an operation is open (abandon it
    /// first via [`Game::abandon`]) or with no card in play.
    pub fn return_card(&mut self) -> Result<CardId, GameError> {
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
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
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let card = self.card.ok_or(GameError::NoCard)?;
        if card.scoring {
            return Err(GameError::ScoringCard);
        }
        let ops = card.ops;
        let side = self.status.active;
        self.op = Some(match kind {
            OperationKind::Influence => Operation::Influence(InfluencePlacement::new(side, ops, &self.board)),
            OperationKind::Realign => Operation::Realign(Realignment::new(side, ops, &self.board)),
            OperationKind::Coup => Operation::Coup(Coup::new(side, ops, &self.board)),
        });
        Ok(())
    }

    /// Places one point of influence in `id`, if an [`InfluencePlacement`]
    /// is open. Errors if a different kind of operation is open, none is,
    /// or the placement itself refuses (see [`InfluencePlacement::place`]).
    pub fn place(&mut self, map: &WorldMap, id: CountryId) -> Result<u8, GameError> {
        match &mut self.op {
            Some(Operation::Influence(p)) => Ok(p.place(map, id)?),
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
        let side = self.status.active;
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
        };
        self.log.push(LogEntry { turn: self.status.turn, action_round: self.status.action_round, side: Some(side), event });
        Ok(outcome)
    }

    /// Takes back the single most recently placed influence point,
    /// refunding its ops — only meaningful for a placement. Refused (with
    /// no state changed) for a realignment or coup, whose rolls already
    /// resolved, and when no operation is open.
    pub fn undo(&mut self, map: &WorldMap) -> Result<CountryId, GameError> {
        match &mut self.op {
            Some(Operation::Influence(p)) => p.undo_last(map).ok_or(GameError::NothingToUndo),
            Some(Operation::Realign(_)) => Err(GameError::CannotUndo { verb: "realignment roll" }),
            Some(Operation::Coup(_)) => Err(GameError::CannotUndo { verb: "coup" }),
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
        let op = self.op.take().ok_or(GameError::NoOperation)?;
        if let Operation::Influence(p) = &op {
            self.board = p.board().clone();
        }
        self.log_close(&op, true);
        self.discard_played_card();
        self.advance();
        Ok(op)
    }

    /// Closes the open operation without committing a placement's pending
    /// points (a realignment's or coup's rolls, already on the board,
    /// stay there — there's nothing to discard), and hands the turn to
    /// the other side.
    pub fn cancel(&mut self) -> Result<Operation, GameError> {
        let op = self.op.take().ok_or(GameError::NoOperation)?;
        self.log_close(&op, false);
        self.discard_played_card();
        self.advance();
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
        match &self.op {
            None => Err(GameError::NoOperation),
            // Placement never rolls dice, so there's nothing it could
            // reveal — always safe to discard, no matter how many points
            // are pending.
            Some(Operation::Influence(_)) => Ok(self.op.take().expect("checked Some above")),
            Some(op) => {
                let ops_spent = op.ops_spent();
                if ops_spent > 0 {
                    return Err(GameError::CannotAbandon { verb: op.verb(), ops_spent, ops_total: op.ops_total() });
                }
                Ok(self.op.take().expect("checked Some above"))
            }
        }
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
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        if let Some(card) = self.card {
            return Err(GameError::CardInPlay { card: card.id });
        }
        self.log.push(LogEntry {
            turn: self.status.turn,
            action_round: self.status.action_round,
            side: Some(self.status.active),
            event: Event::Pass,
        });
        self.advance();
        Ok(())
    }

    /// Resolves the event text of whichever card is currently in play —
    /// the other thing a played card can fund, alongside [`Game::begin`].
    /// Only a scoring card's event is implemented so far
    /// ([`crate::events::is_implemented`]); refused otherwise with
    /// [`GameError::EventNotImplemented`]. Also refused with no card in
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
        if self.winner.is_some() {
            return Err(GameError::GameOver);
        }
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let card = self.card.ok_or(GameError::NoCard)?;
        if !events::is_implemented(card.id) {
            return Err(GameError::EventNotImplemented { card: card.id });
        }
        let outcome = events::resolve(map, &self.board, card.id).expect("is_implemented checked above");
        self.log_card_selected();

        match &outcome {
            EventOutcome::Scoring(result) => {
                self.apply_vp(result.vp_delta);
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
                if let Some(victory) = self.winner {
                    self.log.push(LogEntry {
                        turn: self.status.turn,
                        action_round: self.status.action_round,
                        side: Some(self.status.active),
                        event: Event::GameOver(victory),
                    });
                }
            }
        }

        self.discard_or_remove_event_card(cards);
        if self.winner.is_none() {
            self.advance();
        }
        Ok(outcome)
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
            self.winner = Some(Victory { side, reason });
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
        if card.id == CHINA_CARD {
            self.status.china_card = self.status.active.opponent();
            self.status.china_card_face_up = false;
        } else {
            self.hands.discard(card.id);
        }
    }

    /// [`Game::discard_played_card`]'s counterpart for
    /// [`Game::play_event`]: sends the card to the removed-from-play pile
    /// instead of the discard pile if its *event* is
    /// `removed_after_event` (rule 4.4) — never the China Card, since
    /// none of the cards [`events::is_implemented`] recognises is it.
    fn discard_or_remove_event_card(&mut self, cards: &CardCatalog) {
        let Some(card) = self.card.take() else { return };
        if cards.card(card.id).removed_after_event {
            self.hands.remove_from_game(card.id);
        } else {
            self.hands.discard(card.id);
        }
    }

    /// Hands the turn to the other side: USSR to USA, or USA to USSR —
    /// which also completes an action round, so it increments
    /// `action_round`, rolling `turn` over (and flipping the China Card
    /// face up again, wherever it's landed) and resetting `action_round`
    /// to 1 once it passes `action_rounds_per_turn`. The only writer of
    /// `active`/`turn`/`action_round`, called from `confirm`, `cancel`,
    /// and `pass` — nowhere else.
    fn advance(&mut self) {
        match self.status.active {
            Superpower::Ussr => self.status.active = Superpower::Us,
            Superpower::Us => {
                self.status.active = Superpower::Ussr;
                self.status.action_round += 1;
                if self.status.action_round > self.status.action_rounds_per_turn {
                    self.status.action_round = 1;
                    self.status.turn += 1;
                    self.status.china_card_face_up = true;
                }
            }
        }
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
        assert_eq!(game.status().turn, 2);
        assert_eq!(game.status().action_round, 1);
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
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, &cards, "Poland", 1, 0));
        let fidel = play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        assert!(game.log().is_empty(), "playing a card and opening an operation shouldn't log anything yet");

        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 2, "the roll's first action should log the card, then the roll itself");
        assert!(matches!(entries[0].event, Event::Selected { card } if card == fidel));
        assert!(matches!(entries[1].event, Event::Coup(_)));

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

        game.pass().unwrap(); // Us -> Ussr, rolls the turn over
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
        let mut game = Game::from_scenario(&scenario(&map, &cards));
        let sg = play(&mut game, &cards, "Socialist Governments");
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
        assert_eq!(game.winner(), Some(Victory { side: Us, reason: VictoryReason::Vp }));
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
        assert_eq!(game.winner(), Some(Victory { side: Ussr, reason: VictoryReason::Vp }));
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
        assert_eq!(game.winner(), Some(Victory { side: Ussr, reason: VictoryReason::EuropeControl }));
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
