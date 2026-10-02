//! Scoring-card events (rule 10.1): resolving control/domination/presence
//! across a region — or, for Southeast Asia, per-country control — into a
//! VP swing. Pure functions of `(map, board, card)`, mirroring
//! `ops::realign`'s own `modifiers`/`odds` split: nothing here needs a
//! [`crate::game::Game`] open, which is what lets [`resolve`] be driven
//! from [`crate::events::resolve`] with no state of its own.
//!
//! Every scoring card shares the same two extra bonuses, on top of
//! whichever tier a side reaches: +1 VP per battleground country it
//! controls in the region, and +1 VP per country it controls that
//! borders the *opponent's* superpower. The Middle East, Africa, and
//! South America cards' own text omits the second bonus — but no country
//! in any of those three regions borders a superpower at all (pinned by
//! `tests/standard_map.rs::superpower_borders_match_the_real_board`), so
//! applying it uniformly is behaviourally identical to special-casing it
//! per card, and the latter would just be more code for the same result.

use crate::board::Board;
use crate::cards::CardId;
use crate::country::{CountryId, Region, SubRegion, Superpower};
use crate::map::WorldMap;

/// How a region scoring card's Control tier turns into VP: a flat value
/// for five of the six region cards, or an outright win for the sixth
/// (Europe Scoring, rule 10.1) — ending the game rather than scoring any
/// particular number of points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlVp {
    Vp(u8),
    AutomaticVictory,
}

/// One region scoring card's own point table (rule 10.1's chart) —
/// everything else about how it scores is shared by every region card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionScoring {
    region: Region,
    presence: u8,
    domination: u8,
    control: ControlVp,
}

const REGION_SCORING: &[(CardId, RegionScoring)] = &[
    (CardId(1), RegionScoring { region: Region::Asia, presence: 3, domination: 7, control: ControlVp::Vp(9) }),
    (CardId(2), RegionScoring { region: Region::Europe, presence: 3, domination: 7, control: ControlVp::AutomaticVictory }),
    (CardId(3), RegionScoring { region: Region::MiddleEast, presence: 3, domination: 5, control: ControlVp::Vp(7) }),
    (CardId(37), RegionScoring { region: Region::CentralAmerica, presence: 1, domination: 3, control: ControlVp::Vp(5) }),
    (CardId(79), RegionScoring { region: Region::Africa, presence: 1, domination: 4, control: ControlVp::Vp(6) }),
    (CardId(81), RegionScoring { region: Region::SouthAmerica, presence: 2, domination: 5, control: ControlVp::Vp(6) }),
];

/// Southeast Asia Scoring (#38) — the one scoring card that pays out per
/// country rather than by region tier, and the one `scoring` card that's
/// also `removed_after_event` (its own text's "MAY NOT BE HELD" is true
/// of every scoring card, but this is the only one the catalog also
/// marks removed).
pub const SOUTHEAST_ASIA: CardId = CardId(38);

/// Whether `card` is one of the seven scoring cards this module knows how
/// to resolve — every `scoring` card in the catalog, once this stage is
/// wired in.
pub fn is_scoring_card(card: CardId) -> bool {
    REGION_SCORING.iter().any(|&(id, _)| id == card) || card == SOUTHEAST_ASIA
}

/// How far a side got in one region (rule 10.1, in ascending order —
/// `Control` implies `Domination` implies `Presence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tier {
    #[default]
    None,
    Presence,
    Domination,
    Control,
}

/// One side's full breakdown for one region: which tier it reached, the
/// raw counts behind that, and every VP contribution — kept itemised
/// (rather than just the total) so a view can explain the number instead
/// of just showing it, the same reasoning `realign::Modifiers` follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SideScore {
    pub tier: Tier,
    pub countries: u8,
    pub battlegrounds: u8,
    pub tier_vp: u8,
    pub battleground_vp: u8,
    pub adjacency_vp: u8,
}

impl SideScore {
    pub fn total(&self) -> u8 {
        self.tier_vp + self.battleground_vp + self.adjacency_vp
    }
}

/// What one scoring card actually resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoringKind {
    Region { region: Region, us: SideScore, ussr: SideScore },
    /// Southeast Asia Scoring: no tiers, just every controlled country in
    /// the sub-region and who holds it — `vp` is `2` for Thailand, `1`
    /// for the other six (see [`score_southeast_asia`]'s own doc for why
    /// that's read off the map rather than hand-listed here).
    SoutheastAsia { controlled: Vec<(CountryId, Superpower, u8)> },
}

