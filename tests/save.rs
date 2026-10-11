//! Saving and restoring: a game captured at a resting point comes back identical, carries on
//! identically, and the store survives bad files.

use twilight_struggle::game::Phase;
use twilight_struggle::{
    play_turn, AiKind, CardCatalog, Dice, Game, RandomAi, SaveError, SaveGame, SaveStore, Settings, Superpower, WorldMap,
};

fn fixtures() -> (WorldMap, CardCatalog) {
    (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
}

fn settings() -> Settings {
    Settings { ai_side: Some(Superpower::Us), ai_kind: AiKind::Random, ai_fast: true, debug: false }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ts-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Captures, pushes through JSON text and restores — the same trip a file makes.
fn round_trip(map: &WorldMap, cards: &CardCatalog, game: &Game, dice: &Dice) -> (Game, Dice) {
    let save = SaveGame::capture(map, cards, game, dice, settings()).expect("a resting point");
    let text = serde_json::to_string(&save).unwrap();
    let back: SaveGame = serde_json::from_str(&text).unwrap();
    let restored = back.restore(map, cards).unwrap();
    assert_eq!(restored.settings, settings());
    (restored.game, restored.dice)
}

fn assert_same(a: &Game, b: &Game, ctx: &str) {
    assert_eq!(a.status(), b.status(), "{ctx}: status");
    assert_eq!(a.board(), b.board(), "{ctx}: board");
    assert_eq!(a.hands().deck(), b.hands().deck(), "{ctx}: deck order");
    assert_eq!(a.hands().discards(), b.hands().discards(), "{ctx}: discards");
    assert_eq!(a.hands().removed(), b.hands().removed(), "{ctx}: removed");
    for side in [Superpower::Us, Superpower::Ussr] {
        assert_eq!(a.hands().hand(side), b.hands().hand(side), "{ctx}: {side} hand");
    }
    assert_eq!(a.log().entries(), b.log().entries(), "{ctx}: log");
    assert_eq!(a.winner(), b.winner(), "{ctx}: winner");
    assert_eq!(a.phase(), b.phase(), "{ctx}: phase");
    assert_eq!(a.decider(), b.decider(), "{ctx}: decider");
}

#[test]
fn a_restored_game_matches_and_then_plays_on_identically() {
    let (map, cards) = fixtures();
    let mut checked = 0;
    for seed in 0..6u64 {
        let mut dice = Dice::from_seed(seed);
        let mut game = Game::new_game(&map, &cards, &mut dice);
        let mut ai = RandomAi::from_seed(seed ^ 0x5a5a).careful();
        for call in 0..400 {
            if game.winner().is_some() {
                break;
            }
            play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
            if call % 7 != 0 || game.checkpoint().is_none() {
                continue;
            }
            let ctx = format!("seed {seed} call {call}");
            let (mut copy, mut copy_dice) = round_trip(&map, &cards, &game, &dice);
            assert_same(&game, &copy, &ctx);
            assert_eq!(game.legal_actions(&map, &cards), copy.legal_actions(&map, &cards), "{ctx}: legal actions");
            // Play both on from here with identical AIs: they must not diverge.
            let mut live = game.clone();
            let mut live_dice = Dice::from_seed(dice.state());
            let (mut a1, mut a2) = (RandomAi::from_seed(99).careful(), RandomAi::from_seed(99).careful());
            for _ in 0..12 {
                if live.winner().is_some() {
                    break;
                }
                play_turn(&mut a1, &mut live, &map, &cards, &mut live_dice).unwrap();
                play_turn(&mut a2, &mut copy, &map, &cards, &mut copy_dice).unwrap();
            }
            assert_same(&live, &copy, &format!("{ctx} after playing on"));
            checked += 1;
        }
    }
    assert!(checked > 20, "only {checked} resting points were checked");
}

#[test]
fn only_a_resting_point_can_be_captured() {
    let (map, cards) = fixtures();
    let mut dice = Dice::from_seed(1);
    let mut game = Game::new_game(&map, &cards, &mut dice);
    // The opening placement is an open event, and settling it opens the headline phase.
    assert!(game.checkpoint().is_none() || game.phase() != Phase::Setup);
    let mut ai = RandomAi::from_seed(1).careful();
    let mut saw_unsaved = false;
    for _ in 0..200 {
        if game.winner().is_some() {
            break;
        }
        let legal = game.legal_actions(&map, &cards);
        if legal.is_empty() {
            break;
        }
        use twilight_struggle::Ai;
        let action = ai.choose(&game, &map, &cards, &legal);
        game.apply(action, &map, &cards, &mut dice).unwrap();
        if game.card_in_play().is_some() || game.operation().is_some() {
            assert!(game.checkpoint().is_none(), "a card in play is not a resting point");
            saw_unsaved = true;
        }
        game.settle(&map, &cards, &mut dice);
    }
    assert!(saw_unsaved);
}

#[test]
fn the_store_writes_atomically_lists_and_rejects_bad_files() {
    let (map, cards) = fixtures();
    let dir = temp_dir("store");
    let store = SaveStore::new(&dir);
    assert!(!store.has_autosave());
    assert!(matches!(store.load_autosave(), Err(SaveError::NotFound(_))));

    let mut dice = Dice::from_seed(3);
    let mut game = Game::new_game(&map, &cards, &mut dice);
    let mut ai = RandomAi::from_seed(3).careful();
    while game.checkpoint().is_none() {
        play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
    }
    let save = SaveGame::capture(&map, &cards, &game, &dice, settings()).unwrap();
    store.autosave(&save).unwrap();
    store.save("my-game", &save).unwrap();
    assert!(store.has_autosave());
    assert_eq!(store.list().iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["my-game"]);
    store.load("my-game").unwrap().restore(&map, &cards).unwrap();
    assert!(!std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));

    assert!(matches!(store.save("../evil", &save), Err(SaveError::BadName(_))));
    assert!(matches!(store.load("nope"), Err(SaveError::NotFound(_))));

    std::fs::write(dir.join("autosave.json"), "{ not json").unwrap();
    assert!(matches!(store.load_autosave(), Err(SaveError::Json(_))));

    let mut value = serde_json::to_value(&save).unwrap();
    value["version"] = 999.into();
    std::fs::write(dir.join("autosave.json"), value.to_string()).unwrap();
    let old: SaveGame = store.load_autosave().unwrap();
    assert!(matches!(old.restore(&map, &cards), Err(SaveError::Version(999))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_autosaver_writes_once_per_resting_point() {
    let (map, cards) = fixtures();
    let dir = temp_dir("auto");
    let mut auto = twilight_struggle::Autosaver::new(SaveStore::new(&dir));
    let mut dice = Dice::from_seed(4);
    let mut game = Game::new_game(&map, &cards, &mut dice);
    let mut ai = RandomAi::from_seed(4).careful();
    assert!(!auto.tick(&map, &cards, &game, &dice, settings()).unwrap(), "setup is not a resting point");
    while game.checkpoint().is_none() {
        play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
    }
    assert!(auto.tick(&map, &cards, &game, &dice, settings()).unwrap());
    assert!(!auto.tick(&map, &cards, &game, &dice, settings()).unwrap(), "unchanged: no rewrite");
    play_turn(&mut ai, &mut game, &map, &cards, &mut dice).unwrap();
    if game.checkpoint().is_some() {
        assert!(auto.tick(&map, &cards, &game, &dice, settings()).unwrap());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
