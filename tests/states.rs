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
        EventOutcome::Effect(_) | EventOutcome::Pending { .. } => panic!("expected a scoring outcome"),
    }
}

fn vp_delta(outcome: &EventOutcome) -> i8 {
    match outcome {
        EventOutcome::Scoring(result) => result.vp_delta,
        EventOutcome::Effect(result) => result.vp_delta,
        EventOutcome::Pending { .. } => panic!("a pending event has no VP yet"),
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
        EventOutcome::Effect(_) | EventOutcome::Pending { .. } => panic!("expected a scoring outcome"),
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

#[test]
fn central_america_scoring_pays_control_and_battlegrounds() {
    let (map, cards, lib) = fixtures();
    let (game, outcome) = play_scoring_state(&map, &cards, &lib, "scoring/central-america-us-control");
    assert_eq!(vp_delta(&outcome), 8);
    assert_eq!(game.status().vp, 8);
    assert_eq!(game.winner(), None);
}

/// A backstop for every implemented card, present and future: from a blank
/// board, playing it as an event through a real `Game` (for each side
/// holding it) must succeed, discard or remove it, and hand the turn over.
/// Not a substitute for a card's own state and assertions — just
/// guarantees none is ever implemented without at least running.
#[test]
fn every_implemented_event_plays_through_game() {
    use twilight_struggle::{events, Scenario};
    let (map, cards, _) = fixtures();
    let mut played = 0;
    for card in cards.iter() {
        let id = cards.id_by_name(&card.name).unwrap();
        if !events::is_implemented(id) {
            continue;
        }
        for side in [Superpower::Us, Superpower::Ussr] {
            let mut scenario = Scenario::blank(&map);
            scenario.status.active = side;
            let mut game = Game::from_scenario(&scenario);
            game.hands_mut().push_to_hand(side, id);
            game.play_card(&cards, id).unwrap_or_else(|e| panic!("{} ({side}): play_card: {e}", card.name));
            game.play_event(&map, &cards).unwrap_or_else(|e| panic!("{} ({side}): play_event: {e}", card.name));
            // A choice card opens a session for its own side; let a random
            // chooser carry it out — every session must be finishable.
            let mut ai = twilight_struggle::RandomAi::from_seed(7);
            let mut dice = twilight_struggle::Dice::from_seed(7);
            for _ in 0..3 {
                if game.operation().is_some() {
                    twilight_struggle::play_turn(&mut ai, &mut game, &map, &cards, &mut dice)
                        .unwrap_or_else(|e| panic!("{} ({side}): AI could not finish the event: {e}", card.name));
                }
            }
            assert!(game.operation().is_none(), "{} ({side}) left its event session open", card.name);
            assert!(
                game.discards().contains(&id) || game.removed_from_game().contains(&id),
                "{} ({side}) should end up discarded or removed",
                card.name
            );
            assert!(game.winner().is_some() || game.active() != side, "{} ({side}) should hand the turn over", card.name);
            played += 1;
        }
    }
    assert!(played >= 2 * 45, "expected every implemented card to be exercised, only {played} runs");
}

// ---------------------------------------------------------------------
// Choice events (`events::choice`, `data/states/choices.json`): cards
// whose text makes a player pick countries. Each opens an
// `Operation::Event` session for the *card's own* side, staged on a
// speculative board until `confirm`.
// ---------------------------------------------------------------------

mod choices {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::Operation;
    use twilight_struggle::{CountryId, Scenario};

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    /// Loads `choices/<name>`, plays the active side's first card, and plays
    /// its event, expecting a session to open for `chooser`.
    fn open(name: &str, chooser: Superpower) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("choices/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        let side = game.active();
        let card = game.hand(side)[0];
        game.play_card(&cards, card).unwrap();
        let outcome = game.play_event(&map, &cards).unwrap_or_else(|e| panic!("{name}: play_event: {e}"));
        assert!(matches!(outcome, EventOutcome::Pending { chooser: c, .. } if c == chooser), "{name}: expected a pending session, got {outcome:?}");
        assert_eq!(game.decider(), chooser);
        (map, cards, game)
    }

    fn plus(game: &mut Game, map: &WorldMap, names: &[&str]) {
        for n in names {
            game.place(map, id(map, n)).unwrap_or_else(|e| panic!("+ {n}: {e}"));
        }
    }

    fn minus(game: &mut Game, map: &WorldMap, names: &[&str]) {
        for n in names {
            game.unplace(map, id(map, n)).unwrap_or_else(|e| panic!("- {n}: {e}"));
        }
    }

    fn inf(game: &Game, map: &WorldMap, name: &str, side: Superpower) -> u8 {
        game.board().influence(id(map, name), side)
    }

    #[test]
    fn nothing_reaches_the_real_board_until_confirm() {
        let (map, _cards, mut game) = open("comecon", Superpower::Ussr);
        plus(&mut game, &map, &["Hungary"]);
        assert_eq!(inf(&game, &map, "Hungary", Superpower::Ussr), 0, "staged picks live on the speculative board");
        assert_eq!(game.view_board().influence(id(&map, "Hungary"), Superpower::Ussr), 1);
    }

    #[test]
    fn comecon_needs_four_distinct_countries_and_skips_us_controlled_poland() {
        let (map, cards, mut game) = open("comecon", Superpower::Ussr);
        assert!(game.place(&map, id(&map, "Poland")).is_err(), "US-controlled");
        plus(&mut game, &map, &["East Germany", "Czechoslovakia", "Hungary"]);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "3 of 4 isn't carried out fully");
        assert!(game.place(&map, id(&map, "Hungary")).is_err(), "1 per country");
        plus(&mut game, &map, &["Romania"]);
        assert!(game.place(&map, id(&map, "Bulgaria")).is_err(), "only 4 countries");
        game.confirm().unwrap();
        for n in ["East Germany", "Czechoslovakia", "Hungary", "Romania"] {
            assert_eq!(inf(&game, &map, n, Superpower::Ussr), 1, "{n}");
        }
        assert_eq!(inf(&game, &map, "Bulgaria", Superpower::Ussr), 0);
        assert_eq!(game.active(), Superpower::Us, "confirming hands the turn over");
        assert!(game.removed_from_game().contains(&cards.id_by_name("Comecon").unwrap()));
    }

    #[test]
    fn minus_takes_back_a_staged_add_and_frees_the_budget() {
        let (map, _cards, mut game) = open("comecon", Superpower::Ussr);
        plus(&mut game, &map, &["Hungary", "Romania", "Bulgaria", "East Germany"]);
        minus(&mut game, &map, &["Hungary"]);
        plus(&mut game, &map, &["Czechoslovakia"]);
        assert_eq!(game.view_board().influence(id(&map, "Hungary"), Superpower::Ussr), 0);
        game.confirm().unwrap();
    }

    #[test]
    fn undo_takes_back_the_latest_pick_and_a_session_with_picks_cannot_be_abandoned() {
        let (map, _cards, mut game) = open("comecon", Superpower::Ussr);
        plus(&mut game, &map, &["Hungary"]);
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandonEvent)));
        assert_eq!(game.undo(&map).unwrap(), id(&map, "Hungary"));
        assert!(matches!(game.undo(&map), Err(GameError::NothingToUndo)));
        game.abandon().unwrap();
        assert!(game.card_in_play().is_some(), "abandoning leaves the card in play");
        game.return_card().unwrap();
    }

    #[test]
    fn an_event_cannot_be_cancelled() {
        let (_map, _cards, mut game) = open("comecon", Superpower::Ussr);
        assert!(matches!(game.cancel(), Err(GameError::CannotCancelEvent)));
    }

    #[test]
    fn socialist_governments_removes_three_us_influence_two_per_country() {
        let (map, _cards, mut game) = open("socialist-governments", Superpower::Ussr);
        assert!(game.unplace(&map, id(&map, "Poland")).is_err(), "Eastern Europe");
        minus(&mut game, &map, &["West Germany", "West Germany"]);
        assert!(game.unplace(&map, id(&map, "West Germany")).is_err(), "max 2 per country");
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
        minus(&mut game, &map, &["France"]);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "West Germany", Superpower::Us), 3);
        assert_eq!(inf(&game, &map, "France", Superpower::Us), 2);
    }

    #[test]
    fn warsaw_pact_remove_mode_wipes_four_countries_and_add_mode_places_five() {
        let (map, _cards, mut game) = open("warsaw-pact", Superpower::Ussr);
        assert!(game.place(&map, id(&map, "Poland")).is_err(), "choose a mode first");
        game.choose_mode(&map, 0).unwrap();
        minus(&mut game, &map, &["East Germany", "Poland", "Hungary", "Bulgaria"]);
        assert!(game.unplace(&map, id(&map, "Romania")).is_err(), "only 4 countries");
        game.confirm().unwrap();
        for n in ["East Germany", "Poland", "Hungary", "Bulgaria"] {
            assert_eq!(inf(&game, &map, n, Superpower::Us), 0, "{n}");
        }
        assert_eq!(inf(&game, &map, "Romania", Superpower::Us), 1);

        let (map, _cards, mut game) = open("warsaw-pact", Superpower::Ussr);
        game.choose_mode(&map, 1).unwrap();
        plus(&mut game, &map, &["Poland", "Poland", "Hungary", "Hungary", "Romania"]);
        assert!(game.place(&map, id(&map, "Poland")).is_err(), "max 2 per country");
        assert!(game.place(&map, id(&map, "Bulgaria")).is_err(), "5 points spent");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Poland", Superpower::Ussr), 2);
    }

    #[test]
    fn a_mode_can_be_changed_until_the_first_pick() {
        let (map, _cards, mut game) = open("warsaw-pact", Superpower::Ussr);
        game.choose_mode(&map, 0).unwrap();
        game.choose_mode(&map, 1).unwrap();
        plus(&mut game, &map, &["Poland"]);
        assert!(game.choose_mode(&map, 0).is_err());
        assert!(game.choose_mode(&map, 5).is_err());
    }

    #[test]
    fn truman_doctrine_wipes_one_uncontrolled_european_country() {
        let (map, _cards, mut game) = open("truman-doctrine", Superpower::Us);
        assert!(game.unplace(&map, id(&map, "Greece")).is_err(), "USSR-controlled");
        assert!(game.unplace(&map, id(&map, "Poland")).is_err(), "USSR-controlled");
        minus(&mut game, &map, &["Italy"]);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Italy", Superpower::Ussr), 0);
        assert_eq!(inf(&game, &map, "Greece", Superpower::Ussr), 2);
    }

    #[test]
    fn a_choice_with_nothing_to_choose_resolves_immediately() {
        for name in ["truman-doctrine-nothing-to-do", "special-relationship-uk-not-controlled"] {
            let (map, cards, lib) = fixtures();
            let (scenario, _) = lib.load(&map, &cards, &format!("choices/{name}")).unwrap();
            let mut game = Game::from_scenario(&scenario);
            let card = game.hand(Superpower::Us)[0];
            game.play_card(&cards, card).unwrap();
            let outcome = game.play_event(&map, &cards).unwrap();
            assert!(matches!(outcome, EventOutcome::Effect(_)), "{name}");
            assert!(game.operation().is_none(), "{name}");
            assert_eq!(game.active(), Superpower::Ussr, "{name}");
        }
    }

    #[test]
    fn independent_reds_raises_the_us_to_the_ussr_level() {
        let (map, _cards, mut game) = open("independent-reds", Superpower::Us);
        assert!(game.place(&map, id(&map, "Hungary")).is_err(), "the US is already ahead there");
        assert!(game.place(&map, id(&map, "Poland")).is_err(), "not one of the five");
        plus(&mut game, &map, &["Romania"]);
        assert!(game.place(&map, id(&map, "Yugoslavia")).is_err(), "one country only");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Romania", Superpower::Us), 3);
    }

    #[test]
    fn marshall_plan_skips_ussr_controlled_countries_and_takes_seven() {
        let (map, _cards, mut game) = open("marshall-plan", Superpower::Us);
        assert!(game.place(&map, id(&map, "West Germany")).is_err());
        plus(&mut game, &map, &["UK", "France", "Italy", "Greece", "Turkey", "Benelux", "Norway"]);
        assert!(game.place(&map, id(&map, "Sweden")).is_err(), "7 countries");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Turkey", Superpower::Us), 1);
    }

    #[test]
    fn suez_crisis_removes_four_two_per_country_from_the_named_three() {
        let (map, _cards, mut game) = open("suez-crisis", Superpower::Ussr);
        assert!(game.unplace(&map, id(&map, "Italy")).is_err());
        minus(&mut game, &map, &["France", "France", "Israel", "Israel"]);
        assert!(game.unplace(&map, id(&map, "UK")).is_err(), "4 points spent");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "France", Superpower::Us), 1);
        assert_eq!(inf(&game, &map, "Israel", Superpower::Us), 2);
        assert_eq!(inf(&game, &map, "UK", Superpower::Us), 1);
    }

    #[test]
    fn east_european_unrest_removes_one_early_and_two_late() {
        let (map, _cards, mut game) = open("east-european-unrest-early", Superpower::Us);
        minus(&mut game, &map, &["Poland", "Hungary", "Romania"]);
        assert!(game.unplace(&map, id(&map, "Poland")).is_err(), "one per country early");
        assert!(game.unplace(&map, id(&map, "Bulgaria")).is_err(), "3 countries");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Poland", Superpower::Ussr), 1);
        assert_eq!(inf(&game, &map, "Hungary", Superpower::Ussr), 2);

        let (map, _cards, mut game) = open("east-european-unrest-late", Superpower::Us);
        minus(&mut game, &map, &["Poland", "Hungary", "Romania"]);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Poland", Superpower::Ussr), 0, "2 removed late");
        assert_eq!(inf(&game, &map, "Hungary", Superpower::Ussr), 1);
        assert_eq!(inf(&game, &map, "Romania", Superpower::Ussr), 0, "only 1 there to remove");
    }

    #[test]
    fn decolonization_and_colonial_rear_guards_use_africa_and_southeast_asia() {
        for (name, chooser, side) in [("decolonization", Superpower::Ussr, Superpower::Ussr), ("colonial-rear-guards", Superpower::Us, Superpower::Us)] {
            let (map, _cards, mut game) = open(name, chooser);
            assert!(game.place(&map, id(&map, "Poland")).is_err(), "{name}: Europe");
            plus(&mut game, &map, &["Angola", "Thailand", "Vietnam", "Kenya"]);
            assert!(game.place(&map, id(&map, "Zaire")).is_err(), "{name}: 4 countries");
            game.confirm().unwrap();
            assert_eq!(inf(&game, &map, "Thailand", side), 1, "{name}");
        }
    }

    #[test]
    fn de_stalinization_reallocates_up_to_four_and_may_stop_early() {
        let (map, _cards, mut game) = open("de-stalinization", Superpower::Ussr);
        assert!(game.place(&map, id(&map, "Hungary")).is_err(), "nothing removed yet to move");
        minus(&mut game, &map, &["Poland", "Poland"]);
        plus(&mut game, &map, &["Hungary", "Hungary"]);
        assert!(game.place(&map, id(&map, "Hungary")).is_err(), "max 2 added per country");
        assert!(game.place(&map, id(&map, "Canada")).is_err(), "US-controlled");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Poland", Superpower::Ussr), 1);
        assert_eq!(inf(&game, &map, "Hungary", Superpower::Ussr), 2);

        // An unbalanced reallocation (removed but not yet placed) can't be confirmed…
        let (map, _cards, mut game) = open("de-stalinization", Superpower::Ussr);
        minus(&mut game, &map, &["Poland"]);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
        plus(&mut game, &map, &["Hungary"]);
        game.confirm().unwrap();

        // …while doing nothing at all is allowed ("may").
        let (_map, _cards, mut game) = open("de-stalinization", Superpower::Ussr);
        game.confirm().unwrap();
    }

    #[test]
    fn south_african_unrest_has_two_modes() {
        let (map, _cards, mut game) = open("south-african-unrest", Superpower::Ussr);
        game.choose_mode(&map, 0).unwrap();
        assert_eq!(game.view_board().influence(id(&map, "South Africa"), Superpower::Ussr), 2);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "South Africa", Superpower::Ussr), 2);

        let (map, _cards, mut game) = open("south-african-unrest", Superpower::Ussr);
        game.choose_mode(&map, 1).unwrap();
        assert!(game.place(&map, id(&map, "Zaire")).is_err(), "not adjacent to South Africa");
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
        plus(&mut game, &map, &["Angola"]);
        assert!(game.place(&map, id(&map, "Botswana")).is_err(), "a single country");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "South Africa", Superpower::Ussr), 1);
        assert_eq!(inf(&game, &map, "Angola", Superpower::Ussr), 2);
    }

    #[test]
    fn muslim_revolution_wipes_two_of_the_listed_countries() {
        let (map, _cards, mut game) = open("muslim-revolution", Superpower::Ussr);
        assert!(game.unplace(&map, id(&map, "Israel")).is_err(), "not on the list");
        minus(&mut game, &map, &["Iran", "Iraq"]);
        assert!(game.unplace(&map, id(&map, "Egypt")).is_err(), "only 2 countries");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Iran", Superpower::Us), 0);
        assert_eq!(inf(&game, &map, "Iraq", Superpower::Us), 0);
        assert_eq!(inf(&game, &map, "Egypt", Superpower::Us), 1);
    }

    #[test]
    fn puppet_governments_only_offers_empty_countries_and_may_stop_early() {
        let (map, _cards, mut game) = open("puppet-governments", Superpower::Us);
        assert!(game.place(&map, id(&map, "Poland")).is_err(), "USSR influence there");
        assert!(game.place(&map, id(&map, "Italy")).is_err(), "US influence there");
        plus(&mut game, &map, &["Finland"]);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Finland", Superpower::Us), 1);
    }

    #[test]
    fn oas_and_liberation_theology_respect_their_regions_and_caps() {
        let (map, _cards, mut game) = open("oas-founded", Superpower::Us);
        assert!(game.place(&map, id(&map, "Poland")).is_err());
        plus(&mut game, &map, &["Brazil", "Brazil"]);
        assert!(game.place(&map, id(&map, "Chile")).is_err(), "2 points");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Brazil", Superpower::Us), 2, "no per-country cap");

        let (map, _cards, mut game) = open("liberation-theology", Superpower::Ussr);
        assert!(game.place(&map, id(&map, "Brazil")).is_err(), "South America isn't Central America");
        plus(&mut game, &map, &["Panama", "Panama", "Costa Rica"]);
        assert!(game.place(&map, id(&map, "Panama")).is_err(), "max 2 per country");
        game.confirm().unwrap();
    }

    #[test]
    fn the_reformer_adds_four_when_behind_and_six_when_ahead() {
        for (name, total) in [("the-reformer-behind", 4), ("the-reformer-ahead", 6)] {
            let (map, _cards, mut game) = open(name, Superpower::Ussr);
            let countries = ["Poland", "Hungary", "Romania", "Bulgaria", "East Germany", "Finland"];
            let mut placed = 0;
            'outer: for n in countries {
                for _ in 0..2 {
                    if game.place(&map, id(&map, n)).is_err() {
                        break 'outer;
                    }
                    placed += 1;
                }
            }
            assert_eq!(placed, total, "{name}");
            game.confirm().unwrap();
        }
    }

    #[test]
    fn the_reformer_bars_the_ussr_from_couping_in_europe_from_then_on() {
        let (map, cards, mut game) = open("the-reformer-behind", Superpower::Ussr);
        plus(&mut game, &map, &["Poland", "Poland", "Hungary", "Hungary"]);
        game.confirm().unwrap();
        // Give the US influence in Poland and Egypt, hand the USSR a card, and try coups.
        game.board_mut().set_influence(id(&map, "Poland"), Superpower::Us, 2);
        game.board_mut().set_influence(id(&map, "Egypt"), Superpower::Us, 2);
        game.status_mut().active = Superpower::Ussr;
        let fidel = cards.id_by_name("Fidel").unwrap();
        game.hands_mut().push_to_hand(Superpower::Ussr, fidel);
        game.play_card(&cards, fidel).unwrap();
        game.begin(twilight_struggle::OperationKind::Coup).unwrap();
        let mut dice = twilight_struggle::Dice::from_seed(1);
        assert!(matches!(game.roll(&map, id(&map, "Poland"), &mut dice), Err(GameError::Coup(_))));
        assert!(!game.operation().unwrap().is_legal_target(&map, game.board(), id(&map, "Poland")));
        assert!(game.operation().unwrap().is_legal_target(&map, game.board(), id(&map, "Egypt")), "other regions are unaffected");
    }

    #[test]
    fn marine_barracks_clears_lebanon_then_removes_two_more() {
        let (map, _cards, mut game) = open("marine-barracks-bombing", Superpower::Ussr);
        assert_eq!(game.view_board().influence(id(&map, "Lebanon"), Superpower::Us), 0, "Lebanon goes at once");
        assert!(game.unplace(&map, id(&map, "Lebanon")).is_err(), "nothing left there");
        minus(&mut game, &map, &["Israel", "Jordan"]);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Lebanon", Superpower::Us), 0);
        assert_eq!(inf(&game, &map, "Israel", Superpower::Us), 1);
        assert_eq!(inf(&game, &map, "Jordan", Superpower::Us), 0);
    }

    #[test]
    fn special_relationship_adds_one_us_influence_next_to_a_us_controlled_uk() {
        let (map, _cards, mut game) = open("special-relationship-uk-us-controlled", Superpower::Us);
        assert!(game.place(&map, id(&map, "Italy")).is_err(), "not adjacent to the UK");
        plus(&mut game, &map, &["France"]);
        assert!(game.place(&map, id(&map, "Canada")).is_err(), "a single country");
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "France", Superpower::Us), 1);
    }

    #[test]
    fn the_cards_own_side_chooses_even_when_the_other_side_is_phasing() {
        let (map, cards, lib) = fixtures();
        let (mut scenario, _): (Scenario, _) = lib.load(&map, &cards, "choices/comecon").unwrap();
        // The US is phasing, holding the USSR's Comecon.
        let comecon = cards.id_by_name("Comecon").unwrap();
        scenario.status.active = Superpower::Us;
        scenario.hands.push_to_hand(Superpower::Us, comecon);
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, comecon).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.active(), Superpower::Us);
        assert_eq!(game.decider(), Superpower::Ussr, "the USSR places its own influence");
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandonEvent)), "the USSR can't take back the US player's choice");
        let Some(Operation::Event(e)) = game.operation() else { panic!("expected an open event") };
        assert_eq!(e.chooser(), Superpower::Ussr);
        plus(&mut game, &map, &["East Germany", "Czechoslovakia", "Hungary", "Romania"]);
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Ussr, "after the US's action round the turn passes on");
    }

    #[test]
    fn minus_on_an_influence_placement_takes_back_a_point_in_that_country() {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "events/fidel").unwrap();
        let mut game = Game::from_scenario(&scenario);
        let card = game.hand(Superpower::Ussr)[0];
        game.play_card(&cards, card).unwrap();
        game.begin(twilight_struggle::OperationKind::Influence).unwrap();
        let cuba = id(&map, "Cuba");
        game.place(&map, cuba).unwrap();
        game.place(&map, cuba).unwrap();
        let before = game.ops_available();
        game.unplace(&map, cuba).unwrap();
        assert_eq!(game.view_board().influence(cuba, Superpower::Ussr), 2, "1 placed + 1 already there");
        assert_eq!(game.ops_available(), before + 1);
        game.unplace(&map, cuba).unwrap();
        assert!(matches!(game.unplace(&map, cuba), Err(GameError::NothingToUndo)));
    }

    // ---- shortages: fewer legal targets than the card asks for ----

    /// A blank board with `setup` applied, `card` in the active side's hand and
    /// played as an event. Returns the game and what `play_event` returned.
    fn play_on(active: Superpower, card: &str, setup: impl FnOnce(&WorldMap, &mut twilight_struggle::Board)) -> (WorldMap, Game, EventOutcome) {
        let (map, cards, _) = fixtures();
        let mut scenario = Scenario::blank(&map);
        scenario.status.active = active;
        setup(&map, &mut scenario.board);
        let id = cards.id_by_name(card).unwrap();
        scenario.hands.push_to_hand(active, id);
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, id).unwrap();
        let outcome = game.play_event(&map, &cards).unwrap();
        (map, game, outcome)
    }

    /// US-controls every listed country (5 influence beats any stability here).
    fn us_control(map: &WorldMap, board: &mut twilight_struggle::Board, names: &[&str]) {
        for n in names {
            board.set_influence(id(map, n), Superpower::Us, 5);
        }
    }

    const EASTERN: [&str; 9] =
        ["East Germany", "Poland", "Czechoslovakia", "Hungary", "Romania", "Yugoslavia", "Bulgaria", "Finland", "Austria"];

    #[test]
    fn comecon_with_fewer_than_four_eligible_countries_places_what_it_can_then_confirms() {
        // The US controls all but Hungary and Romania.
        let (map, mut game, outcome) = play_on(Superpower::Ussr, "Comecon", |m, b| {
            us_control(m, b, &EASTERN.iter().copied().filter(|n| !["Hungary", "Romania"].contains(n)).collect::<Vec<_>>());
        });
        assert!(matches!(outcome, EventOutcome::Pending { .. }));
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "two legal picks are still unmade");
        plus(&mut game, &map, &["Hungary"]);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "one legal pick is still unmade");
        plus(&mut game, &map, &["Romania"]);
        game.confirm().expect("nothing else is legal, so 2 of 4 is as full as it gets");
        assert_eq!(inf(&game, &map, "Hungary", Superpower::Ussr), 1);
        assert_eq!(inf(&game, &map, "Romania", Superpower::Ussr), 1);
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn comecon_with_no_eligible_country_resolves_at_once_and_still_spends_the_card() {
        let (map, game, outcome) = play_on(Superpower::Ussr, "Comecon", |m, b| us_control(m, b, &EASTERN));
        match outcome {
            EventOutcome::Effect(result) => assert!(result.influence.is_empty(), "nothing changes"),
            other => panic!("expected an immediate no-op resolution, got {other:?}"),
        }
        assert!(game.operation().is_none(), "no session is opened for nothing");
        assert_eq!(game.active(), Superpower::Us, "the turn is still spent");
        let (_, cards, _) = fixtures();
        assert!(game.removed_from_game().contains(&cards.id_by_name("Comecon").unwrap()), "and the event is used up");
        assert_eq!(inf(&game, &map, "Hungary", Superpower::Ussr), 0);
    }

    #[test]
    fn marshall_plan_with_three_eligible_countries_confirms_after_three() {
        let western: [&str; 12] =
            ["Canada", "UK", "Norway", "Sweden", "Finland", "Denmark", "Benelux", "France", "West Germany", "Greece", "Turkey", "Italy"];
        // Spain/Portugal and Austria are also Western Europe; USSR-control all but three.
        let (map, mut game, _) = play_on(Superpower::Us, "Marshall Plan", |m, b| {
            for n in western.iter().chain(["Spain/Portugal", "Austria"].iter()).filter(|n| !["UK", "France", "Italy"].contains(n)) {
                b.set_influence(id(m, n), Superpower::Ussr, 6);
            }
        });
        plus(&mut game, &map, &["UK", "France"]);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
        plus(&mut game, &map, &["Italy"]);
        game.confirm().unwrap();
        assert_eq!(inf(&game, &map, "Italy", Superpower::Us), 1);
    }

    #[test]
    fn a_removal_card_with_fewer_targets_than_asked_confirms_once_they_are_gone() {
        // Warsaw Pact, remove mode: only two Eastern European countries hold US influence.
        let (map, mut game, _) = play_on(Superpower::Ussr, "Warsaw Pact Formed", |m, b| {
            b.set_influence(id(m, "Poland"), Superpower::Us, 2);
            b.set_influence(id(m, "Hungary"), Superpower::Us, 1);
        });
        game.choose_mode(&map, 0).unwrap();
        minus(&mut game, &map, &["Poland"]);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
        minus(&mut game, &map, &["Hungary"]);
        game.confirm().unwrap();

        // …and with no US influence at all, the mode can simply be confirmed.
        let (map, mut game, _) = play_on(Superpower::Ussr, "Warsaw Pact Formed", |_, _| {});
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
    }

    #[test]
    fn single_target_shortages_for_the_other_choice_cards() {
        // Muslim Revolution asks for 2 countries; only Iran has US influence.
        let (map, mut game, _) = play_on(Superpower::Ussr, "Muslim Revolution", |m, b| b.set_influence(id(m, "Iran"), Superpower::Us, 3));
        minus(&mut game, &map, &["Iran"]);
        game.confirm().unwrap();

        // Suez Crisis asks for 4 points; the US has just 1 across France/UK/Israel.
        let (map, mut game, _) = play_on(Superpower::Ussr, "Suez Crisis", |m, b| b.set_influence(id(m, "France"), Superpower::Us, 1));
        minus(&mut game, &map, &["France"]);
        game.confirm().unwrap();

        // East European Unrest asks for 3 countries; only two hold USSR influence.
        let (map, mut game, _) = play_on(Superpower::Us, "East European Unrest", |m, b| {
            b.set_influence(id(m, "Poland"), Superpower::Ussr, 2);
            b.set_influence(id(m, "Hungary"), Superpower::Ussr, 1);
        });
        minus(&mut game, &map, &["Poland", "Hungary"]);
        game.confirm().unwrap();

        // Socialist Governments asks for 3 points; the US has 2 in Western Europe.
        let (map, mut game, _) = play_on(Superpower::Ussr, "Socialist Governments", |m, b| b.set_influence(id(m, "France"), Superpower::Us, 2));
        minus(&mut game, &map, &["France", "France"]);
        game.confirm().unwrap();

        // Marine Barracks Bombing with no US influence in the Middle East at all.
        let (_map, game, outcome) = play_on(Superpower::Ussr, "Marine Barracks Bombing", |_, _| {});
        assert!(matches!(outcome, EventOutcome::Effect(_)), "nothing to remove: resolves at once");
        assert!(game.operation().is_none());
    }
}

