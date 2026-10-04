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
            // A card that needs another's event first gets it already played.
            if let Some(events::Blocked::Requires { any_of }) = events::blocked(id, &[]) {
                game.hands_mut().remove_from_game(any_of[0]);
            }
            game.play_card(&cards, id).unwrap_or_else(|e| panic!("{} ({side}): play_card: {e}", card.name));
            game.play_event_with(&map, &cards, &mut twilight_struggle::Dice::from_seed(3)).unwrap_or_else(|e| panic!("{} ({side}): play_event: {e}", card.name));
            // A choice card opens a session for its own side; let a random
            // chooser carry it out — every session must be finishable.
            let mut ai = twilight_struggle::RandomAi::from_seed(7);
            let mut dice = twilight_struggle::Dice::from_seed(7);
            for _ in 0..3 {
                if game.operation().is_some() || game.ops_after_event().is_some() {
                    twilight_struggle::play_turn(&mut ai, &mut game, &map, &cards, &mut dice)
                        .unwrap_or_else(|e| panic!("{} ({side}): AI could not finish the event: {e}", card.name));
                }
            }
            assert!(game.operation().is_none(), "{} ({side}) left its event session open", card.name);
            // Shuttle Diplomacy stays in front of the US until a scoring spends it.
            let stays_in_effect = card.name == "Shuttle Diplomacy" && game.status().lasting.shuttle_diplomacy;
            assert!(
                stays_in_effect || game.discards().contains(&id) || game.removed_from_game().contains(&id),
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
        assert!(!game.operation().unwrap().pending_bonus().is_empty());
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

// ---------------------------------------------------------------------
// Lasting events (`ongoing::LastingEffects`, `data/states/lasting.json`):
// effects that outlast the turn — NATO, the US/Japan pact, Formosan
// Resolution, Shuttle Diplomacy, Flower Power, We Will Bury You — plus
// the cards that need or cancel them.
// ---------------------------------------------------------------------

mod lasting {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::{CoupError, Operation, RealignError};
    use twilight_struggle::{Dice, OperationKind, RollOutcome};

    fn load(map: &WorldMap, cards: &CardCatalog, lib: &StateLibrary, name: &str) -> Game {
        let reference = format!("lasting/{name}");
        let (scenario, _) = lib.load(map, cards, &reference).unwrap_or_else(|e| panic!("loading {reference}: {e}"));
        Game::from_scenario(&scenario)
    }

    /// Loads `name`, plays `card` and resolves its event.
    fn play_event(map: &WorldMap, cards: &CardCatalog, lib: &StateLibrary, name: &str, card: &str) -> Game {
        let mut game = load(map, cards, lib, name);
        game.play_card(cards, cards.id_by_name(card).unwrap()).unwrap();
        game.play_event(map, cards).unwrap_or_else(|e| panic!("{name}: play_event: {e}"));
        game
    }

    fn country(map: &WorldMap, name: &str) -> twilight_struggle::CountryId {
        map.id_by_name(name).unwrap()
    }

    fn start_coup(cards: &CardCatalog, game: &mut Game, card: &str) {
        game.play_card(cards, cards.id_by_name(card).unwrap()).unwrap();
        game.begin(OperationKind::Coup).unwrap();
    }

    #[test]
    fn nato_starts_the_lasting_effect_and_the_card_leaves_the_game() {
        let (map, cards, lib) = fixtures();
        let game = play_event(&map, &cards, &lib, "nato", "NATO");
        assert!(game.status().lasting.nato);
        assert!(game.removed_from_game().contains(&cards.id_by_name("NATO").unwrap()));
    }

    #[test]
    fn nato_and_solidarity_need_their_prerequisite_events() {
        let (map, cards, lib) = fixtures();
        for (state, card) in [("nato-needs-prerequisite", "NATO"), ("solidarity-needs-john-paul-ii", "Solidarity")] {
            let mut game = load(&map, &cards, &lib, state);
            game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
            assert!(matches!(game.play_event(&map, &cards), Err(GameError::EventRequires { .. })), "{state}");
            assert!(!game.legal_actions(&map, &cards).contains(&twilight_struggle::Action::Event), "{state}: not offered to the AI");
        }
    }

    #[test]
    fn solidarity_adds_three_us_influence_to_poland() {
        let (map, cards, lib) = fixtures();
        let game = play_event(&map, &cards, &lib, "solidarity", "Solidarity");
        assert_eq!(inf(&map, &game, "Poland"), (3, 0));
    }

    #[test]
    fn nato_bars_ussr_coups_and_realignment_in_us_controlled_europe() {
        let (map, cards, lib) = fixtures();
        let mut game = load(&map, &cards, &lib, "nato-active");
        start_coup(&cards, &mut game, "Comecon");
        let mut dice = Dice::from_seed(1);
        for name in ["West Germany", "Italy", "France"] {
            assert!(matches!(game.roll(&map, country(&map, name), &mut dice), Err(GameError::Coup(CoupError::Protected { .. }))), "{name}");
        }
        // An uncontrolled US presence is fair game.
        assert!(matches!(game.roll(&map, country(&map, "Poland"), &mut dice), Ok(RollOutcome::Coup(_))));

        let mut game = load(&map, &cards, &lib, "nato-active");
        game.play_card(&cards, cards.id_by_name("Comecon").unwrap()).unwrap();
        game.begin(OperationKind::Realign).unwrap();
        assert!(matches!(game.roll(&map, country(&map, "West Germany"), &mut dice), Err(GameError::Realign(RealignError::Protected { .. }))));
        let Some(Operation::Realign(r)) = game.operation() else { panic!("realignment open") };
        assert!(!r.is_legal_target(&map, game.board(), country(&map, "West Germany")), "protected countries are dimmed");
    }

    #[test]
    fn de_gaulle_and_willy_brandt_each_exempt_one_country_from_nato() {
        let (map, cards, lib) = fixtures();
        for (state, exempt, protected) in [("nato-de-gaulle", "France", "West Germany"), ("nato-willy-brandt", "West Germany", "Italy")] {
            let mut game = load(&map, &cards, &lib, state);
            start_coup(&cards, &mut game, "Comecon");
            let Some(Operation::Coup(c)) = game.operation() else { panic!("coup open") };
            assert!(c.is_legal_target(&map, game.board(), country(&map, exempt)), "{state}: {exempt} exempt");
            assert!(!c.is_legal_target(&map, game.board(), country(&map, protected)), "{state}: {protected} protected");
        }
    }

    #[test]
    fn nato_protects_europe_from_brush_war() {
        let (map, cards, lib) = fixtures();
        let mut game = load(&map, &cards, &lib, "brush-war-nato");
        game.play_card(&cards, cards.id_by_name("Brush War").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        let Some(Operation::War(w)) = game.operation() else { panic!("war open") };
        assert!(!w.is_legal_target(&map, country(&map, "Greece")));
        let mut game = load(&map, &cards, &lib, "brush-war-nato");
        game.status_mut().lasting.nato = false;
        game.play_card(&cards, cards.id_by_name("Brush War").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        let Some(Operation::War(w)) = game.operation() else { panic!("war open") };
        assert!(w.is_legal_target(&map, country(&map, "Greece")), "without NATO, Greece is a legal target");
    }

    #[test]
    fn special_relationship_with_nato_adds_two_influence_and_two_vp() {
        let (map, cards, lib) = fixtures();
        let mut game = load(&map, &cards, &lib, "special-relationship-nato");
        game.play_card(&cards, cards.id_by_name("Special Relationship").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        let wg = country(&map, "West Germany");
        game.place(&map, wg).unwrap();
        game.place(&map, wg).unwrap();
        game.confirm().unwrap();
        assert_eq!(inf(&map, &game, "West Germany"), (2, 0));
        assert_eq!(game.status().vp, 2);
    }

    #[test]
    fn us_japan_pact_takes_japan_and_shields_it() {
        let (map, cards, lib) = fixtures();
        let game = play_event(&map, &cards, &lib, "us-japan", "US/Japan Mutual Defense Pact");
        assert_eq!(inf(&map, &game, "Japan"), (4, 0));
        assert!(game.status().lasting.us_japan);

        let mut game = load(&map, &cards, &lib, "us-japan-active");
        start_coup(&cards, &mut game, "Comecon");
        let mut dice = Dice::from_seed(1);
        assert!(matches!(game.roll(&map, country(&map, "Japan"), &mut dice), Err(GameError::Coup(CoupError::Protected { .. }))));
        assert!(matches!(game.roll(&map, country(&map, "South Korea"), &mut dice), Ok(RollOutcome::Coup(_))));
    }

    fn scoring_delta(map: &WorldMap, cards: &CardCatalog, lib: &StateLibrary, name: &str, tweak: impl Fn(&mut Game)) -> (Game, twilight_struggle::scoring::ScoringResult) {
        let mut game = load(map, cards, lib, name);
        tweak(&mut game);
        let side = game.active();
        let card = *game.hand(side).iter().find(|&&id| cards.card(id).scoring).unwrap();
        game.play_card(cards, card).unwrap();
        let EventOutcome::Scoring(result) = game.play_event(map, cards).unwrap() else { panic!("scoring outcome") };
        (game, result)
    }

    #[test]
    fn formosan_makes_a_us_controlled_taiwan_a_battleground_for_asia_scoring() {
        let (map, cards, lib) = fixtures();
        let (_, with) = scoring_delta(&map, &cards, &lib, "formosan-asia-scoring", |_| {});
        let (_, without) = scoring_delta(&map, &cards, &lib, "formosan-asia-scoring", |g| g.status_mut().lasting.formosan = false);
        assert_eq!(with.vp_delta, without.vp_delta + 1, "Taiwan's battleground point");
        assert_eq!(with.modifiers, vec![cards.id_by_name("Formosan Resolution").unwrap()]);
        assert!(without.modifiers.is_empty());
    }

    #[test]
    fn formosan_ends_when_the_us_plays_the_china_card() {
        let (map, cards, lib) = fixtures();
        let mut game = load(&map, &cards, &lib, "formosan-china-card");
        game.play_card(&cards, twilight_struggle::CHINA_CARD).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert!(!game.status().lasting.formosan);
    }

    #[test]
    fn shuttle_diplomacy_stays_in_effect_until_a_middle_east_scoring_spends_it() {
        let (map, cards, lib) = fixtures();
        let game = play_event(&map, &cards, &lib, "shuttle-diplomacy", "Shuttle Diplomacy");
        let shuttle = cards.id_by_name("Shuttle Diplomacy").unwrap();
        assert!(game.status().lasting.shuttle_diplomacy);
        assert!(!game.discards().contains(&shuttle) && !game.removed_from_game().contains(&shuttle));

        let (game, with) = scoring_delta(&map, &cards, &lib, "shuttle-me-scoring", |_| {});
        let (_, without) = scoring_delta(&map, &cards, &lib, "shuttle-me-scoring", |g| g.status_mut().lasting.shuttle_diplomacy = false);
        assert_eq!(with.vp_delta, without.vp_delta + 1, "one fewer USSR battleground");
        assert!(!game.status().lasting.shuttle_diplomacy, "spent by the scoring");
        assert!(game.discards().contains(&shuttle));
    }

    #[test]
    fn willy_brandt_pays_the_ussr_and_adds_west_german_influence() {
        let (map, cards, lib) = fixtures();
        let game = play_event(&map, &cards, &lib, "willy-brandt", "Willy Brandt");
        assert_eq!(game.status().vp, -1);
        assert_eq!(inf(&map, &game, "West Germany"), (0, 1));
        assert!(game.status().lasting.willy_brandt);
    }

    #[test]
    fn flower_power_pays_the_ussr_for_each_us_war_card_spent_for_ops_or_event() {
        let (map, cards, lib) = fixtures();
        assert!(play_event(&map, &cards, &lib, "flower-power", "Flower Power").status().lasting.flower_power);

        // Spent for ops.
        let mut game = load(&map, &cards, &lib, "flower-power-active");
        game.play_card(&cards, cards.id_by_name("Korean War").unwrap()).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().vp, -2);

        // A non-war card pays nothing.
        let mut game = load(&map, &cards, &lib, "flower-power-active");
        game.play_card(&cards, cards.id_by_name("Duck and Cover").unwrap()).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().vp, 0);

        // Played as its event: the roll closes the war and the trigger fires.
        let mut game = load(&map, &cards, &lib, "flower-power-active");
        game.play_card(&cards, cards.id_by_name("Korean War").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        let mut dice = Dice::from_seed(3);
        game.roll(&map, country(&map, "South Korea"), &mut dice).unwrap();
        assert!(game.status().vp <= -2, "Flower Power's 2 VP is paid (plus the war's own, on a win)");
    }

    #[test]
    fn an_evil_empire_cancels_flower_power() {
        let (map, cards, lib) = fixtures();
        let game = play_event(&map, &cards, &lib, "evil-empire-cancels-flower-power", "\u{201c}An Evil Empire\u{201d}");
        assert!(!game.status().lasting.flower_power);
    }

    #[test]
    fn we_will_bury_you_drops_defcon_and_pays_after_the_uss_next_round() {
        let (map, cards, lib) = fixtures();
        let mut game = play_event(&map, &cards, &lib, "we-will-bury-you", "\u{201c}We Will Bury You\u{201d}");
        assert_eq!(game.status().defcon, 3);
        assert_eq!(game.status().vp, 0, "nothing yet");
        assert!(game.status().lasting.we_will_bury_you.is_some());
        // The US's next action round: spending its card pays the USSR 3 VP.
        game.play_card(&cards, cards.id_by_name("Duck and Cover").unwrap()).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().vp, -3);
        assert!(game.status().lasting.we_will_bury_you.is_none());
    }
}

mod china {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::{OperationKind, CHINA_CARD};

    fn load(name: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let reference = format!("china/{name}");
        let (scenario, _) = lib.load(&map, &cards, &reference).unwrap_or_else(|e| panic!("loading {reference}: {e}"));
        (map, cards, Game::from_scenario(&scenario))
    }

    /// Loads `name`, plays `card` and resolves its event.
    fn event(name: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, mut game) = load(name);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap_or_else(|e| panic!("{name}: play_event: {e}"));
        (map, cards, game)
    }

    #[test]
    fn cultural_revolution_takes_the_china_card_for_the_ussr_face_up_or_pays_a_vp() {
        let (_, cards, game) = event("cultural-revolution-us-holds", "Cultural Revolution");
        assert_eq!((game.status().china_card, game.status().china_card_face_up), (Superpower::Ussr, true));
        assert_eq!(game.status().vp, 0);
        assert!(game.removed_from_game().contains(&cards.id_by_name("Cultural Revolution").unwrap()), "removed after its event");

        let (_, _, game) = event("cultural-revolution-ussr-holds", "Cultural Revolution");
        assert_eq!(game.status().china_card, Superpower::Ussr);
        assert_eq!(game.status().vp, -1);
    }

    #[test]
    fn nixon_gives_the_us_the_china_card_face_down_or_pays_two_vp() {
        let (_, cards, mut game) = event("nixon-ussr-holds", "Nixon Plays the China Card");
        assert_eq!((game.status().china_card, game.status().china_card_face_up), (Superpower::Us, false));
        assert_eq!(game.status().vp, 0);
        // It's the USSR's turn now; once the US is next to act, the face-down card can't be played.
        game.status_mut().active = Superpower::Us;
        assert!(matches!(game.play_card(&cards, CHINA_CARD), Err(GameError::ChinaCardFaceDown)));

        let (_, _, game) = event("nixon-us-holds", "Nixon Plays the China Card");
        assert_eq!(game.status().vp, 2);
        assert_eq!(game.status().china_card, Superpower::Us);
    }

    #[test]
    fn ussuri_takes_the_china_card_face_up_when_the_ussr_has_it() {
        let (map, cards, mut game) = load("ussuri-ussr-holds");
        game.play_card(&cards, cards.id_by_name("Ussuri River Skirmish").unwrap()).unwrap();
        let outcome = game.play_event(&map, &cards).unwrap();
        assert!(matches!(outcome, EventOutcome::Effect(_)), "nothing to choose, so it resolves on the spot");
        assert_eq!((game.status().china_card, game.status().china_card_face_up), (Superpower::Us, true));
    }

    #[test]
    fn ussuri_adds_four_us_influence_in_asia_when_the_us_already_has_it() {
        let (map, cards, mut game) = load("ussuri-us-holds");
        game.play_card(&cards, cards.id_by_name("Ussuri River Skirmish").unwrap()).unwrap();
        let outcome = game.play_event(&map, &cards).unwrap();
        assert!(matches!(outcome, EventOutcome::Pending { chooser: Superpower::Us, .. }));
        let id = |n: &str| map.id_by_name(n).unwrap();
        assert!(game.place(&map, id("Poland")).is_err(), "Asia only");
        for n in ["Japan", "Japan", "Taiwan", "Taiwan"] {
            game.place(&map, id(n)).unwrap();
        }
        assert!(game.place(&map, id("Japan")).is_err(), "max 2 per country");
        assert!(game.place(&map, id("India")).is_err(), "only 4 points");
        game.confirm().unwrap();
        assert_eq!(game.board().influence(id("Japan"), Superpower::Us), 2);
        assert_eq!(game.status().china_card, Superpower::Us, "unchanged");
    }

    #[test]
    fn the_china_card_is_worth_one_more_op_when_spent_wholly_in_asia() {
        let (map, cards, mut game) = load("china-card-asia-bonus");
        let japan = map.id_by_name("Japan").unwrap();
        game.play_card(&cards, CHINA_CARD).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        assert!(!game.operation().unwrap().pending_bonus().is_empty());
        for _ in 0..5 {
            game.place(&map, japan).expect("4 ops + the Asia bonus = 5 points");
        }
        assert!(game.place(&map, japan).is_err());
        game.confirm().unwrap();
        assert_eq!((game.status().china_card, game.status().china_card_face_up), (Superpower::Us, false));
    }

    #[test]
    fn a_point_outside_asia_forfeits_the_china_cards_bonus_op() {
        let (map, cards, mut game) = load("china-card-asia-bonus");
        game.play_card(&cards, CHINA_CARD).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        let poland = map.id_by_name("Poland").unwrap();
        for _ in 0..4 {
            game.place(&map, poland).unwrap();
        }
        assert!(game.place(&map, map.id_by_name("Japan").unwrap()).is_err(), "only the card's own 4 ops");
    }
}

mod space {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::space::{SpaceError, SpaceResult};
    use twilight_struggle::{Dice, CHINA_CARD};

    fn load(name: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let reference = format!("space/{name}");
        let (scenario, _) = lib.load(&map, &cards, &reference).unwrap_or_else(|e| panic!("loading {reference}: {e}"));
        (map, cards, Game::from_scenario(&scenario))
    }

    /// A dice seed whose first roll satisfies `pred` — attempts roll exactly once.
    fn dice_rolling(pred: impl Fn(u8) -> bool) -> Dice {
        Dice::from_seed((0..1000u64).find(|&seed| pred(Dice::from_seed(seed).roll())).expect("some seed rolls it"))
    }

    fn attempt(game: &mut Game, cards: &CardCatalog, card: &str, dice: &mut Dice) -> Result<SpaceResult, GameError> {
        game.play_card(cards, cards.id_by_name(card).unwrap()).unwrap();
        game.space(dice)
    }

    #[test]
    fn a_successful_attempt_moves_the_marker_pays_first_in_vp_discards_and_passes_the_turn() {
        let (_, cards, mut game) = load("first-attempt");
        let result = attempt(&mut game, &cards, "Duck and Cover", &mut dice_rolling(|r| r <= 3)).unwrap();
        assert!(result.success);
        assert_eq!((game.status().space_race_ussr, game.status().vp), (1, -2));
        assert_eq!(game.status().active, Superpower::Us);
        assert!(game.discards().contains(&cards.id_by_name("Duck and Cover").unwrap()));
        assert_eq!(game.status().space_attempts_ussr, 1);
    }

    #[test]
    fn a_failed_attempt_still_uses_the_card_and_the_turn() {
        let (_, cards, mut game) = load("first-attempt");
        let result = attempt(&mut game, &cards, "Duck and Cover", &mut dice_rolling(|r| r >= 4)).unwrap();
        assert!(!result.success);
        assert_eq!((game.status().space_race_ussr, game.status().vp), (0, 0));
        assert_eq!(game.status().active, Superpower::Us);
        assert!(game.discards().contains(&cards.id_by_name("Duck and Cover").unwrap()));
    }

    #[test]
    fn a_card_without_enough_ops_is_refused_and_stays_in_play() {
        let (_, cards, mut game) = load("first-attempt");
        let err = attempt(&mut game, &cards, "Romanian Abdication", &mut Dice::from_seed(1)).unwrap_err();
        assert!(matches!(err, GameError::Space(SpaceError::NotEnoughOps { have: 1, need: 2, .. })), "{err}");
        assert!(game.card_in_play().is_some());
        assert_eq!(game.status().active, Superpower::Ussr);
    }

    #[test]
    fn arriving_second_pays_the_second_in_vp() {
        let (_, cards, mut game) = load("second-in");
        attempt(&mut game, &cards, "Duck and Cover", &mut dice_rolling(|r| r <= 3)).unwrap();
        assert_eq!((game.status().space_race_ussr, game.status().vp), (1, -1));
    }

    #[test]
    fn the_box_two_leader_may_make_a_second_attempt_until_the_opponent_arrives() {
        let (_, cards, mut game) = load("animal-in-space-leader");
        assert!(attempt(&mut game, &cards, "Duck and Cover", &mut dice_rolling(|r| r >= 5)).is_ok(), "a second attempt is allowed");
        let (_, cards, mut game) = load("animal-in-space-cancelled");
        let err = attempt(&mut game, &cards, "Duck and Cover", &mut Dice::from_seed(1)).unwrap_err();
        assert!(matches!(err, GameError::Space(SpaceError::NoAttemptsLeft { allowed: 1 })), "{err}");
    }

    #[test]
    fn the_china_card_cannot_be_spaced() {
        let (_, cards, mut game) = load("china-card");
        game.play_card(&cards, CHINA_CARD).unwrap();
        assert!(matches!(game.space(&mut Dice::from_seed(1)), Err(GameError::Space(SpaceError::ChinaCard))));
    }

    #[test]
    fn the_space_station_holder_plays_the_extra_rounds_alone_then_the_turn_rolls_over() {
        let (_, _, mut game) = load("space-station-ussr");
        game.pass().unwrap(); // US finishes AR 6
        assert_eq!((game.status().active, game.status().action_round), (Superpower::Ussr, 7));
        game.pass().unwrap();
        assert_eq!((game.status().active, game.status().action_round), (Superpower::Ussr, 8), "the US has no AR 7");
        game.pass().unwrap();
        assert_eq!((game.status().active, game.status().action_round, game.status().turn), (Superpower::Ussr, 1, 2));
    }

    #[test]
    fn nobody_gets_extra_rounds_once_both_reach_the_space_station() {
        let (_, _, mut game) = load("space-station-cancelled");
        game.pass().unwrap();
        assert_eq!((game.status().active, game.status().action_round, game.status().turn), (Superpower::Ussr, 1, 2));
    }

    #[test]
    fn space_attempts_reset_when_the_turn_rolls_over() {
        let (_, cards, mut game) = load("first-attempt");
        game.status_mut().active = Superpower::Us;
        game.status_mut().action_round = 6;
        game.status_mut().space_attempts_ussr = 1;
        attempt(&mut game, &cards, "Fidel", &mut Dice::from_seed(1)).unwrap();
        assert_eq!((game.status().turn, game.status().space_attempts_us, game.status().space_attempts_ussr), (2, 0, 0));
    }

    #[test]
    fn captured_nazi_scientist_advances_the_player_one_box() {
        let (map, cards, mut game) = load("nazi-scientist");
        game.play_card(&cards, cards.id_by_name("Captured Nazi Scientist").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!((game.status().space_race_us, game.status().vp), (1, 2));
    }

    #[test]
    fn one_small_step_moves_two_boxes_with_vp_only_from_the_last_when_behind() {
        let (map, cards, mut game) = load("one-small-step-behind");
        game.play_card(&cards, cards.id_by_name("“One Small Step…”").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!((game.status().space_race_ussr, game.status().vp), (2, 0));

        let (map, cards, mut game) = load("one-small-step-not-behind");
        game.play_card(&cards, cards.id_by_name("“One Small Step…”").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!((game.status().space_race_ussr, game.status().vp), (2, 0));
    }

    #[test]
    fn reaching_twenty_vp_in_the_space_race_wins_the_game() {
        let (_, cards, mut game) = load("vp-win-by-space");
        attempt(&mut game, &cards, "Duck and Cover", &mut dice_rolling(|r| r <= 3)).unwrap();
        assert_eq!(game.status().vp, 20);
        assert_eq!(game.winner().map(|v| v.side), Some(Superpower::Us));
    }

    #[test]
    fn legal_actions_offer_space_only_when_it_is_allowed() {
        let (map, cards, mut game) = load("first-attempt");
        game.play_card(&cards, cards.id_by_name("Duck and Cover").unwrap()).unwrap();
        assert!(game.legal_actions(&map, &cards).contains(&twilight_struggle::Action::Space));
        game.return_card().unwrap();
        game.play_card(&cards, cards.id_by_name("Romanian Abdication").unwrap()).unwrap();
        assert!(!game.legal_actions(&map, &cards).contains(&twilight_struggle::Action::Space));
    }
}

mod batch_one {
    use super::*;
    use twilight_struggle::ops::Operation;
    use twilight_struggle::CountryId;

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    fn open_choice(name: &str, card: &str) -> (WorldMap, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("choices/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap_or_else(|e| panic!("{name}: play_event: {e}"));
        (map, game)
    }

    #[test]
    fn arms_race_pays_three_with_the_required_amount_one_when_merely_ahead_nothing_on_a_tie() {
        let (map, cards, lib) = fixtures();
        assert_eq!(play_effect_state(&map, &cards, &lib, "arms-race-required-met", "Arms Race").status().vp, -3);
        assert_eq!(play_effect_state(&map, &cards, &lib, "arms-race-ahead-only", "Arms Race").status().vp, -1);
        assert_eq!(play_effect_state(&map, &cards, &lib, "arms-race-not-ahead", "Arms Race").status().vp, 0);
    }

    #[test]
    fn u2_incident_pays_the_ussr_one_vp() {
        let (map, cards, lib) = fixtures();
        assert_eq!(play_effect_state(&map, &cards, &lib, "u2-incident", "U2 Incident").status().vp, -1);
    }

    #[test]
    fn opec_pays_one_vp_per_controlled_oil_producer() {
        let (map, cards, lib) = fixtures();
        assert_eq!(play_effect_state(&map, &cards, &lib, "opec", "OPEC").status().vp, -3);
    }

    #[test]
    fn opec_is_barred_once_north_sea_oil_has_been_played() {
        use twilight_struggle::events::{blocked, Blocked};
        let (_, cards, _) = fixtures();
        let (opec, oil) = (cards.id_by_name("OPEC").unwrap(), cards.id_by_name("North Sea Oil").unwrap());
        assert_eq!(blocked(opec, &[]), None);
        assert_eq!(blocked(opec, &[oil]), Some(Blocked::Prevented { by: oil }));
    }

    #[test]
    fn defectors_pays_the_us_only_when_the_ussr_plays_it() {
        let (map, cards, lib) = fixtures();
        assert_eq!(play_effect_state(&map, &cards, &lib, "defectors-played-by-ussr", "Defectors").status().vp, 1);
        assert_eq!(play_effect_state(&map, &cards, &lib, "defectors-played-by-us", "Defectors").status().vp, 0);
    }

    #[test]
    fn voice_of_america_removes_four_outside_europe_two_per_country() {
        let (map, mut game) = open_choice("voice-of-america", "The Voice of America");
        let ussr = |g: &Game, n: &str| g.board().influence(id(&map, n), Superpower::Ussr);
        assert!(game.unplace(&map, id(&map, "Poland")).is_err(), "Europe is excluded");
        game.unplace(&map, id(&map, "Egypt")).unwrap();
        game.unplace(&map, id(&map, "Egypt")).unwrap();
        assert!(game.unplace(&map, id(&map, "Egypt")).is_err(), "max 2 per country");
        game.unplace(&map, id(&map, "Cuba")).unwrap();
        assert!(game.confirm().is_err(), "one point is still to remove");
        game.unplace(&map, id(&map, "Japan")).unwrap();
        game.confirm().unwrap();
        assert_eq!((ussr(&game, "Egypt"), ussr(&game, "Cuba"), ussr(&game, "Japan"), ussr(&game, "Poland")), (1, 0, 1, 2));
    }

    #[test]
    fn pershing_ii_pays_a_vp_and_removes_one_us_influence_from_three_western_european_countries() {
        let (map, mut game) = open_choice("pershing-ii", "Pershing II Deployed");
        for n in ["France", "UK", "Italy"] {
            game.unplace(&map, id(&map, n)).unwrap();
        }
        assert!(game.unplace(&map, id(&map, "West Germany")).is_err(), "only 3 countries");
        game.confirm().unwrap();
        assert_eq!(game.status().vp, -1);
        let us = |n: &str| game.board().influence(id(&map, n), Superpower::Us);
        assert_eq!((us("France"), us("UK"), us("Italy"), us("West Germany")), (1, 1, 1, 2));
    }

    #[test]
    fn how_i_learned_sets_defcon_to_the_chosen_level_and_adds_five_military_ops() {
        let (map, mut game) = open_choice("how-i-learned", "How I Learned to Stop Worrying");
        assert!(game.confirm().is_err(), "a level has to be chosen first");
        game.choose_mode(&map, 4).unwrap();
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 2);
        assert_eq!(game.status().military_ops_ussr, 5, "1 + 5 clamps to the track's 5");
        assert_eq!(game.winner(), None);
    }

    #[test]
    fn how_i_learned_to_defcon_1_loses_for_the_player() {
        let (map, mut game) = open_choice("how-i-learned", "How I Learned to Stop Worrying");
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Defcon }));
    }

    #[test]
    fn how_i_learned_is_a_mode_only_event_not_a_designation() {
        let (_, game) = open_choice("how-i-learned", "How I Learned to Stop Worrying");
        let Some(Operation::Event(e)) = game.operation() else { panic!("the event opens a session") };
        assert!(!e.is_designation() && !e.picks_countries());
        assert_eq!(e.modes().len(), 5);
    }

    #[test]
    fn wargames_at_defcon_2_ends_the_game_with_the_vp_leader_winning() {
        let (map, mut game) = open_choice("wargames-defcon-2", "Wargames");
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().vp, -2);
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Ussr, reason: VictoryReason::Wargames }));
    }

    #[test]
    fn wargames_can_hand_the_opponent_the_lead() {
        let (map, mut game) = open_choice("wargames-defcon-2-loses-lead", "Wargames");
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().vp, 3);
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Wargames }));
    }

    #[test]
    fn wargames_can_be_declined() {
        let (map, mut game) = open_choice("wargames-defcon-2", "Wargames");
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        assert_eq!((game.status().vp, game.winner()), (-8, None));
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn wargames_outside_defcon_2_does_nothing() {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "choices/wargames-defcon-3").unwrap();
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name("Wargames").unwrap()).unwrap();
        assert!(matches!(game.play_event(&map, &cards).unwrap(), EventOutcome::Effect(_)), "resolves on the spot");
        assert_eq!((game.status().vp, game.winner(), game.active()), (0, None, Superpower::Us));
    }
}

