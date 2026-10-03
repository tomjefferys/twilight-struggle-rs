//! The five war cards (#11 Korean War, #13 Arab-Israeli War, #24
//! Indo-Pakistani War, #36 Brush War, #102 Iran-Iraq War): the first
//! events that roll a die.
//!
//! All five read the same way — pick a target, roll 1d6, subtract 1 for
//! every adjacent country the beneficiary's opponent controls, and on a
//! modified roll of `success_min` or better the beneficiary gains VP and
//! replaces all of its opponent's influence there with its own. Every war
//! also adds to the beneficiary's Military Operations track. The cards
//! differ only in a [`WarSpec`]: who benefits, which countries may be
//! targeted, and the numbers.
//!
//! Shaped like a coup rather than like [`super::choice`]: one immediate,
//! irreversible roll. [`resolve`] is a pure function of
//! `(map, board, card, target, die, side)` — mirroring
//! `ops::coup::coup_resolve` — so a preview or a test needs no session;
//! [`War`] is the open session a chosen-target war carries as
//! `Operation::War` until `Game::roll` resolves it.

use super::effects::InfluenceChange;
use crate::board::Board;
use crate::cards::CardId;
use crate::country::{CountryId, Superpower};
use crate::map::WorldMap;

/// Who a war card benefits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beneficiary {
    /// Always this side, whoever plays the card (Korean War).
    Fixed(Superpower),
    /// Whoever plays the event.
    Player,
}

/// Which countries a war may be launched against.
#[derive(Debug, Clone, Copy)]
pub enum Targets {
    Names(&'static [&'static str]),
    /// Any country with stability this low or lower.
    MaxStability(u8),
}

#[derive(Debug, Clone, Copy)]
pub struct WarSpec {
    pub beneficiary: Beneficiary,
    pub targets: Targets,
    /// The modified roll needed to win.
    pub success_min: u8,
    pub vp: i8,
    pub mil_ops: i8,
    /// The target itself, if the enemy controls it, also lowers the roll
    /// (Arab-Israeli War's "for Israel, if it is US controlled").
    pub target_itself_counts: bool,
}

const WARS: &[(u8, WarSpec)] = &[
    (
        11,
        WarSpec {
            beneficiary: Beneficiary::Fixed(Superpower::Ussr),
            targets: Targets::Names(&["South Korea"]),
            success_min: 4,
            vp: 2,
            mil_ops: 2,
            target_itself_counts: false,
        },
    ),
    (
        13,
        WarSpec {
            beneficiary: Beneficiary::Fixed(Superpower::Ussr),
            targets: Targets::Names(&["Israel"]),
            success_min: 4,
            vp: 2,
            mil_ops: 2,
            target_itself_counts: true,
        },
    ),
    (
        24,
        WarSpec {
            beneficiary: Beneficiary::Player,
            targets: Targets::Names(&["India", "Pakistan"]),
            success_min: 4,
            vp: 2,
            mil_ops: 2,
            target_itself_counts: false,
        },
    ),
    (
        36,
        WarSpec {
            beneficiary: Beneficiary::Player,
            targets: Targets::MaxStability(2),
            success_min: 3,
            vp: 1,
            mil_ops: 3,
            target_itself_counts: false,
        },
    ),
    (
        102,
        WarSpec {
            beneficiary: Beneficiary::Player,
            targets: Targets::Names(&["Iran", "Iraq"]),
            success_min: 4,
            vp: 2,
            mil_ops: 2,
            target_itself_counts: false,
        },
    ),
];

/// The Military Operations track's own bounds (rule 6.3.4's 0-5 track),
/// kept in `GameStatus` as a signed value.
pub const MIL_OPS_MAX: i8 = 5;

pub fn spec_for(card: CardId) -> Option<WarSpec> {
    WARS.iter().find(|&&(n, _)| n == card.0).map(|&(_, s)| s)
}

pub fn is_war_card(card: CardId) -> bool {
    spec_for(card).is_some()
}

/// Who gains from `card` when `player` plays it.
pub fn beneficiary(spec: &WarSpec, player: Superpower) -> Superpower {
    match spec.beneficiary {
        Beneficiary::Fixed(side) => side,
        Beneficiary::Player => player,
    }
}

/// Every country `card` may be launched against.
pub fn eligible_targets(map: &WorldMap, card: CardId) -> Vec<CountryId> {
    let Some(spec) = spec_for(card) else { return Vec::new() };
    map.iter()
        .filter(|(id, c)| match spec.targets {
            Targets::Names(names) => names.iter().any(|n| map.id_by_name(n) == Some(*id)),
            Targets::MaxStability(max) => c.stability <= max,
        })
        .map(|(id, _)| id)
        .collect()
}

