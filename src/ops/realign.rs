//! The realignment-roll operation (rule 6.2): a die-roll contest that
//! reduces an opponent's influence in a country — and can just as easily
//! backfire, reducing the acting player's own.
//!
//! Unlike [`InfluencePlacement`](super::InfluencePlacement), a
//! [`Realignment`] stages nothing: each [`Realignment::roll`] resolves
//! immediately onto the caller's real [`Board`] and can't be taken back,
//! the way a die roll at a physical table can't. That single difference
//! is why this type's shape diverges from `InfluencePlacement` in two
//! ways worth calling out up front:
//!
//! - It keeps a `base` snapshot purely to *report* what changed
//!   (`Realignment::delta`) — never to gate legality. Legality and the
//!   modifier maths always read the live board, since a successful roll
//!   can empty a country mid-action and make a further roll on it
//!   pointless.
//! - There is no `commit`. There's nothing to commit.
//!
//! There is also no presence requirement (rule 6.2.1): unlike placement,
//! a side may target a country it has no influence in or near at all.
//! And there are deliberately no DEFCON restrictions here (rule 6.1.3 is
//! out of scope for this operation) — a target is legal purely on
//! whether the opponent has anything to remove.
//!
//! The modifier and odds maths (`modifiers`, `odds`) are free functions
//! of `(map, board, id, side)` rather than methods, so a preview can be
//! computed with no [`Realignment`] session open at all — which is what
//! lets the country-detail view and the live region footer share one
//! code path.

use std::fmt;

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::dice::Dice;
use crate::map::WorldMap;
use crate::cards::CardId;
use super::TargetScope;
use crate::ongoing::{bonus_membership, bonus_ops, LastingEffects, OpsBonus, TurnEffects};

/// One side's die-roll modifiers for one country, itemised rather than
/// summed, so the UI can explain the number instead of just showing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    /// +1 for each adjacent country this side controls. The target
    /// itself never counts, however it's controlled.
    pub adjacent_controlled: u8,
    /// +1 if this side has *strictly* more influence in the target than
    /// its opponent — an exact tie grants neither side this bonus.
    pub more_influence: bool,
    /// +1 if this side's own superpower borders the target.
    pub superpower_adjacent: bool,
    /// Iran-Contra Scandal: -1 to this side's roll.
    pub iran_contra: bool,
}

impl Modifiers {
    pub fn total(&self) -> i8 {
        self.adjacent_controlled as i8 + self.more_influence as i8 + self.superpower_adjacent as i8 - self.iran_contra as i8
    }

    /// A human-readable breakdown of what's contributing, in the order
    /// listed in rule 6.2.2 — empty when nothing applies.
    pub fn reasons(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.adjacent_controlled > 0 {
            let noun = if self.adjacent_controlled == 1 { "country" } else { "countries" };
            reasons.push(format!("{} adjacent controlled {noun}", self.adjacent_controlled));
        }
        if self.more_influence {
            reasons.push("more influence".to_string());
        }
        if self.superpower_adjacent {
            reasons.push("superpower adjacent".to_string());
        }
        if self.iran_contra {
            reasons.push("Iran-Contra -1".to_string());
        }
        reasons
    }
}

/// `side`'s realignment modifiers for a roll on `id`, computed live
/// against `board` — no session needs to be open to ask this.
pub fn modifiers(map: &WorldMap, board: &Board, id: CountryId, side: Superpower) -> Modifiers {
    modifiers_with(map, board, id, side, &TurnEffects::default())
}

/// [`modifiers`] under the turn-long events in `effects`.
pub fn modifiers_with(map: &WorldMap, board: &Board, id: CountryId, side: Superpower, effects: &TurnEffects) -> Modifiers {
    let country = map.country(id);
    let opponent = side.opponent();
    let adjacent_controlled = country
        .adjacent
        .iter()
        .filter(|&&n| board.is_controlled_by(map, n, side))
        .count() as u8;
    let more_influence = board.influence(id, side) > board.influence(id, opponent);
    let superpower_adjacent = country.borders_superpower(side);
    let iran_contra = effects.realign_roll_mod(side).is_some();
    Modifiers { adjacent_controlled, more_influence, superpower_adjacent, iran_contra }
}

