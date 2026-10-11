//! [`evaluate`]: a cheap static score for a position, from one side's point of view. The
//! heuristic AI ranks moves by the score of the position they lead to, so everything it "knows"
//! about the game is in the terms below. All the weights are named constants — tune them here.

use crate::cards::{CardCatalog, CardPhase, CardSide};
use crate::country::{CountryId, Region, Superpower};
use crate::events::scoring;
use crate::game::Game;
use crate::map::WorldMap;

/// Score of a finished game: a win is this, a loss minus it, a draw zero.
const WIN: f32 = 10_000.0;
/// Points per victory point already on the track.
const VP_WEIGHT: f32 = 10.0;
/// Points per VP a region's scoring card would pay for the current board — less than a banked VP,
/// because the card still has to be drawn and played.
const SCORING_WEIGHT: f32 = 3.0;
/// What holding Europe's automatic-victory tier is worth (or being denied it, negated).
const EUROPE_CONTROL: f32 = 150.0;
/// The deep evaluation's value of a VP a scoring card would pay now, before the likelihood
/// of it being played soon scales it, and that likelihood for a card held, in the discard
/// pile, or still to be drawn.
const DEEP_SCORING_WEIGHT: f32 = 8.0;
const DEEP_HELD: f32 = 0.7;
const DEEP_DISCARDED: f32 = 0.1;
const DEEP_IN_DECK: f32 = 0.3;
/// A country's whole value, scaled by [`country_weight`].
const COUNTRY_SCALE: f32 = 3.0;
/// Where influence is worth putting depends on which region will be scored soon. A region's
/// weight is `REGION_BASE + REGION_URGENCY * urgency` (0-1, see [`region_urgency`]); Europe, where
/// Control wins the game outright, is further multiplied by `EUROPE_BONUS`.
const REGION_BASE: f32 = 0.4;
const REGION_URGENCY: f32 = 1.2;
const EUROPE_BONUS: f32 = 2.0;
/// How likely a region's scoring card is to be played soon, by what `side` can know of where it
/// is: in its own hand (or an opponent's hand an event revealed) it is imminent; in the draw
/// deck or the opponent's hidden hand, possible; not yet dealt (a Mid War card before turn 4),
/// spent or removed, only final scoring is left to bring it.
const URGENCY_HELD: f32 = 1.0;
const URGENCY_UNSEEN: f32 = 0.5;
const URGENCY_DISCARDED: f32 = 0.15;
const URGENCY_NOT_DEALT: f32 = 0.0;
/// A small reward per point of influence, so spending ops is never worse than wasting them.
const INFLUENCE_EPSILON: f32 = 0.05;
/// DEFCON 2 and 3 are dangerous for the side that has to keep playing at them.
const DEFCON_2_PENALTY: f32 = 25.0;
const DEFCON_3_PENALTY: f32 = 5.0;
/// Points per VP the Military Operations shortfall will cost at the turn's end, scaled by how much
/// of the turn has gone.
const MIL_OPS_WEIGHT: f32 = 10.0;
/// Points per space race box of lead.
const SPACE_WEIGHT: f32 = 5.0;
/// Points per op of an own or neutral card still held; an opponent's card is a liability.
const HAND_OPS_WEIGHT: f32 = 1.0;
const OPPONENT_CARD_PENALTY: f32 = 3.0;

/// The board as it stands for judging a position: an open placement or event keeps its pending
/// influence on a speculative board, which must count or every placement scores the same.
fn live_board(game: &Game) -> &crate::board::Board {
    game.operation().and_then(|op| op.board()).unwrap_or_else(|| game.board())
}

/// How much `side` is worth being in `id`: grows with how close it is to control and jumps at it.
fn country_value(map: &WorldMap, game: &Game, id: CountryId, side: Superpower, regions: &[(Region, f32)]) -> f32 {
    let board = live_board(game);
    let country = map.country(id);
    let own = board.influence(id, side) as f32;
    let opp = board.influence(id, side.opponent()) as f32;
    if own == 0.0 {
        return 0.0;
    }
    let region = regions.iter().find(|&&(r, _)| r == country.region).map_or(1.0, |&(_, w)| w);
    let weight = country_weight(country.battleground, country.stability) * region;
    let progress = (own / (opp + country.stability as f32)).min(1.0);
    let control = if board.is_controlled_by(map, id, side) { 1.0 } else { 0.0 };
    COUNTRY_SCALE * weight * (progress + control) + INFLUENCE_EPSILON * own
}