// ---------------------------------------------------------------------
// Turn-long effects (`ongoing`, `data/states/turn-effects.json`): cards
// whose event holds "for the remainder of the turn". The `<card>` states
// start from the card in hand; the `*-active` ones start with the effect
// already in force, as it would be in a later action round.

mod turn_effects {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::Operation;
    use twilight_struggle::{CountryId, Dice, Event, OperationKind, PlacementError, Region, RollOutcome};

    fn load(name: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("turn-effects/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
        (map, cards, Game::from_scenario(&scenario))
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    fn play(game: &mut Game, cards: &CardCatalog, name: &str) {
        game.play_card(cards, cards.id_by_name(name).unwrap_or_else(|| panic!("no card {name}"))).unwrap_or_else(|e| panic!("play {name}: {e}"));
    }

    fn in_force(game: &Game) -> Vec<u8> {
        game.status().effects.active().iter().map(|e| e.card().number()).collect()
    }

    #[test]
    fn every_turn_long_card_starts_its_effect_and_hands_over_the_turn() {
        for (state, card, number) in [
            ("containment", "Containment", 25),
            ("red-scare", "Red Scare/Purge", 31),
            ("nuclear-subs", "Nuclear Subs", 41),
            ("brezhnev-doctrine", "Brezhnev Doctrine", 51),
            ("latin-american-death-squads", "Latin American Death Squads", 69),
            ("north-sea-oil", "North Sea Oil", 86),
            ("iran-contra-scandal", "Iran-Contra Scandal", 93),
            ("yuri-and-samantha", "Yuri and Samantha", 109),
            ("vietnam-revolts", "Vietnam Revolts", 9),
        ] {
            let (map, cards, mut game) = load(state);
            let side = game.active();
            assert!(in_force(&game).is_empty(), "{state}: starts with nothing in force");
            play(&mut game, &cards, card);
            game.play_event(&map, &cards).unwrap_or_else(|e| panic!("{state}: {e}"));
            assert_eq!(in_force(&game), vec![number], "{state}");
            assert!(game.operation().is_none(), "{state}: resolves at once");
            if state != "north-sea-oil" {
                assert_ne!(game.active(), side, "{state}: the turn passes");
            }
        }
    }

    #[test]
    fn red_scare_and_death_squads_favour_the_side_that_played_them() {
        let (map, cards, mut game) = load("red-scare");
        play(&mut game, &cards, "Red Scare/Purge");
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().effects.red_scare, Some(Superpower::Us), "the USSR played it: the US is penalised");

        let (map, cards, mut game) = load("latin-american-death-squads");
        play(&mut game, &cards, "Latin American Death Squads");
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().effects.death_squads, Some(Superpower::Ussr));
    }

