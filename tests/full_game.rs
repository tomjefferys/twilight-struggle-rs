//! Soak tests: two AIs play whole games from a new game's setup to a result. Every game has to
//! finish (a winner, or a draw) without a panic, a refused move, a stalled phase, an invalid
//! status, or a card in two places at once — which is the main guard against the turn
//! structure (setup, headline, action rounds, turn end, final scoring) getting stuck.

use std::collections::HashSet;

use twilight_struggle::game::{Phase, VictoryReason};
use twilight_struggle::{play_turn, Ai, CardCatalog, CardId, Dice, Game, HeuristicAi, RandomAi, Superpower, WorldMap};

/// How many seeds each soak test plays: 50 normally, more with `SOAK_SEEDS=1000 cargo test --test full_game`.
fn seeds() -> u64 {
    std::env::var("SOAK_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(50)
}

fn fixtures() -> (WorldMap, CardCatalog) {
    (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
}

/// No card may be in two piles (or a pile and the play area) at once.
fn assert_no_duplicate_cards(game: &Game, context: &str) {
    let hands = game.hands();
    let mut seen: HashSet<CardId> = HashSet::new();
    let all = [twilight_struggle::Superpower::Us, twilight_struggle::Superpower::Ussr]
        .into_iter()
        .flat_map(|side| hands.hand(side).iter().copied())
        .chain(hands.deck().iter().copied())
        .chain(hands.discards().iter().copied())
        .chain(hands.removed().iter().copied())
        .chain(game.card_in_play());
    for card in all {
        assert!(seen.insert(card), "{context}: card {} is in two places", card.number());
    }
}

/// Plays one full game; returns how it ended.
fn play_game(seed: u64, careful: bool) -> (u8, Option<VictoryReason>) {
    let mut ai = RandomAi::from_seed(seed ^ 0xabcdef);
    if careful {
        ai = ai.careful();
    }
    let (turn, victory) = play_with(seed, &mut ai, None);
    (turn, Some(victory.reason))
}

/// Plays a whole game. With `other`, the USSR is driven by `ai` and the US by `other`; without it
/// `ai` plays both. Returns the last turn reached and the result.
fn play_with(seed: u64, ai: &mut dyn Ai, mut other: Option<&mut dyn Ai>) -> (u8, twilight_struggle::game::Victory) {
    let (map, cards) = fixtures();
    let mut dice = Dice::from_seed(seed);
    let mut game = Game::new_game(&map, &cards, &mut dice);
    for call in 0..3000 {
        if game.winner().is_some() {
            break;
        }
        let context = format!("seed {seed} call {call} (turn {}, {:?})", game.status().turn, game.phase());
        let result = match other.as_mut() {
            Some(us) if game.decider() == Superpower::Us => play_turn(&mut **us, &mut game, &map, &cards, &mut dice),
            _ => play_turn(&mut *ai, &mut game, &map, &cards, &mut dice),
        };
        result.unwrap_or_else(|e| panic!("{context}: {e}"));
        game.status().validate().unwrap_or_else(|e| panic!("{context}: invalid status: {e}"));
        assert_no_duplicate_cards(&game, &context);
    }
    let victory = game.winner().unwrap_or_else(|| panic!("seed {seed}: no result after 3000 turns — stuck in {:?} on turn {}", game.phase(), game.status().turn));
    assert!(game.legal_actions(&map, &cards).is_empty(), "seed {seed}: a finished game offers no moves");
    (game.status().turn, victory)
}

#[test]
fn careful_ais_play_whole_games_to_a_result() {
    let mut reached_final_scoring = 0;
    let mut longest = 0;
    for seed in 0..seeds() {
        let (turn, reason) = play_game(seed, true);
        longest = longest.max(turn);
        if reason == Some(VictoryReason::FinalScoring) {
            reached_final_scoring += 1;
        }
    }
    assert_eq!(longest, 10, "some game should go the distance");
    assert!(reached_final_scoring > 0, "and at least one should end by final scoring");
}

#[test]
fn uniformly_random_ais_never_get_a_game_stuck_either() {
    for seed in 0..seeds() {
        play_game(seed, false);
    }
}

#[test]
fn a_game_that_ends_leaves_the_phase_alone() {
    let (map, cards) = fixtures();
    let mut dice = Dice::from_seed(3);
    let mut game = Game::new_game(&map, &cards, &mut dice);
    assert_eq!(game.phase(), Phase::Setup);
    let mut ai = RandomAi::from_seed(3).careful();
    while game.winner().is_none() {
        play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
    }
    assert!(game.winner().is_some());
}

#[test]
fn heuristic_ais_play_whole_games_to_a_result() {
    for seed in 0..(seeds() / 10).max(3) {
        let mut ai = HeuristicAi::from_seed(seed);
        play_with(seed, &mut ai, None);
    }
}

#[test]
fn heuristic_and_random_ais_finish_games_against_each_other() {
    for seed in 0..(seeds() / 10).max(3) {
        let mut smart = HeuristicAi::from_seed(seed);
        let mut random = RandomAi::from_seed(seed).careful();
        play_with(seed, &mut smart, Some(&mut random));
        let mut smart = HeuristicAi::from_seed(seed);
        let mut random = RandomAi::from_seed(seed).careful();
        play_with(seed, &mut random, Some(&mut smart));
    }
}
