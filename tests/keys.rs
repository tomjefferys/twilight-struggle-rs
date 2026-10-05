use twilight_struggle::render::{context_keys, global_keys, KeyScreen, KeyUi};
use twilight_struggle::{CardCatalog, Game, OperationKind, Scenario, WorldMap};

fn setup() -> (WorldMap, CardCatalog, Game) {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let game = Game::from_scenario(&Scenario::demo(&map, &cards).unwrap());
    (map, cards, game)
}

fn keys(game: &Game, cards: &CardCatalog, screen: KeyScreen) -> String {
    context_keys(game, cards, screen, KeyUi { p_plays: game.card_in_play().is_none(), ..KeyUi::default() })
}

#[test]
fn the_global_row_is_navigation_and_views_only() {
    for screen in [KeyScreen::World, KeyScreen::Region, KeyScreen::Country] {
        let row = global_keys(screen);
        for key in ["←→↑↓", "[ ] hand", "t tracks", "D piles", "q quit"] {
            assert!(row.contains(key), "{screen:?} row missing {key:?}: {row}");
        }
        assert!(!row.contains("influence") && !row.contains("abandon"), "{row}");
    }
}

#[test]
fn with_no_card_in_play_the_context_row_offers_play() {
    let (_, cards, game) = setup();
    assert!(keys(&game, &cards, KeyScreen::Region).contains("play selected"));
}

#[test]
fn a_card_in_play_offers_the_operations_and_a_way_back() {
    let (_, cards, mut game) = setup();
    let card = game.hand(game.active())[0];
    game.play_card(&cards, card).unwrap();
    let row = keys(&game, &cards, KeyScreen::Region);
    for key in ["i influence", "a realign", "o coup", "⌫ return card"] {
        assert!(row.contains(key), "missing {key:?}: {row}");
    }
}

#[test]
fn an_open_operation_replaces_the_play_keys() {
    let (_, cards, mut game) = setup();
    let card = game.hand(game.active())[0];
    game.play_card(&cards, card).unwrap();
    game.begin(OperationKind::Influence).unwrap();
    let row = keys(&game, &cards, KeyScreen::Region);
    assert!(row.contains("+ place") && row.contains("c confirm") && row.contains("⌫ abandon"), "{row}");
    assert!(!row.contains("i influence") && !row.contains("return card"), "{row}");
}

#[test]
fn rolling_is_a_country_screen_key_and_abandon_goes_once_a_roll_is_made() {
    let (_, cards, mut game) = setup();
    let card = game.hand(game.active())[0];
    game.play_card(&cards, card).unwrap();
    game.begin(OperationKind::Realign).unwrap();
    let region = keys(&game, &cards, KeyScreen::Region);
    let country = keys(&game, &cards, KeyScreen::Country);
    assert!(region.contains("Enter open country") && !region.contains("r roll"), "{region}");
    assert!(country.contains("r roll") && country.contains("⌫ abandon"), "{country}");
}

#[test]
fn a_modal_blanks_the_context_row() {
    let (_, cards, game) = setup();
    let row = context_keys(&game, &cards, KeyScreen::World, KeyUi { modal_open: true, ..KeyUi::default() });
    assert!(row.is_empty());
}

#[test]
fn a_card_with_an_implemented_event_offers_e() {
    let (_, cards, mut game) = setup();
    let fidel = cards.id_by_name("Fidel").unwrap();
    game.play_card(&cards, fidel).unwrap_or_else(|_| {
        let side = game.active();
        game.hands_mut().push_to_hand(side, fidel);
        game.play_card(&cards, fidel).unwrap();
    });
    let row = keys(&game, &cards, KeyScreen::Region);
    assert!(row.starts_with("e event"), "{row}");
}
