//! Tests for `Game::legal_actions`/`Game::apply` (`src/action.rs`) and the
//! `RandomAi`/`play_turn` framework built on them (`src/ai/`).

use twilight_struggle::{play_turn, Action, CardCatalog, CardId, Dice, Game, OperationKind, RandomAi, Scenario, Superpower, WorldMap};

fn started_game() -> (WorldMap, CardCatalog, Game) {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let game = Game::from_scenario(&scenario);
    (map, cards, game)
}

/// Every card id 1..=110, looked up through [`CardCatalog::find`] (a
/// [`CardId`]'s inner field is crate-private, so this is the only way an
/// integration test can get hold of one without already knowing it from a
/// name).
fn all_card_ids(cards: &CardCatalog) -> Vec<CardId> {
    (1..=110u8)
        .map(|n| match cards.find(&n.to_string()) {
            twilight_struggle::CardFound::One(id) => id,
            other => panic!("card #{n} should resolve to exactly one id, got {other:?}"),
        })
        .collect()
}

/// Checks `game.legal_actions(..)` against what `Game`'s own methods
/// actually accept, called independently on throwaway [`Game::lookahead`]
/// clones — a brute-force differential check, not a restatement of
/// `legal_actions`'s own logic. A `Roll` probe uses a fresh
/// `Dice::from_seed(0)` rather than the caller's real dice: whether a roll
/// is accepted depends only on legality (checked before any die is drawn),
/// never on the die's value, so this can't affect — or be affected by —
/// the real game's own die sequence.
fn assert_legal_actions_are_exact(game: &Game, map: &WorldMap, cards: &CardCatalog, card_ids: &[CardId]) {
    let legal = game.legal_actions(map, cards);
    assert!(!legal.is_empty(), "legal_actions returned an empty list");

    for &id in card_ids {
        let expected = game.lookahead().play_card(cards, id).is_ok();
        let actual = legal.contains(&Action::PlayCard(id));
        assert_eq!(actual, expected, "PlayCard({id}) disagreement");
    }

    for kind in [OperationKind::Influence, OperationKind::Realign, OperationKind::Coup] {
        let expected = game.lookahead().begin(kind).is_ok();
        let actual = legal.contains(&Action::Begin(kind));
        assert_eq!(actual, expected, "Begin({kind:?}) disagreement");
    }

    let expected_space = game.lookahead().space(&mut Dice::from_seed(0)).is_ok();
    assert_eq!(legal.contains(&Action::Space), expected_space, "Space disagreement");

    let expected_event = game.lookahead().play_event(map, cards).is_ok();
    assert_eq!(legal.contains(&Action::Event), expected_event, "Event disagreement");

    for (id, _) in map.iter() {
        let expected_place = game.lookahead().place(map, id).is_ok();
        let actual_place = legal.contains(&Action::Place(id));
        assert_eq!(actual_place, expected_place, "Place({:?}) disagreement", map.country(id).name);

        let expected_roll = game.lookahead().roll(map, id, &mut Dice::from_seed(0)).is_ok();
        let actual_roll = legal.contains(&Action::Roll(id));
        assert_eq!(actual_roll, expected_roll, "Roll({:?}) disagreement", map.country(id).name);
    }

    let expected_confirm = game.lookahead().confirm().is_ok();
    assert_eq!(legal.contains(&Action::Confirm), expected_confirm, "Confirm disagreement");

    let expected_pass = game.lookahead().pass().is_ok();
    assert_eq!(legal.contains(&Action::Pass), expected_pass, "Pass disagreement");
}

#[test]
fn legal_actions_matches_game_methods_at_every_phase_of_a_turn() {
    let (map, cards, mut game) = started_game();
    let card_ids = all_card_ids(&cards);

    // No card in play, no operation: only PlayCard/Pass should be legal.
    assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);

    let play_id = game.hand(Superpower::Ussr)[0];
    game.play_card(&cards, play_id).unwrap();
    // Card in play, no operation: only Begin should be legal.
    assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);

    game.begin(OperationKind::Influence).unwrap();
    // Placement open, nothing placed yet.
    assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
    let poland = map.id_by_name("Poland").unwrap();
    game.place(&map, poland).unwrap();
    // Placement open, one point already placed (cost/ops bookkeeping now
    // live, not just presence).
    assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
    game.confirm().unwrap();

    let play_id = game.hand(game.active())[0];
    game.play_card(&cards, play_id).unwrap();
    game.begin(OperationKind::Realign).unwrap();
    // Realignment open, before any roll.
    assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
    let italy = map.id_by_name("Italy").unwrap();
    let mut dice = Dice::from_seed(1);
    if game.roll(&map, italy, &mut dice).is_ok() {
        // Realignment open, after one roll (ops_spent > 0 now).
        assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
    }
    game.confirm().unwrap();

    let play_id = game.hand(game.active())[0];
    game.play_card(&cards, play_id).unwrap();
    game.begin(OperationKind::Coup).unwrap();
    // Coup open, before its one attempt.
    assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
    let west_germany = map.id_by_name("West Germany").unwrap();
    if game.roll(&map, west_germany, &mut dice).is_ok() {
        // Coup open, after its attempt resolved.
        assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
    }
}

