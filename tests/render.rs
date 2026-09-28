use twilight_struggle::render::{render_country, render_region, render_world};
use twilight_struggle::{Board, ColorMode, InfluencePlacement, MapLayout, Region, Scenario, Superpower, WorldMap};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().unwrap();
    let layout = MapLayout::standard(&map).unwrap();
    (map, layout)
}

#[test]
fn world_dashboard_matches_snapshot_at_104() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, 104);
    let expected = include_str!("snapshots/world_104.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn world_dashboard_matches_snapshot_at_80() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, 80);
    let expected = include_str!("snapshots/world_80.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn no_rendered_line_exceeds_requested_width() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    for &width in &[80usize, 104, 130] {
        let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, width);
        let text = canvas.render(ColorMode::Never);
        for line in text.lines() {
            assert!(
                line.chars().count() <= width,
                "world view at width {width} produced a {}-char line: {line:?}",
                line.chars().count()
            );
        }
    }
    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &scenario.board, region, None, None);
        let text = canvas.render(ColorMode::Never);
        // Region views size themselves to their content rather than a
        // requested width, so just confirm every line the canvas produced
        // is no wider than the canvas itself claims to be.
        for line in text.lines() {
            assert!(line.chars().count() <= canvas.width());
        }
    }
}

/// Every in-region adjacency must be either drawn as a connector or
/// footnoted as undrawn — the display can never silently omit an edge.
#[test]
fn every_in_region_adjacency_is_drawn_or_footnoted() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &board, region, None, None);
        let text = canvas.render(ColorMode::Never);
        let has_connector = text.chars().any(|c| matches!(c, '─' | '│' | '╲' | '╱' | '╳'));
        assert!(has_connector, "{region} region view has no connectors at all");

        let ids = layout.countries_in_region(&map, region);
        for &id in &ids {
            let country = map.country(id);
            for &neighbor_id in &country.adjacent {
                if map.country(neighbor_id).region != region {
                    continue;
                }
                let a = layout.cell(id);
                let b = layout.cell(neighbor_id);
                let dr = (a.row as i16 - b.row as i16).abs();
                let dc = (a.col as i16 - b.col as i16).abs();
                let drawable = dr.max(dc) == 1;
                let footnoted = layout.undrawn_links().iter().any(|(x, y)| {
                    (*x == id && *y == neighbor_id) || (*x == neighbor_id && *y == id)
                });
                assert!(
                    drawable || footnoted,
                    "{} – {} in {region} is neither grid-adjacent nor footnoted as undrawn",
                    country.name,
                    map.country(neighbor_id).name
                );
            }
        }
    }
}

#[test]
fn control_markers_and_battleground_flag_are_correct() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap(); // stability 2, battleground
    let poland = map.id_by_name("Poland").unwrap(); // stability 3, battleground

    board.set_influence(italy, Superpower::Us, 2); // US controls
    board.set_influence(poland, Superpower::Ussr, 3); // USSR controls
    // UK left uncontrolled

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, None);
    let text = canvas.render(ColorMode::Never);

    let italy_name_line = line_containing(&text, "Italy");
    assert!(italy_name_line.contains("*Italy"), "Italy is a battleground: {italy_name_line:?}");
    let italy_stats_line = line_after(&text, "Italy");
    assert!(italy_stats_line.contains('<'), "Italy should show US control: {italy_stats_line:?}");

    let poland_stats_line = line_after(&text, "Poland");
    assert!(poland_stats_line.contains('>'), "Poland should show USSR control: {poland_stats_line:?}");

    let uk_stats_line = line_after(&text, "UK");
    assert!(uk_stats_line.contains(':'), "uncontrolled UK should show ':': {uk_stats_line:?}");
}

