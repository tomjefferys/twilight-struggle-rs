//! Turn structure: alternating US/USSR action rounds, enforced. `Game`
//! owns the three things that have to move in lockstep — the status
//! (turn/action round/active side), the board, and whichever [`Operation`]
//! is open — so "an operation belongs to the active side, and closing it
//! passes the turn" lives in exactly one place, rather than being
//! duplicated between the REPL and interactive mode.
//!
//! This module is deliberately the only place that mutates
//! [`GameStatus::active`], `turn`, or `action_round` — see [`Game::advance`].
//! It is also, by design, the whole surface a future AI opponent needs:
//! nothing here touches a terminal, and `Game` is cheap to clone (per
//! [`Board`]'s own design note) so a lookahead search can try a line of
//! play on a copy without touching the real game.
//!
//! Rule 6.x's one-card-one-action shape means a turn spends exactly one
//! operation: [`Game::begin`] opens it with the active side's 4 ops, and
//! [`Game::confirm`]/[`Game::cancel`] — the only two ways to close it —
//! both hand the turn to the other side. Ending a turn with ops unspent is
//! allowed (a confirm needn't spend everything, matching the ops modules'
//! own behaviour) and simply forfeits them; [`Game::pass`] is the same
//! forfeiture with no operation opened at all.

use std::fmt;

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::dice::Dice;
use crate::log::{Event, GameLog, LogEntry};
use crate::map::WorldMap;
use crate::ops::{
    CoupError, InfluencePlacement, Operation, PlacementError, RealignError, Realignment, RollResult,
};
use crate::ops::{Coup, CoupResult};
use crate::scenario::Scenario;
use crate::status::GameStatus;

/// Operation points the active side gets each turn. There is no per-turn
/// pool separate from this — an operation's own `ops_total` *is* the
/// turn's budget, since a turn spends exactly one operation.
pub const OPS_PER_ACTION_ROUND: u8 = 4;

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

/// The live game: status (including whose turn it is), the committed
/// board, and whichever operation the active side currently has open.
///
/// `Clone` is cheap, per [`Board`]'s own design note, and exists for the
/// same reason: AI lookahead over a copy, without touching the real game.
#[derive(Clone)]
pub struct Game {
    status: GameStatus,
    board: Board,
    op: Option<Operation>,
    log: GameLog,
}

impl Game {
    /// Starts from a scenario's status and board, with no operation open
    /// and an empty history.
    pub fn from_scenario(scenario: &Scenario) -> Self {
        Game { status: scenario.status, board: scenario.board.clone(), op: None, log: GameLog::new() }
    }

