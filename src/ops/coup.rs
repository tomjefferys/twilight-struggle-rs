//! The coup operation (rule 6.3): an attempt to force opposing influence
//! out of a country by force, with a chance of installing friendly
//! influence in its place.
//!
//! Like [`Realignment`](super::Realignment), a [`Coup`] stages nothing —
//! [`Coup::attempt`] resolves immediately onto the caller's real [`Board`]
//! and can't be taken back, the way a die roll at a physical table can't.
//! It keeps the same `base`-snapshot-for-display trick, and its maths is
//! split into free functions (`coup_resolve`, `coup_odds`) the same way,
//! so a preview can be computed with no [`Coup`] session open at all.
//!
//! Two things set it apart from realignment, though:
//!
//! - A card is spent all at once on a *single* attempt (rule 6.3.2 rolls
//!   one die and adds the card's whole Ops value), not one roll per op.
//!   Once [`Coup::attempt`] has resolved, the action is done — a second
//!   attempt is refused, the same way a placement action with no ops left
//!   would be.
//! - Rule 6.3.4 (DEFCON degradation) and Military Operations are out of
//!   scope here, the same way rule 6.1.3 is out of scope for realignment —
//!   see that module's doc for the precedent.
//!
//! There is still no presence requirement (rule 6.3.1): the acting side
//! need not have influence in or adjacent to the target, only the
//! opponent needs to have something there to coup.

use std::fmt;

use crate::board::Board;
use crate::country::{CountryId, Region, Superpower};
use crate::dice::Dice;
use crate::map::WorldMap;
use crate::cards::CardId;
use crate::ongoing::{LastingEffects, TurnEffects};
use crate::country::SubRegion;

/// The number a coup's modified roll must strictly exceed to succeed:
/// the target country's stability, doubled (rule 6.3.2).
pub fn coup_target_number(map: &WorldMap, id: CountryId) -> u8 {
    map.country(id).stability * 2
}

/// One resolved (or would-be) coup attempt: the die, the ops added to it,
/// and what actually happened on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoupResult {
    pub target: CountryId,
    pub die: u8,
    pub ops: u8,
    /// An ongoing event's adjustment to the die roll (Latin American
    /// Death Squads): added to `die + ops` before comparing.
    pub modifier: i8,
    pub target_number: u8,
    /// `(die + ops) - target_number` when that's positive, else 0 — the
    /// total influence swing on a success, and the reason a tie fails
    /// (rule 6.3.2 requires the modified roll to be *greater than*, not
    /// merely equal to, the doubled stability).
    pub margin: u8,
    /// Opposing influence removed from the target, capped by what was
    /// actually there.
    pub removed: u8,
    /// Friendly influence added to the target to make up any shortfall
    /// between `margin` and `removed` (rule 6.3.3).
    pub added: u8,
}

impl CoupResult {
    pub fn success(&self) -> bool {
        self.margin > 0
    }
}

/// Resolves one coup attempt: a pure function of the die, the ops spent,
/// the target's stability, and the opponent's influence actually present
/// — factored out from [`Coup::attempt`] so every rules test can run
/// without touching [`Dice`] at all.
pub fn coup_resolve(target: CountryId, acting: Superpower, die: u8, ops: u8, target_number: u8, board: &Board) -> CoupResult {
    coup_resolve_with(target, acting, die, ops, 0, target_number, board)
}

/// [`coup_resolve`] with an ongoing event's `modifier` added to the roll.
pub fn coup_resolve_with(target: CountryId, acting: Superpower, die: u8, ops: u8, modifier: i8, target_number: u8, board: &Board) -> CoupResult {
    let opponent = acting.opponent();
    let modified = die as i32 + ops as i32 + modifier as i32;
    let margin = (modified - target_number as i32).max(0) as u8;
    let removed = margin.min(board.influence(target, opponent));
    let added = margin - removed;
    CoupResult { target, die, ops, modifier, target_number, margin, removed, added }
}

