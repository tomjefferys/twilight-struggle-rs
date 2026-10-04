//! Card events that last "for the remainder of this turn": Containment,
//! Brezhnev Doctrine, Nuclear Subs, and so on. [`TurnEffects`] is the
//! state they leave behind — plain, `Copy` data held in
//! [`crate::status::GameStatus`] (so it saves/loads with a scenario and
//! `Game` stays cheap to clone) and cleared by `Game::advance` when the
//! turn rolls over. It holds no rules of its own beyond the small,
//! pure queries the ops code asks of it (`card_ops`, `coup_roll_mods`,
//! ...), so each rule reads in one place and a view can explain the
//! number it shows by asking the same question.

use serde::{Deserialize, Serialize};

use crate::board::Board;
use crate::cards::CardId;
use crate::country::{Area, Country, CountryId, Region, SubRegion, Superpower};
use crate::map::WorldMap;

/// The most operations points a card can ever be worth (Containment's and
/// Brezhnev Doctrine's own cap).
pub const MAX_CARD_OPS: u8 = 4;

/// A card's short name as an ops/roll modifier's source, for itemising
/// what a number is made of (`2 +1 Brezhnev`).
pub fn short_name(card: CardId) -> &'static str {
    match card.0 {
        9 => "Vietnam Revolts",
        25 => "Containment",
        31 => "Red Scare",
        51 => "Brezhnev",
        69 => "Death Squads",
        93 => "Iran-Contra",
        _ => "event",
    }
}

/// An extra op for an operation spent wholly inside `area`, from `card`
/// (the China Card's Asia, Vietnam Revolts' Southeast Asia). An operation
/// holds a list of these and tracks which still apply with a bitmask
/// (bit *i* = bonus *i*), so two can stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpsBonus {
    pub area: Area,
    pub ops: u8,
    pub card: CardId,
}

/// Which of `bonuses` cover `country`, as a bitmask.
pub fn bonus_membership(bonuses: &[OpsBonus], country: &Country) -> u8 {
    bonuses.iter().enumerate().filter(|(_, b)| country.is_in_area(b.area)).fold(0, |m, (i, _)| m | 1 << i)
}

/// The ops the bonuses in `mask` add up to.
pub fn bonus_ops(bonuses: &[OpsBonus], mask: u8) -> u8 {
    bonuses.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, b)| b.ops).sum()
}

/// One card's turn-long effect, as played — what an event produces and
/// [`TurnEffects::apply`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OngoingEffect {
    /// #9: the USSR gets +1 ops for a card spent entirely in Southeast Asia.
    VietnamRevolts,
    /// #25: US ops cards get +1 ops.
    Containment,
    /// #31: the side that *didn't* play it gets -1 ops on its cards.
    RedScare { penalised: Superpower },
    /// #41: US battleground coups don't degrade DEFCON.
    NuclearSubs,
    /// #51: USSR ops cards get +1 ops.
    Brezhnev,
    /// #69: coups in Central/South America: +1 for `beneficiary`, -1 for the other.
    DeathSquads { beneficiary: Superpower },
    /// #86: the US plays an eighth action round this turn.
    NorthSeaOil,
    /// #93: US realignment rolls get -1.
    IranContra,
    /// #94: the USSR can't place influence with ops in `region`.
    Chernobyl { region: Region },
    /// #109: the USSR gets 1 VP per US coup.
    YuriSamantha,
    /// #26 CIA Created / #62 "Lone Gunman" / #98 Aldrich Ames Remix: `side`'s hand is
    /// shown to its opponent for the rest of the turn. `card` is the card that revealed it.
    HandRevealed { side: Superpower, card: u8 },
    /// #40: DEFCON 2; a coup by the opponent of `by` this turn loses them the game, unless the crisis is defused.
    CubanMissileCrisis { by: Superpower },
}

impl OngoingEffect {
    /// The card that started this effect.
    pub fn card(&self) -> CardId {
        CardId(match self {
            OngoingEffect::VietnamRevolts => 9,
            OngoingEffect::Containment => 25,
            OngoingEffect::RedScare { .. } => 31,
            OngoingEffect::NuclearSubs => 41,
            OngoingEffect::Brezhnev => 51,
            OngoingEffect::DeathSquads { .. } => 69,
            OngoingEffect::NorthSeaOil => 86,
            OngoingEffect::IranContra => 93,
            OngoingEffect::Chernobyl { .. } => 94,
            OngoingEffect::YuriSamantha => 109,
            OngoingEffect::HandRevealed { card, .. } => *card,
            OngoingEffect::CubanMissileCrisis { .. } => 40,
        })
    }