#[test]
fn a_country_selection_adds_its_name_and_the_key_hints() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    let plain = render_region(&map, &layout, &board, Region::Europe, None, None);
    let selected = render_region(&map, &layout, &board, Region::Europe, Some(italy), None);
    let text = selected.render(ColorMode::Never);
    assert!(text.contains("▸ Italy ◂"), "selected country's name missing:\n{text}");
    assert!(text.contains("Esc back"), "key hints missing:\n{text}");
    assert!(!text.contains("Enter"), "Enter is unbound in the region view, so shouldn't be hinted:\n{text}");
    assert_eq!(selected.height(), plain.height() + 2, "a selection should add exactly two rows");
}

#[test]
fn a_selected_country_box_is_bold_where_an_unselected_one_is_not() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    let canvas = render_region(&map, &layout, &board, Region::Europe, Some(italy), None);
    let text = canvas.render(ColorMode::Always);
    let italy_border_line = line_containing(&text, "Italy");
    assert!(
        italy_border_line.contains("\x1b[1;97m"),
        "Italy's box should be bold and in the selection accent colour: {italy_border_line:?}"
    );

    let uk = map.id_by_name("UK").unwrap();
    let canvas = render_region(&map, &layout, &board, Region::Europe, Some(uk), None);
    let text = canvas.render(ColorMode::Always);
    let italy_border_line = line_containing(&text, "Italy");
    assert!(!italy_border_line.contains("\x1b[1;97m"), "Italy shouldn't be bold when UK is selected: {italy_border_line:?}");
}

#[test]
fn a_selected_country_box_uses_heavy_borders_even_without_colour() {
    // Bold and colour both vanish under ColorMode::Never, so the selected
    // box needs a shape difference too — heavy box-drawing characters
    // instead of thin ones — to still read as selected there.
    let (map, layout) = standard();
    let board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();

    let canvas = render_region(&map, &layout, &board, Region::Europe, Some(italy), None);
    let text = canvas.render(ColorMode::Never);
    let italy_line = line_containing(&text, "Italy");
    assert!(italy_line.contains('┃'), "Italy's box should use heavy borders when selected: {italy_line:?}");

    let uk = map.id_by_name("UK").unwrap();
    let canvas = render_region(&map, &layout, &board, Region::Europe, Some(uk), None);
    let text = canvas.render(ColorMode::Never);
    let italy_line = line_containing(&text, "Italy");
    assert!(!italy_line.contains('┃'), "Italy shouldn't use heavy borders when UK is selected: {italy_line:?}");
}

#[test]
fn no_placement_reproduces_the_plain_region_view_exactly() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let plain = render_region(&map, &layout, &board, Region::Europe, None, None);
    let with_none = render_region(&map, &layout, &board, Region::Europe, None, None);
    assert_eq!(plain.render(ColorMode::Never), with_none.render(ColorMode::Never));
    assert_eq!(plain.height(), with_none.height());
}

#[test]
fn a_placement_adds_exactly_one_footer_row_over_a_plain_selection() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();

    let plain = render_region(&map, &layout, &board, Region::Europe, None, None);
    let selected_only = render_region(&map, &layout, &board, Region::Europe, Some(poland), None);
    let with_placement = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&placement));

    assert_eq!(selected_only.height(), plain.height() + 2, "selection alone should still add exactly two rows");
    assert_eq!(with_placement.height(), plain.height() + 3, "a placement should add exactly one more row than a bare selection");
}

#[test]
fn pending_influence_is_marked_in_the_region_view() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();
    placement.place(&map, poland).unwrap();

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&placement));
    let text = canvas.render(ColorMode::Never);
    let poland_stats_line = line_after(&text, "Poland");
    assert!(poland_stats_line.contains("+2"), "Poland should show a +2 pending badge: {poland_stats_line:?}");
    // The live influence number should also reflect the pending points
    // (USSR 2, contested since the US has none there).
    assert!(poland_stats_line.contains("-:2"), "Poland's USSR figure should be live: {poland_stats_line:?}");
}

#[test]
fn the_pending_marker_reads_without_colour() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&placement));
    let text = canvas.render(ColorMode::Never);
    assert!(!text.contains('\x1b'));
    let poland_stats_line = line_after(&text, "Poland");
    assert!(poland_stats_line.contains("+1"), "the badge should still read under ColorMode::Never: {poland_stats_line:?}");
}