/// Win/draw/loss odds for `side` rolling against `id` right now, counted
/// in 36ths (every outcome over two d6 is a multiple of 1/36) rather
/// than stored as floats, so there's nothing to round and no drift
/// between platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Odds {
    pub win: u8,
    pub draw: u8,
    pub loss: u8,
    /// Expected opponent influence removed, in 36ths — capped, per
    /// pair, by the opponent's *own* influence actually present.
    pub removed_36ths: u16,
    /// Expected influence `side` loses of its own, in 36ths — capped,
    /// per pair, by `side`'s *own* influence actually present. This is
    /// deliberately a separate cap from `removed_36ths`: realigning
    /// somewhere you have little or no presence can have a real chance
    /// of losing the roll while costing nothing, because there's
    /// nothing of yours there to remove.
    pub lost_36ths: u16,
}

/// `side`'s odds rolling against `id` right now, enumerated over all 36
/// die pairs.
pub fn odds(map: &WorldMap, board: &Board, id: CountryId, side: Superpower) -> Odds {
    odds_with(map, board, id, side, &TurnEffects::default())
}

/// [`odds`] under the turn-long events in `effects`.
pub fn odds_with(map: &WorldMap, board: &Board, id: CountryId, side: Superpower, effects: &TurnEffects) -> Odds {
    let opponent = side.opponent();
    let acting_mods = modifiers_with(map, board, id, side, effects);
    let opposing_mods = modifiers_with(map, board, id, opponent, effects);

    let mut result = Odds::default();
    for acting_die in 1..=6u8 {
        for opposing_die in 1..=6u8 {
            let roll = resolve(id, side, acting_die, acting_mods, opposing_die, opposing_mods, board);
            match roll.loser {
                None => result.draw += 1,
                Some(loser) if loser == opponent => {
                    result.win += 1;
                    result.removed_36ths += roll.removed as u16;
                }
                Some(_) => {
                    result.loss += 1;
                    result.lost_36ths += roll.removed as u16;
                }
            }
        }
    }
    result
}

/// One resolved roll, kept whole for the session log and for the
/// interactive footer to display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollResult {
    pub target: CountryId,
    pub acting_die: u8,
    pub acting_mods: Modifiers,
    pub opposing_die: u8,
    pub opposing_mods: Modifiers,
    /// `None` on a draw; otherwise whichever side rolled the lower
    /// modified total — which may be the acting side itself.
    pub loser: Option<Superpower>,
    /// What was actually removed from `loser`'s influence — the raw die
    /// difference capped by what `loser` actually had in the target, so
    /// this is always safe to subtract without saturating.
    pub removed: u8,
}

/// Resolves one roll: a pure function of the two dice, each side's
/// modifiers, and the influence actually present — factored out from
/// [`Realignment::roll`] so every rules test can run without touching
/// [`Dice`] at all.
pub fn resolve(
    target: CountryId,
    acting: Superpower,
    acting_die: u8,
    acting_mods: Modifiers,
    opposing_die: u8,
    opposing_mods: Modifiers,
    board: &Board,
) -> RollResult {
    let opposing = acting.opponent();
    let acting_total = acting_die as i32 + acting_mods.total() as i32;
    let opposing_total = opposing_die as i32 + opposing_mods.total() as i32;

    let (loser, removed) = match acting_total.cmp(&opposing_total) {
        std::cmp::Ordering::Greater => {
            let diff = (acting_total - opposing_total) as u8;
            (Some(opposing), diff.min(board.influence(target, opposing)))
        }
        std::cmp::Ordering::Less => {
            let diff = (opposing_total - acting_total) as u8;
            (Some(acting), diff.min(board.influence(target, acting)))
        }
        std::cmp::Ordering::Equal => (None, 0),
    };

    RollResult { target, acting_die, acting_mods, opposing_die, opposing_mods, loser, removed }
}