/// Win/fail odds for `side` couping `id` with `ops` operation points right
/// now, counted in sixths (one d6, six outcomes) rather than stored as
/// floats, so there's nothing to round and no drift between platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoupOdds {
    pub success: u8,
    pub failure: u8,
    /// Expected opponent influence removed, in 6ths — capped, per face,
    /// by the opponent's actual influence present.
    pub removed_6ths: u16,
    /// Expected friendly influence added, in 6ths — the flip side of
    /// `removed_6ths`: whatever margin the opponent's influence couldn't
    /// absorb.
    pub added_6ths: u16,
}

/// `side`'s odds couping `id` with `ops` operation points, enumerated over
/// all six die faces. Unlike [`super::odds`], this takes `ops` explicitly
/// — a coup's only modifier is the card's own Ops value, so there's no
/// ops-free preview to compute.
pub fn coup_odds(map: &WorldMap, board: &Board, id: CountryId, side: Superpower, ops: u8) -> CoupOdds {
    coup_odds_with(map, board, id, side, ops, 0)
}

/// [`coup_odds`] with an ongoing event's `modifier` added to the roll.
pub fn coup_odds_with(map: &WorldMap, board: &Board, id: CountryId, side: Superpower, ops: u8, modifier: i8) -> CoupOdds {
    let target_number = coup_target_number(map, id);
    let mut result = CoupOdds::default();
    for die in 1..=6u8 {
        let roll = coup_resolve_with(id, side, die, ops, modifier, target_number, board);
        if roll.success() {
            result.success += 1;
            result.removed_6ths += roll.removed as u16;
            result.added_6ths += roll.added as u16;
        } else {
            result.failure += 1;
        }
    }
    result
}

/// Why a coup attempt was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoupError {
    /// The opponent has no influence in `country` for `side` to coup.
    NoOpponentInfluence { country: String, side: Superpower },
    /// This action has already resolved its one attempt.
    AlreadyResolved { country: String },
    /// An ongoing event forbids `side` coups in `country`'s region (The
    /// Reformer: no more USSR coups in Europe).
    Banned { country: String, region: Region },
    /// A lasting event (NATO, the US/Japan pact) shields `country` from this side's coups.
    Protected { country: String, by: CardId },
}

impl fmt::Display for CoupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoupError::NoOpponentInfluence { country, side } => {
                write!(f, "{} has no influence in {country} for {side} to coup", side.opponent())
            }
            CoupError::Protected { country, by } => write!(f, "card #{} protects {country} from coups", by.0),
            CoupError::Banned { country, region } => write!(f, "an event forbids coups in {region} ({country})"),
            CoupError::AlreadyResolved { country } => {
                write!(f, "this coup has already resolved its one attempt (against {country})")
            }
        }
    }
}

impl std::error::Error for CoupError {}

/// An open coup action: the ops it will spend, and its one attempt once
/// resolved. Holds **no speculative board** — see the module doc for why.
///
/// `Clone` (like [`Board`]'s own) is cheap and exists for the same
/// reason: a [`crate::game::Game`] needs to be clonable for AI lookahead.
#[derive(Clone)]
pub struct Coup {
    side: Superpower,
    ops_total: u8,
    result: Option<CoupResult>,
    /// The board as it stood when this action started — display only.
    /// `delta` reads it; legality never does.
    base: Board,
    /// Regions an ongoing event bars this side from couping in.
    banned: Vec<Region>,
    /// Turn-long events that adjust the roll or the ops (Death Squads,
    /// Vietnam Revolts).
    effects: TurnEffects,
    /// Game-long events that shield countries (NATO, the US/Japan pact).
    lasting: LastingEffects,
}

impl Coup {
    pub fn new(side: Superpower, ops: u8, board: &Board) -> Self {
        Coup { side, ops_total: ops, result: None, base: board.clone(), banned: Vec::new(), effects: TurnEffects::default(), lasting: LastingEffects::default() }
    }