    #[test]
    fn vietnam_revolts_adds_two_ussr_influence_to_vietnam() {
        let (map, cards, mut game) = load("vietnam-revolts");
        play(&mut game, &cards, "Vietnam Revolts");
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.board().influence(id(&map, "Vietnam"), Superpower::Ussr), 2);
    }

    #[test]
    fn chernobyl_is_designated_by_mode_and_bars_the_ussr_for_the_rest_of_the_turn() {
        let (map, cards, mut game) = load("chernobyl");
        play(&mut game, &cards, "Chernobyl");
        let outcome = game.play_event(&map, &cards).unwrap();
        assert!(matches!(outcome, EventOutcome::Pending { chooser: Superpower::Us, .. }));
        assert!(game.confirm().is_err(), "no region picked yet");
        let europe = Region::ALL.iter().position(|&r| r == Region::Europe).unwrap();
        game.choose_mode(&map, europe).unwrap();
        // Changing her mind is fine until confirmed.
        game.choose_mode(&map, (europe + 1) % 6).unwrap();
        game.choose_mode(&map, europe).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().effects.chernobyl, Some(Region::Europe));
        assert_eq!(game.active(), Superpower::Ussr);
    }

    #[test]
    fn backspace_first_clears_a_chosen_region_then_abandons_the_event() {
        let (map, cards, mut game) = load("chernobyl");
        play(&mut game, &cards, "Chernobyl");
        game.play_event(&map, &cards).unwrap();
        assert!(!game.clear_event_mode(&map), "nothing chosen yet: fall through to abandon");
        game.choose_mode(&map, 2).unwrap();
        assert!(game.clear_event_mode(&map));
        let Some(Operation::Event(e)) = game.operation() else { panic!("the event stays open") };
        assert_eq!((e.mode(), e.designated_region()), (None, None));
        assert!(game.confirm().is_err());
        game.abandon().expect("a second step backs out of the event");
        assert!(game.operation().is_none() && game.card_in_play().is_some(), "card still in play");
    }

    #[test]
    fn chernobyl_refuses_ussr_placement_in_the_region_but_not_elsewhere() {
        let (map, cards, mut game) = load("chernobyl-europe-active");
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Influence).unwrap();
        let op = game.operation().unwrap();
        assert!(!op.is_legal_target(&map, game.board(), id(&map, "Poland")), "dimmed on the map");
        assert!(op.is_legal_target(&map, game.board(), id(&map, "Egypt")));
        assert!(matches!(game.place(&map, id(&map, "Poland")), Err(GameError::Placement(PlacementError::Banned { .. }))));
        game.place(&map, id(&map, "Egypt")).unwrap();
    }

    #[test]
    fn chernobyl_does_not_stop_a_coup_or_the_us() {
        let (map, cards, mut game) = load("chernobyl-europe-active");
        game.board_mut().set_influence(id(&map, "Poland"), Superpower::Us, 1);
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        game.roll(&map, id(&map, "Poland"), &mut Dice::from_seed(1)).expect("coups aren't influence placement");
    }

    #[test]
    fn containment_adds_one_op_capped_at_four() {
        let (map, cards, mut game) = load("containment-active");
        let _ = &map;
        play(&mut game, &cards, "Truman Doctrine");
        assert_eq!(game.ops_available(), 2, "1 op +1");
        game.begin(OperationKind::Influence).unwrap();
        assert_eq!(game.operation().unwrap().ops_total(), 2);

        let (_, cards, mut game) = load("containment-active");
        play(&mut game, &cards, "Marshall Plan");
        assert_eq!(game.ops_available(), 4, "already 4: capped");
    }

    #[test]
    fn red_scare_takes_one_op_with_a_floor_of_one() {
        let (_, cards, mut game) = load("red-scare-active");
        play(&mut game, &cards, "Truman Doctrine");
        assert_eq!(game.ops_available(), 1);
        let (_, cards, mut game) = load("red-scare-active");
        play(&mut game, &cards, "Marshall Plan");
        assert_eq!(game.ops_available(), 3);
    }

    #[test]
    fn containment_and_red_scare_cancel() {
        let (_, cards, mut game) = load("containment-and-red-scare-active");
        play(&mut game, &cards, "Marshall Plan");
        assert_eq!(game.ops_available(), 4);
    }

    #[test]
    fn vietnam_revolts_gives_a_bonus_op_only_for_a_card_spent_wholly_in_southeast_asia() {
        let (map, cards, mut game) = load("vietnam-revolts-active");
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Influence).unwrap();
        assert!(game.operation().unwrap().pending_bonus().is_some());
        for _ in 0..3 {
            game.place(&map, id(&map, "Vietnam")).expect("2 ops + the bonus = 3 points");
        }
        assert!(game.place(&map, id(&map, "Vietnam")).is_err(), "a fourth would exceed it");
        assert_eq!(game.operation().unwrap().ops_total(), 3);

        // Placing outside Southeast Asia forfeits the bonus.
        let (map, cards, mut game) = load("vietnam-revolts-active");
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, id(&map, "Vietnam")).unwrap();
        game.place(&map, id(&map, "Poland")).unwrap();
        assert_eq!(game.operation().unwrap().ops_total(), 2);
        assert!(game.place(&map, id(&map, "Vietnam")).is_err());
        // ...and undoing the outsider gives it back.
        game.unplace(&map, id(&map, "Poland")).unwrap();
        game.place(&map, id(&map, "Thailand")).unwrap();
        assert_eq!(game.operation().unwrap().ops_total(), 3);
    }

    #[test]
    fn vietnam_revolts_bonus_applies_to_a_coup_in_southeast_asia() {
        let (map, cards, mut game) = load("vietnam-revolts-active");
        game.board_mut().set_influence(id(&map, "Thailand"), Superpower::Us, 2);
        game.board_mut().set_influence(id(&map, "Italy"), Superpower::Us, 2);
        play(&mut game, &cards, "Fidel");
        game.begin(OperationKind::Coup).unwrap();
        let Some(Operation::Coup(c)) = game.operation() else { panic!() };
        assert_eq!(c.ops_for(&map, id(&map, "Thailand")), 3);
        assert_eq!(c.ops_for(&map, id(&map, "Italy")), 2);
        let RollOutcome::Coup(result) = game.roll(&map, id(&map, "Thailand"), &mut Dice::from_seed(3)).unwrap() else { panic!() };
        assert_eq!(result.ops, 3);
        assert_eq!(game.operation().unwrap().ops_total(), 3);
    }

    fn coup_modifier(state: &str, card: &str, target: &str) -> i8 {
        let (map, cards, mut game) = load(state);
        play(&mut game, &cards, card);
        game.begin(OperationKind::Coup).unwrap();
        let RollOutcome::Coup(result) = game.roll(&map, id(&map, target), &mut Dice::from_seed(5)).unwrap() else { panic!() };
        result.modifier
    }

    #[test]
    fn death_squads_help_the_beneficiary_hurt_the_other_and_only_in_the_americas() {
        assert_eq!(coup_modifier("death-squads-active", "Fidel", "Colombia"), 1);
        assert_eq!(coup_modifier("death-squads-active", "Fidel", "Italy"), 0);
        assert_eq!(coup_modifier("death-squads-against-active", "Marshall Plan", "Colombia"), -1);
    }

    #[test]
    fn iran_contra_penalises_us_realignment_rolls_only() {
        let (map, cards, mut game) = load("iran-contra-active");
        play(&mut game, &cards, "Marshall Plan");
        game.begin(OperationKind::Realign).unwrap();
        let Some(Operation::Realign(r)) = game.operation() else { panic!() };
        let (acting, opposing, _) = r.preview(&map, game.board(), id(&map, "Italy"));
        assert!(acting.iran_contra && !opposing.iran_contra);
        assert!(acting.reasons().iter().any(|s| s.contains("Iran-Contra")));
        let RollOutcome::Realign(result) = game.roll(&map, id(&map, "Italy"), &mut Dice::from_seed(2)).unwrap() else { panic!() };
        assert!(result.acting_mods.iran_contra);
    }

    fn aftermath_of_us_coup(state: &str) -> (Game, Option<twilight_struggle::CoupAftermath>) {
        let (map, cards, mut game) = load(state);
        let card = if game.active() == Superpower::Us { "Truman Doctrine" } else { "Fidel" };
        play(&mut game, &cards, card);
        game.begin(OperationKind::Coup).unwrap();
        let target = match state {
            "yuri-active" => "Czechoslovakia",
            "battleground-coup-defcon-2" => "West Germany",
            _ => "Poland",
        };
        game.roll(&map, id(&map, target), &mut Dice::from_seed(4)).unwrap();
        let aftermath = game.log().entries().iter().find_map(|e| match e.event {
            Event::CoupAftermath(a) => Some(a),
            _ => None,
        });
        (game, aftermath)
    }

    #[test]
    fn a_battleground_coup_degrades_defcon_but_nuclear_subs_spares_a_us_one() {
        let (game, aftermath) = aftermath_of_us_coup("us-battleground-coup");
        assert_eq!(game.status().defcon, 3);
        assert_eq!(aftermath.unwrap().defcon, Some((4, 3)));

        let (game, aftermath) = aftermath_of_us_coup("nuclear-subs-active");
        assert_eq!(game.status().defcon, 4);
        assert!(aftermath.unwrap().defcon_spared);
    }

    #[test]
    fn a_coup_taking_defcon_to_1_loses_for_the_phasing_player() {
        let (game, _) = aftermath_of_us_coup("battleground-coup-defcon-2");
        assert_eq!(game.status().defcon, 1);
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Defcon }));
    }

    #[test]
    fn a_non_battleground_coup_leaves_defcon_alone() {
        let (game, aftermath) = aftermath_of_us_coup("yuri-active");
        assert_eq!(game.status().defcon, 5);
        assert!(aftermath.is_some_and(|a| a.defcon.is_none() && !a.defcon_spared));
    }

    #[test]
    fn yuri_and_samantha_pays_the_ussr_one_vp_per_us_coup() {
        let (game, aftermath) = aftermath_of_us_coup("yuri-active");
        assert_eq!(game.status().vp, -1);
        assert_eq!(aftermath.unwrap().vp, Some((-1, -1)));
    }

    #[test]
    fn north_sea_oil_gives_the_us_an_extra_round_then_everything_expires_with_the_turn() {
        let (map, cards, mut game) = load("north-sea-oil-final-round");
        let _ = &map;
        assert_eq!((game.status().turn, game.status().action_round), (3, 6));
        play(&mut game, &cards, "Truman Doctrine");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        // The US goes again, alone, in an extra 7th round of a 6-round turn.
        assert_eq!((game.status().turn, game.status().action_round, game.active()), (3, 7, Superpower::Us));
        assert_eq!(in_force(&game), vec![86]);
        game.status().validate().expect("the extra round is a valid status");
        play(&mut game, &cards, "Marshall Plan");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!((game.status().turn, game.status().action_round, game.active()), (4, 1, Superpower::Ussr));
        assert!(in_force(&game).is_empty(), "the turn's effects end with it");
    }

    #[test]
    fn effects_survive_between_rounds_of_a_turn_and_clear_at_its_end() {
        let (_, cards, mut game) = load("containment-active");
        play(&mut game, &cards, "Truman Doctrine");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(in_force(&game), vec![25], "still in force after the next side acts");
        game.status_mut().action_round = game.status().action_rounds_per_turn;
        game.status_mut().active = Superpower::Us;
        play(&mut game, &cards, "Marshall Plan");
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert!(in_force(&game).is_empty());
    }

    #[test]
    fn a_random_walk_through_every_active_state_always_finishes() {
        let (map, cards, lib) = fixtures();
        for entry in lib.list().unwrap().iter().filter(|e| e.file == "turn-effects") {
            let (scenario, _) = lib.load(&map, &cards, &entry.reference()).unwrap();
            let mut game = Game::from_scenario(&scenario);
            let mut ai = twilight_struggle::RandomAi::from_seed(11);
            let mut dice = Dice::from_seed(11);
            for _ in 0..40 {
                if game.winner().is_some() || game.hand(game.decider()).is_empty() && game.card_in_play().is_none() {
                    break;
                }
                twilight_struggle::play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap_or_else(|e| panic!("{}: {e}", entry.reference()));
            }
        }
    }
}