/// `Pass` is only ever offered with no card in play, and `Begin` only ever
/// with one in play and no operation open yet — the two states
/// `legal_actions`'s own doc says are mutually exclusive.
#[test]
fn pass_and_begin_are_mutually_exclusive_with_each_other() {
    let (map, cards, mut game) = started_game();

    let legal = game.legal_actions(&map, &cards);
    assert!(legal.contains(&Action::Pass));
    assert!(!legal.iter().any(|a| matches!(a, Action::Begin(_))));

    let play_id = game.hand(Superpower::Ussr)[0];
    game.play_card(&cards, play_id).unwrap();
    let legal = game.legal_actions(&map, &cards);
    assert!(!legal.contains(&Action::Pass));
    assert!(legal.iter().any(|a| matches!(a, Action::Begin(_))));
}

/// Across many seeds, a full random walk of alternating turns: every
/// action `legal_actions` lists at every step applies cleanly to a
/// lookahead clone (soundness), the list is never empty, and `play_turn`
/// always hands the turn to the other side (unless an event just ended
/// the game).
#[test]
fn random_play_stays_sound_and_always_ends_the_turn() {
    for seed in 0..15u64 {
        let (map, cards, mut game) = started_game();
        let card_ids = all_card_ids(&cards);
        let mut dice = Dice::from_seed(seed);
        let mut ai = RandomAi::from_seed(seed ^ 0x5151);

        for _ in 0..40 {
            let before = game.active();
            let legal = game.legal_actions(&map, &cards);
            assert!(!legal.is_empty(), "seed {seed}: legal_actions returned an empty list");
            for &action in &legal {
                let mut probe = game.lookahead();
                probe.apply(action, &map, &cards, &mut Dice::from_seed(0)).unwrap_or_else(|e| {
                    panic!("seed {seed}: listed action {action:?} was refused by Game::apply: {e}")
                });
            }

            // One `play_turn` call stops when the *decision* changes hands —
            // a choice event passes it to the card's own side mid-turn — so
            // keep going until the phasing side itself changes.
            for _ in 0..4 {
                play_turn(&mut ai, &mut game, &map, &cards, &mut dice).expect("play_turn should only apply actions legal_actions listed");
                if game.active() != before || game.winner().is_some() {
                    break;
                }
            }
            if game.winner().is_some() {
                // An event can end the game mid-turn (VP cap, DEFCON 1) —
                // `active` never changes then, and there's nothing left to play.
                break;
            }
            assert_ne!(game.active(), before, "seed {seed}: play_turn did not hand the turn over");
        }

        // The brute-force differential check too, at whatever state 40
        // alternating random turns happened to land on.
        // (A finished game lists nothing by design — covered by
        // `a_finished_game_lists_no_legal_actions`.)
        if game.winner().is_none() {
            assert_legal_actions_are_exact(&game, &map, &cards, &card_ids);
        }
    }
}

/// The same AI seed and the same dice seed must choose, and roll, exactly
/// the same way every time — an `Ai`'s own `Dice` never shares state with
/// the game's realignment/coup dice (see `RandomAi`'s own doc), so this
/// also guards against the two accidentally being wired together.
#[test]
fn same_seeds_produce_an_identical_log() {
    fn run(seed: u64) -> Vec<String> {
        let (map, cards, mut game) = started_game();
        let mut dice = Dice::from_seed(seed);
        let mut ai = RandomAi::from_seed(seed);
        for _ in 0..30 {
            play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
        }
        game.log().entries().iter().map(|e| format!("{e:?}")).collect()
    }

    assert_eq!(run(99), run(99));
}

/// A scenario where the USSR controls every Europe battleground plus the
/// UK — Europe Scoring's Control tier — with "Europe Scoring" in its
/// hand, matching `Game`'s own test fixture of the same name.
fn europe_control_scenario(map: &WorldMap, cards: &CardCatalog) -> Scenario {
    let json = r#"{
        "hands":{"us":["Duck and Cover","Five Year Plan"],"ussr":["Europe Scoring","Fidel"]},
        "influence":{
            "France":[0,10],"West Germany":[0,10],"East Germany":[0,10],
            "Poland":[0,10],"Italy":[0,10],"UK":[0,10]
        }
    }"#;
    Scenario::from_json(map, cards, json).unwrap()
}

#[test]
fn a_scoring_card_in_play_offers_only_event() {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let scenario = europe_control_scenario(&map, &cards);
    let mut game = Game::from_scenario(&scenario);
    let scoring = cards.id_by_name("Europe Scoring").unwrap();

    game.play_card(&cards, scoring).unwrap();
    let legal = game.legal_actions(&map, &cards);
    assert_eq!(legal, vec![Action::Event], "a scoring card has no ops, so Begin should not be offered");
}