/// One scoring card's resolved outcome: the breakdown (`kind`) plus the
/// net VP swing it produces (`vp_delta`, positive favouring the US, the
/// same convention [`crate::status::GameStatus::vp`] already uses) and
/// whether it won the game outright (Europe Scoring's Control tier
/// only — every other card's `control` is a plain [`ControlVp::Vp`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoringResult {
    pub card: CardId,
    pub kind: ScoringKind,
    pub vp_delta: i8,
    pub automatic_victory: Option<Superpower>,
}

/// Resolves `card` — refuses (returns `None`) anything [`is_scoring_card`]
/// doesn't recognise, the same "not every card is implemented yet" gate
/// [`crate::events::resolve`] reads.
pub fn resolve(map: &WorldMap, board: &Board, card: CardId) -> Option<ScoringResult> {
    if card == SOUTHEAST_ASIA {
        return Some(score_southeast_asia(map, board));
    }
    let &(_, scoring) = REGION_SCORING.iter().find(|&&(id, _)| id == card)?;
    Some(score_region(map, board, card, scoring))
}

fn score_region(map: &WorldMap, board: &Board, card: CardId, scoring: RegionScoring) -> ScoringResult {
    // One pass over the region, tallying both sides (and the region's
    // own battleground total, needed for Control's "every battleground"
    // test) at once rather than scoring each side with its own full scan.
    let mut us_countries = 0u8;
    let mut us_battlegrounds = 0u8;
    let mut us_non_battlegrounds = 0u8;
    let mut us_adjacency = 0u8;
    let mut ussr_countries = 0u8;
    let mut ussr_battlegrounds = 0u8;
    let mut ussr_non_battlegrounds = 0u8;
    let mut ussr_adjacency = 0u8;
    let mut region_battlegrounds = 0u8;

    for (id, country) in map.iter() {
        if country.region != scoring.region {
            continue;
        }
        if country.battleground {
            region_battlegrounds += 1;
        }
        match board.controller(map, id) {
            Some(Superpower::Us) => {
                us_countries += 1;
                if country.battleground {
                    us_battlegrounds += 1;
                } else {
                    us_non_battlegrounds += 1;
                }
                if country.borders_superpower(Superpower::Ussr) {
                    us_adjacency += 1;
                }
            }
            Some(Superpower::Ussr) => {
                ussr_countries += 1;
                if country.battleground {
                    ussr_battlegrounds += 1;
                } else {
                    ussr_non_battlegrounds += 1;
                }
                if country.borders_superpower(Superpower::Us) {
                    ussr_adjacency += 1;
                }
            }
            None => {}
        }
    }

    let us = side_score(
        &scoring,
        region_battlegrounds,
        us_countries,
        us_battlegrounds,
        us_non_battlegrounds,
        ussr_countries,
        ussr_battlegrounds,
        us_adjacency,
    );
    let ussr = side_score(
        &scoring,
        region_battlegrounds,
        ussr_countries,
        ussr_battlegrounds,
        ussr_non_battlegrounds,
        us_countries,
        us_battlegrounds,
        ussr_adjacency,
    );

    let automatic_victory = match scoring.control {
        ControlVp::AutomaticVictory if us.tier == Tier::Control => Some(Superpower::Us),
        ControlVp::AutomaticVictory if ussr.tier == Tier::Control => Some(Superpower::Ussr),
        _ => None,
    };
    let vp_delta = us.total() as i8 - ussr.total() as i8;

    ScoringResult { card, kind: ScoringKind::Region { region: scoring.region, us, ussr }, vp_delta, automatic_victory }
}

