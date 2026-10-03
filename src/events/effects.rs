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
use crate::ongoing::{LastingEffect, OngoingEffect};
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
    /// A turn-long effect the event starts (see [`crate::ongoing`]).
    pub ongoing: Option<OngoingEffect>,
    /// A game-long effect the event starts, or one it ends (see [`crate::ongoing`]).
    pub lasting: Option<LastingEffect>,
    pub cancels: Option<LastingEffect>,
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
    ongoing: Option<OngoingEffect>,
    lasting: Option<LastingEffect>,
    cancels: Option<LastingEffect>,
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

    /// Starts a turn-long effect, in force until the turn ends.
    fn start(&mut self, effect: OngoingEffect) {
        self.ongoing = Some(effect);
    }

    /// Starts a game-long effect.
    fn persist(&mut self, effect: LastingEffect) {
        self.lasting = Some(effect);
    }

    /// Ends another card's game-long effect.
    fn cancel(&mut self, effect: LastingEffect) {
        self.cancels = Some(effect);
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
    (9, vietnam_revolts),
    (12, romanian_abdication),
    (15, nasser),
    (17, de_gaulle_leads_france),
    (21, nato),
    (25, containment),
    (31, red_scare_purge),
    (27, us_japan_mutual_defense_pact),
    (34, nuclear_test_ban),
    (35, formosan_resolution),
    (41, nuclear_subs),
    (48, kitchen_debates),
    (50, we_will_bury_you),
    (51, brezhnev_doctrine),
    (52, portuguese_empire_crumbles),
    (54, allende),
    (55, willy_brandt),
    (59, flower_power),
    (64, panama_canal_returned),
    (65, camp_david_accords),
    (68, john_paul_ii_elected_pope),
    (69, latin_american_death_squads),
    (72, sadat_expels_soviets),
    (78, alliance_for_progress),
    (82, iranian_hostage_crisis),
    (83, the_iron_lady),
    (84, reagan_bombs_libya),
    (73, shuttle_diplomacy),
    (86, north_sea_oil),
    (93, iran_contra_scandal),
    (97, an_evil_empire),
    (101, solidarity),
    (109, yuri_and_samantha),
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
    let mut ctx = Ctx { map, before: board, working: board.clone(), status, changes: Vec::new(), vp_delta: 0, defcon: None, ongoing: None, lasting: None, cancels: None };
    effect(&mut ctx);
    Some(EffectResult { card, player: status.active, influence: ctx.changes, vp_delta: ctx.vp_delta, defcon: ctx.defcon, ongoing: ctx.ongoing, lasting: ctx.lasting, cancels: ctx.cancels })
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

/// #9 Vietnam Revolts: +2 USSR in Vietnam; ops spent wholly in Southeast Asia get +1 this turn.
fn vietnam_revolts(c: &mut Ctx) {
    c.add("Vietnam", Superpower::Ussr, 2);
    c.start(OngoingEffect::VietnamRevolts);
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
    c.persist(LastingEffect::DeGaulle);
}

/// #25 Containment: US ops cards get +1 ops (max 4) for the rest of the turn.
fn containment(c: &mut Ctx) {
    c.start(OngoingEffect::Containment);
}

/// #31 Red Scare/Purge: the opponent's ops cards get -1 ops (min 1) for the rest of the turn.
fn red_scare_purge(c: &mut Ctx) {
    let penalised = c.player().opponent();
    c.start(OngoingEffect::RedScare { penalised });
}

/// #34 Nuclear Test Ban: the player receives VP from the *current* level, then improves it by 2.
fn nuclear_test_ban(c: &mut Ctx) {
    let player = c.player();
    c.award_vp(player, c.status.defcon.saturating_sub(2) as i8);
    c.set_defcon(c.status.defcon + 2);
}

/// #41 Nuclear Subs: US battleground coups don't degrade DEFCON for the rest of the turn.
fn nuclear_subs(c: &mut Ctx) {
    c.start(OngoingEffect::NuclearSubs);
}

/// #48 Kitchen Debates: 2 VP to the US if it controls strictly more battlegrounds worldwide.
fn kitchen_debates(c: &mut Ctx) {
    if c.controlled_battlegrounds(Superpower::Us, None) > c.controlled_battlegrounds(Superpower::Ussr, None) {
        c.award_vp(Superpower::Us, 2);
    }
}

/// #51 Brezhnev Doctrine: USSR ops cards get +1 ops (max 4) for the rest of the turn.
fn brezhnev_doctrine(c: &mut Ctx) {
    c.start(OngoingEffect::Brezhnev);
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

/// #69 Latin American Death Squads: the player's coups in Central/South America get +1, the opponent's -1.
fn latin_american_death_squads(c: &mut Ctx) {
    let beneficiary = c.player();
    c.start(OngoingEffect::DeathSquads { beneficiary });
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

/// #86 North Sea Oil: the US plays an eighth action round this turn.
fn north_sea_oil(c: &mut Ctx) {
    c.start(OngoingEffect::NorthSeaOil);
}

/// #93 Iran-Contra Scandal: US realignment rolls get -1 for the rest of the turn.
fn iran_contra_scandal(c: &mut Ctx) {
    c.start(OngoingEffect::IranContra);
}

/// #97 “An Evil Empire”
fn an_evil_empire(c: &mut Ctx) {
    c.award_vp(Superpower::Us, 1);
    c.cancel(LastingEffect::FlowerPower);
}

/// #109 Yuri and Samantha: the USSR gets 1 VP per US coup for the rest of the turn.
fn yuri_and_samantha(c: &mut Ctx) {
    c.start(OngoingEffect::YuriSamantha);
}

/// #110 AWACS Sale to Saudis
fn awacs_sale_to_saudis(c: &mut Ctx) {
    c.add("Saudi Arabia", Superpower::Us, 2);
}

/// #21 NATO: the USSR can't coup or realign US-controlled Europe.
fn nato(c: &mut Ctx) {
    c.persist(LastingEffect::Nato);
}

/// #27 US/Japan Mutual Defense Pact: the US takes control of Japan; the USSR can't coup or realign it.
fn us_japan_mutual_defense_pact(c: &mut Ctx) {
    c.take_control("Japan", Superpower::Us);
    c.persist(LastingEffect::UsJapan);
}

/// #35 Formosan Resolution: Taiwan scores as a battleground while US-controlled, until the US plays the China Card.
fn formosan_resolution(c: &mut Ctx) {
    c.persist(LastingEffect::Formosan);
}

/// #50 "We Will Bury You": DEFCON -1; the USSR gets 3 VP when the US finishes its next action round.
fn we_will_bury_you(c: &mut Ctx) {
    c.set_defcon(c.status.defcon.saturating_sub(1));
    // Played by the US (as its own round), the "next" round is the one after.
    c.persist(LastingEffect::WeWillBuryYou { skip: (c.player() == Superpower::Us) as u8 });
}

/// #55 Willy Brandt: USSR +1 VP and +1 West Germany; NATO no longer protects West Germany.
fn willy_brandt(c: &mut Ctx) {
    c.award_vp(Superpower::Ussr, 1);
    c.add("West Germany", Superpower::Ussr, 1);
    c.persist(LastingEffect::WillyBrandt);
}

/// #59 Flower Power: the USSR gets 2 VP for each war card the US plays from now on.
fn flower_power(c: &mut Ctx) {
    c.persist(LastingEffect::FlowerPower);
}

/// #73 Shuttle Diplomacy: the next Asia/Middle East scoring counts one fewer USSR battleground.
fn shuttle_diplomacy(c: &mut Ctx) {
    c.persist(LastingEffect::ShuttleDiplomacy);
}

/// #101 Solidarity (needs #68 first — see `events::blocked`).
fn solidarity(c: &mut Ctx) {
    c.add("Poland", Superpower::Us, 3);
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
