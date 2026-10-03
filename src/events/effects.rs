//! Fixed-effect card events: the cards whose text only moves influence,
//! VP, or DEFCON by amounts the card itself dictates — no choice of
//! target, no die roll, and no reading of any other card's state. Pure
//! functions of `(map, board, status, card)`, mirroring
//! [`super::scoring`]: [`resolve`] computes an [`EffectResult`] and
//! applies nothing, so `Game::play_event` is what writes it to the real
//! board/status, the same split `ops::realign::resolve` keeps against
//! `Game::roll`.
//!
//! The "this event prevents/allows card #N" clauses on some of these
//! cards (Camp David Accords, John Paul II, ...) aren't modelled here:
//! they only matter once the card they refer to is implemented, and a
//! played event's card already lands in `Hands`'s removed pile, which is
//! all that later card needs to check.

use crate::board::Board;
use crate::cards::CardId;
use crate::country::{CountryId, Region, Superpower};
use crate::map::WorldMap;
use crate::status::GameStatus;

/// One country's influence for one side, before and after an event —
/// both kept (rather than just a delta) so the log line, the result
/// modal, and a test can all say what the event did without re-deriving
/// "before" from the board it was applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfluenceChange {
    pub country: CountryId,
    pub side: Superpower,
    pub before: u8,
    pub after: u8,
}

/// What resolving a fixed-effect card produced. `vp_delta` follows
/// [`GameStatus::vp`]'s convention (positive favours the US); `defcon` is
/// `(before, after)`, already clamped to the track, and `None` for a card
/// that doesn't touch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectResult {
    pub card: CardId,
    /// The side that played the event — the one a `Both` card's VP/DEFCON
    /// text ("the player receives...") refers to.
    pub player: Superpower,
    pub influence: Vec<InfluenceChange>,
    pub vp_delta: i8,
    pub defcon: Option<(u8, u8)>,
}

/// What one card's effect function sees and mutates: the board as it was
/// when the event was played (`before`), a working copy the card's
/// influence steps apply to in order (so a card touching the same country
/// twice — Fidel: wipe the US, then top up the USSR — sees its own
/// earlier step), and the status it was played under. Everything a card
/// can do goes through a method here, so a card's function reads as its
/// own rules text and `resolve` is the only place that assembles the
/// [`EffectResult`].
struct Ctx<'a> {
    map: &'a WorldMap,
    before: &'a Board,
    working: Board,
    status: &'a GameStatus,
    changes: Vec<InfluenceChange>,
    vp_delta: i8,
    defcon: Option<(u8, u8)>,
}

impl Ctx<'_> {
    /// The side playing the event.
    fn player(&self) -> Superpower {
        self.status.active
    }

    fn id(&self, name: &str) -> CountryId {
        self.map.id_by_name(name).unwrap_or_else(|| panic!("event refers to unknown country {name:?}"))
    }

    /// `side`'s influence in `name` when the event was played.
    fn influence(&self, name: &str, side: Superpower) -> u8 {
        self.before.influence(self.id(name), side)
    }

    fn set(&mut self, name: &str, side: Superpower, value: u8) {
        let country = self.id(name);
        let before = self.working.influence(country, side);
        if before != value {
            self.working.set_influence(country, side, value);
            self.changes.push(InfluenceChange { country, side, before, after: value });
        }
    }

    fn add(&mut self, name: &str, side: Superpower, n: u8) {
        let current = self.working.influence(self.id(name), side);
        self.set(name, side, current.saturating_add(n));
    }

    fn remove(&mut self, name: &str, side: Superpower, n: u8) {
        let current = self.working.influence(self.id(name), side);
        self.set(name, side, current.saturating_sub(n));
    }

    /// Removes all of `winner`'s opponent's influence from `name`, then
    /// raises `winner`'s to the least that controls it (its influence at
    /// least the opponent's plus stability) — Fidel and Romanian
    /// Abdication's shared text.
    fn take_control(&mut self, name: &str, winner: Superpower) {
        self.set(name, winner.opponent(), 0);
        let stability = self.map.country(self.id(name)).stability;
        let current = self.working.influence(self.id(name), winner);
        self.set(name, winner, current.max(stability));
    }

    /// Awards `n` VP to `side`, in [`GameStatus::vp`]'s signed convention.
    fn award_vp(&mut self, side: Superpower, n: i8) {
        self.vp_delta += match side {
            Superpower::Us => n,
            Superpower::Ussr => -n,
        };
    }

    /// Moves DEFCON to `after` (clamped to the track), recording the move.
    fn set_defcon(&mut self, after: u8) {
        self.defcon = Some((self.status.defcon, after.clamp(1, 5)));
    }

    /// Battleground countries `side` controls, optionally restricted to
    /// some regions.
    fn controlled_battlegrounds(&self, side: Superpower, regions: Option<&[Region]>) -> i8 {
        self.map
            .iter()
            .filter(|(id, c)| {
                c.battleground && regions.is_none_or(|rs| rs.contains(&c.region)) && self.before.is_controlled_by(self.map, *id, side)
            })
            .count() as i8
    }
}