mod event_then_ops {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::{Action, CountryId, OperationKind};

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    /// Loads `events/<name>`, plays `card` and resolves its event.
    fn played(name: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("events/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        assert!(matches!(game.play_event(&map, &cards).unwrap(), EventOutcome::Effect(_)));
        (map, cards, game)
    }

    #[test]
    fn abm_treaty_improves_defcon_then_leaves_the_card_in_play_for_any_operation() {
        let (map, cards, mut game) = played("abm-treaty", "ABM Treaty");
        assert_eq!(game.status().defcon, 4);
        assert_eq!(game.active(), Superpower::Ussr, "the turn doesn't pass until the ops are done");
        assert!(game.ops_after_event().is_some());
        assert!(game.discards().is_empty() && game.removed_from_game().is_empty(), "still in play");
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, id(&map, "Poland")).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.board().influence(id(&map, "Poland"), Superpower::Ussr), 2);
        assert_eq!(game.active(), Superpower::Us);
        assert!(game.discards().contains(&cards.id_by_name("ABM Treaty").unwrap()), "ABM Treaty isn't removed after its event");
    }

    #[test]
    fn the_ops_of_a_played_event_can_be_skipped_with_pass() {
        let (_, cards, mut game) = played("abm-treaty", "ABM Treaty");
        game.pass().unwrap();
        assert_eq!(game.active(), Superpower::Us);
        assert!(game.card_in_play().is_none());
        assert!(game.discards().contains(&cards.id_by_name("ABM Treaty").unwrap()));
    }

    #[test]
    fn a_played_event_cannot_be_replayed_returned_or_spaced() {
        let (map, cards, mut game) = played("abm-treaty", "ABM Treaty");
        assert!(matches!(game.play_event(&map, &cards), Err(GameError::EventPlayed { .. })));
        assert!(matches!(game.return_card(), Err(GameError::EventPlayed { .. })));
        assert!(!game.can_space());
        let legal = game.legal_actions(&map, &cards);
        assert!(!legal.contains(&Action::Event) && !legal.contains(&Action::Space));
        assert!(legal.contains(&Action::Pass) && legal.contains(&Action::Begin(OperationKind::Coup)));
    }

    #[test]
    fn a_placement_can_be_abandoned_and_swapped_for_another_kind() {
        let (_, _, mut game) = played("abm-treaty", "ABM Treaty");
        game.begin(OperationKind::Influence).unwrap();
        game.abandon().unwrap();
        game.begin(OperationKind::Realign).unwrap();
        game.cancel().unwrap();
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn kal_007_drops_defcon_pays_the_us_and_allows_everything_but_a_coup_with_south_korea_controlled() {
        let (map, cards, mut game) = played("kal-007-south-korea-us-controlled", "Soviets Shoot Down KAL-007");
        assert_eq!((game.status().defcon, game.status().vp), (3, 2));
        assert!(matches!(game.begin(OperationKind::Coup), Err(GameError::OpsNotGranted { .. })));
        let legal = game.legal_actions(&map, &cards);
        assert!(!legal.contains(&Action::Begin(OperationKind::Coup)));
        assert!(legal.contains(&Action::Begin(OperationKind::Influence)) && legal.contains(&Action::Begin(OperationKind::Realign)));
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, id(&map, "Japan")).unwrap();
        game.confirm().unwrap();
        assert!(game.removed_from_game().contains(&cards.id_by_name("Soviets Shoot Down KAL-007").unwrap()), "KAL-007 leaves the game after its event");
        assert!(game.discards().is_empty());
    }

    #[test]
    fn kal_007_without_a_us_controlled_south_korea_just_hands_the_turn_over() {
        let (map, cards, lib) = fixtures();
        let game = play_effect_state(&map, &cards, &lib, "kal-007-south-korea-not-controlled", "Soviets Shoot Down KAL-007");
        assert_eq!((game.status().defcon, game.status().vp, game.active()), (3, 2, Superpower::Ussr));
        assert!(game.ops_after_event().is_none());
    }

    #[test]
    fn only_the_cards_own_side_gets_the_operation() {
        let (map, cards, lib) = fixtures();
        let game = play_effect_state(&map, &cards, &lib, "kal-007-played-by-ussr", "Soviets Shoot Down KAL-007");
        assert_eq!((game.status().defcon, game.status().vp, game.active()), (3, 2, Superpower::Us), "the event's VP and DEFCON still apply");
        assert!(game.ops_after_event().is_none());
    }

    #[test]
    fn glasnost_needs_the_reformer_for_its_operation() {
        let (map, cards, mut game) = played("glasnost-reformer-played", "Glasnost");
        assert_eq!((game.status().defcon, game.status().vp), (4, -2));
        assert!(matches!(game.begin(OperationKind::Coup), Err(GameError::OpsNotGranted { .. })));
        game.begin(OperationKind::Influence).unwrap();
        game.place(&map, id(&map, "Poland")).unwrap();
        game.confirm().unwrap();
        assert!(game.removed_from_game().contains(&cards.id_by_name("Glasnost").unwrap()));

        let game = play_effect_state(&map, &cards, &fixtures().2, "glasnost-reformer-not-played", "Glasnost");
        assert_eq!((game.status().defcon, game.status().vp, game.active()), (4, -2, Superpower::Us));
    }

    #[test]
    fn cia_created_and_lone_gunman_reveal_the_hand_then_allow_any_operation() {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "events/cia-created").unwrap();
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name("CIA Created").unwrap()).unwrap();
        let EventOutcome::Effect(returned) = game.play_event(&map, &cards).unwrap() else { panic!("a fixed effect") };
        assert_eq!(
            returned.reveals.map(|r| r.cards),
            Some(vec![cards.id_by_name("Fidel").unwrap(), cards.id_by_name("Nasser").unwrap()]),
            "the result returned to the caller (and shown in the modal) lists the hand too"
        );
        assert!(game.ops_after_event().is_some_and(|g| g.coup));
        let Some(twilight_struggle::Event::EventResolved { result, .. }) = game.log().entries().iter().rev().find_map(|e| match &e.event {
            e @ twilight_struggle::Event::EventResolved { .. } => Some(e.clone()),
            _ => None,
        }) else {
            panic!("the event is logged")
        };
        let reveal = result.reveals.expect("the hand is revealed");
        assert_eq!(reveal.side, Superpower::Ussr);
        assert_eq!(reveal.cards, vec![cards.id_by_name("Fidel").unwrap(), cards.id_by_name("Nasser").unwrap()]);

        let (_, cards, mut game) = played("lone-gunman", "“Lone Gunman”");
        assert!(game.ops_after_event().is_some());
        game.begin(OperationKind::Coup).unwrap();
        game.cancel().unwrap();
        assert!(game.removed_from_game().contains(&cards.id_by_name("“Lone Gunman”").unwrap()));
        let _ = map;
    }

    #[test]
    fn an_ordinary_ops_play_of_these_cards_is_unaffected() {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "events/abm-treaty").unwrap();
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name("ABM Treaty").unwrap()).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 3, "no event");
    }
}