    pub fn status(&self) -> &GameStatus {
        &self.status
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

    /// A copy for AI lookahead: same status, board, and open operation,
    /// but an empty history. `Game` is cheap to clone by design (see the
    /// module doc), and the log is the one field whose size isn't
    /// bounded — a search that clones many nodes should use this instead
    /// of [`Clone::clone`], so lookahead doesn't drag a growing `Vec`
    /// through every branch it never plays out.
    pub fn lookahead(&self) -> Game {
        Game { status: self.status, board: self.board.clone(), op: self.op.clone(), log: GameLog::new() }
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
    /// disagree mid-action), otherwise the full per-turn allowance.
    pub fn ops_available(&self) -> u8 {
        self.op.as_ref().map_or(OPS_PER_ACTION_ROUND, Operation::remaining)
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

    /// Opens a new operation for the active side with a full turn's ops.
    /// This is the entire enforcement mechanism: there is no argument
    /// through which a caller could name the wrong side or the wrong ops
    /// count. Refused if an operation is already open.
    pub fn begin(&mut self, kind: OperationKind) -> Result<(), GameError> {
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        let side = self.status.active;
        self.op = Some(match kind {
            OperationKind::Influence => Operation::Influence(InfluencePlacement::new(side, OPS_PER_ACTION_ROUND, &self.board)),
            OperationKind::Realign => Operation::Realign(Realignment::new(side, OPS_PER_ACTION_ROUND, &self.board)),
            OperationKind::Coup => Operation::Coup(Coup::new(side, OPS_PER_ACTION_ROUND, &self.board)),
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
    pub fn roll(&mut self, map: &WorldMap, id: CountryId, dice: &mut Dice) -> Result<RollOutcome, GameError> {
        let side = self.status.active;
        let outcome = match &mut self.op {
            Some(Operation::Realign(r)) => RollOutcome::Realign(r.roll(map, &mut self.board, id, dice)?),
            Some(Operation::Coup(c)) => RollOutcome::Coup(c.attempt(map, &mut self.board, id, dice)?),
            Some(op) => return Err(GameError::WrongKind { open: op.verb() }),
            None => return Err(GameError::NoOperation),
        };
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
    /// Refused if one is already open — cancel it first.
    pub fn pass(&mut self) -> Result<(), GameError> {
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
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

    /// Pushes the log entry (entries, for a placement) for a closing
    /// operation — called from `confirm`/`cancel` *before* [`Game::advance`],
    /// so they carry the turn/AR the operation actually happened in, not
    /// the next one.
    fn log_close(&mut self, op: &Operation, committed: bool) {
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

    /// Hands the turn to the other side: USSR to USA, or USA to USSR —
    /// which also completes an action round, so it increments
    /// `action_round`, rolling `turn` over and resetting `action_round`
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
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Superpower::{Us, Ussr};
    use crate::map::WorldMap;

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country named {name:?}"))
    }

    /// A scenario with one country pre-seeded, so realignment/coup targets
    /// (which need opponent presence) have something to aim at.
    fn scenario_with(map: &WorldMap, country: &str, us: u8, ussr: u8) -> Scenario {
        let json = format!(r#"{{"influence":{{"{country}":[{us},{ussr}]}}}}"#);
        Scenario::from_json(map, &json).unwrap()
    }

    #[test]
    fn ussr_acts_first_and_confirm_hands_over_to_us() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        assert_eq!(game.active(), Ussr);

        game.begin(OperationKind::Influence).unwrap();
        assert_eq!(game.operation().unwrap().side(), Ussr);
        game.confirm().unwrap();

        assert_eq!(game.active(), Us);
    }

    #[test]
    fn action_round_only_increments_after_us_acts() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        let start_ar = game.status().action_round;

        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // USSR -> US
        assert_eq!(game.status().action_round, start_ar);

        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // US -> USSR
        assert_eq!(game.status().action_round, start_ar + 1);
    }

    #[test]
    fn action_round_rolls_the_turn_over_when_it_passes_the_limit() {
        let map = map();
        let json = r#"{"status":{"turn":1,"action_round":2,"action_rounds_per_turn":2}}"#;
        let mut game = Game::from_scenario(&Scenario::from_json(&map, json).unwrap());
        assert_eq!(game.status().active, Ussr);

        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // USSR -> US, still AR 2
        assert_eq!(game.status().turn, 1);
        assert_eq!(game.status().action_round, 2);

        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // US -> USSR: AR 2 was the last of the turn
        assert_eq!(game.status().turn, 2);
        assert_eq!(game.status().action_round, 1);
    }

    #[test]
    fn begin_is_refused_while_an_operation_is_open() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        assert!(matches!(game.begin(OperationKind::Coup), Err(GameError::OperationOpen { .. })));
    }

    #[test]
    fn every_operation_opens_with_the_active_side_and_a_full_turns_ops() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Realign).unwrap();
        let op = game.operation().unwrap();
        assert_eq!(op.side(), Ussr);
        assert_eq!(op.ops_total(), OPS_PER_ACTION_ROUND);
        assert_eq!(game.ops_available(), OPS_PER_ACTION_ROUND);
    }

    #[test]
    fn cancel_advances_the_turn_just_like_confirm() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.cancel().unwrap();
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn pass_advances_with_no_operation_open() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.pass().unwrap();
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn pass_is_refused_while_an_operation_is_open() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Coup).unwrap();
        assert!(matches!(game.pass(), Err(GameError::OperationOpen { .. })));
    }

    #[test]
    fn abandon_is_refused_with_no_operation_open() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        assert!(matches!(game.abandon(), Err(GameError::NoOperation)));
    }

    #[test]
    fn an_untouched_operation_can_be_abandoned_without_costing_the_turn() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.abandon().unwrap();
        assert_eq!(game.active(), Ussr, "abandoning before anything's spent shouldn't hand the turn over");
        assert!(game.operation().is_none());
    }