/// Why a roll was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealignError {
    /// The opponent has no influence in `country` for `side` to remove.
    NoOpponentInfluence { country: String, side: Superpower },
    /// No ops remain to spend on another roll.
    InsufficientOps { country: String, remaining: u8 },
    /// A lasting event (NATO, the US/Japan pact) shields `country` from this side's rolls.
    Protected { country: String, by: CardId },
    /// The card's event allows realignment rolls only in certain countries.
    OutOfScope { reason: String },
}

impl fmt::Display for RealignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RealignError::NoOpponentInfluence { country, side } => {
                write!(f, "{} has no influence in {country} for {side} to realign against", side.opponent())
            }
            RealignError::OutOfScope { reason } => write!(f, "{reason}"),
            RealignError::Protected { country, by } => write!(f, "card #{} protects {country} from realignment", by.0),
            RealignError::InsufficientOps { country, remaining } => {
                write!(f, "rolling in {country} costs 1 op, but only {remaining} remain")
            }
        }
    }
}

impl std::error::Error for RealignError {}

/// An open realignment action: ops still to spend, plus the log of
/// every roll this action has already resolved. Holds **no speculative
/// board** — see the module doc for why.
///
/// `Clone` (like [`Board`]'s own) is cheap and exists for the same
/// reason: a [`crate::game::Game`] needs to be clonable for AI lookahead.
#[derive(Clone)]
pub struct Realignment {
    side: Superpower,
    ops_total: u8,
    ops_spent: u8,
    history: Vec<RollResult>,
    effects: TurnEffects,
    /// Game-long events that shield countries (NATO, the US/Japan pact).
    lasting: LastingEffects,
    /// Where a card's event confines these rolls (Junta, Tear Down this Wall).
    scope: Option<TargetScope>,
    /// Extra ops for rolls spent wholly in one area (China Card, Vietnam Revolts).
    bonuses: Vec<OpsBonus>,
    /// The bonuses every roll so far has stayed inside (bit per bonus) —
    /// rolls can't be undone, so a bit once cleared stays cleared.
    bonus_mask: u8,
    /// The board as it stood when this action started — display only.
    /// `delta` reads it; legality and the modifier/odds maths never do.
    base: Board,
}