mod scoped_ops {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::{CoupError, RealignError};
    use twilight_struggle::{CountryId, Dice, OperationKind};

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    /// A seed whose first roll is `die`.
    fn seed_for(die: u8) -> u64 {
        (0..1000).find(|&s| Dice::from_seed(s).roll() == die).unwrap()
    }

    fn start(state: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, state).unwrap_or_else(|e| panic!("{state}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        (map, cards, game)
    }

    fn played(state: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, mut game) = start(state, card);
        game.play_event(&map, &cards).unwrap_or_else(|e| panic!("{state}: play_event: {e}"));
        (map, cards, game)
    }

    fn us(game: &Game, map: &WorldMap, name: &str) -> u8 {
        game.board().influence(id(map, name), Superpower::Us)
    }

    #[test]
    fn ortega_clears_nicaragua_then_allows_one_coup_next_door_only() {
        let (map, cards, mut game) = played("events/ortega", "Ortega Elected in Nicaragua");
        assert_eq!(us(&game, &map, "Nicaragua"), 0);
        assert!(matches!(game.begin(OperationKind::Influence), Err(GameError::OpsNotGranted { .. })));
        assert!(matches!(game.begin(OperationKind::Realign), Err(GameError::OpsNotGranted { .. })));
        game.begin(OperationKind::Coup).unwrap();
        let op = game.operation().unwrap();
        assert!(op.is_legal_target(&map, game.board(), id(&map, "Honduras")), "adjacent to Nicaragua");
        assert!(!op.is_legal_target(&map, game.board(), id(&map, "Mexico")), "not adjacent: dimmed");
        let err = game.roll(&map, id(&map, "Mexico"), &mut Dice::from_seed(1)).unwrap_err();
        assert!(matches!(err, GameError::Coup(CoupError::OutOfScope { .. })), "{err}");
        game.roll(&map, id(&map, "Honduras"), &mut Dice::from_seed(seed_for(6))).unwrap();
        assert_eq!(us(&game, &map, "Honduras"), 0, "6 + 2 ops beats Honduras's target of 4 by 4");
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Us, "no follow-up: the turn is over");
        assert!(game.removed_from_game().contains(&cards.id_by_name("Ortega Elected in Nicaragua").unwrap()));
    }

    #[test]
    fn tear_down_this_wall_adds_influence_ends_willy_brandt_and_confines_ops_to_europe() {
        let (map, _, mut game) = played("events/tear-down-this-wall", "Tear Down this Wall");
        assert_eq!(us(&game, &map, "East Germany"), 3);
        assert!(!game.status().lasting.willy_brandt, "Tear Down this Wall cancels Willy Brandt");
        assert!(matches!(game.begin(OperationKind::Influence), Err(GameError::OpsNotGranted { .. })));
        game.begin(OperationKind::Realign).unwrap();
        let err = game.roll(&map, id(&map, "Egypt"), &mut Dice::from_seed(1)).unwrap_err();
        assert!(matches!(err, GameError::Realign(RealignError::OutOfScope { .. })), "{err}");
        game.roll(&map, id(&map, "Poland"), &mut Dice::from_seed(1)).expect("Poland is in Europe");
    }

    #[test]
    fn tear_down_this_wall_bars_a_later_willy_brandt() {
        use twilight_struggle::events::{blocked, Blocked};
        let (_, cards, _) = fixtures();
        let (wb, wall) = (cards.id_by_name("Willy Brandt").unwrap(), cards.id_by_name("Tear Down this Wall").unwrap());
        assert_eq!(blocked(wb, &[]), None);
        assert_eq!(blocked(wb, &[wall]), Some(Blocked::Prevented { by: wall }));
    }

    #[test]
    fn junta_adds_two_influence_to_one_american_country_then_allows_a_coup_or_realignment_there() {
        let (map, cards, mut game) = start("choices/junta", "Junta");
        game.play_event(&map, &cards).unwrap();
        assert!(game.place(&map, id(&map, "Egypt")).is_err(), "not in Central or South America");
        game.place(&map, id(&map, "Brazil")).unwrap();
        game.place(&map, id(&map, "Brazil")).unwrap();
        game.confirm().unwrap();
        assert_eq!(us(&game, &map, "Brazil"), 2);
        assert_eq!(game.active(), Superpower::Us, "the card stays in play for its operation");
        assert!(game.ops_after_event().is_some());
        assert!(matches!(game.begin(OperationKind::Influence), Err(GameError::OpsNotGranted { .. })));
        game.begin(OperationKind::Realign).unwrap();
        assert!(matches!(game.roll(&map, id(&map, "Egypt"), &mut Dice::from_seed(1)), Err(GameError::Realign(RealignError::OutOfScope { .. }))));
        game.roll(&map, id(&map, "Brazil"), &mut Dice::from_seed(1)).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Ussr);
        assert!(game.discards().contains(&cards.id_by_name("Junta").unwrap()), "Junta isn't removed after its event");
    }

    #[test]
    fn che_allows_a_coup_in_a_non_battleground_only() {
        let (map, _, mut game) = played("events/che", "Che");
        game.begin(OperationKind::Coup).unwrap();
        let op = game.operation().unwrap();
        assert!(op.is_legal_target(&map, game.board(), id(&map, "Honduras")));
        assert!(!op.is_legal_target(&map, game.board(), id(&map, "Chile")), "a battleground");
        let err = game.roll(&map, id(&map, "Chile"), &mut Dice::from_seed(1)).unwrap_err();
        assert!(matches!(err, GameError::Coup(CoupError::OutOfScope { .. })), "{err}");
    }

    #[test]
    fn che_allows_a_second_coup_in_a_different_country_if_the_first_removed_us_influence() {
        let (map, cards, mut game) = played("events/che", "Che");
        game.begin(OperationKind::Coup).unwrap();
        game.roll(&map, id(&map, "Honduras"), &mut Dice::from_seed(seed_for(6))).unwrap();
        assert_eq!(us(&game, &map, "Honduras"), 0);
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Ussr, "the card isn't spent yet");
        let grant = game.ops_after_event().expect("a second coup is allowed");
        assert!(!grant.follow_up && grant.coup && !grant.influence && !grant.realign);
        game.begin(OperationKind::Coup).unwrap();
        let op = game.operation().unwrap();
        assert!(!op.is_legal_target(&map, game.board(), id(&map, "Honduras")), "a different country");
        assert!(op.is_legal_target(&map, game.board(), id(&map, "Guatemala")));
        game.roll(&map, id(&map, "Guatemala"), &mut Dice::from_seed(seed_for(6))).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Us);
        assert!(game.discards().contains(&cards.id_by_name("Che").unwrap()), "Che isn't removed after its event");
    }

    #[test]
    fn che_gets_no_second_coup_when_the_first_removes_nothing() {
        let (map, _, mut game) = played("events/che", "Che");
        game.begin(OperationKind::Coup).unwrap();
        game.roll(&map, id(&map, "Honduras"), &mut Dice::from_seed(seed_for(1))).unwrap();
        assert_eq!(us(&game, &map, "Honduras"), 3, "1 + 3 ops doesn't beat 4");
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Us);
        assert!(game.ops_after_event().is_none());
    }

    #[test]
    fn che_second_coup_can_be_skipped_with_pass() {
        let (map, _, mut game) = played("events/che", "Che");
        game.begin(OperationKind::Coup).unwrap();
        game.roll(&map, id(&map, "Honduras"), &mut Dice::from_seed(seed_for(6))).unwrap();
        game.confirm().unwrap();
        game.pass().unwrap();
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn the_scoped_cards_only_offer_their_targets_to_the_ai() {
        use twilight_struggle::Action;
        let (map, cards, game) = played("events/ortega", "Ortega Elected in Nicaragua");
        let mut game = game;
        game.begin(OperationKind::Coup).unwrap();
        let rolls: Vec<CountryId> = game.legal_actions(&map, &cards).into_iter().filter_map(|a| if let Action::Roll(c) = a { Some(c) } else { None }).collect();
        assert!(rolls.contains(&id(&map, "Honduras")) && rolls.contains(&id(&map, "Cuba")));
        assert!(!rolls.contains(&id(&map, "Mexico")));
    }
}