/// How soon `side` should expect `region`'s scoring card, from 0 to 1, from what it can see.
fn region_urgency(game: &Game, cards: &CardCatalog, side: Superpower, region: Region) -> f32 {
    let Some(card) = cards.ids().find(|&c| scoring::region_of(c) == Some(region)) else {
        return URGENCY_UNSEEN;
    };
    let status = game.status();
    let in_hand = |s: Superpower| game.hand(s).contains(&card);
    let here = if in_hand(side) || (in_hand(side.opponent()) && status.effects.hand_revealed(side.opponent())) {
        URGENCY_HELD
    } else if game.hands().discards().contains(&card) || game.hands().removed().contains(&card) {
        URGENCY_DISCARDED
    } else if cards.card(card).phase == CardPhase::Mid && status.turn < 4
        || cards.card(card).phase == CardPhase::Late && status.turn < 8
    {
        URGENCY_NOT_DEALT
    } else {
        URGENCY_UNSEEN
    };
    // Final scoring pays every region at the end, whatever the card does before then.
    let late = ((status.turn as f32 - 4.0) / 6.0).clamp(0.0, 1.0);
    here + (1.0 - here) * late * late
}

/// The multiplier on a country's worth, per region.
fn region_weights(game: &Game, cards: &CardCatalog, side: Superpower) -> Vec<(Region, f32)> {
    [Region::Europe, Region::Asia, Region::MiddleEast, Region::Africa, Region::CentralAmerica, Region::SouthAmerica]
        .iter()
        .map(|&r| {
            let w = REGION_BASE + REGION_URGENCY * region_urgency(game, cards, side, r);
            (r, if r == Region::Europe { w * EUROPE_BONUS } else { w })
        })
        .collect()
}

/// Battlegrounds matter most; a shaky country is cheap to hold, so slightly more worth having.
fn country_weight(battleground: bool, stability: u8) -> f32 {
    let base = if battleground { 3.0 } else { 1.0 };
    base + 0.1 * (4 - stability.min(4)) as f32
}

/// The position's score for `side`: positive is good for it. Antisymmetric in everything but the
/// DEFCON penalty and the hand (which only count the side's own).
pub fn evaluate(game: &Game, map: &WorldMap, cards: &CardCatalog, side: Superpower) -> f32 {
    eval(game, map, cards, side, false)
}

/// Like [`evaluate`], but for a search's leaves, where the scoring cards are weighed by where
/// they are and how late in the game it is (a card in a hand will be played this turn; one in the
/// deck probably won't; at the end of the game every region scores) rather than a flat discount.
pub fn evaluate_deep(game: &Game, map: &WorldMap, cards: &CardCatalog, side: Superpower) -> f32 {
    eval(game, map, cards, side, true)
}

/// How likely a scoring card is to pay out soon, by where it sits.
fn scoring_likelihood(game: &Game, card: crate::cards::CardId) -> f32 {
    let status = game.status();
    // Final scoring pays every region at the end: ever more certain as the game runs out.
    let late = ((status.turn as f32 - 4.0) / 6.0).clamp(0.0, 1.0);
    let held = [Superpower::Us, Superpower::Ussr].iter().any(|&s| game.hand(s).contains(&card));
    let here = if held {
        DEEP_HELD
    } else if game.hands().discards().contains(&card) {
        DEEP_DISCARDED
    } else {
        DEEP_IN_DECK
    };
    here + (1.0 - here) * late * late
}

