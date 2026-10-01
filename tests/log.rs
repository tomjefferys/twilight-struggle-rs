use twilight_struggle::render::{log_text, render_log};
use twilight_struggle::{CardCatalog, ColorMode, Dice, Game, OperationKind, Scenario, Superpower, WorldMap};

fn started_game() -> (WorldMap, CardCatalog, Game) {
    let map = WorldMap::standard().unwrap();
    let cards = CardCatalog::standard().unwrap();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let game = Game::from_scenario(&scenario);
    (map, cards, game)
}

/// Plays `name` (a card in the demo scenario's deal) for the active side —
/// the `play_card` analogue of `started_game`'s own shortcut, since every
/// scripted action now needs a card in play before `begin` will open an
/// operation with it.
fn play(game: &mut Game, cards: &CardCatalog, name: &str) {
    let id = cards.id_by_name(name).unwrap_or_else(|| panic!("no card named {name:?}"));
    game.play_card(cards, id).unwrap();
}

/// Scripts one action round for each operation kind plus a pass and a
/// debug edit, then checks the resulting log text against an inline
/// expectation. Not a `tests/snapshots/` file: the documented
/// regeneration workflow (`CLAUDE.md`, `cargo run -- --color never
/// <command>`) needs a one-shot command that produces the output, and no
/// one-shot command can produce a game with history — a fresh `Game`
/// starts with an empty log every time. So this is deliberately the
/// honest form: an inline expectation, built the same way `Dice::from_seed(0)`
/// makes every other seeded test in this crate reproducible.
fn scripted_game() -> (WorldMap, CardCatalog, Game) {
    let (map, cards, mut game) = started_game();
    let mut dice = Dice::from_seed(0);

    let poland = map.id_by_name("Poland").unwrap();
    let italy = map.id_by_name("Italy").unwrap();
    let west_germany = map.id_by_name("West Germany").unwrap();
    let iran = map.id_by_name("Iran").unwrap();

    // USSR plays Socialist Governments (3 ops) and places two points in
    // Poland, where it already has presence.
    play(&mut game, &cards, "Socialist Governments");
    game.begin(OperationKind::Influence).unwrap();
    game.place(&map, poland).unwrap();
    game.place(&map, poland).unwrap();
    game.confirm().unwrap();

    // US plays Duck and Cover (3 ops) and realigns against Italy, where
    // USSR has influence to remove.
    play(&mut game, &cards, "Duck and Cover");
    game.begin(OperationKind::Realign).unwrap();
    game.roll(&map, italy, &mut dice).unwrap();
    game.confirm().unwrap();

    // USSR plays Fidel (2 ops) and coups West Germany, where the US has
    // influence to remove.
    play(&mut game, &cards, "Fidel");
    game.begin(OperationKind::Coup).unwrap();
    game.roll(&map, west_germany, &mut dice).unwrap();
    game.confirm().unwrap();

    // US passes — no card needed.
    game.pass().unwrap();

    // A debug edit, exactly as `main.rs`'s `set` command records one.
    let before = game.board().influence(iran, Superpower::Us);
    game.board_mut().set_influence(iran, Superpower::Us, 5);
    let after = game.board().influence(iran, Superpower::Us);
    game.record_edit(iran, Superpower::Us, before, after);

    (map, cards, game)
}

/// A closed placement should read the same way a closed realignment or
/// coup does: an "influence" line naming what was placed (the placement
/// analogue of a resolved roll), followed by its own `confirm`/`cancel`
/// line — never merged onto one line the way a bare `influence` summary
/// used to be.
#[test]
fn a_cancelled_placement_gets_an_influence_line_and_its_own_cancel_line() {
    use twilight_struggle::render::log_entry_line;

    let (map, cards, mut game) = started_game();
    let poland = map.id_by_name("Poland").unwrap();
    play(&mut game, &cards, "Socialist Governments");
    game.begin(OperationKind::Influence).unwrap();
    game.place(&map, poland).unwrap();
    game.cancel().unwrap();

    let entries = game.log().entries();
    assert_eq!(entries.len(), 2, "expected an influence line and a separate cancel line");

    let influence_line = log_entry_line(&map, &cards, &entries[0]);
    assert!(influence_line.contains("Poland +1"), "expected the placed points, got {influence_line:?}");
    assert!(!influence_line.contains("cancel"), "the influence line itself shouldn't say cancel: {influence_line:?}");

    let cancel_line = log_entry_line(&map, &cards, &entries[1]);
    assert!(cancel_line.contains("cancel"), "expected a cancel line, got {cancel_line:?}");
    assert!(cancel_line.contains("influence,"), "expected the kind named in the detail, got {cancel_line:?}");
    assert!(cancel_line.contains("Socialist Governments"), "expected the card named in the detail, got {cancel_line:?}");
}

