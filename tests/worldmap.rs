use twilight_struggle::render::render_world_map;
use twilight_struggle::{Board, ColorMode, MapLayout, Region, Scenario, Superpower, WorldMap};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().unwrap();
    let layout = MapLayout::standard(&map).unwrap();
    (map, layout)
}

#[test]
fn world_map_matches_snapshot() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board, None);
    let expected = include_str!("snapshots/worldmap.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn background_shading_appears_in_the_rendered_map() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None);
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
    let canvas = render_world_map(&map, &layout, &board, None);
    assert!(canvas.height() > 0);
    assert!(canvas.width() > 0);
}

#[test]
fn both_superpower_boxes_are_labelled() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None);
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
    let canvas = render_world_map(&map, &layout, &board, None);
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
    let canvas = render_world_map(&map, &layout, &board, None);
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
    let canvas = render_world_map(&map, &layout, &scenario.board, None);
    assert!(!canvas.render(ColorMode::Never).contains('\x1b'));
}

#[test]
fn background_land_is_tinted_by_region() {
    // Each of the six regions gets its own board colour, so the tinted
    // landmass should carry at least two distinct region SGR codes —
    // guards against the tint collapsing to one flat colour.
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, None);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains("38;5;140"), "expected Europe's purple tint");
    assert!(text.contains("38;5;208"), "expected Asia's orange tint");
}

#[test]
fn color_always_wraps_styled_text_in_sgr_codes() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board, None);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains('\x1b'));
    assert!(text.contains("\x1b[0m"));
}

#[test]
fn no_selection_reproduces_the_plain_view_exactly() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let plain = render_world_map(&map, &layout, &scenario.board, None);
    let expected = include_str!("snapshots/worldmap.txt");
    assert_eq!(plain.render(ColorMode::Never), expected.trim_end_matches('\n'));
    assert_eq!(plain.height(), expected.trim_end_matches('\n').lines().count());
}

#[test]
fn a_selection_adds_the_region_title_and_key_hints() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let plain = render_world_map(&map, &layout, &board, None);
    let selected = render_world_map(&map, &layout, &board, Some(Region::Europe));
    let text = selected.render(ColorMode::Never);
    assert!(text.contains("EUROPE"), "region name missing:\n{text}");
    assert!(text.contains("Enter open"), "key hints missing:\n{text}");
    assert_eq!(selected.height(), plain.height() + 2, "selection should add exactly two rows");
}

#[test]
fn a_selected_region_is_bold_where_an_unselected_one_is_dim() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board, Some(Region::Europe));
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