    /// Forbids this coup from targeting any country in `regions` — what an
    /// ongoing event (The Reformer) does to its victim.
    pub fn with_banned_regions(mut self, regions: Vec<Region>) -> Self {
        self.banned = regions;
        self
    }

    /// Applies the turn-long events in force (see [`crate::ongoing`]).
    pub fn with_effects(mut self, effects: TurnEffects) -> Self {
        self.effects = effects;
        self
    }

    /// Applies the game-long events in force (see [`crate::ongoing`]).
    pub fn with_lasting(mut self, lasting: LastingEffects) -> Self {
        self.lasting = lasting;
        self
    }

    /// The card shielding `id` from this coup right now, if any.
    pub fn protected_by(&self, map: &WorldMap, board: &Board, id: CountryId) -> Option<CardId> {
        self.lasting.protects(map, board, self.side, id)
    }

    pub fn side(&self) -> Superpower {
        self.side
    }

    /// The ops this coup is worth: those of the attempt once it has
    /// resolved (which may include a Southeast Asia bonus), else the
    /// card's own.
    pub fn ops_total(&self) -> u8 {
        self.result.map_or(self.ops_total, |r| r.ops)
    }

    /// The ops a coup on `id` would use: the card's, plus Vietnam Revolts'
    /// bonus when `id` is in Southeast Asia.
    pub fn ops_for(&self, map: &WorldMap, id: CountryId) -> u8 {
        match self.effects.sub_region_bonus(self.side) {
            Some((sub, n)) if map.country(id).is_in_sub_region(sub) => self.ops_total + n,
            _ => self.ops_total,
        }
    }

    /// The Southeast Asia-style bonus still on offer, for display.
    pub fn sub_region_bonus(&self) -> Option<(SubRegion, u8)> {
        self.effects.sub_region_bonus(self.side)
    }

    /// The die modifier a coup on `id` gets, with the card responsible.
    pub fn roll_mod(&self, map: &WorldMap, id: CountryId) -> Option<(crate::cards::CardId, i8)> {
        self.effects.coup_roll_mod(self.side, map.country(id).region)
    }

    /// All the ops at once, once this action's one attempt has resolved;
    /// zero until then.
    pub fn ops_spent(&self) -> u8 {
        self.result.map_or(0, |r| r.ops)
    }

    pub fn remaining(&self) -> u8 {
        self.ops_total() - self.ops_spent()
    }

    /// The target number and odds for a coup on `id` right now — everything
    /// the UI needs to show before spending the card. `board` should be
    /// the caller's live board, not `base`.
    pub fn preview(&self, map: &WorldMap, board: &Board, id: CountryId) -> (u8, CoupOdds) {
        let target_number = coup_target_number(map, id);
        let modifier = self.roll_mod(map, id).map_or(0, |(_, m)| m);
        let odds = coup_odds_with(map, board, id, self.side, self.ops_for(map, id), modifier);
        (target_number, odds)
    }

    /// Whether `id` is a legal target right now: the opponent has any
    /// influence there at all (rule 6.3.1). No presence of the acting
    /// side's own is required. Deliberately doesn't check whether this
    /// action's attempt has already resolved — like
    /// [`Realignment::is_legal_target`](super::Realignment::is_legal_target),
    /// that's `attempt`'s job, so a spent session doesn't dim every
    /// country in the region.
    pub fn is_legal_target(&self, map: &WorldMap, board: &Board, id: CountryId) -> bool {
        !self.banned.contains(&map.country(id).region) && board.influence(id, self.side.opponent()) > 0 && self.protected_by(map, board, id).is_none()
    }