    #[test]
    fn a_placement_can_always_be_abandoned_even_with_points_pending() {
        // Placement never rolls a die, so nothing about it is hidden or
        // irreversible until `confirm` runs — unlike realignment/coup,
        // `abandon` never refuses it.
        let map = map();
        let poland = id(&map, "Poland"); // borders the USSR itself
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.place(&map, poland).unwrap();
        game.place(&map, poland).unwrap();

        game.abandon().unwrap();

        assert_eq!(game.active(), Ussr, "abandoning a placement shouldn't hand the turn over");
        assert!(game.operation().is_none());
        assert_eq!(game.board().influence(poland, Ussr), 0, "every pending point should be discarded, not committed");
        assert_eq!(game.ops_available(), OPS_PER_ACTION_ROUND, "every op it cost should be refunded");
    }

    #[test]
    fn undoing_every_placed_point_makes_the_operation_abandonable_too() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.undo(&map).unwrap();
        game.abandon().unwrap();
        assert_eq!(game.active(), Ussr);
    }

    #[test]
    fn abandon_is_refused_once_a_realignment_roll_has_been_made() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
        game.begin(OperationKind::Realign).unwrap();
        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandon { ops_spent: 1, .. })));
    }

    #[test]
    fn abandon_is_refused_once_a_coup_has_been_attempted() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
        game.begin(OperationKind::Coup).unwrap();
        let mut dice = Dice::from_seed(0);
        game.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandon { ops_total: 4, .. })));
    }

    #[test]
    fn abandoning_leaves_no_trace_in_the_log() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Coup).unwrap();
        game.abandon().unwrap();
        assert!(game.log().is_empty(), "an abandoned operation should leave the log exactly as it was");
    }

    #[test]
    fn abandoning_a_placement_with_pending_points_leaves_no_trace_in_the_log_either() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.abandon().unwrap();
        assert!(game.log().is_empty(), "as far as the history's concerned, an abandoned placement never happened either");
    }

    #[test]
    fn a_confirmed_placement_lands_on_the_board_but_a_cancelled_one_does_not() {
        let map = map();
        let poland = id(&map, "Poland");

        let mut confirmed = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        confirmed.begin(OperationKind::Influence).unwrap();
        confirmed.place(&map, poland).unwrap();
        confirmed.confirm().unwrap();
        assert_eq!(confirmed.board().influence(poland, Ussr), 1);

        let mut cancelled = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        cancelled.begin(OperationKind::Influence).unwrap();
        cancelled.place(&map, poland).unwrap();
        cancelled.cancel().unwrap();
        assert_eq!(cancelled.board().influence(poland, Ussr), 0);
    }

    #[test]
    fn place_is_refused_when_a_different_kind_of_operation_is_open() {
        let map = map();
        let mut game = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
        game.begin(OperationKind::Realign).unwrap();
        assert!(matches!(game.place(&map, id(&map, "Poland")), Err(GameError::WrongKind { .. })));
    }

    #[test]
    fn undo_is_refused_for_a_resolved_realignment_or_coup() {
        let map = map();
        let poland = id(&map, "Poland");

        let mut realigning = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
        realigning.begin(OperationKind::Realign).unwrap();
        let mut dice = Dice::from_seed(0);
        realigning.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(realigning.undo(&map), Err(GameError::CannotUndo { .. })));

        let mut couping = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
        couping.begin(OperationKind::Coup).unwrap();
        couping.roll(&map, poland, &mut dice).unwrap();
        assert!(matches!(couping.undo(&map), Err(GameError::CannotUndo { .. })));
    }

    #[test]
    fn roll_does_not_advance_the_turn_only_confirm_and_cancel_do() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
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
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        let poland = id(&map, "Poland");

        game.begin(OperationKind::Influence).unwrap(); // USSR
        game.place(&map, poland).unwrap();
        game.place(&map, poland).unwrap();
        game.confirm().unwrap(); // -> US

        let mut dice = Dice::from_seed(0);
        game.begin(OperationKind::Realign).unwrap(); // US
        game.roll(&map, poland, &mut dice).unwrap();
        game.cancel().unwrap(); // -> USSR

        game.pass().unwrap(); // USSR -> US

        let events: Vec<_> = game.log().entries().iter().map(|e| &e.event).collect();
        assert!(matches!(events[0], Event::Placed { .. }));
        assert!(matches!(events[1], Event::Closed { kind: OperationKind::Influence, committed: true, .. }));
        assert!(matches!(events[2], Event::Realign(_)));
        assert!(matches!(events[3], Event::Closed { kind: OperationKind::Realign, committed: false, .. }));
        assert!(matches!(events[4], Event::Pass));
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn a_confirm_entry_carries_the_turn_and_side_it_actually_happened_in() {
        let map = map();
        let json = r#"{"status":{"turn":1,"action_round":2,"action_rounds_per_turn":2}}"#;
        let mut game = Game::from_scenario(&Scenario::from_json(&map, json).unwrap());

        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap(); // USSR -> US, rolls the turn over on the *next* US confirm

        let entry = &game.log().entries()[0];
        assert_eq!(entry.turn, 1);
        assert_eq!(entry.action_round, 2);
        assert_eq!(entry.side, Some(Ussr));
        // advance() has already run, so status has moved on from what the entry recorded.
        assert_eq!(game.active(), Us);
    }

    #[test]
    fn a_cancelled_placement_gets_its_own_placed_line_and_a_cancel_line() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, poland).unwrap();
        game.cancel().unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 2);
        match &entries[0].event {
            Event::Placed { countries } => assert_eq!(countries, &vec![(poland, 1)]),
            other => panic!("expected a Placed event, got {other:?}"),
        }
        assert!(matches!(
            entries[1].event,
            Event::Closed { kind: OperationKind::Influence, committed: false, .. }
        ));
        assert_eq!(game.board().influence(poland, Ussr), 0);
    }

    #[test]
    fn an_immediately_cancelled_placement_with_nothing_placed_has_no_placed_line() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.begin(OperationKind::Influence).unwrap();
        game.cancel().unwrap();

        let entries = game.log().entries();
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].event,
            Event::Closed { kind: OperationKind::Influence, committed: false, .. }
        ));
    }

    #[test]
    fn a_realignment_roll_is_logged_before_the_operation_closes() {
        let map = map();
        let poland = id(&map, "Poland");
        let mut game = Game::from_scenario(&scenario_with(&map, "Poland", 1, 0));
        let mut dice = Dice::from_seed(0);
        game.begin(OperationKind::Realign).unwrap();
        game.roll(&map, poland, &mut dice).unwrap();

        assert_eq!(game.log().len(), 1);
        assert!(matches!(game.log().entries()[0].event, Event::Realign(_)));

        game.confirm().unwrap();
        assert_eq!(game.log().len(), 2);
    }

    #[test]
    fn lookahead_clones_state_but_starts_with_an_empty_log() {
        let map = map();
        let mut game = Game::from_scenario(&Scenario::from_json(&map, "{}").unwrap());
        game.pass().unwrap();
        assert_eq!(game.log().len(), 1);

        let ahead = game.lookahead();
        assert!(ahead.log().is_empty());
        assert_eq!(ahead.status(), game.status());
        assert_eq!(ahead.active(), game.active());
    }
}
