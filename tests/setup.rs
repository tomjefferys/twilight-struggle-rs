//! New-game setup: the printed start, the shuffled and dealt Early War deck, and the opening
//! placement (USSR 6 in Eastern Europe, then US 7 in Western Europe) leading into the first
//! turn's headline phase.

use twilight_struggle::game::{GameError, Phase};
use twilight_struggle::{play_turn, CardCatalog, CardPhase, Dice, Game, RandomAi, Superpower, WorldMap, CHINA_CARD};

fn fixtures() -> (WorldMap, CardCatalog) {
    (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
}

fn started(seed: u64) -> (WorldMap, CardCatalog, Game, Dice) {
    let (map, cards) = fixtures();
    let mut dice = Dice::from_seed(seed);
    let game = Game::new_game(&map, &cards, &mut dice);
    (map, cards, game, dice)
}

fn place(game: &mut Game, map: &WorldMap, country: &str, n: u8) {
    let id = map.id_by_name(country).unwrap();
    for _ in 0..n {
        game.place(map, id).unwrap_or_else(|e| panic!("{country}: {e}"));
    }
}

#[test]
fn the_printed_start_is_on_the_board() {
    let (map, _, game, _) = started(1);
    let inf = |name: &str| {
        let id = map.id_by_name(name).unwrap();
        (game.board().influence(id, Superpower::Us), game.board().influence(id, Superpower::Ussr))
    };
    assert_eq!(inf("UK"), (5, 0));
    assert_eq!(inf("Australia"), (4, 0));
    assert_eq!(inf("Canada"), (2, 0));
    assert_eq!(inf("North Korea"), (0, 3));
    assert_eq!(inf("East Germany"), (0, 3));
    assert_eq!(inf("Finland"), (0, 1));
    assert_eq!(inf("Poland"), (0, 0), "Eastern Europe gets its influence in the opening placement");
    let status = game.status();
    assert_eq!((status.turn, status.defcon, status.vp), (1, 5, 0));
    assert_eq!((status.china_card, status.china_card_face_up), (Superpower::Ussr, true));
}

#[test]
fn the_early_war_cards_are_shuffled_and_eight_dealt_to_each_side() {
    let (_, cards, game, _) = started(1);
    let early: Vec<_> = cards.ids().filter(|&c| c != CHINA_CARD && cards.card(c).phase == CardPhase::Early).collect();
    assert_eq!((game.hand(Superpower::Us).len(), game.hand(Superpower::Ussr).len()), (8, 8));
    assert_eq!(game.hands().deck().len(), early.len() - 16);
    assert!(early.iter().all(|&c| game.hands().contains(c)), "every Early War card is in the game");
    assert!(cards.ids().filter(|&c| cards.card(c).phase != CardPhase::Early).all(|c| !game.hands().contains(c)), "Mid and Late War cards wait");
    assert!(!game.hands().contains(CHINA_CARD));
}

#[test]
fn the_same_seed_deals_the_same_game_and_another_does_not() {
    let (_, _, a, _) = started(7);
    let (_, _, b, _) = started(7);
    let (_, _, c, _) = started(8);
    assert_eq!(a.hand(Superpower::Ussr), b.hand(Superpower::Ussr));
    assert_ne!(a.hand(Superpower::Ussr), c.hand(Superpower::Ussr));
}

#[test]
fn the_opening_placement_runs_ussr_then_us_then_the_first_headline() {
    let (map, cards, mut game, mut dice) = started(1);
    assert_eq!(game.phase(), Phase::Setup);
    assert!(game.settlement_due() && game.operation().is_none());
    assert!(matches!(game.play_card(&cards, game.hand(Superpower::Ussr)[0]), Err(GameError::Trap(_))), "no card can be played before the setup is done");
    assert!(matches!(game.pass(), Err(GameError::Trap(_))));

    game.settle(&map, &cards, &mut dice);
    assert_eq!((game.decider(), game.active()), (Superpower::Ussr, Superpower::Ussr));
    // Only Eastern Europe is open to the USSR, and every one of the 6 points must be placed.
    assert!(game.place(&map, map.id_by_name("France").unwrap()).is_err());
    place(&mut game, &map, "Poland", 4);
    assert!(matches!(game.confirm(), Err(GameError::EventIncomplete { .. })), "6 points to place, only 4 so far");
    place(&mut game, &map, "Hungary", 2);
    game.confirm().unwrap();
    assert_eq!(game.board().influence(map.id_by_name("Poland").unwrap(), Superpower::Ussr), 4);

    assert_eq!(game.phase(), Phase::Setup, "the US still has to place");
    game.settle(&map, &cards, &mut dice);
    assert_eq!((game.decider(), game.active()), (Superpower::Us, Superpower::Us));
    assert!(game.place(&map, map.id_by_name("Poland").unwrap()).is_err(), "the US places in Western Europe");
    place(&mut game, &map, "West Germany", 3);
    place(&mut game, &map, "Italy", 4);
    game.confirm().unwrap();

    assert_eq!((game.phase(), game.status().action_round, game.active()), (Phase::Headline, 0, Superpower::Ussr));
    assert!(game.picking_headline());
    assert_eq!(game.hand(Superpower::Ussr).len(), 8, "the setup spends no cards");
}

#[test]
fn the_setup_can_not_be_backed_out_of() {
    let (map, cards, mut game, mut dice) = started(1);
    game.settle(&map, &cards, &mut dice);
    assert!(game.abandon().is_err());
}

#[test]
fn the_log_records_each_placement_under_the_setup_title() {
    let (map, cards, mut game, mut dice) = started(1);
    game.settle(&map, &cards, &mut dice);
    place(&mut game, &map, "Poland", 6);
    game.confirm().unwrap();
    let text = twilight_struggle::render::log_text(&map, &cards, game.log());
    assert!(text.contains("Setup: Poland USSR 0→6"), "{text}");
}

#[test]
fn two_ais_play_a_new_game_through_setup_and_into_the_turns() {
    for seed in 0..10 {
        let (map, cards, mut game, mut dice) = started(seed);
        let mut ai = RandomAi::from_seed(seed);
        for _ in 0..80 {
            if game.winner().is_some() {
                break;
            }
            play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
        assert!(game.status().turn >= 2 || game.winner().is_some(), "seed {seed}: stuck on turn {}", game.status().turn);
    }
}
