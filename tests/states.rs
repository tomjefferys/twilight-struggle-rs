//! Integration tests for the named test-state library (`src/states.rs`,
//! `data/states/`): every bundled state loads cleanly and, for the
//! scoring states specifically, playing the card and its event through a
//! real [`Game`] produces the outcome the state's own description
//! promises. A new card implementation is expected to add its own states
//! here alongside its entry in `data/states/<topic>.json` — see
//! `CLAUDE.md`'s own convention note.

use twilight_struggle::game::{Victory, VictoryReason};
use twilight_struggle::scoring::{ScoringKind, Tier};
use twilight_struggle::{CardCatalog, EventOutcome, Game, StateLibrary, Superpower, WorldMap};

fn fixtures() -> (WorldMap, CardCatalog, StateLibrary) {
    (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap(), StateLibrary::standard())
}

/// Loads `reference`, plays the active side's scoring card, then its
/// event — the same two steps `play <card>` then `event` drive from the
/// REPL — and returns the resolved [`Game`] plus the event's own outcome
/// for the test to inspect.
fn play_scoring_state(map: &WorldMap, cards: &CardCatalog, lib: &StateLibrary, reference: &str) -> (Game, EventOutcome) {
    let (scenario, _description) = lib.load(map, cards, reference).unwrap_or_else(|e| panic!("loading {reference}: {e}"));
    let mut game = Game::from_scenario(&scenario);
    let side = game.active();
    let card = *game.hand(side).iter().find(|&&id| cards.card(id).scoring).unwrap_or_else(|| panic!("{reference}: active side's hand has no scoring card"));
    game.play_card(cards, card).unwrap_or_else(|e| panic!("{reference}: play_card: {e}"));
    let outcome = game.play_event(map, cards).unwrap_or_else(|e| panic!("{reference}: play_event: {e}"));
    (game, outcome)
}

fn region_tiers(outcome: &EventOutcome) -> (Tier, Tier) {
    match outcome {
        EventOutcome::Scoring(result) => match &result.kind {
            ScoringKind::Region { us, ussr, .. } => (us.tier, ussr.tier),
            ScoringKind::SoutheastAsia { .. } => panic!("expected a region scoring result"),
        },
    }
}

fn vp_delta(outcome: &EventOutcome) -> i8 {
    match outcome {
        EventOutcome::Scoring(result) => result.vp_delta,
    }
}

#[test]
fn every_bundled_state_loads_cleanly() {
    let (map, cards, lib) = fixtures();
    let entries = lib.list().unwrap();
    assert!(!entries.is_empty(), "expected at least the scoring states to be bundled");
    for entry in &entries {
        lib.load(&map, &cards, &entry.reference()).unwrap_or_else(|e| panic!("{}: {e}", entry.reference()));
    }
}

#[test]
fn europe_ussr_control_wins_the_game_outright() {
    let (map, cards, lib) = fixtures();
    let (game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/europe-ussr-control-wins");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(ussr_tier, Tier::Control);
    assert_ne!(us_tier, Tier::Control);
    assert_eq!(game.winner(), Some(Victory { side: Superpower::Ussr, reason: VictoryReason::EuropeControl }));
    // A card whose event just won the game is never discarded — there's
    // nothing left to hand the turn to.
    assert!(game.discards().contains(&cards.id_by_name("Europe Scoring").unwrap()));
}

#[test]
fn europe_us_domination_beats_ussr_presence() {
    let (map, cards, lib) = fixtures();
    let (game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/europe-us-domination");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(us_tier, Tier::Domination);
    assert_eq!(ussr_tier, Tier::Presence);
    assert!(vp_delta(&outcome) > 0, "US domination should outweigh USSR presence");
    assert_eq!(game.winner(), None);
}

#[test]
fn asia_us_control_scores_the_full_bonus_stack() {
    let (map, cards, lib) = fixtures();
    let (_game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/asia-us-control");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(us_tier, Tier::Control);
    assert_eq!(ussr_tier, Tier::None);
    // Control (9) + 6 battlegrounds + 2 adjacency (Afghanistan, North Korea) = 17.
    assert_eq!(vp_delta(&outcome), 17);
}

#[test]
fn middle_east_presence_vs_presence_is_a_net_tie() {
    let (map, cards, lib) = fixtures();
    let (game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/middle-east-tie");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(us_tier, Tier::Presence);
    assert_eq!(ussr_tier, Tier::Presence);
    assert_eq!(vp_delta(&outcome), 0);
    assert_eq!(game.winner(), None);
}

#[test]
fn domination_needs_a_non_battleground_country_too() {
    let (map, cards, lib) = fixtures();
    let (_game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/domination-needs-non-battleground");
    let (us_tier, _) = region_tiers(&outcome);
    assert_eq!(us_tier, Tier::Presence, "2 of 3 battlegrounds and more countries than the USSR still isn't Domination without a non-battleground");
}

#[test]
fn africa_domination_applies_once_theres_a_non_battleground() {
    let (map, cards, lib) = fixtures();
    let (_game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/africa-us-domination");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(us_tier, Tier::Domination);
    assert_eq!(ussr_tier, Tier::None);
}

#[test]
fn south_america_ussr_control_scores_without_an_adjacency_bonus() {
    let (map, cards, lib) = fixtures();
    let (_game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/south-america-ussr-control");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(ussr_tier, Tier::Control);
    assert_eq!(us_tier, Tier::None);
    // Control (6) + 4 battlegrounds + 0 adjacency (no South American
    // country borders a superpower) = 10, favouring the USSR.
    assert_eq!(vp_delta(&outcome), -10);
}

#[test]
fn southeast_asia_pays_per_country_and_is_removed_not_discarded() {
    let (map, cards, lib) = fixtures();
    let (game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/southeast-asia-mixed");
    match &outcome {
        EventOutcome::Scoring(result) => match &result.kind {
            ScoringKind::SoutheastAsia { controlled } => assert_eq!(controlled.len(), 4),
            ScoringKind::Region { .. } => panic!("expected a Southeast Asia result"),
        },
    }
    assert_eq!(vp_delta(&outcome), 1);
    let card = cards.id_by_name("Southeast Asia Scoring").unwrap();
    assert!(game.removed_from_game().contains(&card), "a removed_after_event card's event should remove it, not discard it");
    assert!(!game.discards().contains(&card));
}

#[test]
fn the_vp_cap_ends_the_game_for_the_us() {
    let (map, cards, lib) = fixtures();
    let (game, _outcome) = play_scoring_state(&map, &cards, &lib, "scoring/vp-cap-us-wins");
    assert_eq!(game.status().vp, 20);
    assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Vp }));
}

#[test]
fn the_vp_cap_ends_the_game_for_the_ussr() {
    let (map, cards, lib) = fixtures();
    let (game, _outcome) = play_scoring_state(&map, &cards, &lib, "scoring/vp-cap-ussr-wins");
    assert_eq!(game.status().vp, -20);
    assert_eq!(game.winner(), Some(Victory { side: Superpower::Ussr, reason: VictoryReason::Vp }));
}

#[test]
fn an_empty_region_scores_nothing() {
    let (map, cards, lib) = fixtures();
    let (game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/empty-region");
    let (us_tier, ussr_tier) = region_tiers(&outcome);
    assert_eq!(us_tier, Tier::None);
    assert_eq!(ussr_tier, Tier::None);
    assert_eq!(vp_delta(&outcome), 0);
    assert_eq!(game.status().vp, 0);
    assert_eq!(game.winner(), None);
}