mod discard_or_suffer {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::Operation;
    use twilight_struggle::CountryId;

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    /// Loads `choices/<state>` and plays the USSR's only card as an event.
    fn play(state: &str, card: &str) -> (WorldMap, CardCatalog, Game, EventOutcome) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("choices/{state}")).unwrap_or_else(|e| panic!("{state}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        let outcome = game.play_event(&map, &cards).unwrap();
        (map, cards, game, outcome)
    }

    fn us(game: &Game, map: &WorldMap, name: &str) -> u8 {
        game.board().influence(id(map, name), Superpower::Us)
    }

    fn ussr(game: &Game, map: &WorldMap, name: &str) -> u8 {
        game.board().influence(id(map, name), Superpower::Ussr)
    }

    fn mode_labels(game: &Game) -> Vec<String> {
        let Some(Operation::Event(e)) = game.operation() else { panic!("an event is open") };
        e.modes().iter().map(|m| m.label.clone()).collect()
    }

    #[test]
    fn blockade_asks_the_us_which_3_plus_ops_card_to_discard_or_to_keep_them() {
        let (_, _, game, outcome) = play("blockade", "Blockade");
        assert!(matches!(outcome, EventOutcome::Pending { chooser: Superpower::Us, .. }));
        assert_eq!(game.decider(), Superpower::Us, "the US decides, though the USSR is phasing");
        let labels = mode_labels(&game);
        assert_eq!(labels.len(), 3, "keep, or discard one of the two 3+ ops cards: {labels:?}");
        assert!(labels[0].contains("West Germany") && labels[1].contains("Marshall Plan") && labels[2].contains("Containment"));
        assert!(!labels.iter().any(|l| l.contains("Truman")), "a 1-op card doesn't qualify");
    }

    #[test]
    fn discarding_a_big_card_saves_west_germany_and_loses_the_card() {
        let (map, cards, mut game, _) = play("blockade", "Blockade");
        game.choose_mode(&map, 2).unwrap();
        game.confirm().unwrap();
        assert_eq!(us(&game, &map, "West Germany"), 4);
        let contain = cards.id_by_name("Containment").unwrap();
        assert!(game.discards().contains(&contain) && !game.hand(Superpower::Us).contains(&contain));
        assert!(game.removed_from_game().contains(&cards.id_by_name("Blockade").unwrap()), "Blockade is removed after its event");
        assert_eq!(game.active(), Superpower::Us, "the turn passes");
    }

    #[test]
    fn declining_blockade_clears_us_influence_from_west_germany_and_keeps_every_card() {
        let (map, _, mut game, _) = play("blockade", "Blockade");
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert_eq!(us(&game, &map, "West Germany"), 0);
        assert_eq!(game.hand(Superpower::Us).len(), 3);
    }

    #[test]
    fn the_blockade_decision_cannot_be_confirmed_unchosen_or_abandoned_by_the_phasing_player() {
        let (map, _, mut game, _) = play("blockade", "Blockade");
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "a choice has to be made");
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandonEvent)), "the USSR can't take the card back once the US must decide");
        game.choose_mode(&map, 0).unwrap();
        game.choose_mode(&map, 1).unwrap();
        game.choose_mode(&map, 0).unwrap();
    }

    #[test]
    fn the_victim_can_clear_their_choice_before_confirming() {
        let (map, _, mut game, _) = play("blockade", "Blockade");
        assert!(!game.clear_event_mode(&map), "nothing chosen yet");
        game.choose_mode(&map, 1).unwrap();
        assert_eq!(game.active(), Superpower::Ussr, "the USSR is still the phasing side");
        assert!(game.clear_event_mode(&map), "the US can take back its pick");
        let Some(Operation::Event(e)) = game.operation() else { panic!("still open") };
        assert_eq!(e.mode(), None);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
    }

    #[test]
    fn a_discard_decision_leaves_the_map_alone() {
        use twilight_struggle::render::{render_region, render_world_map};
        use twilight_struggle::{ColorMode, MapLayout, Region};
        let (map, _, game, _) = play("blockade", "Blockade");
        let layout = MapLayout::standard(&map).unwrap();
        let op = game.operation();
        let region = render_region(&map, &layout, game.board(), Region::Europe, None, op).render(ColorMode::Never);
        let world = render_world_map(&map, &layout, game.board(), None, op).render(ColorMode::Never);
        for text in [&region, &world] {
            assert!(!text.contains("not eligible") && !text.contains("can act here"), "no target legend:\n{text}");
            assert!(!text.contains('║'), "no chip is marked as live:\n{text}");
        }
        assert!(region.contains("look around") && region.contains("space discard it"), "the hint names the card keys:\n{region}");
        assert!(!region.contains("+ add"));
    }

    #[test]
    fn blockade_with_no_big_card_to_discard_just_applies() {
        let (map, _, game, outcome) = play("blockade-nothing-to-discard", "Blockade");
        assert!(matches!(outcome, EventOutcome::Effect(_)));
        assert_eq!(us(&game, &map, "West Germany"), 0);
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn declining_debt_crisis_hands_the_ussr_a_doubling_of_two_south_american_countries() {
        let (map, cards, mut game, _) = play("debt-crisis", "Latin American Debt Crisis");
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.decider(), Superpower::Ussr, "now the USSR chooses");
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandonEvent)), "the US has already answered: no taking the card back");
        game.place(&map, id(&map, "Chile")).unwrap();
        assert_eq!(game.operation().and_then(|op| op.board()).map(|b| b.influence(id(&map, "Chile"), Superpower::Ussr)), Some(6), "3 doubled, staged");
        assert!(game.place(&map, id(&map, "Chile")).is_err(), "once per country");
        assert!(game.place(&map, id(&map, "Peru")).is_err(), "nothing there to double");
        assert!(game.place(&map, id(&map, "Egypt")).is_err(), "not in South America");
        game.place(&map, id(&map, "Brazil")).unwrap();
        assert!(game.place(&map, id(&map, "Argentina")).is_err(), "only two countries");
        game.confirm().unwrap();
        assert_eq!((ussr(&game, &map, "Brazil"), ussr(&game, &map, "Chile"), ussr(&game, &map, "Argentina")), (4, 6, 1));
        assert_eq!(game.hand(Superpower::Us).len(), 3, "the US kept its cards");
        assert!(game.discards().contains(&cards.id_by_name("Latin American Debt Crisis").unwrap()), "not removed after its event");
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn a_doubling_can_be_taken_back() {
        let (map, _, mut game, _) = play("debt-crisis-nothing-to-discard", "Latin American Debt Crisis");
        game.place(&map, id(&map, "Chile")).unwrap();
        game.unplace(&map, id(&map, "Chile")).unwrap();
        assert_eq!(ussr(&game, &map, "Chile"), 3);
        game.place(&map, id(&map, "Brazil")).unwrap();
        game.confirm().unwrap();
        assert_eq!((ussr(&game, &map, "Brazil"), ussr(&game, &map, "Chile")), (4, 3));
    }

    #[test]
    fn discarding_to_debt_crisis_costs_the_card_and_spares_south_america() {
        let (map, cards, mut game, _) = play("debt-crisis", "Latin American Debt Crisis");
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        assert!(game.operation().is_none(), "no second stage");
        assert_eq!(ussr(&game, &map, "Chile"), 3);
        assert!(game.discards().contains(&cards.id_by_name("Marshall Plan").unwrap()));
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn debt_crisis_with_nothing_to_discard_goes_straight_to_the_ussr() {
        let (_, _, game, outcome) = play("debt-crisis-nothing-to-discard", "Latin American Debt Crisis");
        assert!(matches!(outcome, EventOutcome::Pending { chooser: Superpower::Ussr, .. }));
        assert_eq!(game.decider(), Superpower::Ussr);
    }

    #[test]
    fn the_discard_decision_is_logged() {
        let (map, cards, mut game, _) = play("blockade", "Blockade");
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        let text = twilight_struggle::render::log_text(&map, &cards, game.log());
        assert!(text.contains("USA discards Marshall Plan"), "{text}");
    }

    #[test]
    fn the_ai_can_always_finish_a_discard_decision() {
        use twilight_struggle::{play_turn, Dice, RandomAi};
        for seed in 0..12 {
            for (state, card) in [("blockade", "Blockade"), ("debt-crisis", "Latin American Debt Crisis")] {
                let (map, cards, mut game, _) = play(state, card);
                let mut ai = RandomAi::from_seed(seed);
                let mut dice = Dice::from_seed(seed);
                for _ in 0..4 {
                    if game.operation().is_some() {
                        play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
                    }
                }
                assert!(game.operation().is_none() && game.active() == Superpower::Us, "{state} seed {seed}");
            }
        }
    }
}