#[test]
fn log_text_matches_the_expected_transcript() {
    let (map, cards, game) = scripted_game();
    let actual = log_text(&map, &cards, game.log());
    let expected = include_str!("snapshots/log.txt");
    assert_eq!(actual, expected.trim_end_matches('\n'));
}

/// The whole point of one format serving both the view and the export:
/// what `render_log` draws under `ColorMode::Never` must be byte-identical
/// to what `log_text` produces, for the full log and for a `tail`.
#[test]
fn render_log_under_color_never_matches_log_text_exactly() {
    let (map, cards, game) = scripted_game();
    assert_eq!(render_log(&map, &cards, game.log(), None).render(ColorMode::Never), log_text(&map, &cards, game.log()));

    let tail_text = {
        let header = "# twilight-struggle game log\n# turn ar   side action   detail";
        let entries: Vec<String> =
            game.log().tail(2).iter().map(|e| twilight_struggle::render::log_entry_line(&map, &cards, e)).collect();
        format!("{header}\n{}", entries.join("\n"))
    };
    assert_eq!(render_log(&map, &cards, game.log(), Some(2)).render(ColorMode::Never), tail_text);
}

#[test]
fn color_never_emits_no_escape_codes() {
    let (map, cards, game) = scripted_game();
    let rendered = render_log(&map, &cards, game.log(), None).render(ColorMode::Never);
    assert!(!rendered.contains('\x1b'));
}

#[test]
fn color_always_wraps_styled_text_in_sgr_codes() {
    let (map, cards, game) = scripted_game();
    let rendered = render_log(&map, &cards, game.log(), None).render(ColorMode::Always);
    assert!(rendered.contains('\x1b'));
}

#[test]
fn every_line_has_no_trailing_whitespace_and_fits_the_canvas_width() {
    let (map, cards, game) = scripted_game();
    let canvas = render_log(&map, &cards, game.log(), None);
    let rendered = canvas.render(ColorMode::Never);
    for line in rendered.split('\n') {
        assert_eq!(line, line.trim_end(), "line has trailing whitespace: {line:?}");
        assert!(line.chars().count() <= canvas.width(), "line exceeds canvas width: {line:?}");
    }
}

/// The exact case the old `4+4=8 vs 4` phrasing made ambiguous: a die, a
/// modifier/ops value, and a resulting sum that could all coincide or be
/// mistaken for one another. Every number in the rendered line must be
/// individually tagged with where it came from.
#[test]
fn realign_and_coup_lines_tag_every_number_with_its_source() {
    use twilight_struggle::render::log_entry_line;
    use twilight_struggle::{resolve, CoupResult, Event, GameLog, LogEntry, Modifiers, RollResult};

    let (map, cards, game) = started_game();
    let poland = map.id_by_name("Poland").unwrap();
    let board = game.board().clone();

    let acting_mods = Modifiers { adjacent_controlled: 2, more_influence: false, superpower_adjacent: false };
    let opposing_mods = Modifiers { adjacent_controlled: 0, more_influence: true, superpower_adjacent: false };
    let roll: RollResult = resolve(poland, Superpower::Us, 3, acting_mods, 5, opposing_mods, &board);

    let mut log = GameLog::new();
    log.push(LogEntry { turn: 1, action_round: 1, side: Some(Superpower::Us), event: Event::Realign(roll) });
    let realign_line = log_entry_line(&map, &cards, &log.entries()[0]);
    assert!(realign_line.contains("d6:3"), "die (3) should appear tagged d6: {realign_line:?}");
    assert!(realign_line.contains("mod:+2"), "acting modifier (+2) should appear tagged mod: {realign_line:?}");
    assert!(realign_line.contains("sum:5"), "acting sum (3+2) should appear tagged sum: {realign_line:?}");
    assert!(realign_line.contains("d6:5"), "opposing die (5) should appear tagged d6: {realign_line:?}");
    assert!(realign_line.contains("mod:+1"), "opposing modifier (+1) should appear tagged mod: {realign_line:?}");
    assert!(realign_line.contains("sum:6"), "opposing sum (5+1) should appear tagged sum: {realign_line:?}");

    let coup = CoupResult { target: poland, die: 4, ops: 2, target_number: 6, margin: 0, removed: 0, added: 0 };
    let mut coup_log = GameLog::new();
    coup_log.push(LogEntry { turn: 1, action_round: 1, side: Some(Superpower::Ussr), event: Event::Coup(coup) });
    let coup_line = log_entry_line(&map, &cards, &coup_log.entries()[0]);
    assert!(coup_line.contains("d6:4"), "die (4) should appear tagged d6: {coup_line:?}");
    assert!(coup_line.contains("ops:+2"), "ops (2) should appear tagged ops: {coup_line:?}");
    assert!(coup_line.contains("sum:6"), "sum (4+2) should appear tagged sum: {coup_line:?}");
    assert!(coup_line.contains("target:6"), "target number (6) should appear tagged target: {coup_line:?}");
}