    /// The side the effect favours (or, for a penalty, the side that
    /// played it) — what a view colours it by.
    pub fn side(&self) -> Superpower {
        match self {
            OngoingEffect::VietnamRevolts | OngoingEffect::Brezhnev | OngoingEffect::YuriSamantha => Superpower::Ussr,
            OngoingEffect::Containment | OngoingEffect::NuclearSubs | OngoingEffect::NorthSeaOil | OngoingEffect::IranContra => Superpower::Us,
            OngoingEffect::Chernobyl { .. } => Superpower::Us,
            OngoingEffect::RedScare { penalised } => penalised.opponent(),
            OngoingEffect::DeathSquads { beneficiary } => *beneficiary,
            // The side that gets to look.
            OngoingEffect::HandRevealed { side, .. } => side.opponent(),
            OngoingEffect::CubanMissileCrisis { by } => *by,
        }
    }
}

/// Every turn-long effect currently in force. One field per card, since
/// each can be in force at most once at a time (a card is played once).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnEffects {
    pub vietnam_revolts: bool,
    pub containment: bool,
    pub red_scare: Option<Superpower>,
    pub nuclear_subs: bool,
    pub brezhnev: bool,
    pub death_squads: Option<Superpower>,
    pub north_sea_oil: bool,
    pub iran_contra: bool,
    pub chernobyl: Option<Region>,
    pub yuri_samantha: bool,
    /// The card that has revealed the US hand to the USSR, if any.
    pub us_hand_revealed: Option<u8>,
    /// The card that has revealed the USSR hand to the US, if any.
    pub ussr_hand_revealed: Option<u8>,
    /// Cuban Missile Crisis, and which side played it.
    pub cuban_missile_crisis: Option<Superpower>,
}

impl TurnEffects {
    pub fn is_empty(&self) -> bool {
        *self == TurnEffects::default()
    }

    /// Records `effect` as in force for the rest of the turn.
    pub fn apply(&mut self, effect: OngoingEffect) {
        match effect {
            OngoingEffect::VietnamRevolts => self.vietnam_revolts = true,
            OngoingEffect::Containment => self.containment = true,
            OngoingEffect::RedScare { penalised } => self.red_scare = Some(penalised),
            OngoingEffect::NuclearSubs => self.nuclear_subs = true,
            OngoingEffect::Brezhnev => self.brezhnev = true,
            OngoingEffect::DeathSquads { beneficiary } => self.death_squads = Some(beneficiary),
            OngoingEffect::NorthSeaOil => self.north_sea_oil = true,
            OngoingEffect::IranContra => self.iran_contra = true,
            OngoingEffect::Chernobyl { region } => self.chernobyl = Some(region),
            OngoingEffect::YuriSamantha => self.yuri_samantha = true,
            OngoingEffect::CubanMissileCrisis { by } => self.cuban_missile_crisis = Some(by),
            OngoingEffect::HandRevealed { side: Superpower::Us, card } => self.us_hand_revealed = Some(card),
            OngoingEffect::HandRevealed { side: Superpower::Ussr, card } => self.ussr_hand_revealed = Some(card),
        }
    }

    /// Everything in force, in card-number order — what the status bar lists.
    pub fn active(&self) -> Vec<OngoingEffect> {
        let mut v = Vec::new();
        if self.vietnam_revolts {
            v.push(OngoingEffect::VietnamRevolts);
        }
        if self.containment {
            v.push(OngoingEffect::Containment);
        }
        if let Some(penalised) = self.red_scare {
            v.push(OngoingEffect::RedScare { penalised });
        }
        if self.nuclear_subs {
            v.push(OngoingEffect::NuclearSubs);
        }
        if self.brezhnev {
            v.push(OngoingEffect::Brezhnev);
        }
        if let Some(beneficiary) = self.death_squads {
            v.push(OngoingEffect::DeathSquads { beneficiary });
        }
        if self.north_sea_oil {
            v.push(OngoingEffect::NorthSeaOil);
        }
        if self.iran_contra {
            v.push(OngoingEffect::IranContra);
        }
        if let Some(region) = self.chernobyl {
            v.push(OngoingEffect::Chernobyl { region });
        }
        if self.yuri_samantha {
            v.push(OngoingEffect::YuriSamantha);
        }
        if let Some(by) = self.cuban_missile_crisis {
            v.push(OngoingEffect::CubanMissileCrisis { by });
        }
        if let Some(card) = self.us_hand_revealed {
            v.push(OngoingEffect::HandRevealed { side: Superpower::Us, card });
        }
        if let Some(card) = self.ussr_hand_revealed {
            v.push(OngoingEffect::HandRevealed { side: Superpower::Ussr, card });
        }
        v
    }

