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
use crate::space;
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

/// An event passing the China Card to `to`, face up (playable at once) or
/// face down (not until the turn rolls over).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChinaTransfer {
    pub to: Superpower,
    pub face_up: bool,
}

/// A hand an event makes its owner show (CIA Created, "Lone Gunman"):
/// whose, and — filled in by `Game` once the event applies, since the
/// effect itself never sees the hands — which cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reveal {
    pub side: Superpower,
    pub cards: Vec<CardId>,
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
    /// The China Card changing hands (Cultural Revolution, Nixon, Ussuri River Skirmish).
    pub china: Option<ChinaTransfer>,
    /// The space race marker moving: the side, and the box it was at and moves to.
    pub space: Option<(Superpower, u8, u8)>,
    /// Military Operations the `player` gains (How I Learned to Stop Worrying); applied clamped to the track.
    pub mil_ops: i8,
    /// The event ends the game outright, the VP leader winning (Wargames).
    pub ends_game: bool,
    /// A hand the event reveals.
    pub reveals: Option<Reveal>,
    /// Cards discarded from a side's hand: paid to avoid Blockade's penalty, picked
    /// out of the US hand by Aldrich Ames, or lost at random to Terrorism.
    pub discards: Vec<(Superpower, CardId)>,
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
    china: Option<ChinaTransfer>,
    space: Option<(Superpower, u8, u8)>,
    reveals: Option<Reveal>,
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

    /// Advances `side`'s space race marker by up to `n` boxes (stopping at
    /// the end of the track), awarding each box's arrival VP — or only the
    /// last box's, with `vp_from_last_only`.
    fn advance_space(&mut self, side: Superpower, n: u8, vp_from_last_only: bool) {
        let from = space::position(self.status, side);
        let to = (from + n).min(space::MAX_BOX);
        if to == from {
            return;
        }
        let first = if vp_from_last_only { to } else { from + 1 };
        for b in first..=to {
            self.vp_delta += space::arrival_vp(self.status, side, b);
        }
        self.space = Some((side, from, to));
    }

    /// Makes `side` show their hand.
    fn reveal_hand(&mut self, side: Superpower) {
        self.reveals = Some(Reveal { side, cards: Vec::new() });
    }

    /// Whether `side` controlled `name` when the event was played.
    fn controls(&self, name: &str, side: Superpower) -> bool {
        self.before.is_controlled_by(self.map, self.id(name), side)
    }

    /// Who holds the China Card, face up or down.
    fn china_holder(&self) -> Superpower {
        self.status.china_card
    }

    /// Passes the China Card to `to`.
    fn give_china(&mut self, to: Superpower, face_up: bool) {
        self.china = Some(ChinaTransfer { to, face_up });
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
    (10, blockade),
    (12, romanian_abdication),
    (15, nasser),
    (17, de_gaulle_leads_france),
    (18, captured_nazi_scientist),
    (21, nato),
    (26, cia_created),
    (25, containment),
    (31, red_scare_purge),
    (27, us_japan_mutual_defense_pact),
    (34, nuclear_test_ban),
    (39, arms_race),
    (35, formosan_resolution),
    (80, one_small_step),
    (41, nuclear_subs),
    (48, kitchen_debates),
    (50, we_will_bury_you),
    (51, brezhnev_doctrine),
    (52, portuguese_empire_crumbles),
    (54, allende),
    (55, willy_brandt),
    (57, abm_treaty),
    (58, cultural_revolution),
    (59, flower_power),
    (60, u2_incident),
    (61, opec),
    (62, lone_gunman),
    (64, panama_canal_returned),
    (65, camp_david_accords),
    (68, john_paul_ii_elected_pope),
    (69, latin_american_death_squads),
    (71, nixon_plays_the_china_card),
    (72, sadat_expels_soviets),
    (78, alliance_for_progress),
    (82, iranian_hostage_crisis),
    (83, the_iron_lady),
    (84, reagan_bombs_libya),
    (73, shuttle_diplomacy),
    (86, north_sea_oil),
    (89, soviets_shoot_down_kal_007),
    (90, glasnost),
    (91, ortega_elected_in_nicaragua),
    (92, terrorism),
    (93, iran_contra_scandal),
    (96, tear_down_this_wall),
    (97, an_evil_empire),
    (98, aldrich_ames_remix),
    (101, solidarity),
    (103, defectors),
    (104, the_cambridge_five),
    (107, che),
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
    let mut ctx = Ctx { map, before: board, working: board.clone(), status, changes: Vec::new(), vp_delta: 0, defcon: None, ongoing: None, lasting: None, cancels: None, china: None, space: None, reveals: None };
    effect(&mut ctx);
    Some(EffectResult { card, player: status.active, influence: ctx.changes, vp_delta: ctx.vp_delta, defcon: ctx.defcon, ongoing: ctx.ongoing, lasting: ctx.lasting, cancels: ctx.cancels, china: ctx.china, space: ctx.space, mil_ops: 0, ends_game: false, reveals: ctx.reveals, discards: Vec::new() })
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

/// #10 Blockade, once the US has declined (or been unable) to discard a 3+ ops card: all US
/// influence leaves West Germany. (The decision itself is `EventChoice::discard_gate`.)
fn blockade(c: &mut Ctx) {
    c.set("West Germany", Superpower::Us, 0);
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

/// #18 Captured Nazi Scientist: the player advances one box on the space race.
fn captured_nazi_scientist(c: &mut Ctx) {
    let player = c.player();
    c.advance_space(player, 1, false);
}

/// #25 Containment: US ops cards get +1 ops (max 4) for the rest of the turn.
fn containment(c: &mut Ctx) {
    c.start(OngoingEffect::Containment);
}

/// #80 One Small Step: if behind on the space race, advance two boxes — VP only from the last.
fn one_small_step(c: &mut Ctx) {
    let player = c.player();
    if space::position(c.status, player) < space::position(c.status, player.opponent()) {
        c.advance_space(player, 2, true);
    }
}

/// #31 Red Scare/Purge: the opponent's ops cards get -1 ops (min 1) for the rest of the turn.
fn red_scare_purge(c: &mut Ctx) {
    let penalised = c.player().opponent();
    c.start(OngoingEffect::RedScare { penalised });
}

/// #26 CIA Created: the USSR reveals its hand; the US may then use the card's ops (`events::ops_grant`).
fn cia_created(c: &mut Ctx) {
    c.reveal_hand(Superpower::Ussr);
    c.start(OngoingEffect::HandRevealed { side: Superpower::Ussr, card: 26 });
}

/// #34 Nuclear Test Ban: the player receives VP from the *current* level, then improves it by 2.
fn nuclear_test_ban(c: &mut Ctx) {
    let player = c.player();
    c.award_vp(player, c.status.defcon.saturating_sub(2) as i8);
    c.set_defcon(c.status.defcon + 2);
}

/// #39 Arms Race: the player is ahead on Military Operations → 1 VP, or 3 if they've also met the
/// required amount (the DEFCON level).
fn arms_race(c: &mut Ctx) {
    let player = c.player();
    let mil = |side| match side {
        Superpower::Us => c.status.military_ops_us,
        Superpower::Ussr => c.status.military_ops_ussr,
    };
    let (mine, theirs) = (mil(player), mil(player.opponent()));
    if mine > theirs {
        let vp = if mine >= c.status.defcon as i8 { 3 } else { 1 };
        c.award_vp(player, vp);
    }
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

/// #60 U2 Incident: USSR +1 VP (the extra VP if #32 follows this turn waits for UN Intervention).
fn u2_incident(c: &mut Ctx) {
    c.award_vp(Superpower::Ussr, 1);
}

/// #61 OPEC: the USSR gets 1 VP per controlled country among seven oil producers
/// (barred once #86 has been played — see `events::blocked`).
fn opec(c: &mut Ctx) {
    let n = ["Egypt", "Iran", "Libya", "Saudi Arabia", "Iraq", "Gulf States", "Venezuela"]
        .into_iter()
        .filter(|name| c.controls(name, Superpower::Ussr))
        .count();
    c.award_vp(Superpower::Ussr, n as i8);
}

/// #62 “Lone Gunman”: the US reveals its hand; the USSR may then use the card's ops.
fn lone_gunman(c: &mut Ctx) {
    c.reveal_hand(Superpower::Us);
    c.start(OngoingEffect::HandRevealed { side: Superpower::Us, card: 62 });
}

/// #64 Panama Canal Returned
fn panama_canal_returned(c: &mut Ctx) {
    for name in ["Panama", "Costa Rica", "Venezuela"] {
        c.add(name, Superpower::Us, 1);
    }
}

/// #57 ABM Treaty: improve DEFCON by 1; the player may then conduct operations with the card.
fn abm_treaty(c: &mut Ctx) {
    c.set_defcon(c.status.defcon + 1);
}

/// #58 Cultural Revolution: the US gives up the China Card (face up); if the USSR already holds it, +1 VP.
fn cultural_revolution(c: &mut Ctx) {
    if c.china_holder() == Superpower::Us {
        c.give_china(Superpower::Ussr, true);
    } else {
        c.award_vp(Superpower::Ussr, 1);
    }
}

/// #71 Nixon Plays the China Card: the USSR gives it up (face down); if the US already holds it, +2 VP.
fn nixon_plays_the_china_card(c: &mut Ctx) {
    if c.china_holder() == Superpower::Ussr {
        c.give_china(Superpower::Us, false);
    } else {
        c.award_vp(Superpower::Us, 2);
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

/// #89 Soviets Shoot Down KAL-007: DEFCON -1 and 2 VP to the US; with South Korea
/// US-controlled the US may then place influence or realign with the card.
fn soviets_shoot_down_kal_007(c: &mut Ctx) {
    c.set_defcon(c.status.defcon.saturating_sub(1));
    c.award_vp(Superpower::Us, 2);
}

/// #90 Glasnost: DEFCON +1 and 2 VP to the USSR; once The Reformer has been played the
/// USSR may then place influence or realign with the card.
fn glasnost(c: &mut Ctx) {
    c.set_defcon(c.status.defcon + 1);
    c.award_vp(Superpower::Ussr, 2);
}

/// #91 Ortega Elected in Nicaragua: all US influence leaves Nicaragua; the USSR may then make a
/// free coup in a country adjacent to it (`events::ops_grant`).
fn ortega_elected_in_nicaragua(c: &mut Ctx) {
    c.set("Nicaragua", Superpower::Us, 0);
}

/// #92 Terrorism: the opponent discards 1 card at random (2 for the US once #82 has been
/// played). Chance is `Game::play_event_with`'s business, so nothing happens here.
fn terrorism(_: &mut Ctx) {}

/// #93 Iran-Contra Scandal: US realignment rolls get -1 for the rest of the turn.
fn iran_contra_scandal(c: &mut Ctx) {
    c.start(OngoingEffect::IranContra);
}

/// #96 Tear Down this Wall: +3 US in East Germany, and Willy Brandt (#55) ends; the US may then
/// make a free coup or realignment in Europe (`events::ops_grant`).
fn tear_down_this_wall(c: &mut Ctx) {
    c.add("East Germany", Superpower::Us, 3);
    c.cancel(LastingEffect::WillyBrandt);
}

/// #97 “An Evil Empire”
fn an_evil_empire(c: &mut Ctx) {
    c.award_vp(Superpower::Us, 1);
    c.cancel(LastingEffect::FlowerPower);
}

/// #107 Che: no direct effect — the USSR may make a coup in a non-battleground country in Central
/// America, South America or Africa, and a second if the first removes US influence
/// (`events::ops_grant`).
fn che(_: &mut Ctx) {}

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

/// #98 Aldrich Ames Remix with no US cards to discard: the US hand (empty) is still open
/// to the USSR for the rest of the turn. (With cards, `EventChoice::pick_from_hand`.)
fn aldrich_ames_remix(c: &mut Ctx) {
    c.reveal_hand(Superpower::Us);
    c.start(OngoingEffect::HandRevealed { side: Superpower::Us, card: 98 });
}

/// #104 The Cambridge Five with no region to add influence to: the US scoring cards are
/// revealed and that is all. (Otherwise `EventChoice::in_named_regions`.)
fn the_cambridge_five(c: &mut Ctx) {
    c.reveal_hand(Superpower::Us);
}

/// #103 Defectors: played in an action round by the USSR, the US gets 1 VP. (The headline
/// half — cancelling the USSR's headline event — waits for a headline phase.)
fn defectors(c: &mut Ctx) {
    if c.player() == Superpower::Ussr {
        c.award_vp(Superpower::Us, 1);
    }
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
