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
}

impl Game {
    /// Starts from a scenario's status and board, with no operation open.
    pub fn from_scenario(scenario: &Scenario) -> Self {
        Game { status: scenario.status, board: scenario.board.clone(), op: None }
    }

    pub fn status(&self) -> &GameStatus {
        &self.status
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
        match &mut self.op {
            Some(Operation::Realign(r)) => Ok(RollOutcome::Realign(r.roll(map, &mut self.board, id, dice)?)),
            Some(Operation::Coup(c)) => Ok(RollOutcome::Coup(c.attempt(map, &mut self.board, id, dice)?)),
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
        self.advance();
        Ok(op)
    }

    /// Closes the open operation without committing a placement's pending
    /// points (a realignment's or coup's rolls, already on the board,
    /// stay there — there's nothing to discard), and hands the turn to
    /// the other side.
    pub fn cancel(&mut self) -> Result<Operation, GameError> {
        let op = self.op.take().ok_or(GameError::NoOperation)?;
        self.advance();
        Ok(op)
    }

    /// Forfeits the active side's turn without opening an operation.
    /// Refused if one is already open — cancel it first.
    pub fn pass(&mut self) -> Result<(), GameError> {
        if let Some(op) = &self.op {
            return Err(GameError::OperationOpen { verb: op.verb(), remaining: op.remaining(), total: op.ops_total() });
        }
        self.advance();
        Ok(())
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
}