/// One card's rules text as code.
type Effect = fn(&mut Ctx);

/// Every card this module resolves, by printed number. Adding a card is
/// one function below plus one line here — `is_effect_card` and `resolve`
/// both read this table, so there's no second list to keep in step.
const EFFECTS: &[(u8, Effect)] = &[
    (4, duck_and_cover),
    (8, fidel),
    (12, romanian_abdication),
    (15, nasser),
    (17, de_gaulle_leads_france),
    (34, nuclear_test_ban),
    (48, kitchen_debates),
    (52, portuguese_empire_crumbles),
    (54, allende),
    (64, panama_canal_returned),
    (65, camp_david_accords),
    (68, john_paul_ii_elected_pope),
    (72, sadat_expels_soviets),
    (78, alliance_for_progress),
    (82, iranian_hostage_crisis),
    (83, the_iron_lady),
    (84, reagan_bombs_libya),
    (97, an_evil_empire),
    (110, awacs_sale_to_saudis),
];

fn effect_for(card: CardId) -> Option<Effect> {
    EFFECTS.iter().find(|&&(n, _)| n == card.0).map(|&(_, f)| f)
}

pub fn is_effect_card(card: CardId) -> bool {
    effect_for(card).is_some()
}

/// Resolves `card`'s fixed effect against `(map, board, status)` — `None`
/// for a card [`is_effect_card`] doesn't recognise. Panics on a country
/// name the map doesn't know, which `tests` below pins for every card.
pub fn resolve(map: &WorldMap, board: &Board, status: &GameStatus, card: CardId) -> Option<EffectResult> {
    let effect = effect_for(card)?;
    let mut ctx = Ctx { map, before: board, working: board.clone(), status, changes: Vec::new(), vp_delta: 0, defcon: None };
    effect(&mut ctx);
    Some(EffectResult { card, player: status.active, influence: ctx.changes, vp_delta: ctx.vp_delta, defcon: ctx.defcon })
}

// ---- the cards, in printed-number order ----

/// #4 Duck and Cover: degrade DEFCON, then the US receives VP equal to 5 minus the new level.
fn duck_and_cover(c: &mut Ctx) {
    let after = c.status.defcon.saturating_sub(1).max(1);
    c.set_defcon(after);
    c.award_vp(Superpower::Us, 5 - after as i8);
}

/// #8 Fidel
fn fidel(c: &mut Ctx) {
    c.take_control("Cuba", Superpower::Ussr);
}

/// #12 Romanian Abdication
fn romanian_abdication(c: &mut Ctx) {
    c.take_control("Romania", Superpower::Ussr);
}

/// #15 Nasser: +2 USSR, then the US removes half (rounded up) of what it had.
fn nasser(c: &mut Ctx) {
    let us = c.influence("Egypt", Superpower::Us);
    c.add("Egypt", Superpower::Ussr, 2);
    c.remove("Egypt", Superpower::Us, us.div_ceil(2));
}

/// #17 De Gaulle Leads France
fn de_gaulle_leads_france(c: &mut Ctx) {
    c.remove("France", Superpower::Us, 2);
    c.add("France", Superpower::Ussr, 1);
}

/// #34 Nuclear Test Ban: the player receives VP from the *current* level, then improves it by 2.
fn nuclear_test_ban(c: &mut Ctx) {
    let player = c.player();
    c.award_vp(player, c.status.defcon.saturating_sub(2) as i8);
    c.set_defcon(c.status.defcon + 2);
}