/// `side`'s own [`SideScore`] against the opponent's counts — a free
/// function rather than a method so the two (symmetric) calls above read
/// the same way for either side, with nothing to get backwards.
#[allow(clippy::too_many_arguments)]
fn side_score(
    scoring: &RegionScoring,
    region_battlegrounds: u8,
    countries: u8,
    battlegrounds: u8,
    non_battlegrounds: u8,
    opponent_countries: u8,
    opponent_battlegrounds: u8,
    adjacency: u8,
) -> SideScore {
    let more_countries = countries > opponent_countries;
    let control = more_countries && battlegrounds == region_battlegrounds && region_battlegrounds > 0;
    let domination =
        more_countries && battlegrounds > opponent_battlegrounds && battlegrounds > 0 && non_battlegrounds > 0;
    let presence = countries > 0;

    let (tier, tier_vp) = if control {
        (Tier::Control, match scoring.control {
            ControlVp::Vp(n) => n,
            ControlVp::AutomaticVictory => 0,
        })
    } else if domination {
        (Tier::Domination, scoring.domination)
    } else if presence {
        (Tier::Presence, scoring.presence)
    } else {
        (Tier::None, 0)
    };

    SideScore { tier, countries, battlegrounds, tier_vp, battleground_vp: battlegrounds, adjacency_vp: adjacency }
}