    /// Resolves this action's one attempt against `id`, spending every op
    /// at once and writing straight to `board`. Refused — with no die
    /// drawn and no state changed — if this action has already resolved
    /// or `id` isn't a legal target. Irreversible on success.
    pub fn attempt(&mut self, map: &WorldMap, board: &mut Board, id: CountryId, dice: &mut Dice) -> Result<CoupResult, CoupError> {
        if self.result.is_some() {
            return Err(CoupError::AlreadyResolved { country: map.country(id).name.clone() });
        }
        let region = map.country(id).region;
        if self.banned.contains(&region) {
            return Err(CoupError::Banned { country: map.country(id).name.clone(), region });
        }
        if let Some(by) = self.protected_by(map, board, id) {
            return Err(CoupError::Protected { country: map.country(id).name.clone(), by });
        }
        if !self.is_legal_target(map, board, id) {
            return Err(CoupError::NoOpponentInfluence { country: map.country(id).name.clone(), side: self.side });
        }

        let target_number = coup_target_number(map, id);
        let die = dice.roll();
        let modifier = self.roll_mod(map, id).map_or(0, |(_, m)| m);
        let result = coup_resolve_with(id, self.side, die, self.ops_for(map, id), modifier, target_number, board);

        if result.removed > 0 {
            board.remove_influence(id, self.side.opponent(), result.removed);
        }
        if result.added > 0 {
            board.add_influence(id, self.side, result.added);
        }
        self.result = Some(result);
        Ok(result)
    }

    /// How much `side`'s influence in `id` has changed, relative to
    /// `board`, since this action began — positive or negative, since a
    /// success can both remove opposing influence and add friendly
    /// influence to the same country.
    pub fn delta(&self, board: &Board, id: CountryId, side: Superpower) -> i8 {
        let current = board.influence(id, side) as i16;
        let original = self.base.influence(id, side) as i16;
        (current - original) as i8
    }

    /// The target country, once this action has resolved — empty
    /// before then.
    pub fn touched(&self) -> Vec<CountryId> {
        self.result.map(|r| vec![r.target]).unwrap_or_default()
    }

    pub fn result(&self) -> Option<&CoupResult> {
        self.result.as_ref()
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

    /// Brute-forces a seed whose first rolls are exactly `values` — see
    /// `realign.rs`'s copy of this helper for why.
    fn dice_rolling(values: &[u8]) -> Dice {
        for seed in 0u64.. {
            let mut probe = Dice::from_seed(seed);
            if values.iter().all(|&v| probe.roll() == v) {
                return Dice::from_seed(seed);
            }
        }
        unreachable!("every u64 seed exhausted without a match")
    }

    // --- target number ---------------------------------------------------

    #[test]
    fn the_target_number_is_stability_doubled() {
        let map = map();
        let venezuela = id(&map, "Venezuela"); // stability 2
        assert_eq!(map.country(venezuela).stability, 2);
        assert_eq!(coup_target_number(&map, venezuela), 4);
    }

    // --- resolving an attempt ---------------------------------------------

    #[test]
    fn an_exactly_equal_modified_roll_fails() {
        // Rule 6.3.2: the modified roll must be *greater than* the
        // doubled stability, not merely equal to it.
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela"); // stability 2, target number 4
        board.set_influence(venezuela, Us, 3);

        let mut coup = Coup::new(Ussr, 2, &board); // die 2 + ops 2 = 4, exactly the target
        let mut dice = dice_rolling(&[2]);
        let result = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();

        assert!(!result.success());
        assert_eq!(result.margin, 0);
        assert_eq!(result.removed, 0);
        assert_eq!(board.influence(venezuela, Us), 3, "a failed coup changes nothing");
    }

    #[test]
    fn a_roll_one_over_succeeds_by_exactly_one() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela"); // target number 4
        board.set_influence(venezuela, Us, 3);

        let mut coup = Coup::new(Ussr, 2, &board); // die 3 + ops 2 = 5, one over
        let mut dice = dice_rolling(&[3]);
        let result = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();

        assert!(result.success());
        assert_eq!(result.margin, 1);
        assert_eq!(result.removed, 1);
        assert_eq!(result.added, 0);
        assert_eq!(board.influence(venezuela, Us), 2);
    }

