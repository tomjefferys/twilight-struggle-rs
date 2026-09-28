use twilight_struggle::render::render_world_map;
use twilight_struggle::{Board, ColorMode, InfluencePlacement, MapLayout, Operation, Realignment, Region, Scenario, Superpower, WorldMap};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().unwrap();
    let layout = MapLayout::standard(&map).unwrap();
    (map, layout)
}

#[test]
fn world_map_matches_snapshot() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board, None, None);
    let expected = include_str!("snapshots/worldmap.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn background_shading_appears_in_the_rendered_map() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None, None);
    let text = canvas.render(ColorMode::Never);
    for shade in ['▒', '▓'] {
        assert!(
            text.contains(shade),
            "expected landmass shading character {shade:?} in the rendered map"
        );
    }
}

#[test]
fn renders_without_panicking_on_an_empty_board() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None, None);
    assert!(canvas.height() > 0);
    assert!(canvas.width() > 0);
}

#[test]
fn both_superpower_boxes_are_labelled() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None, None);
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("USA"), "USA box label missing:\n{text}");
    assert!(text.contains("USSR"), "USSR box label missing:\n{text}");
}

#[test]
fn superpower_legend_lists_real_borders() {
    // The precise complement to the approximate boxes: still real data,
    // computed from Country::adjacent_superpowers, not hand-authored.
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None, None);
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("USA: "));
    assert!(text.contains("USSR: "));
    assert!(text.contains("Canada"));
    assert!(text.contains("Finland"));
}

#[test]
fn every_country_has_a_distinct_code_chip_on_the_map() {
    // Every code should appear in the rendered text exactly once (chips
    // don't overlap each other or a superpower box) — a coarse but real
    // check that the hand-placed layout is collision-free end to end,
    // through the renderer rather than just the loader's own bookkeeping.
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None, None);
    let text = canvas.render(ColorMode::Never);
    for (id, country) in map.iter() {
        let code = layout.code(id);
        assert!(text.contains(code), "{}'s code {code:?} missing from the map", country.name);
    }
}

#[test]
fn superpower_box_cell_is_never_a_country_world_cell() {
    let (map, layout) = standard();
    for sp in [Superpower::Us, Superpower::Ussr] {
        let b = layout.superpower_box(sp);
        for (id, country) in map.iter() {
            let cell = layout.world_cell(id);
            let inside = cell.row >= b.cell.row
                && cell.row < b.cell.row + b.rows
                && cell.col >= b.cell.col
                && cell.col < b.cell.col + b.cols;
            assert!(!inside, "{}'s world cell falls inside {sp}'s box", country.name);
        }
    }
}

#[test]
fn color_never_emits_no_escape_codes() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board, None, None);
    assert!(!canvas.render(ColorMode::Never).contains('\x1b'));
}

#[test]
fn background_land_is_tinted_by_region() {
    // Each of the six regions gets its own board colour, so the tinted
    // landmass should carry at least two distinct region SGR codes —
    // guards against the tint collapsing to one flat colour.
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None, None);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains("38;5;140"), "expected Europe's purple tint");
    assert!(text.contains("38;5;208"), "expected Asia's orange tint");
}

#[test]
fn color_always_wraps_styled_text_in_sgr_codes() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board, None, None);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains('\x1b'));
    assert!(text.contains("\x1b[0m"));
}

#[test]
fn no_selection_reproduces_the_plain_view_exactly() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let plain = render_world_map(&map, &layout, &scenario.board, None, None);
    let expected = include_str!("snapshots/worldmap.txt");
    assert_eq!(plain.render(ColorMode::Never), expected.trim_end_matches('\n'));
    assert_eq!(plain.height(), expected.trim_end_matches('\n').lines().count());
}

#[test]
fn a_selection_adds_the_region_title_and_key_hints() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let plain = render_world_map(&map, &layout, &board, None, None);
    let selected = render_world_map(&map, &layout, &board, Some(Region::Europe), None);
    let text = selected.render(ColorMode::Never);
    assert!(text.contains("EUROPE"), "region name missing:\n{text}");
    assert!(text.contains("Enter open"), "key hints missing:\n{text}");
    assert_eq!(selected.height(), plain.height() + 2, "selection should add exactly two rows");
}

#[test]
fn a_pending_chip_is_marked_with_a_plus() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();

    let op = Operation::Influence(placement);
    let canvas = render_world_map(&map, &layout, &board, None, Some(&op));
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("+Pol"), "Poland's chip should show a '+' once it has pending influence:\n{text}");
    // Poland isn't a battleground, so its plain chip has a blank flag
    // slot; confirm the '+' actually replaced that, not just appeared
    // somewhere else on the map.
    let plain = render_world_map(&map, &layout, &board, None, None).render(ColorMode::Never);
    assert!(!plain.contains("+Pol"), "sanity check: the plain view shouldn't already have a '+' before Pol");
}