/// What lowers the die roll: each enemy-controlled neighbour of the
/// target, plus the target itself where the card says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarModifier {
    pub neighbours: Vec<CountryId>,
    pub target_itself: bool,
}

impl WarModifier {
    /// The (non-positive) adjustment to the die.
    pub fn total(&self) -> i8 {
        -(self.neighbours.len() as i8 + self.target_itself as i8)
    }
}

pub fn modifier(map: &WorldMap, board: &Board, card: CardId, target: CountryId, beneficiary: Superpower) -> WarModifier {
    let enemy = beneficiary.opponent();
    let spec = spec_for(card);
    WarModifier {
        neighbours: map.country(target).adjacent.iter().copied().filter(|&n| board.is_controlled_by(map, n, enemy)).collect(),
        target_itself: spec.is_some_and(|s| s.target_itself_counts) && board.is_controlled_by(map, target, enemy),
    }
}

/// Winning rolls out of 6, for a preview.
pub fn odds(map: &WorldMap, board: &Board, card: CardId, target: CountryId, beneficiary: Superpower) -> u8 {
    let Some(spec) = spec_for(card) else { return 0 };
    let m = modifier(map, board, card, target, beneficiary).total();
    (1..=6u8).filter(|&d| d as i8 + m >= spec.success_min as i8).count() as u8
}

/// One resolved war.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarResult {
    pub card: CardId,
    pub side: Superpower,
    pub target: CountryId,
    pub die: u8,
    pub modifier: WarModifier,
    pub success_min: u8,
    pub success: bool,
    pub influence: Vec<InfluenceChange>,
    /// Signed like `GameStatus::vp` (positive favours the US).
    pub vp_delta: i8,
    pub mil_ops: i8,
}

impl WarResult {
    /// The modified roll.
    pub fn modified(&self) -> i8 {
        self.die as i8 + self.modifier.total()
    }
}

/// Resolves `card` against `target` with a given die — pure; applies
/// nothing. `player` is whoever plays the event. `None` for a card that
/// isn't a war.
pub fn resolve(map: &WorldMap, board: &Board, card: CardId, target: CountryId, player: Superpower, die: u8) -> Option<WarResult> {
    let spec = spec_for(card)?;
    let side = beneficiary(&spec, player);
    let modifier = modifier(map, board, card, target, side);
    let success = die as i8 + modifier.total() >= spec.success_min as i8;
    let mut influence = Vec::new();
    let mut vp_delta = 0;
    if success {
        let theirs = board.influence(target, side.opponent());
        let ours = board.influence(target, side);
        if theirs > 0 {
            influence.push(InfluenceChange { country: target, side: side.opponent(), before: theirs, after: 0 });
            influence.push(InfluenceChange { country: target, side, before: ours, after: ours + theirs });
        }
        vp_delta = match side {
            Superpower::Us => spec.vp,
            Superpower::Ussr => -spec.vp,
        };
    }
    Some(WarResult { card, side, target, die, modifier, success_min: spec.success_min, success, influence, vp_delta, mil_ops: spec.mil_ops })
}

/// Why a war launch was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarError {
    NotATarget { country: String },
}

impl std::fmt::Display for WarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarError::NotATarget { country } => write!(f, "{country} isn't a legal target for this war"),
        }
    }
}

impl std::error::Error for WarError {}

/// An open chosen-target war, awaiting `Game::roll` on its target.
#[derive(Debug, Clone)]
pub struct War {
    card: CardId,
    /// Whoever played the event (the beneficiary, unless the card fixes it).
    player: Superpower,
}

impl War {
    pub fn new(card: CardId, player: Superpower) -> Self {
        War { card, player }
    }

    pub fn card(&self) -> CardId {
        self.card
    }

    pub fn player(&self) -> Superpower {
        self.player
    }

    /// The side the war benefits.
    pub fn side(&self) -> Superpower {
        spec_for(self.card).map_or(self.player, |s| beneficiary(&s, self.player))
    }

    pub fn success_min(&self) -> u8 {
        spec_for(self.card).map_or(4, |s| s.success_min)
    }

    pub fn is_legal_target(&self, map: &WorldMap, id: CountryId) -> bool {
        eligible_targets(map, self.card).contains(&id)
    }

    pub fn modifier(&self, map: &WorldMap, board: &Board, id: CountryId) -> WarModifier {
        modifier(map, board, self.card, id, self.side())
    }

    pub fn odds(&self, map: &WorldMap, board: &Board, id: CountryId) -> u8 {
        odds(map, board, self.card, id, self.side())
    }