/// #48 Kitchen Debates: 2 VP to the US if it controls strictly more battlegrounds worldwide.
fn kitchen_debates(c: &mut Ctx) {
    if c.controlled_battlegrounds(Superpower::Us, None) > c.controlled_battlegrounds(Superpower::Ussr, None) {
        c.award_vp(Superpower::Us, 2);
    }
}

/// #52 Portuguese Empire Crumbles
fn portuguese_empire_crumbles(c: &mut Ctx) {
    c.add("Angola", Superpower::Ussr, 2);
    c.add("SE African States", Superpower::Ussr, 2);
}

/// #54 Allende
fn allende(c: &mut Ctx) {
    c.add("Chile", Superpower::Ussr, 2);
}

/// #64 Panama Canal Returned
fn panama_canal_returned(c: &mut Ctx) {
    for name in ["Panama", "Costa Rica", "Venezuela"] {
        c.add(name, Superpower::Us, 1);
    }
}

/// #65 Camp David Accords
fn camp_david_accords(c: &mut Ctx) {
    c.award_vp(Superpower::Us, 1);
    for name in ["Israel", "Jordan", "Egypt"] {
        c.add(name, Superpower::Us, 1);
    }
}

/// #68 John Paul II Elected Pope
fn john_paul_ii_elected_pope(c: &mut Ctx) {
    c.remove("Poland", Superpower::Ussr, 2);
    c.add("Poland", Superpower::Us, 1);
}

/// #72 Sadat Expels Soviets
fn sadat_expels_soviets(c: &mut Ctx) {
    c.set("Egypt", Superpower::Ussr, 0);
    c.add("Egypt", Superpower::Us, 1);
}

/// #78 Alliance for Progress: 1 VP per US-controlled battleground in Central and South America.
fn alliance_for_progress(c: &mut Ctx) {
    let n = c.controlled_battlegrounds(Superpower::Us, Some(&[Region::CentralAmerica, Region::SouthAmerica]));
    c.award_vp(Superpower::Us, n);
}

/// #82 Iranian Hostage Crisis
fn iranian_hostage_crisis(c: &mut Ctx) {
    c.set("Iran", Superpower::Us, 0);
    c.add("Iran", Superpower::Ussr, 2);
}

/// #83 The Iron Lady
fn the_iron_lady(c: &mut Ctx) {
    c.add("Argentina", Superpower::Ussr, 1);
    c.set("UK", Superpower::Ussr, 0);
    c.award_vp(Superpower::Us, 1);
}

/// #84 Reagan Bombs Libya: 1 VP per 2 USSR influence in Libya, rounded down.
fn reagan_bombs_libya(c: &mut Ctx) {
    let n = c.influence("Libya", Superpower::Ussr) / 2;
    c.award_vp(Superpower::Us, n as i8);
}

/// #97 “An Evil Empire”
fn an_evil_empire(c: &mut Ctx) {
    c.award_vp(Superpower::Us, 1);
}

/// #110 AWACS Sale to Saudis
fn awacs_sale_to_saudis(c: &mut Ctx) {
    c.add("Saudi Arabia", Superpower::Us, 2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardCatalog;

    fn fixtures() -> (WorldMap, CardCatalog) {
        (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
    }

    #[test]
    fn every_effect_card_resolves_on_a_blank_board() {
        // Pins every hard-coded country name: `Builder::id` panics on a typo.
        let (map, _) = fixtures();
        let board = Board::new(&map);
        let status = GameStatus::default();
        for &(n, _) in EFFECTS {
            assert!(resolve(&map, &board, &status, CardId(n)).is_some(), "card #{n}");
        }
    }

    #[test]
    fn effect_cards_are_all_real_non_scoring_cards() {
        let (_, cards) = fixtures();
        for &(n, _) in EFFECTS {
            assert!(!cards.card(CardId(n)).scoring, "card #{n} is a scoring card");
        }
    }

    #[test]
    fn the_effect_table_has_no_duplicate_ids() {
        let mut ids: Vec<u8> = EFFECTS.iter().map(|&(n, _)| n).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), EFFECTS.len());
    }

    #[test]
    fn an_unrecognised_card_resolves_to_none() {
        let (map, _) = fixtures();
        let board = Board::new(&map);
        assert!(resolve(&map, &board, &GameStatus::default(), CardId(5)).is_none());
        assert!(!is_effect_card(CardId(5)));
    }
}