#[test]
fn a_pending_chip_keeps_its_control_colour() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    // Give the USSR outright control of Poland before staging more.
    let stability = map.country(poland).stability;
    board.set_influence(poland, Superpower::Ussr, stability);
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();

    let op = Operation::Influence(placement);
    let canvas = render_world_map(&map, &layout, &board, None, Some(&op));
    let text = canvas.render(ColorMode::Always);
    let poland_line = text.lines().find(|l| l.contains("Pol")).expect("no line with Poland's chip");
    assert!(poland_line.contains("\x1b[1;91m"), "a pending, USSR-controlled chip should stay bold red: {poland_line:?}");
}

#[test]
fn no_placement_reproduces_the_plain_view_exactly_even_with_the_new_param() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board, None, None);
    let expected = include_str!("snapshots/worldmap.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn the_placement_footer_is_not_clipped() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let east_germany = map.id_by_name("East Germany").unwrap();
    // Give the USSR base presence in both up front — legality is checked
    // against the board as it stood when the action started, not against
    // influence placed earlier in this same action.
    board.set_influence(poland, Superpower::Ussr, 1);
    board.set_influence(east_germany, Superpower::Ussr, 1);
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 8, &board);
    placement.place(&map, poland).unwrap();
    placement.place(&map, east_germany).unwrap();

    let op = Operation::Influence(placement);
    let plain = render_world_map(&map, &layout, &board, None, None);
    let with_placement = render_world_map(&map, &layout, &board, Some(Region::Europe), Some(&op));
    assert_eq!(with_placement.height(), plain.height() + 3, "a region selection plus a placement should add exactly three rows");

    let text = with_placement.render(ColorMode::Never);
    for line in text.lines() {
        assert!(line.chars().count() <= with_placement.width(), "world map placement view exceeded its own width: {line:?}");
    }
    assert!(text.contains("USSR placing"), "the full balance line should not be clipped:\n{text}");
    assert!(text.contains("u undo"), "the placement hint should not be clipped:\n{text}");
}

#[test]
fn a_selected_region_is_bold_where_an_unselected_one_is_dim() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, Some(Region::Europe), None);
    let text = canvas.render(ColorMode::Always);
    assert!(
        text.contains("\x1b[1;38;5;140m"),
        "expected Europe's tint bolded while selected:\n{text}"
    );
    assert!(
        text.contains("\x1b[2;38;5;208m"),
        "expected Asia's tint still dimmed while unselected:\n{text}"
    );
    assert!(
        !text.contains("\x1b[1;38;5;208m"),
        "Asia should not be bolded when Europe is selected:\n{text}"
    );
}

#[test]
fn a_realigned_chip_is_marked() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    board.set_influence(poland, Superpower::Us, 3);

    // The realignment's `base` is captured here, before the board is
    // mutated below — exactly as a real win would leave it.
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    board.set_influence(poland, Superpower::Us, 1); // as if a roll just removed 2

    let canvas = render_world_map(&map, &layout, &board, None, Some(&op));
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("!Pol"), "Poland's chip should show a '!' once a realignment has changed it:\n{text}");
    let plain = render_world_map(&map, &layout, &board, None, None).render(ColorMode::Never);
    assert!(!plain.contains("!Pol"), "sanity check: the plain view shouldn't already have a '!' before Pol");
}

#[test]
fn the_realignment_footer_is_not_clipped() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    board.set_influence(poland, Superpower::Us, 3);

    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 8, &board));
    board.set_influence(poland, Superpower::Us, 1);

    let plain = render_world_map(&map, &layout, &board, None, None);
    let with_realign = render_world_map(&map, &layout, &board, Some(Region::Europe), Some(&op));
    assert_eq!(with_realign.height(), plain.height() + 3, "a region selection plus a realignment should add exactly three rows");

    let text = with_realign.render(ColorMode::Never);
    for line in text.lines() {
        assert!(line.chars().count() <= with_realign.width(), "world map realignment view exceeded its own width: {line:?}");
    }
    assert!(text.contains("USSR realigning"), "the full balance line should not be clipped:\n{text}");
    assert!(text.contains("r roll"), "the realign hint should not be clipped:\n{text}");
    assert!(!text.contains("u undo"), "the world map hint shouldn't offer undo during a realignment:\n{text}");
}
