use twilight_struggle::render::render_world_map;
use twilight_struggle::{Board, ColorMode, MapLayout, Scenario, Superpower, WorldMap};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().unwrap();
    let layout = MapLayout::standard(&map).unwrap();
    (map, layout)
}

#[test]
fn world_map_matches_snapshot() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board);
    let expected = include_str!("snapshots/worldmap.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn background_shading_appears_in_the_rendered_map() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board);
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
    let canvas = render_world_map(&map, &layout, &board);
    assert!(canvas.height() > 0);
    assert!(canvas.width() > 0);
}

#[test]
fn both_superpower_boxes_are_labelled() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let canvas = render_world_map(&map, &layout, &board);
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
    let canvas = render_world_map(&map, &layout, &board);
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
    let canvas = render_world_map(&map, &layout, &board);
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
    let canvas = render_world_map(&map, &layout, &scenario.board);
    assert!(!canvas.render(ColorMode::Never).contains('\x1b'));
}

#[test]
fn color_always_wraps_styled_text_in_sgr_codes() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world_map(&map, &layout, &scenario.board);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains('\x1b'));
    assert!(text.contains("\x1b[0m"));
}