fn eval(game: &Game, map: &WorldMap, cards: &CardCatalog, side: Superpower, deep: bool) -> f32 {
    if let Some(victory) = game.winner() {
        return match victory.side {
            Some(s) if s == side => WIN,
            Some(_) => -WIN,
            None => 0.0,
        };
    }
    let status = game.status();
    let sign = |us_positive: f32| if side == Superpower::Us { us_positive } else { -us_positive };
    let mut score = sign(status.vp as f32) * VP_WEIGHT;

    // Who holds what, country by country.
    let regions = region_weights(game, cards, side);
    for (id, _) in map.iter() {
        score += country_value(map, game, id, side, &regions) - country_value(map, game, id, side.opponent(), &regions);
    }

    // What each region's scoring card would pay now.
    for card in cards.ids().filter(|&c| scoring::is_scoring_card(c)) {
        if let Some(result) = scoring::resolve(map, live_board(game), &status.lasting, card) {
            let weight = if deep { DEEP_SCORING_WEIGHT * scoring_likelihood(game, card) } else { SCORING_WEIGHT };
            score += sign(result.vp_delta as f32) * weight;
            if let Some(winner) = result.automatic_victory {
                let europe = if deep { EUROPE_CONTROL * scoring_likelihood(game, card).max(0.3) } else { EUROPE_CONTROL };
                score += if winner == side { europe } else { -europe };
            }
        }
    }

    // DEFCON is a risk to the side that must keep acting at it.
    score -= match status.defcon {
        2 => DEFCON_2_PENALTY,
        3 => DEFCON_3_PENALTY,
        _ => 0.0,
    };

    // Military Operations: the shortfall at the turn's end is paid to the other side.
    let rounds = status.rounds_for(side).max(1) as f32;
    let progress = (status.action_round as f32 / rounds).min(1.0);
    let shortfall = |s: Superpower| {
        let ops = if s == Superpower::Us { status.military_ops_us } else { status.military_ops_ussr };
        (status.defcon as i8 - ops).max(0) as f32
    };
    score += MIL_OPS_WEIGHT * progress * (shortfall(side.opponent()) - shortfall(side));

    // The space race.
    let (us, ussr) = (status.space_race_us as f32, status.space_race_ussr as f32);
    score += sign(us - ussr) * SPACE_WEIGHT;

    // What is still in hand.
    for &card in game.hand(side) {
        let card = cards.card(card);
        if card.scoring {
            continue;
        }
        let ops = card.ops as f32;
        score += match card.side {
            CardSide::Neutral => HAND_OPS_WEIGHT * ops,
            s if s == crate::cards::side_of(side) => HAND_OPS_WEIGHT * ops,
            _ => 0.5 * ops - OPPONENT_CARD_PENALTY,
        };
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::OperationKind;

    /// A pending placement lives on the operation's speculative board; the evaluation must see it,
    /// or every `Place` scores the same and the AI always takes the first country listed.
    #[test]
    fn pending_influence_counts() {
        let map = WorldMap::standard().unwrap();
        let cards = CardCatalog::standard().unwrap();
        let mut game = Game::from_scenario(&crate::scenario::Scenario::demo(&map, &cards).unwrap());
        game.status_mut().active = Superpower::Us;
        let card = game.hand(Superpower::Us).iter().copied().find(|&c| !cards.card(c).scoring).unwrap();
        game.play_card(&cards, card).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        let before = evaluate(&game, &map, &cards, Superpower::Us);
        let crate::map::Found::One(canada) = map.find("Canada") else { panic!("Canada") };
        game.place(&map, canada).unwrap();
        assert!(evaluate(&game, &map, &cards, Superpower::Us) > before, "a pending point must raise the score");
    }

    /// In the Early War only Europe, Asia and the Middle East can be scored, and Europe counts most.
    #[test]
    fn early_war_weights_favour_europe_over_regions_not_yet_scorable() {
        let map = WorldMap::standard().unwrap();
        let cards = CardCatalog::standard().unwrap();
        let mut game = Game::from_scenario(&crate::scenario::Scenario::demo(&map, &cards).unwrap());
        game.status_mut().turn = 1;
        let w = |r| region_weights(&game, &cards, Superpower::Us).iter().find(|&&(x, _)| x == r).unwrap().1;
        assert!(w(Region::Europe) > w(Region::Asia));
        assert!(w(Region::Asia) > w(Region::Africa));
        assert!(w(Region::Africa) > 0.0);
    }
}