mod hands {
    use super::*;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::Operation;
    use twilight_struggle::{CountryId, Dice};

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    fn start(file: &str, state: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("{file}/{state}")).unwrap_or_else(|e| panic!("{state}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        (map, cards, game)
    }

    fn names(cards: &CardCatalog, ids: &[twilight_struggle::cards::CardId]) -> Vec<String> {
        ids.iter().map(|&c| cards.card(c).name.clone()).collect()
    }

    #[test]
    fn terrorism_needs_dice_and_discards_one_random_card() {
        let (map, cards, mut game) = start("events", "terrorism", "Terrorism");
        assert!(matches!(game.play_event(&map, &cards), Err(GameError::NeedsDice { .. })));
        let EventOutcome::Effect(result) = game.play_event_with(&map, &cards, &mut Dice::from_seed(5)).unwrap() else { panic!("a fixed effect") };
        assert_eq!(result.discards.len(), 1);
        let (side, card) = result.discards[0];
        assert_eq!(side, Superpower::Us);
        assert!(game.discards().contains(&card) && !game.hand(Superpower::Us).contains(&card));
        assert_eq!(game.hand(Superpower::Us).len(), 2);
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn terrorism_picks_differently_for_different_dice() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..30 {
            let (map, cards, mut game) = start("events", "terrorism", "Terrorism");
            let EventOutcome::Effect(r) = game.play_event_with(&map, &cards, &mut Dice::from_seed(seed)).unwrap() else { panic!() };
            seen.insert(r.discards[0].1);
        }
        assert_eq!(seen.len(), 3, "every card in the hand can be the one lost");
    }

    #[test]
    fn terrorism_costs_the_us_two_cards_once_the_hostage_crisis_has_been_played() {
        let (map, cards, mut game) = start("events", "terrorism-after-hostage-crisis", "Terrorism");
        let EventOutcome::Effect(result) = game.play_event_with(&map, &cards, &mut Dice::from_seed(1)).unwrap() else { panic!() };
        assert_eq!(result.discards.len(), 2);
        assert_ne!(result.discards[0].1, result.discards[1].1, "two different cards");
        assert_eq!(game.hand(Superpower::Us).len(), 1);
    }

    #[test]
    fn terrorism_played_by_the_us_costs_the_ussr_only_one_card() {
        let (map, cards, mut game) = start("events", "terrorism-played-by-us", "Terrorism");
        let EventOutcome::Effect(result) = game.play_event_with(&map, &cards, &mut Dice::from_seed(1)).unwrap() else { panic!() };
        assert_eq!(result.discards.len(), 1);
        assert_eq!(result.discards[0].0, Superpower::Ussr);
        assert_eq!(game.hand(Superpower::Ussr).len(), 2);
    }

    #[test]
    fn terrorism_with_an_empty_hand_discards_nothing() {
        let (map, cards, mut game) = start("events", "terrorism", "Terrorism");
        for c in game.hand(Superpower::Us).to_vec() {
            game.hands_mut().take(c);
            game.hands_mut().discard(c);
        }
        assert!(game.hand(Superpower::Us).is_empty());
        let EventOutcome::Effect(result) = game.play_event_with(&map, &cards, &mut Dice::from_seed(1)).unwrap() else { panic!() };
        assert!(result.discards.is_empty());
    }

    #[test]
    fn aldrich_ames_lets_the_ussr_pick_a_us_card_and_opens_the_hand_for_the_turn() {
        let (map, cards, mut game) = start("choices", "aldrich-ames", "Aldrich Ames Remix");
        assert!(matches!(game.play_event(&map, &cards).unwrap(), EventOutcome::Pending { chooser: Superpower::Ussr, .. }));
        assert_eq!(game.decider(), Superpower::Ussr);
        let Some(Operation::Event(e)) = game.operation() else { panic!("a session is open") };
        assert_eq!(e.gate_side(), Superpower::Us, "the USSR picks from the US hand");
        assert_eq!((e.gate_offset(), e.gate_cards().len()), (0, 3));
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "a card has to be picked");
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        let containment = cards.id_by_name("Containment").unwrap();
        assert!(game.discards().contains(&containment) && !game.hand(Superpower::Us).contains(&containment));
        assert_eq!(game.hand(Superpower::Us).len(), 2);
        assert!(game.status().effects.hand_revealed(Superpower::Us), "the US hand is open for the rest of the turn");
        assert!(game.removed_from_game().contains(&cards.id_by_name("Aldrich Ames Remix").unwrap()));
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn aldrich_ames_is_a_card_pick_not_a_region_designation() {
        let (map, cards, mut game) = start("choices", "aldrich-ames", "Aldrich Ames Remix");
        game.play_event(&map, &cards).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!("a session is open") };
        assert!(!e.is_designation(), "only Chernobyl designates a region");
        assert!(e.is_mode_only(), "so the map is left alone and the confirm reminder applies");
        assert!(e.prompt().contains("discard Marshall Plan") && !e.prompt().contains("region"), "{}", e.prompt());
    }

    #[test]
    fn the_revealed_hand_closes_when_the_turn_rolls_over() {
        let (map, cards, mut game) = start("choices", "aldrich-ames", "Aldrich Ames Remix");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert!(game.status().effects.hand_revealed(Superpower::Us));
        // Both sides pass out the rest of the turn.
        for _ in 0..30 {
            if game.status().turn > 8 {
                break;
            }
            game.pass().unwrap();
        }
        assert!(!game.status().effects.hand_revealed(Superpower::Us));
    }

