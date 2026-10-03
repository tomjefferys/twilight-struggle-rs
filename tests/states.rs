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
        EventOutcome::Effect(_) => panic!("expected a scoring outcome"),
    }
}

fn vp_delta(outcome: &EventOutcome) -> i8 {
    match outcome {
        EventOutcome::Scoring(result) => result.vp_delta,
        EventOutcome::Effect(result) => result.vp_delta,
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
        EventOutcome::Effect(_) => panic!("expected a scoring outcome"),
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

// ---- fixed-effect card events (`data/states/events.json`) ----

/// Loads `events/<name>`, plays the active side's `card_name` and resolves
/// its event, returning the finished [`Game`] for the test to inspect.
fn play_effect_state(map: &WorldMap, cards: &CardCatalog, lib: &StateLibrary, name: &str, card_name: &str) -> Game {
    let reference = format!("events/{name}");
    let (scenario, _) = lib.load(map, cards, &reference).unwrap_or_else(|e| panic!("loading {reference}: {e}"));
    let mut game = Game::from_scenario(&scenario);
    let card = cards.id_by_name(card_name).unwrap_or_else(|| panic!("no card named {card_name}"));
    game.play_card(cards, card).unwrap_or_else(|e| panic!("{reference}: play_card: {e}"));
    let outcome = game.play_event(map, cards).unwrap_or_else(|e| panic!("{reference}: play_event: {e}"));
    assert!(matches!(outcome, EventOutcome::Effect(_)), "{reference}: expected an effect outcome");
    game
}

/// `(us, ussr)` influence in `country` on `game`'s board.
fn inf(map: &WorldMap, game: &Game, country: &str) -> (u8, u8) {
    let id = map.id_by_name(country).unwrap_or_else(|| panic!("no country {country}"));
    (game.board().influence(id, Superpower::Us), game.board().influence(id, Superpower::Ussr))
}

#[test]
fn duck_and_cover_degrades_defcon_and_pays_the_us() {
    let (map, cards, lib) = fixtures();
    let game = play_effect_state(&map, &cards, &lib, "duck-and-cover", "Duck and Cover");
    assert_eq!(game.status().defcon, 2);
    assert_eq!(game.status().vp, 3);
    assert_eq!(game.winner(), None);
    assert_eq!(game.active(), Superpower::Ussr, "the turn should pass");
    assert!(game.discards().contains(&cards.id_by_name("Duck and Cover").unwrap()), "Duck and Cover isn't removed after its event");
}

#[test]
fn duck_and_cover_at_defcon_2_loses_the_game_for_the_us() {
    let (map, cards, lib) = fixtures();
    let game = play_effect_state(&map, &cards, &lib, "duck-and-cover-defcon-1-loses", "Duck and Cover");
    assert_eq!(game.status().defcon, 1);
    assert_eq!(game.winner(), Some(Victory { side: Superpower::Ussr, reason: VictoryReason::Defcon }));
    assert_eq!(game.active(), Superpower::Us, "a finished game doesn't hand the turn over");
}

#[test]
fn fidel_and_romanian_abdication_take_control() {
    let (map, cards, lib) = fixtures();
    let game = play_effect_state(&map, &cards, &lib, "fidel", "Fidel");
    assert_eq!(inf(&map, &game, "Cuba"), (0, 3));
    assert!(game.removed_from_game().contains(&cards.id_by_name("Fidel").unwrap()));
    let game = play_effect_state(&map, &cards, &lib, "fidel-ussr-already-above-stability", "Fidel");
    assert_eq!(inf(&map, &game, "Cuba"), (0, 5), "USSR influence already past stability isn't topped up");
    let game = play_effect_state(&map, &cards, &lib, "romanian-abdication", "Romanian Abdication");
    assert_eq!(inf(&map, &game, "Romania"), (0, 3));
}

#[test]
fn nasser_removes_half_the_us_influence_rounded_up() {
    let (map, cards, lib) = fixtures();
    let game = play_effect_state(&map, &cards, &lib, "nasser-rounds-up", "Nasser");
    assert_eq!(inf(&map, &game, "Egypt"), (1, 2));
}

#[test]
fn the_simple_influence_swaps_land_where_the_card_says() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "de-gaulle-leads-france", "De Gaulle Leads France");
    assert_eq!(inf(&map, &g, "France"), (1, 1));
    let g = play_effect_state(&map, &cards, &lib, "john-paul-ii-elected-pope", "John Paul II Elected Pope");
    assert_eq!(inf(&map, &g, "Poland"), (1, 1));
    let g = play_effect_state(&map, &cards, &lib, "sadat-expels-soviets", "Sadat Expels Soviets");
    assert_eq!(inf(&map, &g, "Egypt"), (2, 0));
    let g = play_effect_state(&map, &cards, &lib, "iranian-hostage-crisis", "Iranian Hostage Crisis");
    assert_eq!(inf(&map, &g, "Iran"), (0, 3));
}