    /// Whether `side`'s hand is open to its opponent this turn.
    pub fn hand_revealed(&self, side: Superpower) -> bool {
        match side {
            Superpower::Us => self.us_hand_revealed.is_some(),
            Superpower::Ussr => self.ussr_hand_revealed.is_some(),
        }
    }

    /// The ops value `side`'s card of printed value `base` is worth right
    /// now, plus each card's contribution (for a view to itemise). The
    /// modifiers are summed first and the total clamped to `1..=4` — so
    /// Containment and Red Scare cancel rather than each clamping on its
    /// own — but a card with no modifier at all is returned as printed.
    pub fn card_ops(&self, base: u8, side: Superpower) -> (u8, Vec<(CardId, i8)>) {
        let mut mods = Vec::new();
        if self.containment && side == Superpower::Us {
            mods.push((CardId(25), 1));
        }
        if self.brezhnev && side == Superpower::Ussr {
            mods.push((CardId(51), 1));
        }
        if self.red_scare == Some(side) {
            mods.push((CardId(31), -1));
        }
        if mods.is_empty() {
            return (base, mods);
        }
        let total = base as i8 + mods.iter().map(|(_, m)| m).sum::<i8>();
        (total.clamp(1, MAX_CARD_OPS as i8) as u8, mods)
    }

    /// The modifier to `side`'s coup die roll in `region`, with the card
    /// responsible: Latin American Death Squads.
    pub fn coup_roll_mod(&self, side: Superpower, region: Region) -> Option<(CardId, i8)> {
        let beneficiary = self.death_squads?;
        if !matches!(region, Region::CentralAmerica | Region::SouthAmerica) {
            return None;
        }
        Some((CardId(69), if side == beneficiary { 1 } else { -1 }))
    }

    /// The modifier to `side`'s realignment die rolls: Iran-Contra.
    pub fn realign_roll_mod(&self, side: Superpower) -> Option<(CardId, i8)> {
        (self.iran_contra && side == Superpower::Us).then_some((CardId(93), -1))
    }

    /// The region `side` can't place influence in with ops: Chernobyl.
    pub fn placement_banned(&self, side: Superpower) -> Option<Region> {
        if side == Superpower::Ussr { self.chernobyl } else { None }
    }

    /// The extra ops `side`'s `card` earns for being spent wholly in one
    /// area: Vietnam Revolts (USSR, Southeast Asia) and the China Card's
    /// own Asia op. They stack.
    pub fn ops_bonuses(&self, side: Superpower, card: CardId) -> Vec<OpsBonus> {
        let mut bonuses = Vec::new();
        if card == crate::cards::CHINA_CARD {
            bonuses.push(OpsBonus { area: Area::Region(Region::Asia), ops: 1, card });
        }
        if self.vietnam_revolts && side == Superpower::Ussr {
            bonuses.push(OpsBonus { area: Area::Sub(SubRegion::SoutheastAsia), ops: 1, card: CardId(9) });
        }
        bonuses
    }

    /// Whether a US coup in a battleground leaves DEFCON alone: Nuclear Subs.
    pub fn spares_defcon(&self, side: Superpower) -> bool {
        self.nuclear_subs && side == Superpower::Us
    }

    /// VP the USSR earns for a coup by `side`: Yuri and Samantha.
    pub fn coup_vp(&self, side: Superpower) -> Option<(CardId, Superpower, u8)> {
        (self.yuri_samantha && side == Superpower::Us).then_some((CardId(109), Superpower::Ussr, 1))
    }

    /// How many extra action rounds `side` gets this turn: North Sea Oil's
    /// eighth round for the US.
    /// The side a coup would cost the game, while Cuban Missile Crisis is in force.
    pub fn coup_forbidden(&self, side: Superpower) -> bool {
        self.cuban_missile_crisis == Some(side.opponent())
    }