    #[test]
    fn removal_is_capped_and_the_surplus_becomes_friendly_influence() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela"); // target number 4
        board.set_influence(venezuela, Us, 1); // less than the margin below

        let mut coup = Coup::new(Ussr, 4, &board); // die 6 + ops 4 = 10, margin 6
        let mut dice = dice_rolling(&[6]);
        let result = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();

        assert_eq!(result.margin, 6);
        assert_eq!(result.removed, 1, "capped at the 1 that was actually there");
        assert_eq!(result.added, 5, "the rest of the margin becomes friendly influence (rule 6.3.3)");
        assert_eq!(board.influence(venezuela, Us), 0);
        assert_eq!(board.influence(venezuela, Ussr), 5);
    }

    #[test]
    fn total_swing_always_equals_the_margin() {
        let map = map();
        let venezuela = id(&map, "Venezuela");
        for opponent_influence in 1..=6u8 {
            // opponent_influence == 0 would make Venezuela an illegal
            // target (rule 6.3.1) rather than exercise the removal/added
            // split this test is after.
            for die in 1..=6u8 {
                let mut board = Board::new(&map);
                board.set_influence(venezuela, Us, opponent_influence);
                let mut coup = Coup::new(Ussr, 3, &board);
                let mut dice = dice_rolling(&[die]);
                let result = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();
                assert_eq!(result.removed + result.added, result.margin);
            }
        }
    }

    #[test]
    fn a_failed_coup_still_spends_every_op() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela"); // target number 4
        board.set_influence(venezuela, Us, 3);

        let mut coup = Coup::new(Ussr, 1, &board); // die 1 + ops 1 = 2, well short
        let mut dice = dice_rolling(&[1]);
        let result = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();

        assert!(!result.success());
        assert_eq!(coup.ops_spent(), 1);
        assert_eq!(coup.remaining(), 0);
    }

    // --- legality ----------------------------------------------------------

    #[test]
    fn no_presence_is_required_to_coup() {
        let map = map();
        let mut board = Board::new(&map);
        let chile = id(&map, "Chile");
        board.set_influence(chile, Us, 2);
        let coup = Coup::new(Ussr, 3, &board);
        assert!(coup.is_legal_target(&map, &board, chile));
    }

    #[test]
    fn a_target_with_no_opponent_influence_is_refused() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        let mut coup = Coup::new(Ussr, 3, &board);
        let mut dice = Dice::from_seed(1);
        let err = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, CoupError::NoOpponentInfluence { country: "Venezuela".to_string(), side: Ussr });
    }

    #[test]
    fn a_second_attempt_is_refused() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        let mut coup = Coup::new(Ussr, 3, &board);
        let mut dice = Dice::from_seed(2);

        coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();
        let err = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, CoupError::AlreadyResolved { country: "Venezuela".to_string() });
    }

    #[test]
    fn a_refused_attempt_consumes_no_dice() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        let mut coup = Coup::new(Ussr, 3, &board);
        let mut dice = Dice::from_seed(555);
        let mut control = Dice::from_seed(555);

        // Illegal: no US influence in Venezuela yet.
        let err = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, CoupError::NoOpponentInfluence { country: "Venezuela".to_string(), side: Ussr });

        // Now make it legal and attempt for real — the die drawn should
        // be exactly what a fresh die from the same seed rolls first,
        // proving the refused attempt above never touched the sequence.
        board.set_influence(venezuela, Us, 3);
        let result = coup.attempt(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(result.die, control.roll());
    }

    // --- odds ----------------------------------------------------------------

    #[test]
    fn success_and_failure_sum_to_six() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        let o = coup_odds(&map, &board, venezuela, Ussr, 2);
        assert_eq!(o.success as u16 + o.failure as u16, 6);
    }

    #[test]
    fn odds_expectations_agree_with_resolve_across_every_face() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        let target_number = coup_target_number(&map, venezuela);

        let mut removed_total = 0u16;
        let mut added_total = 0u16;
        let mut success_total = 0u8;
        for die in 1..=6u8 {
            let result = coup_resolve(venezuela, Ussr, die, 2, target_number, &board);
            if result.success() {
                success_total += 1;
                removed_total += result.removed as u16;
                added_total += result.added as u16;
            }
        }

        let o = coup_odds(&map, &board, venezuela, Ussr, 2);
        assert_eq!(o.success, success_total);
        assert_eq!(o.removed_6ths, removed_total);
        assert_eq!(o.added_6ths, added_total);
    }

    #[test]
    fn odds_never_show_an_impossible_success_when_ops_can_never_clear_the_target() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela"); // target number 4
        board.set_influence(venezuela, Us, 3);
        let o = coup_odds(&map, &board, venezuela, Ussr, 0); // best possible roll: 6 + 0 = 6 > 4, still winnable
        assert!(o.success > 0);

        let brezhnev = id(&map, "Finland"); // stability 4, target number 8: unreachable with 0 ops (max 6)
        board.set_influence(brezhnev, Us, 3);
        let o2 = coup_odds(&map, &board, brezhnev, Ussr, 0);
        assert_eq!(o2.success, 0, "die alone (max 6) can never exceed a target number of 8");
    }

    // --- ongoing events ----------------------------------------------------

    #[test]
    fn a_roll_modifier_shifts_the_modified_roll_and_the_odds() {
        let map = map();
        let mut board = Board::new(&map);
        let colombia = id(&map, "Colombia"); // stability 1: target number 2
        board.set_influence(colombia, Us, 5);
        // Ops 2 beat target 2 only with a die of 1+... die+2 > 2 always; +modifier -1 on die 1 ties and fails.
        assert_eq!(coup_odds(&map, &board, colombia, Ussr, 2).success, 6);
        assert_eq!(coup_odds_with(&map, &board, colombia, Ussr, 2, -1).success, 5);
        let result = coup_resolve_with(colombia, Ussr, 4, 2, -1, 2, &board);
        assert_eq!((result.margin, result.modifier), (3, -1));
    }

    #[test]
    fn death_squads_modify_a_coup_in_the_americas_only() {
        use crate::ongoing::{OngoingEffect, TurnEffects};
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Colombia"), Us, 3);
        board.set_influence(id(&map, "Italy"), Us, 3);
        let mut effects = TurnEffects::default();
        effects.apply(OngoingEffect::DeathSquads { beneficiary: Ussr });
        let coup = Coup::new(Ussr, 2, &board).with_effects(effects);
        assert_eq!(coup.roll_mod(&map, id(&map, "Colombia")).map(|(_, m)| m), Some(1));
        assert_eq!(coup.roll_mod(&map, id(&map, "Italy")), None);
        let against = Coup::new(Us, 2, &board).with_effects(effects);
        assert_eq!(against.roll_mod(&map, id(&map, "Colombia")).map(|(_, m)| m), Some(-1));
    }

    #[test]
    fn a_southeast_asia_bonus_adds_an_op_there_and_is_reported_once_spent() {
        use crate::ongoing::{OngoingEffect, TurnEffects};
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Thailand"), Us, 2);
        let mut effects = TurnEffects::default();
        effects.apply(OngoingEffect::VietnamRevolts);
        let mut coup = Coup::new(Ussr, 2, &board).with_effects(effects);
        assert_eq!(coup.ops_for(&map, id(&map, "Thailand")), 3);
        assert_eq!(coup.ops_for(&map, id(&map, "Poland")), 2);
        assert_eq!(coup.ops_total(), 2, "before the attempt it is the card's own value");
        let mut live = board.clone();
        let result = coup.attempt(&map, &mut live, id(&map, "Thailand"), &mut Dice::from_seed(1)).unwrap();
        assert_eq!((result.ops, coup.ops_total(), coup.ops_spent(), coup.remaining()), (3, 3, 3, 0));
    }
}