#[test]
fn the_pure_additions_add_to_every_named_country() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "portuguese-empire-crumbles", "Portuguese Empire Crumbles");
    assert_eq!((inf(&map, &g, "Angola"), inf(&map, &g, "SE African States")), ((0, 2), (0, 2)));
    let g = play_effect_state(&map, &cards, &lib, "allende", "Allende");
    assert_eq!(inf(&map, &g, "Chile"), (0, 2));
    let g = play_effect_state(&map, &cards, &lib, "panama-canal-returned", "Panama Canal Returned");
    for c in ["Panama", "Costa Rica", "Venezuela"] {
        assert_eq!(inf(&map, &g, c), (1, 0), "{c}");
    }
    let g = play_effect_state(&map, &cards, &lib, "awacs-sale-to-saudis", "AWACS Sale to Saudis");
    assert_eq!(inf(&map, &g, "Saudi Arabia"), (2, 0));
}

#[test]
fn camp_david_pays_one_vp_and_adds_influence() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "camp-david-accords", "Camp David Accords");
    assert_eq!(g.status().vp, 1);
    for c in ["Israel", "Jordan", "Egypt"] {
        assert_eq!(inf(&map, &g, c), (1, 0), "{c}");
    }
}

#[test]
fn the_iron_lady_wipes_the_uk_and_pays_the_us() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "the-iron-lady", "The Iron Lady");
    assert_eq!(inf(&map, &g, "UK"), (0, 0));
    assert_eq!(inf(&map, &g, "Argentina"), (0, 1));
    assert_eq!(g.status().vp, 1);
}

#[test]
fn nuclear_test_ban_pays_the_player_and_caps_defcon_at_5() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "nuclear-test-ban-us-defcon-4", "Nuclear Test Ban");
    assert_eq!((g.status().vp, g.status().defcon), (2, 5));
    let g = play_effect_state(&map, &cards, &lib, "nuclear-test-ban-ussr-defcon-3", "Nuclear Test Ban");
    assert_eq!((g.status().vp, g.status().defcon), (-1, 5), "the USSR's VP is negative on the track");
}

#[test]
fn kitchen_debates_needs_the_us_strictly_ahead_on_battlegrounds() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "kitchen-debates-us-ahead", "Kitchen Debates");
    assert_eq!(g.status().vp, 2);
    let g = play_effect_state(&map, &cards, &lib, "kitchen-debates-tied", "Kitchen Debates");
    assert_eq!(g.status().vp, 0);
}

#[test]
fn alliance_for_progress_counts_only_americas_battlegrounds() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "alliance-for-progress", "Alliance for Progress");
    assert_eq!(g.status().vp, 3, "Costa Rica isn't a battleground");
}

#[test]
fn reagan_bombs_libya_rounds_down() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "reagan-bombs-libya-odd", "Reagan Bombs Libya");
    assert_eq!(g.status().vp, 1);
}

#[test]
fn an_evil_empire_pays_one_vp_and_can_hit_the_vp_cap() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "an-evil-empire", "“An Evil Empire”");
    assert_eq!(g.status().vp, 1);
    let g = play_effect_state(&map, &cards, &lib, "vp-cap-us-wins", "“An Evil Empire”");
    assert_eq!(g.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Vp }));
}

#[test]
fn either_side_can_play_the_opponents_event() {
    let (map, cards, lib) = fixtures();
    let g = play_effect_state(&map, &cards, &lib, "opponents-event", "Kitchen Debates");
    assert_eq!(g.status().vp, 2, "Kitchen Debates pays the US even when the USSR plays it");
    assert_eq!(g.active(), Superpower::Us);
}

#[test]
fn a_defcon_loss_goes_against_the_phasing_player_not_the_cards_side() {
    let (map, cards, lib) = fixtures();
    let (scenario, _) = lib.load(&map, &cards, "events/duck-and-cover-defcon-1-loses").unwrap();
    let mut game = Game::from_scenario(&scenario);
    // Hand Duck and Cover (a US card) to the USSR, who is now phasing.
    game.status_mut().active = Superpower::Ussr;
    let card = cards.id_by_name("Duck and Cover").unwrap();
    game.hands_mut().take(card);
    game.hands_mut().push_to_hand(Superpower::Ussr, card);
    game.play_card(&cards, card).unwrap();
    game.play_event(&map, &cards).unwrap();
    assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Defcon }));
    assert_eq!(game.status().vp, 4, "Duck and Cover still pays the US 5-1 = 4 VP");
}

#[test]
fn an_implemented_non_scoring_card_offers_both_its_event_and_its_ops() {
    use twilight_struggle::{Action, OperationKind};
    let (map, cards, lib) = fixtures();
    let (scenario, _) = lib.load(&map, &cards, "events/fidel").unwrap();
    let mut game = Game::from_scenario(&scenario);
    game.play_card(&cards, cards.id_by_name("Fidel").unwrap()).unwrap();
    let legal = game.legal_actions(&map, &cards);
    assert!(legal.contains(&Action::Event));
    assert!(legal.contains(&Action::Begin(OperationKind::Coup)));
}