#[test]
fn a_finished_game_lists_no_legal_actions() {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let scenario = europe_control_scenario(&map, &cards);
    let mut game = Game::from_scenario(&scenario);
    let scoring = cards.id_by_name("Europe Scoring").unwrap();

    game.play_card(&cards, scoring).unwrap();
    game.play_event(&map, &cards).unwrap();
    assert!(game.winner().is_some());
    assert!(game.legal_actions(&map, &cards).is_empty());
}

/// From mid-turn (the card already played), `Event` is the only legal
/// action, so `play_turn` must apply it regardless of which `RandomAi`
/// seed is driving — and then stop cleanly rather than looping on an
/// empty `legal_actions` once the win ends the game.
#[test]
fn a_random_walk_ends_cleanly_at_game_over() {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let scenario = europe_control_scenario(&map, &cards);
    let mut game = Game::from_scenario(&scenario);
    let scoring = cards.id_by_name("Europe Scoring").unwrap();
    game.play_card(&cards, scoring).unwrap();

    let mut dice = Dice::from_seed(7);
    let mut ai = RandomAi::from_seed(7);
    play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();

    assert!(game.winner().is_some());
    assert!(game.legal_actions(&map, &cards).is_empty());
}

/// A US player (phasing) plays the USSR's Comecon: the *USSR* makes the
/// choices, so an AI playing the USSR is asked to choose even though the
/// US is still the active side — and `play_turn` hands control back the
/// moment the event closes.
#[test]
fn the_ai_makes_the_choices_for_its_own_card_when_the_human_is_phasing() {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let lib = twilight_struggle::StateLibrary::standard();
    let (mut scenario, _) = lib.load(&map, &cards, "choices/comecon").unwrap();
    let comecon = cards.id_by_name("Comecon").unwrap();
    scenario.status.active = Superpower::Us;
    scenario.hands.push_to_hand(Superpower::Us, comecon);
    let mut game = Game::from_scenario(&scenario);
    game.play_card(&cards, comecon).unwrap();
    game.play_event(&map, &cards).unwrap();
    assert_eq!(game.active(), Superpower::Us);
    assert_eq!(game.decider(), Superpower::Ussr);

    // Only forward steps and (once complete) Confirm are ever offered.
    let legal = game.legal_actions(&map, &cards);
    assert!(legal.iter().all(|a| matches!(a, Action::Place(_))), "{legal:?}");
    assert!(!legal.contains(&Action::Confirm), "nothing picked yet");

    let mut ai = RandomAi::from_seed(3);
    let mut dice = Dice::from_seed(3);
    play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
    assert!(game.operation().is_none(), "the AI finished the event");
    // Comecon is the USSR's card, played by the US for its event: the US may now use its operations.
    assert!(game.ops_after_event().is_some(), "the US's operations follow the opponent's event");
    game.pass().unwrap();
    assert_eq!(game.active(), Superpower::Ussr, "the US's action round is over");
    let placed = ["East Germany", "Czechoslovakia", "Hungary", "Romania", "Bulgaria", "Yugoslavia", "Finland", "Austria"]
        .iter()
        .filter(|n| game.board().influence(map.id_by_name(n).unwrap(), Superpower::Ussr) > 0)
        .count();
    assert_eq!(placed, 4, "Comecon places exactly 4 (Poland is US-controlled)");
}

/// The other direction: the USSR is phasing and plays the US's Truman
/// Doctrine, so the *US* is the one offered the choices — `legal_actions`
/// lists steps for `Game::decider`, never for the phasing side.
#[test]
fn legal_actions_are_for_the_decider_not_the_phasing_side() {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let lib = twilight_struggle::StateLibrary::standard();
    let (mut scenario, _) = lib.load(&map, &cards, "choices/truman-doctrine").unwrap();
    let truman = cards.id_by_name("Truman Doctrine").unwrap();
    scenario.status.active = Superpower::Ussr;
    scenario.hands.push_to_hand(Superpower::Ussr, truman);
    let mut game = Game::from_scenario(&scenario);
    game.play_card(&cards, truman).unwrap();
    game.apply(Action::Event, &map, &cards, &mut Dice::from_seed(1)).unwrap();
    assert_eq!(game.decider(), Superpower::Us);
    assert_eq!(game.active(), Superpower::Ussr);

    let italy = map.id_by_name("Italy").unwrap();
    let legal = game.legal_actions(&map, &cards);
    assert_eq!(legal, vec![Action::Unplace(italy)], "Truman can only wipe uncontrolled Italy, and can't confirm until it has");

    // `play_turn` is told the same thing: it plays for the decider.
    let mut ai = RandomAi::from_seed(1);
    play_turn(&mut ai, &mut game, &map, &cards, &mut Dice::from_seed(1)).unwrap();
    assert_eq!(game.board().influence(italy, Superpower::Ussr), 0);
}
