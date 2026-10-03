//! The influence-placement operation: spending operation points to add
//! influence to countries.
//!
//! An [`InfluencePlacement`] stages one or more placements against a
//! cloned copy of the board — cheap, per [`Board`]'s own design note.
//! Cost and presence are deliberately evaluated against two *different*
//! snapshots, matching the real rule: presence/adjacency is fixed as of
//! the start of the action (a `base` clone taken once, in
//! [`InfluencePlacement::new`]), so placing in Poland during this action
//! does *not* make East Germany a legal target until a later action —
//! only influence that was already there when the action began counts.
//! Cost, on the other hand, tracks the *running* state (`board`, updated
//! after every placement), because control genuinely can flip mid-action:
//! the third point spent breaking an opponent's control of a country
//! costs less than the first two. Nothing reaches the real board until
//! [`InfluencePlacement::commit`].

use std::collections::HashMap;
use std::fmt;

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::map::WorldMap;

/// One step in the placement history, so undoing it can refund the exact
/// number of ops it cost — not always 1, since a point placed before
/// control flipped may have cost 2.
#[derive(Clone)]
struct Step {
    id: CountryId,
    cost: u8,
}

/// A staged influence placement: operation points a player has committed
/// to spending, and where, but which haven't yet been written to the real
/// game state. The player can keep adding to it, undo their most recent
/// point, and either [`commit`](InfluencePlacement::commit) it to a
/// [`Board`] or drop it entirely by discarding this value.
///
/// `Clone` (like [`Board`]'s own) is cheap and exists for the same
/// reason: a [`crate::game::Game`] needs to be clonable for AI lookahead.
#[derive(Clone)]
pub struct InfluencePlacement {
    side: Superpower,
    ops_total: u8,
    ops_spent: u8,
    /// The board as it stood when this action started — never mutated.
    /// Presence/adjacency is checked against this, not `board`, so a
    /// point placed earlier in this same action can't unlock a new
    /// country that had no influence in or next to it beforehand.
    base: Board,
    /// The board with every pending placement already applied — used for
    /// cost (which does track the running state; control can flip
    /// mid-action) and for what a view should draw.
    board: Board,
    history: Vec<Step>,
    pending: HashMap<CountryId, u8>,
}

/// Why a placement was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementError {
    /// The placing side has no influence in `country` or any of its
    /// neighbours, and `country` doesn't border the placing side's own
    /// superpower.
    NoPresence { country: String, side: Superpower },
    /// Placing here would cost more than the ops remaining.
    InsufficientOps { country: String, needed: u8, remaining: u8 },
}

impl fmt::Display for PlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlacementError::NoPresence { country, side } => write!(
                f,
                "{side} has no presence in or adjacent to {country}, and it doesn't border {side}"
            ),
            PlacementError::InsufficientOps { country, needed, remaining } => write!(
                f,
                "placing in {country} costs {needed} ops, but only {remaining} remain"
            ),
        }
    }
}

impl std::error::Error for PlacementError {}

impl InfluencePlacement {
    /// Starts a new placement session for `side` with `ops` operation
    /// points to spend, against the current state of `board`. `board` is
    /// snapshotted twice: once as the fixed `base` that presence is
    /// always checked against, and once as the running board that this
    /// action's placements accumulate onto.
    pub fn new(side: Superpower, ops: u8, board: &Board) -> Self {
        InfluencePlacement {
            side,
            ops_total: ops,
            ops_spent: 0,
            base: board.clone(),
            board: board.clone(),
            history: Vec::new(),
            pending: HashMap::new(),
        }
    }

    pub fn side(&self) -> Superpower {
        self.side
    }

    pub fn ops_total(&self) -> u8 {
        self.ops_total
    }

    pub fn ops_spent(&self) -> u8 {
        self.ops_spent
    }

    pub fn remaining(&self) -> u8 {
        self.ops_total - self.ops_spent
    }