/// Southeast Asia Scoring (#38): 1 VP per controlled country in the
/// `SoutheastAsia` sub-region, 2 for Thailand — read off `battleground`
/// rather than hand-matched by name, since the map fix pinned by
/// `tests/standard_map.rs` leaves Thailand as that sub-region's *only*
/// battleground, exactly the one the card's own text singles out for the
/// higher value.
fn score_southeast_asia(map: &WorldMap, board: &Board) -> ScoringResult {
    let mut controlled = Vec::new();
    let mut vp_delta = 0i8;
    for (id, country) in map.iter() {
        if !country.is_in_sub_region(SubRegion::SoutheastAsia) {
            continue;
        }
        let Some(side) = board.controller(map, id) else { continue };
        let vp = if country.battleground { 2 } else { 1 };
        controlled.push((id, side, vp));
        vp_delta += match side {
            Superpower::Us => vp as i8,
            Superpower::Ussr => -(vp as i8),
        };
    }
    ScoringResult { card: SOUTHEAST_ASIA, kind: ScoringKind::SoutheastAsia { controlled }, vp_delta, automatic_victory: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::cards::CardCatalog;
    use crate::map::WorldMap;

    fn map() -> WorldMap {
        WorldMap::standard().unwrap()
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country named {name:?}"))
    }

    fn region_result(r: &ScoringResult) -> (&SideScore, &SideScore) {
        match &r.kind {
            ScoringKind::Region { us, ussr, .. } => (us, ussr),
            ScoringKind::SoutheastAsia { .. } => panic!("expected a region result"),
        }
    }

    #[test]
    fn every_scoring_card_in_the_catalog_has_a_table_entry_and_nothing_else_does() {
        let cards = CardCatalog::standard().unwrap();
        // Ids are unique and contiguous 1..=len (CardCatalog's own
        // invariant), so every one of them names a real card.
        for i in 1..=cards.len() as u8 {
            let cid = CardId(i);
            let card = cards.card(cid);
            assert_eq!(
                card.scoring,
                is_scoring_card(cid),
                "card #{i} ({}) scoring={} is_scoring_card={}",
                card.name,
                card.scoring,
                is_scoring_card(cid)
            );
        }
    }

    #[test]
    fn an_empty_region_scores_zero_zero() {
        let map = map();
        let board = Board::new(&map);
        let result = resolve(&map, &board, CardId(3)).unwrap(); // Middle East
        let (us, ussr) = region_result(&result);
        assert_eq!(us.tier, Tier::None);
        assert_eq!(ussr.tier, Tier::None);
        assert_eq!(result.vp_delta, 0);
    }

    #[test]
    fn one_country_is_presence_only() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Jordan"), Superpower::Us, 5); // stability 2, not a battleground
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.tier, Tier::Presence);
        assert_eq!(us.tier_vp, 3);
        assert_eq!(us.battleground_vp, 0);
    }

    #[test]
    fn domination_needs_a_battleground_and_a_non_battleground() {
        let map = map();
        let mut board = Board::new(&map);
        // US controls two Middle East countries, both battlegrounds — more
        // countries and more battlegrounds than the USSR's one, but no
        // non-battleground of its own, so domination is refused.
        board.set_influence(id(&map, "Iraq"), Superpower::Us, 10); // battleground
        board.set_influence(id(&map, "Saudi Arabia"), Superpower::Us, 10); // battleground
        board.set_influence(id(&map, "Gulf States"), Superpower::Ussr, 5); // not a battleground
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, ussr) = region_result(&result);
        assert_eq!(us.tier, Tier::Presence);
        assert_eq!(ussr.tier, Tier::Presence);

        // Give the US a non-battleground too — now it dominates.
        board.set_influence(id(&map, "Jordan"), Superpower::Us, 5);
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.tier, Tier::Domination);
        assert_eq!(us.tier_vp, 5);
        assert_eq!(us.battleground_vp, 2);
    }

    #[test]
    fn more_countries_but_tied_battlegrounds_is_presence_only() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Iraq"), Superpower::Us, 10); // battleground
        board.set_influence(id(&map, "Jordan"), Superpower::Us, 5);
        board.set_influence(id(&map, "Gulf States"), Superpower::Us, 5);
        board.set_influence(id(&map, "Saudi Arabia"), Superpower::Ussr, 10); // battleground — ties US's one
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.tier, Tier::Presence, "3 countries but tied battlegrounds should not dominate");
    }

    #[test]
    fn tied_country_counts_give_presence_only() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Iraq"), Superpower::Us, 10);
        board.set_influence(id(&map, "Saudi Arabia"), Superpower::Ussr, 10);
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, ussr) = region_result(&result);
        assert_eq!(us.tier, Tier::Presence);
        assert_eq!(ussr.tier, Tier::Presence);
    }

    #[test]
    fn control_needs_every_battleground_and_more_countries() {
        let map = map();
        let mut board = Board::new(&map);
        // US controls every ME battleground (Egypt, Israel, Iraq, Iran,
        // Libya, Saudi Arabia) plus one more country than the USSR's one.
        for name in ["Egypt", "Israel", "Iraq", "Iran", "Libya", "Saudi Arabia", "Jordan"] {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        board.set_influence(id(&map, "Gulf States"), Superpower::Ussr, 10);
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.tier, Tier::Control);
        assert_eq!(us.tier_vp, 7);
        assert_eq!(us.battleground_vp, 6);

        // Missing just one battleground drops it back to domination.
        board.set_influence(id(&map, "Libya"), Superpower::Us, 0);
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.tier, Tier::Domination);
    }

    #[test]
    fn battleground_and_adjacency_bonuses_are_counted() {
        let map = map();
        let mut board = Board::new(&map);
        // Asia: North Korea (battleground, borders USSR) and Japan
        // (battleground, borders USA) both controlled by the US.
        board.set_influence(id(&map, "North Korea"), Superpower::Us, 10);
        board.set_influence(id(&map, "Japan"), Superpower::Us, 10);
        let result = resolve(&map, &board, CardId(1)).unwrap(); // Asia
        let (us, _) = region_result(&result);
        assert_eq!(us.battleground_vp, 2, "both controlled countries are battlegrounds");
        assert_eq!(us.adjacency_vp, 1, "only North Korea borders the USSR");
    }

    #[test]
    fn both_sides_scoring_at_once_nets_correctly() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Lebanon"), Superpower::Us, 5); // presence, 3 VP
        board.set_influence(id(&map, "Iraq"), Superpower::Ussr, 10); // battleground
        board.set_influence(id(&map, "Syria"), Superpower::Ussr, 5);
        board.set_influence(id(&map, "Gulf States"), Superpower::Ussr, 5); // domination, 5 VP + 1 BG
        let result = resolve(&map, &board, CardId(3)).unwrap();
        assert_eq!(result.vp_delta, 3 - (5 + 1));
    }

    #[test]
    fn europe_control_is_an_automatic_victory_for_either_side() {
        let map = map();
        let mut board = Board::new(&map);
        let europe_battlegrounds = ["France", "West Germany", "East Germany", "Poland", "Italy"];
        for name in europe_battlegrounds {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        board.set_influence(id(&map, "UK"), Superpower::Us, 10);
        let result = resolve(&map, &board, CardId(2)).unwrap(); // Europe
        assert_eq!(result.automatic_victory, Some(Superpower::Us));

        let mut board = Board::new(&map);
        for name in europe_battlegrounds {
            board.set_influence(id(&map, name), Superpower::Ussr, 10);
        }
        board.set_influence(id(&map, "UK"), Superpower::Ussr, 10);
        let result = resolve(&map, &board, CardId(2)).unwrap();
        assert_eq!(result.automatic_victory, Some(Superpower::Ussr));
    }

    #[test]
    fn full_value_cases_for_each_region_card() {
        let map = map();

        // Asia: US controls every battleground plus Afghanistan, USSR has
        // nothing — full Control (9) + 6 battlegrounds + North
        // Korea/Japan adjacency (2) = 17.
        let mut board = Board::new(&map);
        for name in ["North Korea", "South Korea", "Japan", "Pakistan", "India", "Thailand", "Afghanistan"] {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        let result = resolve(&map, &board, CardId(1)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.total(), 9 + 6 + 2);

        // Middle East: full Control (7) + 6 battlegrounds, no superpower
        // adjacency anywhere in the region = 13.
        let mut board = Board::new(&map);
        for name in ["Egypt", "Israel", "Iraq", "Iran", "Libya", "Saudi Arabia", "Jordan"] {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        let result = resolve(&map, &board, CardId(3)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.total(), 7 + 6);

        // Central America: full Control (5) + 3 battlegrounds. Mexico and
        // Cuba border the USA, but that's the US's *own* superpower, not
        // the enemy's — no adjacency bonus for the US scoring here.
        let mut board = Board::new(&map);
        for name in ["Mexico", "Panama", "Cuba", "Guatemala"] {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        let result = resolve(&map, &board, CardId(37)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.total(), 5 + 3);
        assert_eq!(us.adjacency_vp, 0);

        // Africa: full Control (6) + 5 battlegrounds, no adjacency = 11.
        let mut board = Board::new(&map);
        for name in ["Algeria", "Nigeria", "Zaire", "Angola", "South Africa", "Morocco"] {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        let result = resolve(&map, &board, CardId(79)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.total(), 6 + 5);

        // South America: full Control (6) + 4 battlegrounds, no
        // adjacency = 10.
        let mut board = Board::new(&map);
        for name in ["Venezuela", "Chile", "Argentina", "Brazil", "Colombia"] {
            board.set_influence(id(&map, name), Superpower::Us, 10);
        }
        let result = resolve(&map, &board, CardId(81)).unwrap();
        let (us, _) = region_result(&result);
        assert_eq!(us.total(), 6 + 4);
    }

    fn se_asia_controlled(result: &ScoringResult) -> &[(CountryId, Superpower, u8)] {
        match &result.kind {
            ScoringKind::SoutheastAsia { controlled } => controlled,
            ScoringKind::Region { .. } => panic!("expected a Southeast Asia result"),
        }
    }

    #[test]
    fn southeast_asia_pays_two_for_thailand_and_one_for_everything_else() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Thailand"), Superpower::Us, 10);
        board.set_influence(id(&map, "Burma"), Superpower::Us, 10);
        let result = resolve(&map, &board, SOUTHEAST_ASIA).unwrap();
        assert_eq!(result.vp_delta, 2 + 1);
        assert_eq!(se_asia_controlled(&result).len(), 2);
    }

    #[test]
    fn southeast_asia_ignores_uncontrolled_and_out_of_region_countries() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Thailand"), Superpower::Us, 1);
        board.set_influence(id(&map, "Thailand"), Superpower::Ussr, 1); // contested, not controlled
        board.set_influence(id(&map, "Japan"), Superpower::Us, 10); // Asia, but not SE Asia
        let result = resolve(&map, &board, SOUTHEAST_ASIA).unwrap();
        assert_eq!(se_asia_controlled(&result).len(), 0);
        assert_eq!(result.vp_delta, 0);
    }

    #[test]
    fn southeast_asia_nets_a_mixed_split() {
        let map = map();
        let mut board = Board::new(&map);
        board.set_influence(id(&map, "Thailand"), Superpower::Ussr, 10); // -2
        board.set_influence(id(&map, "Vietnam"), Superpower::Us, 10); // +1
        board.set_influence(id(&map, "Malaysia"), Superpower::Us, 10); // +1
        let result = resolve(&map, &board, SOUTHEAST_ASIA).unwrap();
        assert_eq!(result.vp_delta, 1 + 1 - 2);
    }

    #[test]
    fn region_score_total_sums_its_own_parts() {
        let score = SideScore { tier: Tier::Domination, countries: 2, battlegrounds: 1, tier_vp: 5, battleground_vp: 1, adjacency_vp: 1 };
        assert_eq!(score.total(), 7);
    }
}