    pub fn extra_rounds(&self, side: Superpower) -> u8 {
        (self.north_sea_oil && side == Superpower::Us) as u8
    }
}

/// One card's game-long effect, as played — what an `EffectResult::lasting`
/// carries and [`LastingEffects::apply`] records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastingEffect {
    /// #17: NATO doesn't protect France.
    DeGaulle,
    /// #21: the USSR can't coup/realign US-controlled Europe (or Brush War it).
    Nato,
    /// #27: the USSR can't coup/realign Japan.
    UsJapan,
    /// #35: Taiwan counts as an Asia battleground while US-controlled.
    Formosan,
    /// #50: the USSR gets 3 VP when the US finishes its next action round.
    /// `skip` is how many US action-round completions to let pass first
    /// (0 if the USSR played it, 1 if the US played it as its own round).
    WeWillBuryYou { skip: u8 },
    /// #55: NATO doesn't protect West Germany.
    WillyBrandt,
    /// #59: the USSR gets 2 VP for each war card the US spends.
    FlowerPower,
    /// #73: the next Asia/Middle East scoring counts one fewer USSR battleground.
    ShuttleDiplomacy,
    /// #42: the US's action rounds become escape attempts (see `Game::trap`).
    Quagmire,
    /// #44: the USSR's action rounds become escape attempts.
    BearTrap,
    /// #106: +1 US influence after an action round that moved DEFCON to 2, while the US holds Canada.
    Norad,
}

impl LastingEffect {
    pub fn card(&self) -> CardId {
        CardId(match self {
            LastingEffect::DeGaulle => 17,
            LastingEffect::Nato => 21,
            LastingEffect::UsJapan => 27,
            LastingEffect::Formosan => 35,
            LastingEffect::WeWillBuryYou { .. } => 50,
            LastingEffect::WillyBrandt => 55,
            LastingEffect::FlowerPower => 59,
            LastingEffect::ShuttleDiplomacy => 73,
            LastingEffect::Quagmire => 42,
            LastingEffect::BearTrap => 44,
            LastingEffect::Norad => 106,
        })
    }

    /// The side the effect favours — what a view colours it by.
    pub fn side(&self) -> Superpower {
        match self {
            LastingEffect::Nato | LastingEffect::UsJapan | LastingEffect::Formosan | LastingEffect::ShuttleDiplomacy | LastingEffect::BearTrap | LastingEffect::Norad => Superpower::Us,
            LastingEffect::DeGaulle | LastingEffect::WeWillBuryYou { .. } | LastingEffect::WillyBrandt | LastingEffect::FlowerPower | LastingEffect::Quagmire => Superpower::Ussr,
        }
    }

    /// Short label for the status bar and log.
    pub fn label(&self) -> &'static str {
        match self {
            LastingEffect::DeGaulle => "De Gaulle",
            LastingEffect::Nato => "NATO",
            LastingEffect::UsJapan => "US/Japan Pact",
            LastingEffect::Formosan => "Formosan Resolution",
            LastingEffect::WeWillBuryYou { .. } => "We Will Bury You",
            LastingEffect::WillyBrandt => "Willy Brandt",
            LastingEffect::FlowerPower => "Flower Power",
            LastingEffect::ShuttleDiplomacy => "Shuttle Diplomacy",
            LastingEffect::Quagmire => "Quagmire",
            LastingEffect::BearTrap => "Bear Trap",
            LastingEffect::Norad => "NORAD",
        }
    }
}

/// Card events that stay in force past the turn they were played — unlike
/// [`TurnEffects`] nothing here is cleared by `Game::advance`; each ends
/// only by its own card's text. Same shape as `TurnEffects`: plain `Copy`
/// data in [`crate::status::GameStatus`], with only small pure queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LastingEffects {
    pub de_gaulle: bool,
    pub nato: bool,
    pub us_japan: bool,
    pub formosan: bool,
    pub we_will_bury_you: Option<u8>,
    pub willy_brandt: bool,
    pub flower_power: bool,
    pub shuttle_diplomacy: bool,
    pub quagmire: bool,
    pub bear_trap: bool,
    pub norad: bool,
}

impl LastingEffects {
    pub fn is_empty(&self) -> bool {
        *self == LastingEffects::default()
    }