impl Realignment {
    pub fn new(side: Superpower, ops: u8, board: &Board) -> Self {
        Realignment {
            side,
            ops_total: ops,
            ops_spent: 0,
            history: Vec::new(),
            effects: TurnEffects::default(),
            lasting: LastingEffects::default(),
            scope: None,
            bonuses: Vec::new(),
            bonus_mask: u8::MAX,
            base: board.clone(),
        }
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

    /// Confines these rolls to `scope` (a card event that allows them only there).
    pub fn with_scope(mut self, scope: Option<TargetScope>) -> Self {
        self.scope = scope;
        self
    }

    fn in_scope(&self, map: &WorldMap, id: CountryId) -> bool {
        self.scope.is_none_or(|s| s.allows(map, id))
    }

    /// The card shielding `id` from this realignment right now, if any —
    /// judged on the live board, since control can flip mid-action.
    pub fn protected_by(&self, map: &WorldMap, board: &Board, id: CountryId) -> Option<CardId> {
        self.lasting.protects(map, board, self.side, id)
    }

    /// Grants extra ops while every roll stays inside a bonus's area.
    pub fn with_bonuses(mut self, bonuses: Vec<OpsBonus>) -> Self {
        self.bonuses = bonuses;
        self
    }

    /// The bonus ops still available: those whose area every roll so far
    /// has stayed inside.
    pub fn bonuses(&self) -> Vec<OpsBonus> {
        self.bonuses.iter().enumerate().filter(|(i, _)| self.bonus_mask & (1 << i) != 0).map(|(_, b)| *b).collect()
    }

    pub fn side(&self) -> Superpower {
        self.side
    }

    /// The ops this action is worth so far: the card's, plus the
    /// sub-region bonus once a roll has used it (and only while every
    /// roll has stayed inside that sub-region).
    pub fn ops_total(&self) -> u8 {
        if self.ops_spent > 0 { self.ops_total + bonus_ops(&self.bonuses, self.bonus_mask) } else { self.ops_total }
    }

    pub fn ops_spent(&self) -> u8 {
        self.ops_spent
    }

    pub fn remaining(&self) -> u8 {
        self.ops_total() - self.ops_spent
    }

    /// Whether a roll in `id` fits in the ops left — counting the bonus
    /// op only if this roll (and every earlier one) is in its sub-region.
    pub fn can_afford(&self, map: &WorldMap, id: CountryId) -> bool {
        let mask = self.bonus_mask & bonus_membership(&self.bonuses, map.country(id));
        self.ops_spent < self.ops_total + bonus_ops(&self.bonuses, mask)
    }

    /// Both sides' modifiers and the resulting odds for a roll on `id`
    /// right now — everything the UI needs to show before spending the
    /// op. `board` should be the caller's live board, not `base`.
    pub fn preview(&self, map: &WorldMap, board: &Board, id: CountryId) -> (Modifiers, Modifiers, Odds) {
        let acting = modifiers_with(map, board, id, self.side, &self.effects);
        let opposing = modifiers_with(map, board, id, self.side.opponent(), &self.effects);
        let odds = odds_with(map, board, id, self.side, &self.effects);
        (acting, opposing, odds)
    }

    /// Whether `id` is a legal target right now: the opponent has any
    /// influence there at all. No presence of the acting side's own is
    /// required (rule 6.2.1), and there's no DEFCON restriction — see
    /// the module doc. Judged against the live `board`, not `base`, so
    /// a country a roll has just emptied stops being legal mid-action.
    pub fn is_legal_target(&self, map: &WorldMap, board: &Board, id: CountryId) -> bool {
        self.in_scope(map, id) && board.influence(id, self.side.opponent()) > 0 && self.protected_by(map, board, id).is_none()
    }

    /// Resolves one roll against `id`, charging exactly 1 op and writing
    /// straight to `board`. Refused — with no dice drawn and no state
    /// changed — if no ops remain or `id` isn't a legal target.
    /// Irreversible on success.
    pub fn roll(
        &mut self,
        map: &WorldMap,
        board: &mut Board,
        id: CountryId,
        dice: &mut Dice,
    ) -> Result<RollResult, RealignError> {
        if !self.can_afford(map, id) {
            return Err(RealignError::InsufficientOps { country: map.country(id).name.clone(), remaining: self.remaining() });
        }
        if let Some(scope) = self.scope.filter(|s| !s.allows(map, id)) {
            return Err(RealignError::OutOfScope { reason: scope.refusal(map, id) });
        }
        if let Some(by) = self.protected_by(map, board, id) {
            return Err(RealignError::Protected { country: map.country(id).name.clone(), by });
        }
        if !self.is_legal_target(map, board, id) {
            return Err(RealignError::NoOpponentInfluence { country: map.country(id).name.clone(), side: self.side });
        }

        let acting_mods = modifiers_with(map, board, id, self.side, &self.effects);
        let opposing_mods = modifiers_with(map, board, id, self.side.opponent(), &self.effects);
        let acting_die = dice.roll();
        let opposing_die = dice.roll();
        let result = resolve(id, self.side, acting_die, acting_mods, opposing_die, opposing_mods, board);

        if let Some(loser) = result.loser {
            board.remove_influence(id, loser, result.removed);
        }
        self.bonus_mask &= bonus_membership(&self.bonuses, map.country(id));
        self.ops_spent += 1;
        self.history.push(result);
        Ok(result)
    }

    /// How much `side`'s influence in `id` has changed, relative to
    /// `board`, since this action began — positive or negative. A
    /// single signed number *per country* (rather than per side) can't
    /// represent a realignment: two rolls on one country can move both
    /// sides' influence independently, so this takes `side` explicitly.
    pub fn delta(&self, board: &Board, id: CountryId, side: Superpower) -> i8 {
        let current = board.influence(id, side) as i16;
        let original = self.base.influence(id, side) as i16;
        (current - original) as i8
    }

    /// Every country this action has rolled against so far, in the
    /// order it was first targeted.
    pub fn touched(&self) -> Vec<CountryId> {
        let mut seen = Vec::new();
        for roll in &self.history {
            if !seen.contains(&roll.target) {
                seen.push(roll.target);
            }
        }
        seen
    }

    pub fn history(&self) -> &[RollResult] {
        &self.history
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

    /// Brute-forces a seed whose first rolls are exactly `values` — the
    /// only way to drive [`Dice`] to a specific outcome, since it has no
    /// direct value-injection constructor. Cheap in practice: for `n`
    /// values there are `6^n` candidate prefixes spread over a
    /// well-mixed 64-bit space, so a match turns up within a few hundred
    /// seeds even for four or five values.
    fn dice_rolling(values: &[u8]) -> Dice {
        for seed in 0u64.. {
            let mut probe = Dice::from_seed(seed);
            if values.iter().all(|&v| probe.roll() == v) {
                return Dice::from_seed(seed);
            }
        }
        unreachable!("every u64 seed exhausted without a match")
    }

    // --- modifiers -----------------------------------------------------

    #[test]
    fn the_target_country_itself_is_not_an_adjacent_controlled_country() {
        let map = map();
        let mut board = Board::new(&map);
        let france = id(&map, "France");
        board.set_influence(france, Us, 10); // US controls France itself
        assert_eq!(modifiers(&map, &board, france, Us).adjacent_controlled, 0);
    }

    #[test]
    fn an_adjacent_superpower_is_not_counted_as_an_adjacent_controlled_country() {
        let map = map();
        let board = Board::new(&map);
        let poland = id(&map, "Poland");
        assert!(map.country(poland).borders_superpower(Ussr));
        let mods = modifiers(&map, &board, poland, Ussr);
        assert!(mods.superpower_adjacent);
        assert_eq!(mods.adjacent_controlled, 0, "USSR's own border must not also count as an adjacent controlled country");
    }

    #[test]
    fn each_adjacent_controlled_country_adds_one() {
        let map = map();
        let mut board = Board::new(&map);
        let poland = id(&map, "Poland");
        board.set_influence(id(&map, "East Germany"), Ussr, 3);
        board.set_influence(id(&map, "Czechoslovakia"), Ussr, 3);
        assert_eq!(modifiers(&map, &board, poland, Ussr).adjacent_controlled, 2);
    }

    #[test]
    fn equal_influence_gives_neither_side_the_more_influence_bonus() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        board.set_influence(venezuela, Ussr, 3);
        assert!(!modifiers(&map, &board, venezuela, Us).more_influence);
        assert!(!modifiers(&map, &board, venezuela, Ussr).more_influence);
    }

    #[test]
    fn modifiers_track_the_running_board() {
        // Contrast with `InfluencePlacement`, whose legality freezes at
        // a `base` snapshot: realignment's modifiers must reflect the
        // board as it stands *right now*, since a roll mid-action can
        // flip an adjacent country's control before the next one.
        let map = map();
        let mut board = Board::new(&map);
        let poland = id(&map, "Poland");
        let before = modifiers(&map, &board, poland, Ussr).adjacent_controlled;
        board.set_influence(id(&map, "East Germany"), Ussr, 3);
        let after = modifiers(&map, &board, poland, Ussr).adjacent_controlled;
        assert_eq!(before, 0);
        assert_eq!(after, 1);
    }

    // --- legality --------------------------------------------------------

    #[test]
    fn no_presence_is_required_to_realign() {
        // The explicit contrast with InfluencePlacement: the USSR may
        // target Chile with no USSR influence anywhere on the board.
        let map = map();
        let mut board = Board::new(&map);
        let chile = id(&map, "Chile");
        board.set_influence(chile, Us, 2);
        let realignment = Realignment::new(Ussr, 5, &board);
        assert!(realignment.is_legal_target(&map, &board, chile));
    }

    #[test]
    fn a_target_with_no_opponent_influence_is_illegal() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        let mut realignment = Realignment::new(Ussr, 5, &board);
        let mut dice = Dice::from_seed(1);
        let err = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, RealignError::NoOpponentInfluence { country: "Venezuela".to_string(), side: Ussr });
    }

    #[test]
    fn a_country_becomes_illegal_mid_action_once_its_opponent_influence_is_gone() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 2);

        let mut realignment = Realignment::new(Ussr, 5, &board);
        assert!(realignment.is_legal_target(&map, &board, venezuela));

        let mut dice = dice_rolling(&[6, 1]); // a big enough win to wipe out the 2 US influence
        let result = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(result.removed, 2);
        assert_eq!(board.influence(venezuela, Us), 0);

        assert!(!realignment.is_legal_target(&map, &board, venezuela));
        let err = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, RealignError::NoOpponentInfluence { country: "Venezuela".to_string(), side: Ussr });
    }

    // --- resolving a roll ------------------------------------------------

    #[test]
    fn a_tie_removes_nothing() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 2);
        board.set_influence(venezuela, Ussr, 2);

        let mut realignment = Realignment::new(Ussr, 5, &board);
        let mut dice = dice_rolling(&[3, 3]); // equal dice, equal (zero) modifiers
        let result = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();

        assert_eq!(result.loser, None);
        assert_eq!(result.removed, 0);
        assert_eq!(board.influence(venezuela, Us), 2);
        assert_eq!(board.influence(venezuela, Ussr), 2);
        // A draw still costs the op.
        assert_eq!(realignment.ops_spent(), 1);
    }

    #[test]
    fn the_acting_player_loses_influence_when_out_rolled() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 1);
        board.set_influence(venezuela, Ussr, 3); // gives US the "more influence" +1

        let mut realignment = Realignment::new(Us, 5, &board);
        let mut dice = dice_rolling(&[1, 6]); // US (acting) rolls 1, USSR (opposing) rolls 6
        let result = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();

        assert_eq!(result.loser, Some(Us), "the acting side can lose its own roll");
        assert_eq!(result.removed, 1);
        assert_eq!(board.influence(venezuela, Us), 0);
        assert_eq!(board.influence(venezuela, Ussr), 3, "no influence is ever added, and the winner's own stays put");
    }

    #[test]
    fn removal_is_capped_by_the_influence_actually_present() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 2); // less than the die difference below

        let mut realignment = Realignment::new(Ussr, 5, &board);
        let mut dice = dice_rolling(&[6, 2]); // modifiers are 0/0 here, so the raw difference is 4
        let result = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();

        assert_eq!(result.loser, Some(Us));
        assert_eq!(result.removed, 2, "capped at the 2 that were actually there, not the raw difference of 4");
        assert_eq!(board.influence(venezuela, Us), 0);
    }

    #[test]
    fn influence_is_never_added() {
        let map = map();
        let venezuela = id(&map, "Venezuela");
        for acting_die in 1..=6u8 {
            for opposing_die in 1..=6u8 {
                let mut board = Board::new(&map);
                board.set_influence(venezuela, Us, 3);
                board.set_influence(venezuela, Ussr, 3);
                let mut realignment = Realignment::new(Ussr, 1, &board);
                let mut dice = dice_rolling(&[acting_die, opposing_die]);
                realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
                assert!(board.influence(venezuela, Us) <= 3);
                assert!(board.influence(venezuela, Ussr) <= 3);
            }
        }
    }

    #[test]
    fn each_roll_costs_exactly_one_op() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        let mut realignment = Realignment::new(Ussr, 5, &board);
        let mut dice = Dice::from_seed(10);
        realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(realignment.ops_spent(), 1);
        assert_eq!(realignment.remaining(), 4);
    }

    #[test]
    fn rolling_with_no_ops_left_changes_nothing() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        let mut realignment = Realignment::new(Ussr, 1, &board);
        let mut dice = Dice::from_seed(11);
        realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(realignment.remaining(), 0);

        let (us_before, ussr_before) = (board.influence(venezuela, Us), board.influence(venezuela, Ussr));
        let history_len_before = realignment.history().len();

        let err = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, RealignError::InsufficientOps { country: "Venezuela".to_string(), remaining: 0 });
        assert_eq!(realignment.history().len(), history_len_before);
        assert_eq!(board.influence(venezuela, Us), us_before);
        assert_eq!(board.influence(venezuela, Ussr), ussr_before);
    }

    #[test]
    fn a_refused_roll_consumes_no_dice() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        let mut realignment = Realignment::new(Ussr, 5, &board);
        let mut dice = Dice::from_seed(555);
        let mut control = Dice::from_seed(555);

        // Illegal: no US influence in Venezuela yet.
        let err = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap_err();
        assert_eq!(err, RealignError::NoOpponentInfluence { country: "Venezuela".to_string(), side: Ussr });

        // Now make it legal and roll for real — the die drawn should be
        // exactly what a fresh die from the same seed rolls first,
        // proving the refused attempt above never touched the sequence.
        board.set_influence(venezuela, Us, 3);
        let result = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(result.acting_die, control.roll());
    }

    #[test]
    fn delta_reports_both_sides_after_a_win_then_a_loss_in_one_country() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        board.set_influence(venezuela, Ussr, 3);

        let mut realignment = Realignment::new(Ussr, 5, &board);
        let mut dice = dice_rolling(&[5, 3, 1, 6]);

        let first = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(first.loser, Some(Us));
        assert_eq!(board.influence(venezuela, Us), 1);

        let second = realignment.roll(&map, &mut board, venezuela, &mut dice).unwrap();
        assert_eq!(second.loser, Some(Ussr));
        assert_eq!(board.influence(venezuela, Ussr), 0);

        // A single signed number per country couldn't have represented
        // this: both sides' influence fell, by different amounts, from
        // two different rolls in the same country within one action.
        assert_eq!(realignment.delta(&board, venezuela, Us), -2);
        assert_eq!(realignment.delta(&board, venezuela, Ussr), -3);
    }

    // --- odds --------------------------------------------------------------

    #[test]
    fn probabilities_sum_to_thirty_six() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        board.set_influence(venezuela, Ussr, 3);
        let o = odds(&map, &board, venezuela, Ussr);
        assert_eq!(o.win as u16 + o.draw as u16 + o.loss as u16, 36);
    }

    #[test]
    fn even_modifiers_give_six_draws_and_fifteen_each_way() {
        // With equal modifiers on both sides, adding the same constant
        // to both totals never changes who wins — so the outcome
        // distribution matches two plain, unmodified d6.
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3);
        board.set_influence(venezuela, Ussr, 3);
        assert_eq!(modifiers(&map, &board, venezuela, Us).total(), 0);
        assert_eq!(modifiers(&map, &board, venezuela, Ussr).total(), 0);

        let o = odds(&map, &board, venezuela, Ussr);
        assert_eq!(o.draw, 6);
        assert_eq!(o.win, 15);
        assert_eq!(o.loss, 15);
    }

    #[test]
    fn expected_lost_is_zero_when_the_acting_side_has_no_influence_there() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 3); // opponent present; acting side has none
        let o = odds(&map, &board, venezuela, Ussr);
        assert!(o.loss > 0, "the acting side must still be able to lose the roll");
        assert_eq!(o.lost_36ths, 0, "but with nothing there, a loss costs nothing");
    }

    #[test]
    fn expected_removed_is_capped_by_the_opponents_actual_influence() {
        let map = map();
        let mut board = Board::new(&map);
        let venezuela = id(&map, "Venezuela");
        board.set_influence(venezuela, Us, 1); // every win can remove at most 1
        let o = odds(&map, &board, venezuela, Ussr);
        assert_eq!(o.removed_36ths, o.win as u16, "every winning pair removes exactly the 1 point available");
    }

    // --- ongoing events ----------------------------------------------------

    #[test]
    fn iran_contra_subtracts_one_from_the_us_total_and_says_so() {
        use crate::ongoing::{OngoingEffect, TurnEffects};
        let map = map();
        let board = Board::new(&map);
        let italy = id(&map, "Italy");
        let mut effects = TurnEffects::default();
        effects.apply(OngoingEffect::IranContra);
        let us = modifiers_with(&map, &board, italy, Us, &effects);
        assert!(us.iran_contra);
        assert_eq!(us.total(), modifiers(&map, &board, italy, Us).total() - 1);
        assert!(us.reasons().iter().any(|r| r.contains("Iran-Contra")));
        assert!(!modifiers_with(&map, &board, italy, Ussr, &effects).iran_contra);
        // The odds shift against the US with the penalty.
        let mut with_board = Board::new(&map);
        with_board.set_influence(italy, Ussr, 2);
        assert!(odds_with(&map, &with_board, italy, Us, &effects).win < odds(&map, &with_board, italy, Us).win);
    }

    #[test]
    fn a_bonus_op_is_available_only_while_every_roll_stays_in_the_sub_region() {
        use crate::ongoing::{OngoingEffect, TurnEffects};
        let map = map();
        let mut board = Board::new(&map);
        for name in ["Thailand", "Vietnam", "Poland"] {
            board.set_influence(id(&map, name), Us, 9);
        }
        let mut effects = TurnEffects::default();
        effects.apply(OngoingEffect::VietnamRevolts);
        let bonuses = effects.ops_bonuses(Ussr, CardId(8));
        let mut dice = Dice::from_seed(8);

        let mut r = Realignment::new(Ussr, 2, &board).with_effects(effects).with_bonuses(bonuses.clone());
        assert_eq!(r.bonuses().len(), 1);
        let mut live = board.clone();
        r.roll(&map, &mut live, id(&map, "Thailand"), &mut dice).unwrap();
        r.roll(&map, &mut live, id(&map, "Vietnam"), &mut dice).unwrap();
        assert!(r.can_afford(&map, id(&map, "Thailand")), "the bonus op");
        assert!(!r.can_afford(&map, id(&map, "Poland")), "not outside the sub-region");
        r.roll(&map, &mut live, id(&map, "Thailand"), &mut dice).unwrap();
        assert_eq!((r.ops_total(), r.remaining()), (3, 0));

        // One roll outside forfeits it for good.
        let mut r = Realignment::new(Ussr, 2, &board).with_effects(effects).with_bonuses(bonuses);
        let mut live = board.clone();
        r.roll(&map, &mut live, id(&map, "Poland"), &mut dice).unwrap();
        r.roll(&map, &mut live, id(&map, "Thailand"), &mut dice).unwrap();
        assert!(r.bonuses().is_empty());
        assert_eq!((r.ops_total(), r.remaining()), (2, 0));
    }

    #[test]
    fn the_china_cards_asia_op_covers_any_asian_country() {
        use crate::cards::CHINA_CARD;
        use crate::ongoing::TurnEffects;
        let map = map();
        let mut board = Board::new(&map);
        for name in ["Japan", "Thailand", "Poland"] {
            board.set_influence(id(&map, name), Us, 9);
        }
        let bonuses = TurnEffects::default().ops_bonuses(Ussr, CHINA_CARD);
        let mut dice = Dice::from_seed(3);
        let mut r = Realignment::new(Ussr, 1, &board).with_bonuses(bonuses);
        let mut live = board.clone();
        r.roll(&map, &mut live, id(&map, "Japan"), &mut dice).unwrap();
        assert!(r.can_afford(&map, id(&map, "Thailand")), "the Asia op");
        assert!(!r.can_afford(&map, id(&map, "Poland")));
        r.roll(&map, &mut live, id(&map, "Thailand"), &mut dice).unwrap();
        assert_eq!((r.ops_total(), r.remaining()), (2, 0));
    }
}