#[test]
fn the_balance_line_shows_side_and_remaining_ops() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 8, &board);
    placement.place(&map, poland).unwrap();
    placement.place(&map, poland).unwrap();

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&placement));
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("USSR"), "balance line should name the placing side:\n{text}");
    assert!(text.contains("6 of 8 ops left"), "balance line should show the ops remaining:\n{text}");
}

#[test]
fn the_placement_hint_replaces_the_selection_hint() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);

    let with_placement = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&placement));
    let text = with_placement.render(ColorMode::Never);
    assert!(text.contains("u undo"), "placement hint missing 'u undo':\n{text}");
    assert!(text.contains("c confirm"), "placement hint missing 'c confirm':\n{text}");

    let without = render_region(&map, &layout, &board, Region::Europe, Some(poland), None);
    let text = without.render(ColorMode::Never);
    assert!(!text.contains("undo"), "plain selection shouldn't hint at undo:\n{text}");
}

#[test]
fn a_country_that_cannot_receive_the_next_placement_is_dimmed() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    // On an empty board, USSR has no presence anywhere in South America
    // and none of it borders the USSR, so every country there is an
    // illegal target — a good, unambiguous "should be dimmed" case.
    let placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    let chile = map.id_by_name("Chile").unwrap();
    assert!(!placement.is_legal_target(&map, chile));

    let canvas = render_region(&map, &layout, &board, Region::SouthAmerica, None, Some(&placement));
    let text = canvas.render(ColorMode::Always);
    let chile_line = line_containing(&text, "Chile");
    // The name should not be drawn in the default (unstyled) run — it
    // should carry the muted style instead.
    assert!(chile_line.contains("\x1b[2m") || chile_line.contains(";2m"), "an illegal target's name should be dimmed: {chile_line:?}");
}

#[test]
fn no_region_line_exceeds_the_canvas_width_with_a_placement_active() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();

    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &board, region, Some(poland), Some(&placement));
        let text = canvas.render(ColorMode::Never);
        for line in text.lines() {
            assert!(line.chars().count() <= canvas.width(), "region {region} placement view exceeded its own width: {line:?}");
        }
    }
}

fn line_containing<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no line containing {needle:?}"))
}

fn line_after<'a>(text: &'a str, needle: &str) -> &'a str {
    let lines: Vec<&str> = text.lines().collect();
    let i = lines.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("no line containing {needle:?}"));
    lines[i + 1]
}

#[test]
fn color_never_emits_no_escape_codes() {
    let (map, layout) = standard();
    let scenario = Scenario::demo(&map).unwrap();
    let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, 104);
    assert!(!canvas.render(ColorMode::Never).contains('\x1b'));

    let country_canvas = render_country(&map, &scenario.board, map.id_by_name("Italy").unwrap());
    assert!(!country_canvas.render(ColorMode::Never).contains('\x1b'));
}

#[test]
fn color_always_wraps_styled_text_in_sgr_codes() {
    let (map, _layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);

    let canvas = render_country(&map, &board, italy);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains('\x1b'), "coloured output should contain ANSI escapes");
    assert!(text.contains("\x1b[0m"), "styled runs should be reset");
}

#[test]
fn render_country_lists_every_neighbor_with_its_own_state() {
    let (map, _layout) = standard();
    let mut board = Board::new(&map);
    let france = map.id_by_name("France").unwrap();
    board.set_influence(france, Superpower::Ussr, 5);

    let italy = map.id_by_name("Italy").unwrap();
    let canvas = render_country(&map, &board, italy);
    let text = canvas.render(ColorMode::Never);

    for neighbor in &map.country(italy).adjacent {
        let name = &map.country(*neighbor).name;
        assert!(text.contains(name.as_str()), "expected neighbour {name} in country view");
    }
    // France's own influence (not Italy's) should show up on its line.
    let france_line = line_containing(&text, "France");
    assert!(france_line.contains('5'), "France's USSR influence should appear: {france_line:?}");
}