    /// Records `effect` as in force.
    pub fn apply(&mut self, effect: LastingEffect) {
        match effect {
            LastingEffect::DeGaulle => self.de_gaulle = true,
            LastingEffect::Nato => self.nato = true,
            LastingEffect::UsJapan => self.us_japan = true,
            LastingEffect::Formosan => self.formosan = true,
            LastingEffect::WeWillBuryYou { skip } => self.we_will_bury_you = Some(skip),
            LastingEffect::WillyBrandt => self.willy_brandt = true,
            LastingEffect::FlowerPower => self.flower_power = true,
            LastingEffect::ShuttleDiplomacy => self.shuttle_diplomacy = true,
            LastingEffect::Quagmire => self.quagmire = true,
            LastingEffect::BearTrap => self.bear_trap = true,
            LastingEffect::Norad => self.norad = true,
        }
    }

    /// Ends `effect` (a later card cancelling it, or its own trigger spent).
    pub fn cancel(&mut self, effect: LastingEffect) {
        match effect {
            LastingEffect::DeGaulle => self.de_gaulle = false,
            LastingEffect::Nato => self.nato = false,
            LastingEffect::UsJapan => self.us_japan = false,
            LastingEffect::Formosan => self.formosan = false,
            LastingEffect::WeWillBuryYou { .. } => self.we_will_bury_you = None,
            LastingEffect::WillyBrandt => self.willy_brandt = false,
            LastingEffect::FlowerPower => self.flower_power = false,
            LastingEffect::ShuttleDiplomacy => self.shuttle_diplomacy = false,
            LastingEffect::Quagmire => self.quagmire = false,
            LastingEffect::BearTrap => self.bear_trap = false,
            LastingEffect::Norad => self.norad = false,
        }
    }

    /// Everything in force, in card-number order.
    pub fn active(&self) -> Vec<LastingEffect> {
        let mut v = Vec::new();
        if self.de_gaulle {
            v.push(LastingEffect::DeGaulle);
        }
        if self.nato {
            v.push(LastingEffect::Nato);
        }
        if self.us_japan {
            v.push(LastingEffect::UsJapan);
        }
        if self.formosan {
            v.push(LastingEffect::Formosan);
        }
        if let Some(skip) = self.we_will_bury_you {
            v.push(LastingEffect::WeWillBuryYou { skip });
        }
        if self.willy_brandt {
            v.push(LastingEffect::WillyBrandt);
        }
        if self.flower_power {
            v.push(LastingEffect::FlowerPower);
        }
        if self.shuttle_diplomacy {
            v.push(LastingEffect::ShuttleDiplomacy);
        }
        if self.quagmire {
            v.push(LastingEffect::Quagmire);
        }
        if self.bear_trap {
            v.push(LastingEffect::BearTrap);
        }
        if self.norad {
            v.push(LastingEffect::Norad);
        }
        v
    }

    /// Which side's action rounds are currently escape attempts (Quagmire: the US; Bear Trap: the USSR),
    /// and the card responsible.
    pub fn trap_on(&self, side: Superpower) -> Option<LastingEffect> {
        match side {
            Superpower::Us if self.quagmire => Some(LastingEffect::Quagmire),
            Superpower::Ussr if self.bear_trap => Some(LastingEffect::BearTrap),
            _ => None,
        }
    }

    /// The card that bars `attacker` from coup/realign rolls (or Brush War)
    /// against `id`, if any: NATO for a US-controlled European country
    /// (France and West Germany exempt once De Gaulle / Willy Brandt are
    /// in force), the US/Japan pact for Japan. Only the USSR is ever barred.
    pub fn protects(&self, map: &WorldMap, board: &Board, attacker: Superpower, id: CountryId) -> Option<CardId> {
        if attacker != Superpower::Ussr {
            return None;
        }
        let country = map.country(id);
        if self.us_japan && country.name == "Japan" {
            return Some(CardId(27));
        }
        let exempt = (self.de_gaulle && country.name == "France") || (self.willy_brandt && country.name == "West Germany");
        if self.nato && country.region == Region::Europe && !exempt && board.is_controlled_by(map, id, Superpower::Us) {
            return Some(CardId(21));
        }
        None
    }

    /// Whether Taiwan counts as a battleground for Asia Scoring: Formosan
    /// Resolution while the US controls it.
    pub fn taiwan_battleground(&self, map: &WorldMap, board: &Board, id: CountryId) -> bool {
        self.formosan && map.country(id).name == "Taiwan" && board.is_controlled_by(map, id, Superpower::Us)
    }