    /// Whether any point has been placed yet.
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }

    /// How much pending influence has been added to `id` so far.
    pub fn pending(&self, id: CountryId) -> u8 {
        self.pending.get(&id).copied().unwrap_or(0)
    }

    /// Every country with pending influence, in the order it was first
    /// placed in.
    pub fn pending_countries(&self) -> Vec<(CountryId, u8)> {
        let mut seen = Vec::new();
        for step in &self.history {
            if !seen.contains(&step.id) {
                seen.push(step.id);
            }
        }
        seen.into_iter().map(|id| (id, self.pending(id))).collect()
    }

    /// The speculative board with every pending placement already applied
    /// — what a view should draw while a placement is in progress.
    pub fn board(&self) -> &Board {
        &self.board
    }

    /// The cost of one more point of influence in `id`, against the
    /// pending state: 1 op if `id` is uncontrolled or already controlled
    /// by the placing side, 2 ops if the opponent controls it.
    pub fn cost(&self, map: &WorldMap, id: CountryId) -> u8 {
        if self.board.is_controlled_by(map, id, self.side.opponent()) {
            2
        } else {
            1
        }
    }

    /// Whether `id` is a legal target: the placing side already had
    /// influence there or in an adjacent country *when this action
    /// started*, or `id` borders the placing side's own superpower.
    /// Presence means *any* influence — control is not required.
    ///
    /// Deliberately checked against `base`, not the running `board`: a
    /// point placed earlier in this same action does not, by itself,
    /// open up a country that had none of the placing side's influence
    /// in or next to it before the action began.
    pub fn is_legal_target(&self, map: &WorldMap, id: CountryId) -> bool {
        let country = map.country(id);
        if self.base.influence(id, self.side) > 0 {
            return true;
        }
        if country.borders_superpower(self.side) {
            return true;
        }
        country.adjacent.iter().any(|&n| self.base.influence(n, self.side) > 0)
    }

    /// Adds one point of influence to `id`, charging the ops it costs.
    /// Refused if `id` isn't a legal target, or if the cost would exceed
    /// the ops remaining — neither changes any state.
    pub fn place(&mut self, map: &WorldMap, id: CountryId) -> Result<u8, PlacementError> {
        if !self.is_legal_target(map, id) {
            return Err(PlacementError::NoPresence {
                country: map.country(id).name.clone(),
                side: self.side,
            });
        }
        let cost = self.cost(map, id);
        if cost > self.remaining() {
            return Err(PlacementError::InsufficientOps {
                country: map.country(id).name.clone(),
                needed: cost,
                remaining: self.remaining(),
            });
        }

        self.board.add_influence(id, self.side, 1);
        self.ops_spent += cost;
        *self.pending.entry(id).or_insert(0) += 1;
        self.history.push(Step { id, cost });
        Ok(cost)
    }

    /// Takes back the single most recently placed point, refunding
    /// exactly the ops it cost. Returns the country it was removed from,
    /// or `None` if nothing has been placed yet.
    pub fn undo_last(&mut self, _map: &WorldMap) -> Option<CountryId> {
        let step = self.history.pop()?;
        self.board.remove_influence(step.id, self.side, 1);
        self.ops_spent -= step.cost;
        match self.pending.get_mut(&step.id) {
            Some(1) => {
                self.pending.remove(&step.id);
            }
            Some(n) => *n -= 1,
            None => {}
        }
        Some(step.id)
    }

    /// Takes back the most recent point placed in `id` specifically (not
    /// the most recent overall), refunding exactly what it cost — the `-`
    /// key. Returns `id`, or `None` if nothing is pending there.
    pub fn unplace(&mut self, id: CountryId) -> Option<CountryId> {
        let pos = self.history.iter().rposition(|s| s.id == id)?;
        let step = self.history.remove(pos);
        self.board.remove_influence(id, self.side, 1);
        self.ops_spent -= step.cost;
        match self.pending.get_mut(&id) {
            Some(1) => {
                self.pending.remove(&id);
            }
            Some(n) => *n -= 1,
            None => {}
        }
        Some(id)
    }

    /// Consumes this placement, handing back the board with every pending
    /// placement now permanent.
    pub fn commit(self) -> Board {
        self.board
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Superpower::{Us, Ussr};

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country named {name:?}"))
    }

    #[test]
    fn cost_is_one_in_an_empty_or_own_controlled_country() {
        let map = map();
        let board = Board::new(&map);
        let placement = InfluencePlacement::new(Ussr, 10, &board);
        assert_eq!(placement.cost(&map, id(&map, "Poland")), 1);
    }

    #[test]
    fn cost_is_two_in_an_opponent_controlled_country() {
        let map = map();
        let mut board = Board::new(&map);
        let france = id(&map, "France");
        // France's stability is high enough that a handful of US
        // influence with none opposing puts the US in clear control.
        board.set_influence(france, Superpower::Us, 10);
        let placement = InfluencePlacement::new(Ussr, 10, &board);
        assert_eq!(placement.cost(&map, france), 2);
    }

    #[test]
    fn bordering_own_superpower_is_legal_on_an_empty_board() {
        let map = map();
        let board = Board::new(&map);
        let placement = InfluencePlacement::new(Ussr, 10, &board);
        let poland = id(&map, "Poland");
        assert!(map.country(poland).borders_superpower(Ussr));
        assert!(placement.is_legal_target(&map, poland));
    }

    #[test]
    fn a_country_with_no_presence_or_border_is_illegal() {
        let map = map();
        let board = Board::new(&map);
        let placement = InfluencePlacement::new(Ussr, 10, &board);
        let chile = id(&map, "Chile");
        assert!(!map.country(chile).borders_superpower(Ussr));
        assert!(!placement.is_legal_target(&map, chile));
    }

    #[test]
    fn influence_in_a_neighbour_grants_presence() {
        let map = map();
        let mut board = Board::new(&map);
        let poland = id(&map, "Poland");
        board.set_influence(poland, Ussr, 1);
        let placement = InfluencePlacement::new(Ussr, 10, &board);
        let east_germany = id(&map, "East Germany");
        assert!(map.country(poland).adjacent.contains(&east_germany) || map.country(east_germany).adjacent.contains(&poland));
        assert!(placement.is_legal_target(&map, east_germany));
    }

    #[test]
    fn placing_in_a_country_does_not_unlock_a_new_neighbour_this_action() {
        // The real rule: presence/adjacency is fixed as of the start of
        // the action. Placing in Poland (legal only via the USSR border,
        // since the board starts empty) must not, by itself, make East
        // Germany — which had no USSR influence in or next to it before
        // this action began, and doesn't border the USSR itself — a
        // legal target within the same action.
        let map = map();
        let board = Board::new(&map);
        let mut placement = InfluencePlacement::new(Ussr, 10, &board);
        let poland = id(&map, "Poland");
        let east_germany = id(&map, "East Germany");
        assert!(!map.country(east_germany).borders_superpower(Ussr));

        assert!(!placement.is_legal_target(&map, east_germany));
        placement.place(&map, poland).unwrap();
        assert!(
            !placement.is_legal_target(&map, east_germany),
            "a point placed this action must not unlock a new neighbour this same action"
        );
        assert_eq!(placement.place(&map, east_germany), Err(PlacementError::NoPresence { country: "East Germany".to_string(), side: Ussr }));
    }

    #[test]
    fn a_country_with_base_presence_stays_placeable_all_action() {
        // Contrast with the above: a country that already had the
        // placing side's influence (or a qualifying neighbour) *before*
        // this action started stays legal for every point placed in it
        // this action, however many.
        let map = map();
        let mut board = Board::new(&map);
        let poland = id(&map, "Poland");
        board.set_influence(poland, Ussr, 1);
        let mut placement = InfluencePlacement::new(Ussr, 10, &board);

        for _ in 0..5 {
            placement.place(&map, poland).unwrap();
        }
        assert_eq!(placement.pending(poland), 5);
    }

    #[test]
    fn a_neighbours_base_presence_keeps_unlocking_a_country_all_action() {
        // A country made legal by a *neighbour's* base presence stays
        // legal for repeated placements too — only using influence
        // placed *this* action to unlock a *different* country is
        // disallowed, not placing repeatedly in the one country that was
        // already reachable.
        let map = map();
        let mut board = Board::new(&map);
        let poland = id(&map, "Poland");
        board.set_influence(poland, Ussr, 1);
        let east_germany = id(&map, "East Germany");
        let mut placement = InfluencePlacement::new(Ussr, 10, &board);

        for _ in 0..3 {
            placement.place(&map, east_germany).unwrap();
        }
        assert_eq!(placement.pending(east_germany), 3);
    }

    #[test]
    fn placing_an_illegal_target_is_refused_and_charges_nothing() {
        let map = map();
        let board = Board::new(&map);
        let mut placement = InfluencePlacement::new(Ussr, 10, &board);
        let chile = id(&map, "Chile");
        let err = placement.place(&map, chile).unwrap_err();
        assert_eq!(err, PlacementError::NoPresence { country: "Chile".to_string(), side: Ussr });
        assert_eq!(placement.remaining(), 10);
        assert_eq!(placement.pending(chile), 0);
    }

    #[test]
    fn overspending_is_refused() {
        let map = map();
        let mut board = Board::new(&map);
        let france = id(&map, "France");
        board.set_influence(france, Us, 10);
        // USSR has no presence in France; give it a border via a
        // neighbour instead, cheaply, so the test isolates the ops check.
        let poland = id(&map, "Poland");
        board.set_influence(poland, Ussr, 1);

        let mut placement = InfluencePlacement::new(Ussr, 1, &board);
        // Poland costs 1 (USSR-controlled already) — affordable.
        placement.place(&map, poland).unwrap();
        assert_eq!(placement.remaining(), 0);

        // Any further placement, even a 1-cost one, must now be refused.
        let east_germany = id(&map, "East Germany");
        let err = placement.place(&map, east_germany).unwrap_err();
        assert_eq!(
            err,
            PlacementError::InsufficientOps { country: "East Germany".to_string(), needed: 1, remaining: 0 }
        );
    }

    #[test]
    fn a_two_cost_target_is_refused_with_only_one_op_left() {
        let map = map();
        let mut board = Board::new(&map);
        let france = id(&map, "France");
        board.set_influence(france, Us, 10);
        board.set_influence(id(&map, "West Germany"), Ussr, 1);

        let mut placement = InfluencePlacement::new(Ussr, 1, &board);
        assert!(map.country(france).adjacent.contains(&id(&map, "West Germany")));
        let err = placement.place(&map, france).unwrap_err();
        assert_eq!(
            err,
            PlacementError::InsufficientOps { country: "France".to_string(), needed: 2, remaining: 1 }
        );
    }

    #[test]
    fn remaining_never_exceeds_total_and_never_underflows() {
        let map = map();
        let board = Board::new(&map);
        let mut placement = InfluencePlacement::new(Ussr, 2, &board);
        let poland = id(&map, "Poland");
        placement.place(&map, poland).unwrap();
        placement.place(&map, poland).unwrap();
        assert_eq!(placement.remaining(), 0);
        assert_eq!(placement.ops_spent(), placement.ops_total());
        // No panic, no further deduction: the third placement is refused.
        assert!(placement.place(&map, poland).is_err());
        assert_eq!(placement.remaining(), 0);
    }

    #[test]
    fn undo_refunds_the_exact_cost_even_after_control_flips() {
        let map = map();
        let mut board = Board::new(&map);
        let france = id(&map, "France");
        let stability = map.country(france).stability;
        board.set_influence(france, Ussr, stability); // USSR controls France
        board.set_influence(id(&map, "West Germany"), Us, 1); // gives the US presence next door

        let mut placement = InfluencePlacement::new(Us, 10, &board);
        assert!(placement.is_legal_target(&map, france));

        // First point: France is still USSR-controlled, so it costs 2.
        assert_eq!(placement.place(&map, france).unwrap(), 2);
        // That one point already breaks USSR's control (influence is now
        // tied, and a tie isn't `>=` stability ahead), so the next point
        // should cost only 1.
        assert_ne!(placement.board().controller(&map, france), Some(Ussr));
        assert_eq!(placement.place(&map, france).unwrap(), 1);

        let spent_before_undo = placement.ops_spent();
        let undone = placement.undo_last(&map);
        assert_eq!(undone, Some(france));
        assert_eq!(placement.pending(france), 1);
        // The refund must equal what the last placement actually cost (1)
        // — not a flat reuse of the first point's cost of 2.
        assert_eq!(placement.ops_spent(), spent_before_undo - 1);
    }

    #[test]
    fn undo_on_an_empty_placement_does_nothing() {
        let map = map();
        let board = Board::new(&map);
        let mut placement = InfluencePlacement::new(Ussr, 5, &board);
        assert_eq!(placement.undo_last(&map), None);
        assert_eq!(placement.remaining(), 5);
    }

    #[test]
    fn commit_hands_back_the_board_untouched_until_then() {
        let map = map();
        let base = Board::new(&map);
        let poland = id(&map, "Poland");
        let mut placement = InfluencePlacement::new(Ussr, 5, &base);
        placement.place(&map, poland).unwrap();

        // The original board passed in must be untouched.
        assert_eq!(base.influence(poland, Ussr), 0);

        let committed = placement.commit();
        assert_eq!(committed.influence(poland, Ussr), 1);
    }
}