// The five war cards (`data/states/wars.json`): the first events that roll.
mod wars {
    use super::fixtures;
    use twilight_struggle::events::WarResult;
    use twilight_struggle::{CardCatalog, CountryId, Dice, EventOutcome, Game, GameError, Operation, RollOutcome, Superpower, WorldMap};

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap()
    }

    /// A seed whose first roll is `die`.
    fn seed_for(die: u8) -> u64 {
        (0..1000).find(|&s| Dice::from_seed(s).roll() == die).unwrap()
    }

    fn load(name: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("wars/{name}")).unwrap();
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        (map, cards, game)
    }

    /// Plays the card's event and rolls `die` on `target`, as `e` then `r` would.
    fn declare(game: &mut Game, map: &WorldMap, cards: &CardCatalog, target: &str, die: u8) -> WarResult {
        assert!(matches!(game.play_event(map, cards).unwrap(), EventOutcome::Pending { .. }));
        let RollOutcome::War(result) = game.roll(map, id(map, target), &mut Dice::from_seed(seed_for(die))).unwrap() else { panic!() };
        result
    }

    #[test]
    fn korean_war_wins_on_a_modified_four_and_replaces_us_influence() {
        let (map, cards, mut game) = load("korean-war", "Korean War");
        // Japan is US-controlled: a 5 becomes a 4, which wins.
        let result = declare(&mut game, &map, &cards, "South Korea", 5);
        assert!(result.success);
        assert_eq!(result.modifier.total(), -1);
        let sk = id(&map, "South Korea");
        assert_eq!((game.board().influence(sk, Superpower::Us), game.board().influence(sk, Superpower::Ussr)), (0, 3));
        assert_eq!(game.status().vp, -2);
        assert_eq!(game.status().military_ops_ussr, 2);
        assert_eq!(game.active(), Superpower::Us, "the turn passes");
        assert!(game.removed_from_game().contains(&cards.id_by_name("Korean War").unwrap()));
    }

    #[test]
    fn korean_war_fails_below_four_and_changes_nothing_but_the_track() {
        let (map, cards, mut game) = load("korean-war", "Korean War");
        let result = declare(&mut game, &map, &cards, "South Korea", 4);
        assert!(!result.success, "a 4 is a 3 after Japan");
        assert_eq!(game.status().vp, 0);
        let sk = id(&map, "South Korea");
        assert_eq!(game.board().influence(sk, Superpower::Us), 2);
        assert_eq!(game.status().military_ops_ussr, 2);
    }

    #[test]
    fn two_us_neighbours_need_a_six() {
        let (map, cards, mut game) = load("korean-war-two-us-neighbours", "Korean War");
        let result = declare(&mut game, &map, &cards, "South Korea", 5);
        assert!(!result.success);
        let (map, cards, mut game) = load("korean-war-two-us-neighbours", "Korean War");
        let result = declare(&mut game, &map, &cards, "South Korea", 6);
        assert!(result.success);
    }

    #[test]
    fn a_winning_war_can_end_the_game_on_vp() {
        let (map, cards, mut game) = load("korean-war-vp-win", "Korean War");
        declare(&mut game, &map, &cards, "South Korea", 6);
        assert_eq!(game.status().vp, -20);
        let victory = game.winner().expect("20 VP ends the game");
        assert_eq!(victory.side, Superpower::Ussr);
        assert_eq!(game.active(), Superpower::Ussr, "no handover once the game is over");
    }

    #[test]
    fn arab_israeli_counts_a_us_controlled_israel() {
        let (map, cards, mut game) = load("arab-israeli-war", "Arab-Israeli War");
        let result = declare(&mut game, &map, &cards, "Israel", 4);
        assert!(result.modifier.target_itself);
        assert!(!result.success);
    }

    #[test]
    fn camp_david_prevents_arab_israeli_war() {
        let (map, cards, mut game) = load("arab-israeli-war-after-camp-david", "Arab-Israeli War");
        assert!(matches!(game.play_event(&map, &cards), Err(GameError::EventPrevented { .. })));
        assert!(!game.legal_actions(&map, &cards).contains(&twilight_struggle::Action::Event));
    }

    #[test]
    fn indo_pakistani_war_opens_a_session_the_player_can_back_out_of() {
        let (map, cards, mut game) = load("indo-pakistani-war-us", "Indo-Pakistani War");
        assert!(matches!(game.play_event(&map, &cards).unwrap(), EventOutcome::Pending { .. }));
        assert!(matches!(game.operation(), Some(Operation::War(_))));
        assert!(matches!(game.confirm(), Err(GameError::WarNotRolled)));
        assert!(matches!(game.cancel(), Err(GameError::WarNotRolled)));
        // Only India and Pakistan are targets.
        let mut dice = Dice::from_seed(1);
        assert!(matches!(game.roll(&map, id(&map, "Poland"), &mut dice), Err(GameError::War(_))));
        game.abandon().unwrap();
        assert!(game.operation().is_none() && game.card_in_play().is_some());
        assert!(game.log().entries().is_empty(), "an abandoned war leaves no trace");
    }

    #[test]
    fn rolling_on_the_chosen_target_resolves_and_closes_the_event() {
        let (map, cards, mut game) = load("indo-pakistani-war-us", "Indo-Pakistani War");
        game.play_event(&map, &cards).unwrap();
        let india = id(&map, "India");
        // Pakistan is USSR-controlled, so attacking India is -1: a 5 wins.
        let RollOutcome::War(result) = game.roll(&map, india, &mut Dice::from_seed(seed_for(5))).unwrap() else { panic!() };
        assert!(result.success && result.side == Superpower::Us);
        assert_eq!((game.board().influence(india, Superpower::Us), game.board().influence(india, Superpower::Ussr)), (3, 0));
        assert_eq!(game.status().vp, 2);
        assert_eq!(game.status().military_ops_us, 2);
        assert!(game.operation().is_none());
        assert_eq!(game.active(), Superpower::Ussr);
        assert!(game.discards().contains(&cards.id_by_name("Indo-Pakistani War").unwrap()), "not removed after its event");
    }

    #[test]
    fn brush_war_wins_on_three_for_one_vp_and_adds_three_mil_ops() {
        let (map, cards, mut game) = load("brush-war-ussr", "Brush War");
        game.play_event(&map, &cards).unwrap();
        // Iran is US-controlled (-1): a 4 is a 3, which wins.
        let RollOutcome::War(result) = game.roll(&map, id(&map, "Afghanistan"), &mut Dice::from_seed(seed_for(4))).unwrap() else { panic!() };
        assert!(result.success);
        assert_eq!(game.status().vp, -1);
        assert_eq!(game.status().military_ops_ussr, 3);
    }

    #[test]
    fn iran_iraq_war_is_removed_after_its_event() {
        let (map, cards, mut game) = load("iran-iraq-war-us", "Iran-Iraq War");
        game.play_event(&map, &cards).unwrap();
        game.roll(&map, id(&map, "Iraq"), &mut Dice::from_seed(seed_for(6))).unwrap();
        assert!(game.removed_from_game().contains(&cards.id_by_name("Iran-Iraq War").unwrap()));
    }

    #[test]
    fn a_random_walk_through_every_war_state_finishes_the_turn() {
        let (map, cards, lib) = fixtures();
        for entry in lib.list().unwrap().iter().filter(|e| e.file == "wars") {
            let (scenario, _) = lib.load(&map, &cards, &entry.reference()).unwrap();
            let mut game = Game::from_scenario(&scenario);
            let mut ai = twilight_struggle::RandomAi::from_seed(5);
            let mut dice = Dice::from_seed(5);
            for _ in 0..10 {
                if game.winner().is_some() || game.hand(game.decider()).is_empty() && game.card_in_play().is_none() {
                    break;
                }
                twilight_struggle::play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap_or_else(|e| panic!("{}: {e}", entry.reference()));
            }
        }
    }
}