    /// Resolves the war on `id` with `die`, or refuses an ineligible target.
    pub fn resolve(&self, map: &WorldMap, board: &Board, id: CountryId, die: u8) -> Result<WarResult, WarError> {
        if !self.is_legal_target(map, id) {
            return Err(WarError::NotATarget { country: map.country(id).name.clone() });
        }
        Ok(resolve(map, board, self.card, id, self.player, die).expect("a War is only opened for a war card"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap()
    }

    #[test]
    fn every_war_card_has_resolvable_targets() {
        let map = map();
        for &(n, _) in WARS {
            assert!(!eligible_targets(&map, CardId(n)).is_empty(), "card {n}");
        }
    }

    #[test]
    fn korean_war_wins_on_four_and_replaces_us_influence() {
        let map = map();
        let mut board = Board::new(&map);
        let sk = id(&map, "South Korea");
        board.set_influence(sk, Superpower::Us, 3);
        board.set_influence(sk, Superpower::Ussr, 1);
        let r = resolve(&map, &board, CardId(11), sk, Superpower::Us, 4).unwrap();
        assert!(r.success);
        assert_eq!(r.side, Superpower::Ussr);
        assert_eq!(r.vp_delta, -2);
        assert_eq!(r.influence.len(), 2);
        assert_eq!((r.influence[1].side, r.influence[1].after), (Superpower::Ussr, 4));
        let fail = resolve(&map, &board, CardId(11), sk, Superpower::Us, 3).unwrap();
        assert!(!fail.success);
        assert!(fail.influence.is_empty() && fail.vp_delta == 0);
    }

    #[test]
    fn enemy_controlled_neighbours_lower_the_roll() {
        let map = map();
        let mut board = Board::new(&map);
        let sk = id(&map, "South Korea");
        // Japan and Taiwan border South Korea; the US controlling both is -2.
        for n in ["Japan", "Taiwan"] {
            let c = id(&map, n);
            board.set_influence(c, Superpower::Us, 9);
        }
        let m = modifier(&map, &board, CardId(11), sk, Superpower::Ussr);
        assert_eq!(m.total(), -2);
        assert!(!resolve(&map, &board, CardId(11), sk, Superpower::Ussr, 5).unwrap().success);
        assert!(resolve(&map, &board, CardId(11), sk, Superpower::Ussr, 6).unwrap().success);
        assert_eq!(odds(&map, &board, CardId(11), sk, Superpower::Ussr), 1);
    }

    #[test]
    fn arab_israeli_counts_israel_itself_but_korean_does_not_count_its_target() {
        let map = map();
        let mut board = Board::new(&map);
        let israel = id(&map, "Israel");
        board.set_influence(israel, Superpower::Us, 9);
        assert_eq!(modifier(&map, &board, CardId(13), israel, Superpower::Ussr).total(), -1);
        let sk = id(&map, "South Korea");
        board.set_influence(sk, Superpower::Us, 9);
        assert_eq!(modifier(&map, &board, CardId(11), sk, Superpower::Ussr).total(), 0);
    }

    #[test]
    fn brush_war_wins_on_three_pays_one_vp_and_targets_low_stability_only() {
        let map = map();
        let targets = eligible_targets(&map, CardId(36));
        assert!(targets.iter().all(|&t| map.country(t).stability <= 2));
        assert!(!targets.contains(&id(&map, "Poland")));
        let mut board = Board::new(&map);
        let t = targets[0];
        board.set_influence(t, Superpower::Ussr, 2);
        let r = resolve(&map, &board, CardId(36), t, Superpower::Us, 3).unwrap();
        assert!(r.success && r.vp_delta == 1 && r.mil_ops == 3);
        assert!(!resolve(&map, &board, CardId(36), t, Superpower::Us, 2).unwrap().success);
    }

    #[test]
    fn a_win_with_no_enemy_influence_still_pays_vp() {
        let map = map();
        let board = Board::new(&map);
        let r = resolve(&map, &board, CardId(24), id(&map, "India"), Superpower::Ussr, 6).unwrap();
        assert!(r.success && r.influence.is_empty() && r.vp_delta == -2);
    }

    #[test]
    fn player_beneficiary_follows_who_plays() {
        let map = map();
        let board = Board::new(&map);
        let t = id(&map, "Iraq");
        assert_eq!(resolve(&map, &board, CardId(102), t, Superpower::Us, 6).unwrap().vp_delta, 2);
        assert_eq!(resolve(&map, &board, CardId(102), t, Superpower::Ussr, 6).unwrap().vp_delta, -2);
    }

}