    #[test]
    fn aldrich_ames_with_an_empty_us_hand_only_reveals() {
        let (map, cards, mut game) = start("choices", "aldrich-ames-empty-hand", "Aldrich Ames Remix");
        assert!(matches!(game.play_event(&map, &cards).unwrap(), EventOutcome::Effect(_)));
        assert!(game.status().effects.hand_revealed(Superpower::Us));
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn the_ai_can_finish_aldrich_ames() {
        use twilight_struggle::{play_turn, RandomAi};
        for seed in 0..10 {
            let (map, cards, mut game) = start("choices", "aldrich-ames", "Aldrich Ames Remix");
            game.play_event(&map, &cards).unwrap();
            let mut ai = RandomAi::from_seed(seed);
            let mut dice = Dice::from_seed(seed);
            for _ in 0..3 {
                if game.operation().is_some() {
                    play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
                }
            }
            assert!(game.operation().is_none() && game.hand(Superpower::Us).len() == 2, "seed {seed}");
        }
    }

    #[test]
    fn cia_created_and_lone_gunman_open_the_hand_for_the_turn_too() {
        let (map, cards, mut game) = start("events", "cia-created", "CIA Created");
        game.play_event(&map, &cards).unwrap();
        assert!(game.status().effects.hand_revealed(Superpower::Ussr));
        assert!(!game.status().effects.hand_revealed(Superpower::Us));
        let (map, cards, mut game) = start("events", "lone-gunman", "“Lone Gunman”");
        game.play_event(&map, &cards).unwrap();
        assert!(game.status().effects.hand_revealed(Superpower::Us));
    }

    #[test]
    fn cambridge_five_reveals_the_scoring_cards_and_adds_influence_in_a_named_region() {
        let (map, cards, mut game) = start("events", "cambridge-five", "The Cambridge Five");
        assert!(matches!(game.play_event(&map, &cards).unwrap(), EventOutcome::Pending { chooser: Superpower::Ussr, .. }));
        assert!(game.place(&map, id(&map, "Egypt")).is_err(), "the Middle East isn't named by a revealed card");
        game.place(&map, id(&map, "Poland")).unwrap();
        assert!(game.place(&map, id(&map, "Japan")).is_err(), "a single country only");
        game.confirm().unwrap();
        assert_eq!(game.board().influence(id(&map, "Poland"), Superpower::Ussr), 1);
        let logged = game.log().entries().iter().rev().find_map(|e| match &e.event {
            twilight_struggle::Event::EventResolved { result, .. } => Some(result.clone()),
            _ => None,
        });
        let reveal = logged.and_then(|r| r.reveals).expect("the scoring cards are revealed");
        assert_eq!(names(&cards, &reveal.cards), ["Asia Scoring", "Europe Scoring", "Southeast Asia Scoring"]);
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn cambridge_five_can_be_skipped_and_southeast_asia_names_no_region() {
        let (map, cards, mut game) = start("events", "cambridge-five", "The Cambridge Five");
        game.play_event(&map, &cards).unwrap();
        game.confirm().expect("a 'may' event can be confirmed with nothing added");
        assert_eq!(game.board().influence(id(&map, "Poland"), Superpower::Ussr), 0);
    }

    #[test]
    fn cambridge_five_with_no_scoring_cards_just_reveals_nothing() {
        let (map, cards, mut game) = start("events", "cambridge-five-no-scoring-cards", "The Cambridge Five");
        let EventOutcome::Effect(result) = game.play_event(&map, &cards).unwrap() else { panic!("resolves on the spot") };
        assert_eq!(result.reveals.map(|r| r.cards.len()), Some(0));
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn cambridge_five_cannot_be_played_in_the_late_war() {
        let (map, cards, mut game) = start("events", "cambridge-five-late-war", "The Cambridge Five");
        assert!(matches!(game.play_event(&map, &cards), Err(GameError::EventTooLate { .. })));
        let legal = game.legal_actions(&map, &cards);
        assert!(!legal.contains(&twilight_struggle::Action::Event), "not offered to the AI either");
    }
}

mod contests {
    use super::*;
    use twilight_struggle::events::Contest;
    use twilight_struggle::game::GameError;
    use twilight_struggle::ops::Operation;
    use twilight_struggle::{CountryId, Dice, OperationKind};

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    /// A seed whose first two rolls are `(a, b)`.
    fn seed_for(a: u8, b: u8) -> u64 {
        (0..10_000)
            .find(|&s| {
                let mut d = Dice::from_seed(s);
                d.roll() == a && d.roll() == b
            })
            .unwrap()
    }

    fn start(state: &str, card: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("events/{state}")).unwrap_or_else(|e| panic!("{state}: {e}"));
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name(card).unwrap()).unwrap();
        (map, cards, game)
    }

    fn event_choice_modes(game: &Game) -> Vec<String> {
        let Some(Operation::Event(e)) = game.operation() else { panic!("an event is open") };
        e.modes().iter().map(|m| m.label.clone()).collect()
    }

    fn last_contest(game: &Game) -> Contest {
        game.log()
            .entries()
            .iter()
            .rev()
            .find_map(|e| match &e.event {
                twilight_struggle::Event::EventResolved { result, .. } => result.contest.clone(),
                _ => None,
            })
            .expect("the event logged its roll-off")
    }

    // ---- Summit ----

    /// Plays Summit's event (opening its roll-off) and throws the dice `(us, ussr)`.
    fn summit_rolled(game: &mut Game, map: &WorldMap, cards: &CardCatalog, a: u8, b: u8) -> Contest {
        assert!(matches!(game.play_event(map, cards).unwrap(), EventOutcome::Pending { .. }));
        game.roll_contest(map, &mut Dice::from_seed(seed_for(a, b))).unwrap()
    }

    #[test]
    fn summit_waits_for_the_roll_and_shows_the_odds_first() {
        let (map, cards, mut game) = start("summit", "Summit");
        let outcome = game.play_event(&map, &cards).unwrap();
        assert!(matches!(outcome, EventOutcome::Pending { chooser: Superpower::Us, .. }), "the player rolls");
        let Some(Operation::Event(e)) = game.operation() else { panic!("an event is open") };
        assert!(e.needs_roll() && e.has_session_modal());
        let (us, ussr) = e.pending_bonuses().unwrap();
        assert_eq!((us.0, us.1.as_str(), ussr.0), (1, "Central America", 0));
        // US d6+1 against USSR d6: 21 ways to win, 5 to tie, 10 to lose.
        assert_eq!(e.roll_odds(), Some((21, 5, 10)));
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "roll first");
        assert!(matches!(game.choose_mode(&map, 0), Err(GameError::Event(_))), "nothing to choose before the roll");
    }

    #[test]
    fn an_unrolled_summit_can_be_taken_back_but_a_rolled_one_cannot() {
        let (map, cards, mut game) = start("summit", "Summit");
        game.play_event(&map, &cards).unwrap();
        game.abandon().expect("nothing has been rolled");
        assert!(game.card_in_play().is_some() && game.operation().is_none());

        game.play_event(&map, &cards).unwrap();
        game.roll_contest(&map, &mut Dice::from_seed(seed_for(4, 3))).unwrap();
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandonEvent)), "the dice are thrown");
        assert!(matches!(game.roll_contest(&map, &mut Dice::from_seed(1)), Err(GameError::Event(_))), "only one roll");
    }

    #[test]
    fn summit_adds_one_per_dominated_or_controlled_region_and_the_winner_chooses_defcon() {
        let (map, cards, mut game) = start("summit", "Summit");
        let contest = summit_rolled(&mut game, &map, &cards, 4, 3);
        assert_eq!(contest.winner(), Some(Superpower::Us), "US 4+1 beats USSR 3");
        assert_eq!(game.decider(), Superpower::Us);
        let labels = event_choice_modes(&game);
        assert_eq!(labels, ["improve DEFCON to 4", "degrade DEFCON to 2", "leave DEFCON at 3"]);
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "the winner has to choose");
        game.choose_mode(&map, 0).unwrap();
        game.confirm().unwrap();
        assert_eq!((game.status().defcon, game.status().vp), (4, 2));
        assert_eq!(game.active(), Superpower::Ussr);
        let contest = last_contest(&game);
        assert_eq!((contest.us.die, contest.us.bonus, contest.us.note.as_str(), contest.ussr.die, contest.ussr.bonus), (4, 1, "Central America", 3, 0));
    }

    #[test]
    fn summit_won_by_the_ussr_pays_the_ussr_and_the_ussr_picks() {
        let (map, cards, mut game) = start("summit", "Summit");
        let contest = summit_rolled(&mut game, &map, &cards, 1, 4);
        assert_eq!(contest.winner(), Some(Superpower::Ussr), "US 1+1 loses to USSR 4");
        assert_eq!(game.decider(), Superpower::Ussr, "the winner decides, though the US is phasing");
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        assert_eq!((game.status().defcon, game.status().vp), (2, -2));
    }

    #[test]
    fn the_summit_winner_can_change_their_mind_before_confirming() {
        let (map, cards, mut game) = start("summit", "Summit");
        summit_rolled(&mut game, &map, &cards, 1, 4);
        game.choose_mode(&map, 0).unwrap();
        assert!(game.clear_event_mode(&map), "the USSR isn't the phasing side, but it is the one choosing");
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })));
    }

    #[test]
    fn summit_winner_may_leave_defcon_alone() {
        let (map, cards, mut game) = start("summit", "Summit");
        summit_rolled(&mut game, &map, &cards, 6, 1);
        game.choose_mode(&map, 2).unwrap();
        game.confirm().unwrap();
        assert_eq!((game.status().defcon, game.status().vp), (3, 2));
    }

    #[test]
    fn a_summit_tie_changes_nothing_and_is_not_rerolled() {
        let (map, cards, mut game) = start("summit", "Summit");
        let contest = summit_rolled(&mut game, &map, &cards, 3, 4);
        assert_eq!((contest.us.total(), contest.ussr.total(), contest.rerolls), (4, 4, 0));
        assert_eq!(contest.winner(), None);
        game.confirm().expect("a tie is simply acknowledged");
        assert_eq!((game.status().defcon, game.status().vp, game.active()), (3, 0, Superpower::Ussr));
        assert_eq!(last_contest(&game).winner(), None, "the tie is in the log");
    }

    #[test]
    fn summit_cannot_improve_past_defcon_5() {
        let (map, cards, mut game) = start("summit", "Summit");
        game.status_mut().defcon = 5;
        summit_rolled(&mut game, &map, &cards, 4, 3);
        assert_eq!(event_choice_modes(&game), ["degrade DEFCON to 4", "leave DEFCON at 5"]);
    }

    #[test]
    fn a_summit_degrade_to_defcon_1_loses_for_the_phasing_player() {
        let (map, cards, mut game) = start("summit", "Summit");
        game.status_mut().defcon = 2;
        // The US plays it and the USSR wins the roll-off and degrades DEFCON: rule 8.1.3 still
        // blames the phasing player, so the US loses.
        summit_rolled(&mut game, &map, &cards, 1, 5);
        assert_eq!(game.decider(), Superpower::Ussr);
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 1);
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Ussr, reason: VictoryReason::Defcon }), "the phasing player (the US) is responsible");
    }

    #[test]
    fn the_modal_renders_the_odds_then_the_result_and_the_options() {
        use twilight_struggle::render::render_event_session;
        use twilight_struggle::ColorMode;
        let (map, cards, mut game) = start("summit", "Summit");
        game.play_event(&map, &cards).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        let before = render_event_session(&cards, e, game.status()).render(ColorMode::Never);
        assert!(before.contains("Central America") && before.contains("21/36") && before.contains("r roll"), "{before}");
        game.roll_contest(&map, &mut Dice::from_seed(seed_for(4, 3))).unwrap();
        game.choose_mode(&map, 0).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        let after = render_event_session(&cards, e, game.status()).render(ColorMode::Never);
        assert!(after.contains("USA wins") && after.contains("▶ 1) improve DEFCON to 4") && after.contains("DEFCON 3 → 4") && after.contains("+2 VP to the US"), "{after}");
        assert!(after.contains("c confirm"), "{after}");
        for line in before.lines().chain(after.lines()) {
            assert!(line.chars().count() <= 68 + 1, "a line overflows the modal: {line:?}");
        }
    }

    // ---- Olympic Games ----

    #[test]
    fn olympic_games_asks_the_sponsors_opponent_to_participate_or_boycott() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        let outcome = game.play_event(&map, &cards).unwrap();
        assert!(matches!(outcome, EventOutcome::Pending { chooser: Superpower::Us, .. }));
        assert_eq!(game.decider(), Superpower::Us);
        assert!(matches!(game.abandon(), Err(GameError::CannotAbandonEvent)), "the sponsor can't take it back");
        let labels = event_choice_modes(&game);
        assert!(labels[0].starts_with("participate") && labels[1].starts_with("boycott: DEFCON drops to 2"), "{labels:?}");
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        assert!(e.has_session_modal() && !e.needs_roll(), "nothing to roll until the US takes part");
    }

    #[test]
    fn taking_part_waits_for_the_roll_with_the_sponsor_adding_two() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 0).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        assert!(e.needs_roll() && e.rerolls_ties());
        let (us, ussr) = e.pending_bonuses().unwrap();
        assert_eq!((us.0, ussr.0, ussr.1.as_str()), (0, 2, "sponsor"));
        // US d6 against USSR d6+2: 6 ways to win, 4 to tie, 26 to lose.
        assert_eq!(e.roll_odds(), Some((6, 4, 26)));
        assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "roll first");
        game.choose_mode(&map, 1).expect("they can still change their mind before the dice are thrown");
        game.choose_mode(&map, 0).unwrap();
    }

    #[test]
    fn participating_gives_the_roll_off_winner_2_vp_and_the_sponsor_adds_2() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 0).unwrap();
        let contest = game.roll_contest(&map, &mut Dice::from_seed(seed_for(6, 2))).unwrap();
        assert_eq!(contest.winner(), Some(Superpower::Us), "US 6 beats USSR 2+2");
        game.confirm().unwrap();
        assert_eq!(game.status().vp, 2);
        let contest = last_contest(&game);
        assert_eq!((contest.us.total(), contest.ussr.total(), contest.ussr.note.as_str()), (6, 4, "sponsor"));
        assert_eq!((game.status().defcon, game.active()), (3, Superpower::Us));

        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 0).unwrap();
        game.roll_contest(&map, &mut Dice::from_seed(seed_for(2, 1))).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().vp, -2, "USSR 1+2 = 3 beats US 2: the sponsor wins");
    }

    #[test]
    fn olympic_ties_are_rerolled() {
        // US 3 vs USSR 1+2 ties; the next pair breaks it.
        let seed = (0..100_000)
            .find(|&s| {
                let mut d = Dice::from_seed(s);
                d.roll() == 3 && d.roll() == 1 && d.roll() == 6 && d.roll() == 1
            })
            .unwrap();
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 0).unwrap();
        game.roll_contest(&map, &mut Dice::from_seed(seed)).unwrap();
        game.confirm().unwrap();
        let contest = last_contest(&game);
        assert_eq!(contest.rerolls, 1);
        assert_eq!((contest.us.die, contest.ussr.die), (6, 1));
        assert_eq!(game.status().vp, 2);
    }

    #[test]
    fn once_the_dice_are_thrown_the_olympic_choice_is_locked() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 0).unwrap();
        game.roll_contest(&map, &mut Dice::from_seed(1)).unwrap();
        assert!(matches!(game.choose_mode(&map, 1), Err(GameError::Event(_))), "no switching to a boycott after seeing the result");
        assert!(!game.clear_event_mode(&map));
        assert!(matches!(game.roll_contest(&map, &mut Dice::from_seed(2)), Err(GameError::Event(_))), "only one roll");
    }

    #[test]
    fn boycotting_drops_defcon_and_lets_the_sponsor_conduct_four_ops() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 1).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        assert!(!e.needs_roll(), "a boycott rolls nothing");
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 2);
        assert_eq!(game.active(), Superpower::Ussr, "the sponsor still has the card in play");
        let grant = game.ops_after_event().expect("the sponsor may conduct operations");
        assert_eq!(grant.ops, Some(4));
        assert_eq!(game.ops_available(), 4, "a 2-ops card, worth 4 for this");
        game.begin(OperationKind::Influence).unwrap();
        assert_eq!(game.operation().unwrap().ops_total(), 4);
        game.place(&map, id(&map, "Poland")).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.board().influence(id(&map, "Poland"), Superpower::Ussr), 2);
        assert!(game.discards().contains(&cards.id_by_name("Olympic Games").unwrap()), "not removed after its event");
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn the_sponsor_can_skip_the_boycott_operations() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        game.pass().unwrap();
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn a_boycott_that_takes_defcon_to_1_loses_for_the_sponsor_as_the_phasing_player() {
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.status_mut().defcon = 2;
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 1).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::Defcon }), "the USSR is phasing, so it is responsible even though the US boycotted");
    }

    #[test]
    fn the_olympic_modal_walks_through_choice_odds_and_result() {
        use twilight_struggle::render::render_event_session;
        use twilight_struggle::ColorMode;
        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        let draw = |game: &Game| {
            let Some(Operation::Event(e)) = game.operation() else { panic!() };
            render_event_session(&cards, e, game.status()).render(ColorMode::Never)
        };
        let choosing = draw(&game);
        assert!(choosing.contains("USSR sponsors") && choosing.contains("1) participate") && choosing.contains("2) boycott") && choosing.contains("1-2 choose"), "{choosing}");
        game.choose_mode(&map, 0).unwrap();
        let odds = draw(&game);
        assert!(odds.contains("▶ 1) participate") && odds.contains("6/32") && odds.contains("26/32") && odds.contains("thrown again") && odds.contains("r roll"), "{odds}");
        game.roll_contest(&map, &mut Dice::from_seed(seed_for(6, 2))).unwrap();
        let rolled = draw(&game);
        assert!(rolled.contains("USA wins the Olympic Games") && rolled.contains("+2 VP to the US") && rolled.contains("c confirm"), "{rolled}");
        for line in rolled.lines().chain(odds.lines()).chain(choosing.lines()) {
            assert!(line.chars().count() <= 68 + 1, "a line overflows the modal: {line:?}");
        }

        let (map, cards, mut game) = start("olympic-games", "Olympic Games");
        game.play_event(&map, &cards).unwrap();
        game.choose_mode(&map, 1).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        let boycott = render_event_session(&cards, e, game.status()).render(ColorMode::Never);
        for line in boycott.lines() {
            assert!(line.chars().count() <= 68 + 1, "a line overflows the modal: {line:?}");
        }
        assert!(boycott.contains("DEFCON 3 → 2") && boycott.contains("USSR may then conduct") && boycott.contains("worth 4 ops"), "{boycott}");
    }

    #[test]
    fn the_ai_can_finish_both_decisions() {
        use twilight_struggle::{play_turn, RandomAi};
        for seed in 0..10 {
            for (state, card) in [("summit", "Summit"), ("olympic-games", "Olympic Games")] {
                let (map, cards, mut game) = start(state, card);
                game.play_event(&map, &cards).unwrap();
                let mut ai = RandomAi::from_seed(seed);
                let mut dice = Dice::from_seed(seed);
                for _ in 0..6 {
                    if game.winner().is_none() && (game.operation().is_some() || game.ops_after_event().is_some()) {
                        play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
                    }
                }
                assert!(game.winner().is_some() || (game.operation().is_none() && game.ops_after_event().is_none()), "{state} seed {seed}");
            }
        }
    }
}

