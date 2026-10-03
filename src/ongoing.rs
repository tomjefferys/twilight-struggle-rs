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

use crate::cards::CardId;
use crate::country::{Region, SubRegion, Superpower};

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
        v
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

    /// An extra op for a card `side` spends entirely in the given
    /// sub-region: Vietnam Revolts.
    pub fn sub_region_bonus(&self, side: Superpower) -> Option<(SubRegion, u8)> {
        (self.vietnam_revolts && side == Superpower::Ussr).then_some((SubRegion::SoutheastAsia, 1))
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
    pub fn extra_rounds(&self, side: Superpower) -> u8 {
        (self.north_sea_oil && side == Superpower::Us) as u8
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
        assert!(t.sub_region_bonus(Ussr).is_some() && t.sub_region_bonus(Us).is_none());
    }

    #[test]
    fn active_lists_in_card_order_and_round_trips_through_serde() {
        let t = with(&[OngoingEffect::Chernobyl { region: Region::Europe }, OngoingEffect::Containment, OngoingEffect::VietnamRevolts]);
        let cards: Vec<u8> = t.active().iter().map(|e| e.card().0).collect();
        assert_eq!(cards, vec![9, 25, 94]);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<TurnEffects>(&json).unwrap(), t);
    }
}
