//! `Game::determinize`: the world an AI searches in keeps what it knows and reshuffles what it can't.

use twilight_struggle::{CardCatalog, Dice, Game, Superpower, WorldMap};

fn new_game(seed: u64) -> (WorldMap, CardCatalog, Game) {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let game = Game::new_game(&map, &cards, &mut Dice::from_seed(seed));
    (map, cards, game)
}

fn sorted(mut v: Vec<twilight_struggle::CardId>) -> Vec<twilight_struggle::CardId> {
    v.sort_by_key(|c| format!("{c:?}"));
    v
}

#[test]
fn the_viewers_knowledge_is_kept_and_the_hidden_cards_are_reshuffled() {
    let (_, _, game) = new_game(7);
    let guess = game.determinize(Superpower::Us, &mut Dice::from_seed(1));

    // What the US knows is untouched.
    assert_eq!(guess.hand(Superpower::Us), game.hand(Superpower::Us));
    assert_eq!(guess.discards(), game.discards());
    assert_eq!(guess.removed_from_game(), game.removed_from_game());
    // The USSR still holds as many cards, and the unseen cards are the same set.
    assert_eq!(guess.hand(Superpower::Ussr).len(), game.hand(Superpower::Ussr).len());
    let unseen = |g: &Game| {
        let mut all = g.hand(Superpower::Ussr).to_vec();
        all.extend_from_slice(g.hands().deck());
        sorted(all)
    };
    assert_eq!(unseen(&guess), unseen(&game));
    assert_eq!(guess.hands().deck().len(), game.hands().deck().len());
    // ...but dealt differently.
    assert_ne!(guess.hand(Superpower::Ussr), game.hand(Superpower::Ussr));
}

#[test]
fn a_determinized_world_plays_the_same_board() {
    let (_, _, game) = new_game(3);
    let guess = game.determinize(Superpower::Ussr, &mut Dice::from_seed(9));
    assert_eq!(guess.status().turn, game.status().turn);
    assert_eq!(guess.hand(Superpower::Ussr), game.hand(Superpower::Ussr));
}