/// Bear Trap / Quagmire (escape attempts), NORAD (an end-of-round trigger) and Cuban Missile Crisis.
mod rounds {
    use super::*;
    use twilight_struggle::game::{GameError, Trap};
    use twilight_struggle::ops::Operation;
    use twilight_struggle::{Action, CountryId, Dice, OperationKind};

    fn load(state: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("events/{state}")).unwrap_or_else(|e| panic!("{state}: {e}"));
        (map, cards, Game::from_scenario(&scenario))
    }

    fn id(map: &WorldMap, name: &str) -> CountryId {
        map.id_by_name(name).unwrap_or_else(|| panic!("no country {name}"))
    }

    /// A seed whose first roll satisfies `ok`.
    fn seed_where(ok: impl Fn(u8) -> bool) -> u64 {
        (0..10_000).find(|&s| ok(Dice::from_seed(s).roll())).unwrap()
    }

    fn card(cards: &CardCatalog, name: &str) -> twilight_struggle::CardId {
        cards.id_by_name(name).unwrap()
    }

    #[test]
    fn playing_bear_trap_sets_the_trap_and_removes_the_card() {
        let (map, cards, mut game) = load("bear-trap");
        game.play_card(&cards, card(&cards, "Bear Trap")).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert!(game.status().lasting.bear_trap && !game.status().lasting.quagmire);
        assert!(game.removed_from_game().contains(&card(&cards, "Bear Trap")));
        assert_eq!(game.active(), Superpower::Ussr);
        assert!(matches!(game.trap(), Some((_, Trap::Escape(c))) if c == vec![card(&cards, "Fidel"), card(&cards, "Socialist Governments")]));
    }

    #[test]
    fn a_trapped_side_must_escape_instead_of_playing_or_passing() {
        let (_, cards, mut game) = load("bear-trap-active");
        assert!(matches!(game.play_card(&cards, card(&cards, "Fidel")), Err(GameError::Trap(_))));
        assert!(matches!(game.play_card(&cards, card(&cards, "Asia Scoring")), Err(GameError::Trap(_))));
        assert!(matches!(game.pass(), Err(GameError::Trap(_))));
        // Only discards of 2+ ops cards are offered to an AI.
        let (map, cards2, _) = load("bear-trap-active");
        let legal = game.legal_actions(&map, &cards2);
        assert_eq!(legal, vec![Action::Escape(card(&cards, "Fidel")), Action::Escape(card(&cards, "Socialist Governments"))]);
    }

    #[test]
    fn rolling_one_to_four_escapes_and_spends_the_round() {
        let (_, cards, mut game) = load("bear-trap-active");
        let mut dice = Dice::from_seed(seed_where(|r| r <= 4));
        let fidel = card(&cards, "Fidel");
        assert!(matches!(game.escape_trap(&mut dice, card(&cards, "Asia Scoring")), Err(GameError::Trap(_))), "a scoring card can't be discarded");
        let result = game.escape_trap(&mut dice, fidel).unwrap();
        assert!(result.escaped && result.roll <= 4);
        assert!(!game.status().lasting.bear_trap);
        assert!(game.discards().contains(&fidel) && !game.hand(Superpower::Ussr).contains(&fidel));
        assert_eq!(game.active(), Superpower::Us, "the escape attempt was the USSR's whole round");
        assert!(game.trap().is_none());
    }

    #[test]
    fn rolling_five_or_six_stays_trapped_until_the_next_round() {
        let (_, cards, mut game) = load("bear-trap-active");
        let mut dice = Dice::from_seed(seed_where(|r| r >= 5));
        let result = game.escape_trap(&mut dice, card(&cards, "Fidel")).unwrap();
        assert!(!result.escaped && result.roll >= 5);
        assert!(game.status().lasting.bear_trap);
        assert_eq!(game.active(), Superpower::Us);
        // The US plays its card; the USSR is trapped again.
        game.play_card(&cards, card(&cards, "Duck and Cover")).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.active(), Superpower::Ussr);
        assert!(matches!(game.trap(), Some((_, Trap::Escape(c))) if c == vec![card(&cards, "Socialist Governments")]));
    }

    #[test]
    fn with_no_card_to_discard_only_scoring_cards_can_be_played_then_the_round_is_skipped() {
        let (map, cards, mut game) = load("bear-trap-no-ops-card");
        assert!(matches!(game.trap(), Some((_, Trap::PlayScoring))));
        assert!(matches!(game.play_card(&cards, card(&cards, "Nasser")), Err(GameError::Trap(_))));
        assert!(matches!(game.pass(), Err(GameError::Trap(_))));
        assert_eq!(game.legal_actions(&map, &cards), vec![Action::PlayCard(card(&cards, "Asia Scoring"))]);
        game.play_card(&cards, card(&cards, "Asia Scoring")).unwrap();
        game.play_event(&map, &cards).unwrap();
        // Back round to the USSR: nothing to discard or score — skip.
        game.play_card(&cards, card(&cards, "Duck and Cover")).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert!(matches!(game.trap(), Some((_, Trap::Skip))));
        assert_eq!(game.legal_actions(&map, &cards), vec![Action::Pass]);
        game.pass().unwrap();
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn a_hand_with_nothing_playable_just_skips() {
        let (map, cards, mut game) = load("bear-trap-nothing-to-play");
        assert!(matches!(game.trap(), Some((_, Trap::Skip))));
        assert_eq!(game.legal_actions(&map, &cards), vec![Action::Pass]);
        game.pass().unwrap();
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn quagmire_traps_the_us_and_ends_norad() {
        let (map, cards, mut game) = load("quagmire");
        assert!(game.status().lasting.norad);
        game.play_card(&cards, card(&cards, "Quagmire")).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert!(game.status().lasting.quagmire && !game.status().lasting.norad);
        assert_eq!(game.active(), Superpower::Us);
        assert!(matches!(game.trap(), Some((_, Trap::Escape(c))) if c == vec![card(&cards, "Duck and Cover"), card(&cards, "Fidel")]));
    }

    #[test]
    fn the_us_under_quagmire_discards_a_two_plus_ops_card() {
        let (_, cards, mut game) = load("quagmire-active");
        // Truman Doctrine is 1 op, so only Duck and Cover can go.
        assert!(matches!(game.trap(), Some((_, Trap::Escape(c))) if c == vec![card(&cards, "Duck and Cover")]));
        let mut dice = Dice::from_seed(seed_where(|r| r <= 4));
        assert!(game.escape_trap(&mut dice, card(&cards, "Truman Doctrine")).is_err());
        assert!(game.escape_trap(&mut dice, card(&cards, "Duck and Cover")).unwrap().escaped);
        assert!(!game.status().lasting.quagmire);
    }

    #[test]
    fn the_ai_gets_out_of_a_trap_without_stalling() {
        use twilight_struggle::{play_turn, RandomAi};
        for seed in 0..20 {
            for state in ["bear-trap-active", "bear-trap-no-ops-card", "bear-trap-nothing-to-play"] {
                let (map, cards, mut game) = load(state);
                let mut ai = RandomAi::from_seed(seed);
                let mut dice = Dice::from_seed(seed);
                play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
                assert_eq!(game.active(), Superpower::Us, "{state} seed {seed}");
            }
        }
    }

    #[test]
    fn norad_adds_us_influence_after_a_round_that_moved_defcon_to_two() {
        let (map, cards, mut game) = load("norad-active");
        game.play_card(&cards, card(&cards, "Duck and Cover")).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().defcon, 2);
        assert!(game.settlement_due());
        assert!(matches!(game.legal_actions(&map, &cards).as_slice(), [Action::Settle]));
        assert!(matches!(game.play_card(&cards, card(&cards, "Fidel")), Err(GameError::Trap(_))), "the trigger has to be settled first");
        game.settle(&map);
        let Some(Operation::Event(e)) = game.operation() else { panic!("NORAD opens an event") };
        assert!(e.is_triggered() && e.chooser() == Superpower::Us);
        assert_eq!(game.decider(), Superpower::Us);
        let (canada, italy, france) = (id(&map, "Canada"), id(&map, "Italy"), id(&map, "France"));
        assert!(game.place(&map, france).is_err(), "only countries already holding US influence");
        game.place(&map, italy).unwrap();
        assert!(game.place(&map, canada).is_err(), "a single country");
        assert!(game.abandon().is_err(), "the trigger can't be backed out of");
        let turn = (game.status().turn, game.status().action_round, game.active());
        game.confirm().unwrap();
        assert_eq!(game.board().influence(italy, Superpower::Us), 2);
        assert_eq!((game.status().turn, game.status().action_round, game.active()), turn, "no card was spent and no turn handed over");
        assert!(game.operation().is_none());
    }

    #[test]
    fn norad_needs_canada() {
        let (map, cards, mut game) = load("norad-no-canada");
        game.play_card(&cards, card(&cards, "Duck and Cover")).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().defcon, 2);
        game.settle(&map);
        assert!(game.operation().is_none() && !game.settlement_due());
    }

    #[test]
    fn norad_only_fires_when_the_round_moved_defcon_to_two() {
        let (_, cards, mut game) = load("norad-active");
        // An ordinary round that leaves DEFCON at 3.
        game.play_card(&cards, card(&cards, "Socialist Governments")).unwrap();
        game.begin(OperationKind::Influence).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 3);
        assert!(!game.settlement_due());
    }

    #[test]
    fn the_ai_settles_norad_and_plays_on() {
        use twilight_struggle::{play_turn, RandomAi};
        for seed in 0..20 {
            let (map, cards, mut game) = load("norad-active");
            game.play_card(&cards, card(&cards, "Duck and Cover")).unwrap();
            game.play_event(&map, &cards).unwrap();
            let mut ai = RandomAi::from_seed(seed);
            let mut dice = Dice::from_seed(seed);
            for _ in 0..4 {
                if game.winner().is_none() {
                    play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
                }
            }
            assert!(!game.settlement_due(), "seed {seed}");
        }
    }

    #[test]
    fn cuban_missile_crisis_sets_defcon_two_and_forbids_the_opponents_coups() {
        let (map, cards, mut game) = load("cuban-missile-crisis");
        game.play_card(&cards, card(&cards, "Cuban Missile Crisis")).unwrap();
        game.play_event(&map, &cards).unwrap();
        assert_eq!(game.status().defcon, 2);
        assert_eq!(game.status().effects.cuban_missile_crisis, Some(Superpower::Us));
        assert!(game.status().effects.coup_forbidden(Superpower::Ussr) && !game.status().effects.coup_forbidden(Superpower::Us));
        assert!(game.removed_from_game().contains(&card(&cards, "Cuban Missile Crisis")));
    }

    #[test]
    fn a_coup_during_the_crisis_loses_the_game() {
        let (map, cards, mut game) = load("cuban-missile-crisis-active");
        game.play_card(&cards, card(&cards, "Socialist Governments")).unwrap();
        // The AI is never offered the coup.
        game.begin(OperationKind::Coup).unwrap();
        game.roll(&map, id(&map, "Honduras"), &mut Dice::from_seed(1)).unwrap();
        assert_eq!(game.winner(), Some(Victory { side: Superpower::Us, reason: VictoryReason::CubanMissileCrisis }));
    }

    #[test]
    fn legal_actions_never_offer_the_losing_coup() {
        let (map, cards, mut game) = load("cuban-missile-crisis-active");
        game.play_card(&cards, card(&cards, "Socialist Governments")).unwrap();
        let legal = game.legal_actions(&map, &cards);
        assert!(!legal.contains(&Action::Begin(OperationKind::Coup)));
        assert!(legal.contains(&Action::Begin(OperationKind::Realign)));
    }

    #[test]
    fn the_threatened_side_can_defuse_the_crisis() {
        let (map, cards, mut game) = load("cuban-missile-crisis-active");
        let cuba = id(&map, "Cuba");
        assert!(matches!(game.defuse_crisis(&map, id(&map, "Honduras")), Err(GameError::Trap(_))), "only Cuba for the USSR");
        game.defuse_crisis(&map, cuba).unwrap();
        assert_eq!(game.board().influence(cuba, Superpower::Ussr), 1);
        assert_eq!(game.status().effects.cuban_missile_crisis, None);
        // The USSR may coup freely now.
        game.play_card(&cards, card(&cards, "Socialist Governments")).unwrap();
        game.begin(OperationKind::Coup).unwrap();
        game.roll(&map, id(&map, "Honduras"), &mut Dice::from_seed(1)).unwrap();
        assert!(game.winner().is_none());
        // A second defuse has nothing to defuse.
        assert!(game.defuse_crisis(&map, cuba).is_err());
    }

    #[test]
    fn defusing_needs_two_influence_to_remove() {
        let (map, _, mut game) = load("cuban-missile-crisis-active");
        let cuba = id(&map, "Cuba");
        game.board_mut().set_influence(cuba, Superpower::Ussr, 1);
        assert!(matches!(game.defuse_crisis(&map, cuba), Err(GameError::Trap(_))));
        assert_eq!(game.status().effects.cuban_missile_crisis, Some(Superpower::Us));
    }

    #[test]
    fn the_us_defuses_its_own_crisis_through_west_germany_or_turkey() {
        let (map, cards, mut game) = load("cuban-missile-crisis");
        // The USSR plays the crisis against the US.
        game.status_mut().active = Superpower::Ussr;
        game.play_card(&cards, card(&cards, "Fidel")).unwrap();
        game.return_card().unwrap();
        game.status_mut().effects.cuban_missile_crisis = Some(Superpower::Ussr);
        let (wg, turkey, cuba) = (id(&map, "West Germany"), id(&map, "Turkey"), id(&map, "Cuba"));
        assert!(game.defuse_crisis(&map, cuba).is_err());
        game.defuse_crisis(&map, turkey).unwrap();
        assert_eq!(game.board().influence(turkey, Superpower::Us), 0);
        assert_eq!(game.board().influence(wg, Superpower::Us), 3);
        assert_eq!(game.status().effects.cuban_missile_crisis, None);
    }
}