    /// Whether Shuttle Diplomacy bites on `card` (Asia #1 or Middle East #3 scoring).
    pub fn shuttle_applies(&self, card: CardId) -> bool {
        self.shuttle_diplomacy && matches!(card.0, 1 | 3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Superpower::{Us, Ussr};

    fn with(effects: &[OngoingEffect]) -> TurnEffects {
        let mut t = TurnEffects::default();
        for &e in effects {
            t.apply(e);
        }
        t
    }

    #[test]
    fn no_effects_leaves_ops_as_printed() {
        assert_eq!(TurnEffects::default().card_ops(3, Us), (3, vec![]));
        assert!(TurnEffects::default().is_empty());
    }

    #[test]
    fn containment_and_brezhnev_each_help_only_their_own_side_and_cap_at_four() {
        let t = with(&[OngoingEffect::Containment, OngoingEffect::Brezhnev]);
        assert_eq!(t.card_ops(2, Us).0, 3);
        assert_eq!(t.card_ops(4, Us).0, 4);
        assert_eq!(t.card_ops(2, Ussr).0, 3);
        assert_eq!(t.card_ops(4, Ussr).0, 4);
    }

    #[test]
    fn red_scare_penalises_one_side_and_floors_at_one() {
        let t = with(&[OngoingEffect::RedScare { penalised: Us }]);
        assert_eq!(t.card_ops(3, Us).0, 2);
        assert_eq!(t.card_ops(1, Us).0, 1);
        assert_eq!(t.card_ops(3, Ussr).0, 3);
    }

    #[test]
    fn containment_and_red_scare_cancel_on_the_us() {
        let t = with(&[OngoingEffect::Containment, OngoingEffect::RedScare { penalised: Us }]);
        assert_eq!(t.card_ops(4, Us).0, 4);
        assert_eq!(t.card_ops(2, Us).0, 2);
        assert_eq!(t.card_ops(2, Us).1.len(), 2);
    }

    #[test]
    fn death_squads_favours_the_beneficiary_in_the_americas_only() {
        let t = with(&[OngoingEffect::DeathSquads { beneficiary: Ussr }]);
        assert_eq!(t.coup_roll_mod(Ussr, Region::SouthAmerica).unwrap().1, 1);
        assert_eq!(t.coup_roll_mod(Us, Region::CentralAmerica).unwrap().1, -1);
        assert_eq!(t.coup_roll_mod(Ussr, Region::Africa), None);
    }

    #[test]
    fn iran_contra_hits_only_us_realignment() {
        let t = with(&[OngoingEffect::IranContra]);
        assert_eq!(t.realign_roll_mod(Us).unwrap().1, -1);
        assert_eq!(t.realign_roll_mod(Ussr), None);
    }

    #[test]
    fn chernobyl_bans_only_the_ussr() {
        let t = with(&[OngoingEffect::Chernobyl { region: Region::Asia }]);
        assert_eq!(t.placement_banned(Ussr), Some(Region::Asia));
        assert_eq!(t.placement_banned(Us), None);
    }

    #[test]
    fn the_one_off_rules_are_side_specific() {
        let t = with(&[OngoingEffect::NuclearSubs, OngoingEffect::YuriSamantha, OngoingEffect::NorthSeaOil, OngoingEffect::VietnamRevolts]);
        assert!(t.spares_defcon(Us) && !t.spares_defcon(Ussr));
        assert!(t.coup_vp(Us).is_some() && t.coup_vp(Ussr).is_none());
        assert_eq!(t.extra_rounds(Us), 1);
        assert_eq!(t.extra_rounds(Ussr), 0);
        assert_eq!(t.ops_bonuses(Ussr, CardId(8)).len(), 1);
        assert!(t.ops_bonuses(Us, CardId(8)).is_empty());
        assert_eq!(t.ops_bonuses(Ussr, crate::cards::CHINA_CARD).len(), 2, "the China Card's Asia op stacks with Vietnam Revolts");
    }

    #[test]
    fn active_lists_in_card_order_and_round_trips_through_serde() {
        let t = with(&[OngoingEffect::Chernobyl { region: Region::Europe }, OngoingEffect::Containment, OngoingEffect::VietnamRevolts]);
        let cards: Vec<u8> = t.active().iter().map(|e| e.card().0).collect();
        assert_eq!(cards, vec![9, 25, 94]);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<TurnEffects>(&json).unwrap(), t);
    }

    // --- lasting effects ---

    fn lasting(effects: &[LastingEffect]) -> LastingEffects {
        let mut l = LastingEffects::default();
        for &e in effects {
            l.apply(e);
        }
        l
    }

    fn us_controls(map: &WorldMap, board: &mut Board, name: &str) {
        let id = map.id_by_name(name).unwrap();
        board.set_influence(id, Us, map.country(id).stability + 1);
    }

    #[test]
    fn lasting_effects_start_empty_apply_cancel_and_round_trip_through_serde() {
        let mut l = LastingEffects::default();
        assert!(l.is_empty());
        for e in [LastingEffect::Nato, LastingEffect::Formosan, LastingEffect::WeWillBuryYou { skip: 1 }] {
            l.apply(e);
        }
        assert_eq!(l.active().iter().map(|e| e.card().0).collect::<Vec<_>>(), vec![21, 35, 50]);
        let back: LastingEffects = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
        l.cancel(LastingEffect::Nato);
        l.cancel(LastingEffect::WeWillBuryYou { skip: 0 });
        assert_eq!(l.active(), vec![LastingEffect::Formosan]);
    }

    #[test]
    fn nato_shields_only_us_controlled_europe_from_the_ussr() {
        let map = WorldMap::standard().unwrap();
        let mut board = Board::new(&map);
        us_controls(&map, &mut board, "Italy");
        us_controls(&map, &mut board, "Iran");
        let (italy, iran, spain) = (map.id_by_name("Italy").unwrap(), map.id_by_name("Iran").unwrap(), map.id_by_name("Spain/Portugal").unwrap());
        let l = lasting(&[LastingEffect::Nato]);
        assert_eq!(l.protects(&map, &board, Ussr, italy), Some(CardId(21)));
        assert_eq!(l.protects(&map, &board, Us, italy), None, "only the USSR is barred");
        assert_eq!(l.protects(&map, &board, Ussr, iran), None, "not Europe");
        assert_eq!(l.protects(&map, &board, Ussr, spain), None, "not US-controlled");
        assert_eq!(LastingEffects::default().protects(&map, &board, Ussr, italy), None, "no NATO");
    }

    #[test]
    fn de_gaulle_and_willy_brandt_exempt_france_and_west_germany() {
        let map = WorldMap::standard().unwrap();
        let mut board = Board::new(&map);
        for n in ["France", "West Germany"] {
            us_controls(&map, &mut board, n);
        }
        let (france, wg) = (map.id_by_name("France").unwrap(), map.id_by_name("West Germany").unwrap());
        let l = lasting(&[LastingEffect::Nato, LastingEffect::DeGaulle]);
        assert_eq!((l.protects(&map, &board, Ussr, france), l.protects(&map, &board, Ussr, wg)), (None, Some(CardId(21))));
        let l = lasting(&[LastingEffect::Nato, LastingEffect::WillyBrandt]);
        assert_eq!((l.protects(&map, &board, Ussr, france), l.protects(&map, &board, Ussr, wg)), (Some(CardId(21)), None));
    }

    #[test]
    fn the_japan_pact_shields_japan_even_without_nato() {
        let map = WorldMap::standard().unwrap();
        let board = Board::new(&map);
        let japan = map.id_by_name("Japan").unwrap();
        assert_eq!(lasting(&[LastingEffect::UsJapan]).protects(&map, &board, Ussr, japan), Some(CardId(27)));
    }

    #[test]
    fn formosan_needs_a_us_controlled_taiwan_and_shuttle_only_bites_asia_and_middle_east() {
        let map = WorldMap::standard().unwrap();
        let mut board = Board::new(&map);
        let taiwan = map.id_by_name("Taiwan").unwrap();
        let l = lasting(&[LastingEffect::Formosan, LastingEffect::ShuttleDiplomacy]);
        assert!(!l.taiwan_battleground(&map, &board, taiwan));
        us_controls(&map, &mut board, "Taiwan");
        assert!(l.taiwan_battleground(&map, &board, taiwan));
        assert!(!l.taiwan_battleground(&map, &board, map.id_by_name("Japan").unwrap()));
        assert!([1, 3].iter().all(|&n| l.shuttle_applies(CardId(n))));
        assert!(![2, 79, 38].iter().any(|&n| l.shuttle_applies(CardId(n))));
    }
}