mod salt {
    use super::*;
    use twilight_struggle::{Dice, OperationKind};

    fn load(state: &str) -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, &format!("events/{state}")).unwrap();
        (map, cards, Game::from_scenario(&scenario))
    }

    #[test]
    fn the_event_improves_defcon_by_two_and_starts_the_coup_penalty() {
        let (map, cards, mut game) = load("salt-negotiations");
        game.play_card(&cards, cards.id_by_name("SALT Negotiations").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 4);
        assert!(game.status().effects.salt);
    }

    #[test]
    fn defcon_stops_at_five() {
        let (map, cards, mut game) = load("salt-negotiations-near-top");
        game.play_card(&cards, cards.id_by_name("SALT Negotiations").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.status().defcon, 5);
    }

    #[test]
    fn every_coup_roll_gets_minus_one_for_both_sides() {
        let (map, cards, mut game) = load("salt-active");
        game.play_card(&cards, cards.id_by_name("Socialist Governments").unwrap()).unwrap();
        game.begin(OperationKind::Coup).unwrap();
        let honduras = map.id_by_name("Honduras").unwrap();
        game.roll(&map, honduras, &mut Dice::from_seed(1)).unwrap();
        let twilight_struggle::Event::Coup(result) = &game.log().entries().iter().rev().find(|e| matches!(e.event, twilight_struggle::Event::Coup(_))).unwrap().event else { unreachable!() };
        assert_eq!(result.modifier, -1);
        assert!(!game.status().effects.coup_forbidden(Superpower::Us) && game.status().effects.salt);
    }

    #[test]
    fn it_ends_with_the_turn() {
        let (_, _, mut game) = load("salt-active");
        for _ in 0..12 {
            game.pass().unwrap();
        }
        assert!(!game.status().effects.salt);
        assert_eq!(game.status().turn, 2);
    }
}

mod discard_pile {
    use super::*;
    use twilight_struggle::ops::Operation;

    fn start() -> (WorldMap, CardCatalog, Game) {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "events/salt-negotiations-discards").unwrap();
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name("SALT Negotiations").unwrap()).unwrap();
        (map, cards, game)
    }

    fn names(cards: &CardCatalog, ids: &[twilight_struggle::CardId]) -> Vec<String> {
        ids.iter().map(|&c| cards.card(c).name.clone()).collect()
    }

    #[test]
    fn salt_offers_the_non_scoring_discards_and_a_no_card_option() {
        let (map, cards, mut game) = start();
        game.play_event(&map, &cards).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!("the pick opens") };
        assert_eq!(names(&cards, e.pile()), ["Fidel", "Containment", "Truman Doctrine"]);
        let labels: Vec<&str> = e.modes().iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["take no card", "take Fidel", "take Containment", "take Truman Doctrine"]);
        assert_eq!(game.decider(), Superpower::Ussr);
        assert_eq!(game.status().defcon, 2, "nothing applies until it's confirmed");
    }

    #[test]
    fn taking_a_card_moves_it_to_the_hand_and_applies_the_fixed_effects() {
        let (map, cards, mut game) = start();
        game.play_event(&map, &cards).unwrap();
        game.move_event_cursor(2);
        game.choose_event_cursor(&map).unwrap();
        game.confirm().unwrap();
        let containment = cards.id_by_name("Containment").unwrap();
        assert!(game.hand(Superpower::Ussr).contains(&containment));
        assert!(!game.discards().contains(&containment));
        assert!(game.discards().contains(&cards.id_by_name("Fidel").unwrap()));
        assert_eq!(game.status().defcon, 4);
        assert!(game.status().effects.salt);
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn taking_nothing_still_applies_the_fixed_effects() {
        let (map, cards, mut game) = start();
        game.play_event(&map, &cards).unwrap();
        game.choose_event_cursor(&map).unwrap();
        game.confirm().unwrap();
        assert_eq!(game.hand(Superpower::Ussr).len(), 0);
        assert_eq!(game.status().defcon, 4);
        assert!(game.status().effects.salt);
    }

    #[test]
    fn the_cursor_wraps_and_the_player_can_back_out_before_choosing() {
        let (map, cards, mut game) = start();
        game.play_event(&map, &cards).unwrap();
        game.move_event_cursor(-1);
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        assert_eq!(e.cursor(), 3);
        game.abandon().unwrap();
        assert_eq!(game.status().defcon, 2);
    }

    #[test]
    fn the_modal_lists_the_pile_and_fits() {
        use twilight_struggle::render::render_event_session;
        use twilight_struggle::ColorMode;
        let (map, cards, mut game) = start();
        game.play_event(&map, &cards).unwrap();
        game.move_event_cursor(1);
        game.choose_event_cursor(&map).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!() };
        let text = render_event_session(&cards, e, game.status()).render(ColorMode::Never);
        assert!(text.contains("take Fidel") && text.contains("USSR takes Fidel (revealed)") && text.contains("DEFCON 2 → 4"), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= 69, "{line:?}");
        }
    }

    #[test]
    fn an_empty_pile_opens_the_same_pick_and_says_why_nothing_can_be_taken() {
        use twilight_struggle::render::render_event_session;
        use twilight_struggle::ColorMode;
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "events/salt-negotiations").unwrap();
        let mut game = Game::from_scenario(&scenario);
        game.play_card(&cards, cards.id_by_name("SALT Negotiations").unwrap()).unwrap();
        game.play_event(&map, &cards).unwrap();
        let Some(Operation::Event(e)) = game.operation() else { panic!("the pick opens even over an empty pile") };
        assert!(e.is_pile_pick() && e.pile().is_empty() && e.has_session_modal());
        let text = render_event_session(&cards, e, game.status()).render(ColorMode::Never);
        assert!(text.contains("discard pile is empty") && text.contains("DEFCON 2 → 4"), "{text}");
        assert_eq!(game.status().defcon, 2, "nothing applies until it's confirmed");
        game.confirm().unwrap();
        assert!(game.status().defcon == 4 && game.status().effects.salt);
        assert_eq!(game.active(), Superpower::Us);
    }

    #[test]
    fn the_ai_finishes_the_pick() {
        use twilight_struggle::{play_turn, Dice, RandomAi};
        for seed in 0..10 {
            let (map, cards, mut game) = start();
            game.play_event(&map, &cards).unwrap();
            play_turn(&mut RandomAi::from_seed(seed), &mut game, &map, &cards, &mut Dice::from_seed(seed)).unwrap();
            assert!(game.operation().is_none() && game.status().effects.salt, "seed {seed}");
        }
    }
}

mod discard_pile_reveal {
    use super::*;
    use twilight_struggle::{Dice, Event, RandomAi};

    #[test]
    fn the_logged_result_carries_the_taken_card_for_both_a_human_and_an_ai_pick() {
        let (map, cards, lib) = fixtures();
        let (scenario, _) = lib.load(&map, &cards, "events/salt-negotiations-discards").unwrap();
        for seed in 0..20 {
            let mut game = Game::from_scenario(&scenario);
            game.play_card(&cards, cards.id_by_name("SALT Negotiations").unwrap()).unwrap();
            game.play_event(&map, &cards).unwrap();
            twilight_struggle::play_turn(&mut RandomAi::from_seed(seed), &mut game, &map, &cards, &mut Dice::from_seed(seed)).unwrap();
            let takes: Vec<_> = game
                .log()
                .entries()
                .iter()
                .filter_map(|e| match &e.event {
                    Event::EventResolved { result, .. } => Some(result.takes.clone()),
                    _ => None,
                })
                .collect();
            // Whatever the AI took is recorded, and is in its hand.
            for &(side, card) in takes.iter().flatten() {
                assert!(game.hand(side).contains(&card), "seed {seed}");
            }
            let line = twilight_struggle::render::log_entry_line(&map, &cards, game.log().entries().last().unwrap());
            if takes.iter().any(|t| !t.is_empty()) {
                assert!(line.contains("from the discard pile"), "{line}");
            }
        }
    }
}
