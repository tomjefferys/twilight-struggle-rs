use twilight_struggle::render::{operation_abandoned_line, operation_closed_line, render_card, render_country, render_hand, render_region, render_world};
use twilight_struggle::{
    Board, CardCatalog, ColorMode, CountryId, Coup, GuestEntity, InfluencePlacement, LinkTarget, MapLayout, Operation, Realignment, Region,
    Scenario, Superpower, ViewMode, WorldMap,
};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().unwrap();
    let layout = MapLayout::standard(&map).unwrap();
    (map, layout)
}

fn cards() -> CardCatalog {
    CardCatalog::standard().unwrap()
}

#[test]
fn world_dashboard_matches_snapshot_at_104() {
    let (map, layout) = standard();
    let cards = cards();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, 104);
    let expected = include_str!("snapshots/world_104.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn world_dashboard_matches_snapshot_at_80() {
    let (map, layout) = standard();
    let cards = cards();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, 80);
    let expected = include_str!("snapshots/world_80.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn no_rendered_line_exceeds_requested_width() {
    let (map, layout) = standard();
    let cards = cards();
    let scenario = Scenario::demo(&map, &cards).unwrap();
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

/// Whether `a` and a candidate cell `b` are grid-neighbours — the same
/// Chebyshev-distance-1 rule `Cell::is_adjacent` uses internally.
fn grid_adjacent(a: twilight_struggle::Cell, b: twilight_struggle::Cell) -> bool {
    let dr = (a.row as i16 - b.row as i16).abs();
    let dc = (a.col as i16 - b.col as i16).abs();
    dr.max(dc) == 1
}

/// Every adjacency — in-region, cross-region, or to a superpower — must
/// be either drawn (a connector between two in-region boxes, or between a
/// native box and a guest chip standing in for the far side) or footnoted
/// as undrawn: the display can never silently omit an edge.
#[test]
fn every_adjacency_is_drawn_or_footnoted() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &board, region, None, None);
        let text = canvas.render(ColorMode::Never);
        let has_connector = text.chars().any(|c| matches!(c, '─' | '│' | '╲' | '╱' | '╳'));
        assert!(has_connector, "{region} region view has no connectors at all");

        let ids = layout.countries_in_region(&map, region);
        let guests = layout.guests(region);
        for &id in &ids {
            let country = map.country(id);
            let a = layout.cell(id);
            for &neighbor_id in &country.adjacent {
                let neighbor = map.country(neighbor_id);
                let drawable = if neighbor.region == region {
                    grid_adjacent(a, layout.cell(neighbor_id))
                } else {
                    guests
                        .iter()
                        .any(|g| g.entity == GuestEntity::Country(neighbor_id) && grid_adjacent(a, g.cell))
                };
                let footnoted = layout
                    .undrawn_links()
                    .iter()
                    .any(|link| link.region == region && link.from == id && link.to == LinkTarget::Country(neighbor_id));
                assert!(
                    drawable || footnoted,
                    "{} – {} in {region} is neither grid-adjacent nor footnoted as undrawn",
                    country.name,
                    neighbor.name
                );
            }
            for &sp in &country.adjacent_superpowers {
                let drawable = guests.iter().any(|g| g.entity == GuestEntity::Superpower(sp) && grid_adjacent(a, g.cell));
                let footnoted = layout
                    .undrawn_links()
                    .iter()
                    .any(|link| link.region == region && link.from == id && link.to == LinkTarget::Superpower(sp));
                assert!(
                    drawable || footnoted,
                    "{} – {sp} in {region} is neither grid-adjacent nor footnoted as undrawn",
                    country.name
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
    assert!(text.contains("Enter open"), "a selected country should hint that Enter opens its detail screen:\n{text}");
    assert_eq!(selected.height(), plain.height() + 2, "a selection should add exactly two rows");
}

#[test]
fn the_selection_hints_offer_the_operation_keys() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();

    let region_text = render_region(&map, &layout, &board, Region::Europe, Some(italy), None).render(ColorMode::Never);
    for key in ["space play card", "i/a/o influence/realign/coup", "p pass"] {
        assert!(region_text.contains(key), "region hint missing {key:?}:\n{region_text}");
    }

    let country_text = render_country(&map, &layout, &board, italy, None, ViewMode::Interactive).render(ColorMode::Never);
    for key in ["space play card", "i/a/o influence/realign/coup", "p pass"] {
        assert!(country_text.contains(key), "country hint missing {key:?}:\n{country_text}");
    }
}

#[test]
fn an_open_operation_hides_the_operation_keys() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    let placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
    let op = Operation::Influence(placement);

    let region_text = render_region(&map, &layout, &board, Region::Europe, Some(italy), Some(&op)).render(ColorMode::Never);
    assert!(!region_text.contains("space play card"), "an open operation shouldn't advertise starting another:\n{region_text}");
    assert!(!region_text.contains("p pass"), "an open operation shouldn't advertise passing:\n{region_text}");

    let country_text = render_country(&map, &layout, &board, italy, Some(&op), ViewMode::Interactive).render(ColorMode::Never);
    assert!(!country_text.contains("space play card"), "an open operation shouldn't advertise starting another:\n{country_text}");
    assert!(!country_text.contains("p pass"), "an open operation shouldn't advertise passing:\n{country_text}");
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

    let op = Operation::Influence(placement);
    let plain = render_region(&map, &layout, &board, Region::Europe, None, None);
    let selected_only = render_region(&map, &layout, &board, Region::Europe, Some(poland), None);
    let with_placement = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&op));

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

    let op = Operation::Influence(placement);
    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&op));
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

    let op = Operation::Influence(placement);
    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&op));
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

    let op = Operation::Influence(placement);
    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&op));
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("USSR"), "balance line should name the placing side:\n{text}");
    assert!(text.contains("6 of 8 ops left"), "balance line should show the ops remaining:\n{text}");
}

#[test]
fn operation_closed_line_names_the_side_the_verb_and_who_acts_next() {
    let (map, _) = standard();
    let board = Board::new(&map);
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
    let poland = map.id_by_name("Poland").unwrap();
    placement.place(&map, poland).unwrap();
    let op = Operation::Influence(placement);

    let confirmed = operation_closed_line(&op, true, Superpower::Us);
    assert!(confirmed.contains("USSR"), "should name the side that acted:\n{confirmed}");
    assert!(confirmed.contains("placing"), "should name what it was doing:\n{confirmed}");
    assert!(confirmed.contains("confirmed"), "should say it was confirmed:\n{confirmed}");
    assert!(confirmed.contains("1 of 4"), "should show ops spent of total:\n{confirmed}");
    assert!(confirmed.contains("USA to act"), "should name who acts next:\n{confirmed}");

    let cancelled = operation_closed_line(&op, false, Superpower::Us);
    assert!(cancelled.contains("cancelled"), "should say it was cancelled:\n{cancelled}");
}

#[test]
fn operation_abandoned_line_names_the_side_the_verb_and_that_the_turn_did_not_change() {
    let (map, _) = standard();
    let board = Board::new(&map);
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 4, &board));

    let text = operation_abandoned_line(&op);
    assert!(text.contains("USSR"), "should name the side that abandoned it:\n{text}");
    assert!(text.contains("realigning"), "should name what was abandoned:\n{text}");
    assert!(text.contains("abandoned"), "should say it was abandoned:\n{text}");
    assert!(text.contains("nothing spent"), "should reassure that nothing was spent:\n{text}");
    assert!(text.contains("card still in play"), "should say the card wasn't discarded:\n{text}");
}

#[test]
fn operation_abandoned_line_names_what_was_undone_when_a_placement_had_points_pending() {
    let (map, _) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 4, &board);
    placement.place(&map, poland).unwrap();
    placement.place(&map, poland).unwrap();
    let op = Operation::Influence(placement);

    let text = operation_abandoned_line(&op);
    assert!(text.contains("2 of 4 ops undone"), "should name how many ops were undone:\n{text}");
    assert!(!text.contains("nothing spent"), "shouldn't claim nothing happened when points were pending:\n{text}");
    assert!(text.contains("card still in play"), "should say the card wasn't discarded:\n{text}");
}

#[test]
fn the_placement_hint_replaces_the_selection_hint() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    let op = Operation::Influence(placement);

    let with_placement = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&op));
    let text = with_placement.render(ColorMode::Never);
    assert!(text.contains("u undo"), "placement hint missing 'u undo':\n{text}");
    assert!(text.contains("c confirm"), "placement hint missing 'c confirm':\n{text}");
    assert!(text.contains("abandon"), "placement hint missing 'abandon':\n{text}");

    assert!(!text.contains("i influence"), "a placement in progress shouldn't offer to start another:\n{text}");

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

    let op = Operation::Influence(placement);
    let canvas = render_region(&map, &layout, &board, Region::SouthAmerica, None, Some(&op));
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
    let op = Operation::Influence(placement);

    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &board, region, Some(poland), Some(&op));
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
    let cards = cards();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let canvas = render_world(&map, &layout, &scenario.board, &scenario.status, 104);
    assert!(!canvas.render(ColorMode::Never).contains('\x1b'));

    let country_canvas = render_country(&map, &layout, &scenario.board, map.id_by_name("Italy").unwrap(), None, ViewMode::Static);
    assert!(!country_canvas.render(ColorMode::Never).contains('\x1b'));
}

#[test]
fn color_always_wraps_styled_text_in_sgr_codes() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);

    let canvas = render_country(&map, &layout, &board, italy, None, ViewMode::Static);
    let text = canvas.render(ColorMode::Always);
    assert!(text.contains('\x1b'), "coloured output should contain ANSI escapes");
    assert!(text.contains("\x1b[0m"), "styled runs should be reset");
}

#[test]
fn render_country_lists_every_neighbor_with_its_own_state() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let france = map.id_by_name("France").unwrap();
    board.set_influence(france, Superpower::Ussr, 5);

    let italy = map.id_by_name("Italy").unwrap();
    let canvas = render_country(&map, &layout, &board, italy, None, ViewMode::Static);
    let text = canvas.render(ColorMode::Never);

    for neighbor in &map.country(italy).adjacent {
        let name = &map.country(*neighbor).name;
        assert!(text.contains(name.as_str()), "expected neighbour {name} in country view");
    }
    // France's own influence (not Italy's) should show up on its chip's
    // stats row, immediately below the row naming it.
    let france_stats_line = line_after(&text, "France");
    assert!(france_stats_line.contains('5'), "France's USSR influence should appear: {france_stats_line:?}");
}

#[test]
fn every_country_places_all_its_neighbours_on_the_country_view_grid() {
    // The country-view analogue of
    // `tests/layout.rs::standard_layout_has_no_undrawn_links`: every
    // adjacency, in-region or cross-region/superpower, should land on a
    // grid-adjacent cell in the viewed country's own region and never
    // fall back to the "not shown on this grid" footnote.
    let (map, layout) = standard();
    let board = Board::new(&map);
    for (id, country) in map.iter() {
        let canvas = render_country(&map, &layout, &board, id, None, ViewMode::Static);
        let text = canvas.render(ColorMode::Never);
        assert!(
            !text.contains("not shown on this grid"),
            "{}: every neighbour should be placed on the mini-map:\n{text}",
            country.name
        );
    }
}

#[test]
fn the_selected_country_is_always_the_middle_row_of_the_mini_map() {
    // The mini-map is a fixed 3×3 window centred on the viewed country,
    // not cropped to whoever's actually adjacent — so its size (and the
    // viewed country's row within it) shouldn't depend on how many
    // neighbours that country has, or on which sides they're missing.
    let (map, layout) = standard();
    let board = Board::new(&map);

    let poland = map.id_by_name("Poland").unwrap(); // neighbours on every side
    let reference = centre_chip_offset(&map, &layout, &board, poland, "Poland");

    for name in ["Canada", "South Africa", "Panama", "Ivory Coast"] {
        let id = map.id_by_name(name).unwrap();
        let offset = centre_chip_offset(&map, &layout, &board, id, name);
        assert_eq!(offset, reference, "{name}: the viewed country should sit at the same fixed row as Poland's, regardless of its own neighbour count");
    }
}

/// The centre chip's name row, as a line offset from the "Neighbours"
/// divider — the same for every country once the mini-map's size no
/// longer depends on how many neighbours are actually shown.
fn centre_chip_offset(map: &WorldMap, layout: &MapLayout, board: &Board, id: CountryId, name: &str) -> usize {
    let canvas = render_country(map, layout, board, id, None, ViewMode::Static);
    let text = canvas.render(ColorMode::Never);
    let lines: Vec<&str> = text.lines().collect();
    let divider = lines.iter().position(|l| l.contains("Neighbours")).unwrap();
    // The country's full name appears exactly twice: once in the box's
    // own title border, once as the mini-map's centre chip label.
    let name_rows: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains(name)).map(|(i, _)| i).collect();
    assert_eq!(name_rows.len(), 2, "{name}: expected the title and the centre chip to both name it in full: {name_rows:?}");
    name_rows[1] - divider
}

#[test]
fn the_box_height_does_not_depend_on_whether_the_country_has_sub_regions() {
    // The sub-regions row is always reserved, blank when there are none,
    // rather than only present sometimes — so the box is exactly the
    // same height whether or not the viewed country happens to have any.
    let (map, layout) = standard();
    let board = Board::new(&map);

    let poland = map.id_by_name("Poland").unwrap(); // has a sub-region
    let with_sub_region = render_country(&map, &layout, &board, poland, None, ViewMode::Static);

    let canada = map.id_by_name("Canada").unwrap(); // has none
    let without_sub_region = render_country(&map, &layout, &board, canada, None, ViewMode::Static);

    assert_eq!(with_sub_region.height(), without_sub_region.height(), "the box height shouldn't depend on whether the country has sub-regions");
    // And the blank row is really there, not just coincidentally equal
    // heights: the Neighbours divider should land on the same row either
    // way.
    let divider_row = |canvas: &twilight_struggle::render::Canvas| canvas.render(ColorMode::Never).lines().position(|l| l.contains("Neighbours")).unwrap();
    assert_eq!(divider_row(&with_sub_region), divider_row(&without_sub_region));
}

#[test]
fn the_box_width_does_not_depend_on_which_country_is_shown() {
    // The mini-map's chips are sized off the longest country name in the
    // whole game, not just whoever's actually a neighbour here — so
    // arrowing from a country with short-named neighbours (Poland) to
    // one with long-named ones (Ivory Coast, whose neighbours include
    // "West African States") shouldn't move the box's right border.
    let (map, layout) = standard();
    let board = Board::new(&map);

    let poland = map.id_by_name("Poland").unwrap();
    let reference = render_country(&map, &layout, &board, poland, None, ViewMode::Static).width();

    for name in ["Canada", "South Africa", "Panama", "Ivory Coast", "West African States"] {
        let id = map.id_by_name(name).unwrap();
        let width = render_country(&map, &layout, &board, id, None, ViewMode::Static).width();
        assert_eq!(width, reference, "{name}: the box width shouldn't depend on which country is shown");
    }
}

#[test]
fn a_realignment_adds_exactly_four_more_footer_rows_than_a_bare_selection() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));

    let plain = render_region(&map, &layout, &board, Region::Europe, None, None);
    let selected_only = render_region(&map, &layout, &board, Region::Europe, Some(poland), None);
    let with_realign = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&op));

    assert_eq!(selected_only.height(), plain.height() + 2, "selection alone should still add exactly two rows");
    // Selection title, balance line, this side's modifiers, the
    // opponent's modifiers, and the odds line: four more than a bare
    // selection's title + hint.
    assert_eq!(
        with_realign.height(),
        selected_only.height() + 4,
        "a realignment with a country selected should add exactly four more rows than a bare selection"
    );
}

#[test]
fn a_realigned_country_shows_its_badge() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);
    board.set_influence(italy, Superpower::Ussr, 1);

    // The realignment's `base` is captured here, before the board is
    // mutated below — exactly as a real win would leave it.
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    board.set_influence(italy, Superpower::Us, 1); // as if a roll just removed 2

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&op));
    let text = canvas.render(ColorMode::Never);
    let italy_stats_line = line_after(&text, "Italy");
    assert!(italy_stats_line.contains("-2"), "Italy should show a -2 badge for the US influence it lost: {italy_stats_line:?}");
}

#[test]
fn the_badge_reads_without_colour() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    board.set_influence(italy, Superpower::Us, 1);

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&op));
    let text = canvas.render(ColorMode::Never);
    assert!(!text.contains('\x1b'));
    let italy_stats_line = line_after(&text, "Italy");
    assert!(italy_stats_line.contains("-2"), "the badge should still read under ColorMode::Never: {italy_stats_line:?}");
}

#[test]
fn a_country_with_no_opponent_influence_is_dimmed_during_a_realignment() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    // On an empty board, the US has no influence anywhere in South
    // America, so every country there is an illegal realignment target
    // for the USSR — a good, unambiguous "should be dimmed" case.
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));

    let canvas = render_region(&map, &layout, &board, Region::SouthAmerica, None, Some(&op));
    let text = canvas.render(ColorMode::Always);
    let chile_line = line_containing(&text, "Chile");
    assert!(chile_line.contains("\x1b[2m") || chile_line.contains(";2m"), "an illegal target's name should be dimmed: {chile_line:?}");
}

#[test]
fn the_realign_hint_replaces_the_selection_hint() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));

    // The region screen no longer rolls — it just points at the country
    // screen, which is where the roll (and its hint) actually live.
    let with_realign = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&op));
    let text = with_realign.render(ColorMode::Never);
    assert!(text.contains("Enter target"), "realign region hint should point at the country screen:\n{text}");
    assert!(text.contains("c done"), "realign hint missing 'c done':\n{text}");
    assert!(text.contains("abandon"), "realign hint missing 'abandon':\n{text}");
    assert!(!text.contains("r roll"), "rolling no longer happens on the region screen:\n{text}");
    assert!(!text.contains("u undo"), "a realignment hint shouldn't offer undo, which isn't possible:\n{text}");
    assert!(!text.contains("+ place"), "a realignment hint shouldn't offer placement's '+ place':\n{text}");
    assert!(!text.contains("i influence"), "a realignment in progress shouldn't offer to start another operation:\n{text}");

    let country_view = render_country(&map, &layout, &board, poland, Some(&op), ViewMode::Interactive);
    let country_text = country_view.render(ColorMode::Never);
    assert!(country_text.contains("r roll"), "country screen hint missing 'r roll':\n{country_text}");
    assert!(country_text.contains("c done"), "country screen hint missing 'c done':\n{country_text}");
    assert!(country_text.contains("abandon"), "country screen hint missing 'abandon':\n{country_text}");
    assert!(!country_text.contains("i influence"), "a realignment in progress shouldn't offer to start another operation:\n{country_text}");
}

#[test]
fn the_modifier_summary_names_every_active_modifier() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    // Poland borders the USSR itself, so superpower_adjacent is free;
    // controlling both its other neighbours and out-influencing the US
    // there lights up the remaining two modifiers as well.
    board.set_influence(map.id_by_name("East Germany").unwrap(), Superpower::Ussr, 3);
    board.set_influence(map.id_by_name("Czechoslovakia").unwrap(), Superpower::Ussr, 3);
    board.set_influence(poland, Superpower::Ussr, 2);
    assert!(map.country(poland).borders_superpower(Superpower::Ussr));

    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    let canvas = render_region(&map, &layout, &board, Region::Europe, Some(poland), Some(&op));
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("2 adjacent controlled countries"), "expected the adjacent-controlled count:\n{text}");
    assert!(text.contains("more influence"), "expected the more-influence modifier:\n{text}");
    assert!(text.contains("superpower adjacent"), "expected the superpower-adjacent modifier:\n{text}");
}

#[test]
fn no_region_line_exceeds_the_canvas_width_with_a_realignment_active() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    // Stack every modifier for both sides on the same country, and use
    // one with a long name, to stress the width fold as hard as
    // possible: the country's own line, the balance line, the
    // modifier lines, and the odds line all become long at once.
    let poland = map.id_by_name("Poland").unwrap();
    board.set_influence(map.id_by_name("East Germany").unwrap(), Superpower::Ussr, 3);
    board.set_influence(map.id_by_name("Czechoslovakia").unwrap(), Superpower::Ussr, 3);
    board.set_influence(poland, Superpower::Ussr, 2);

    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &board, region, Some(poland), Some(&op));
        let text = canvas.render(ColorMode::Never);
        for line in text.lines() {
            assert!(line.chars().count() <= canvas.width(), "region {region} realignment view exceeded its own width: {line:?}");
        }
    }
}

#[test]
fn no_country_detail_line_exceeds_its_canvas_width() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    board.set_influence(map.id_by_name("East Germany").unwrap(), Superpower::Ussr, 3);
    board.set_influence(map.id_by_name("Czechoslovakia").unwrap(), Superpower::Ussr, 3);
    board.set_influence(poland, Superpower::Ussr, 2);

    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    let canvas = render_country(&map, &layout, &board, poland, Some(&op), ViewMode::Static);
    let text = canvas.render(ColorMode::Never);
    for line in text.lines() {
        assert!(line.chars().count() <= canvas.width(), "country detail view exceeded its own width: {line:?}");
    }
    assert!(text.contains("odds"), "expected the odds line to be present:\n{text}");
}

#[test]
fn a_coup_adds_exactly_three_more_footer_rows_than_a_bare_selection() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);
    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));

    let plain = render_region(&map, &layout, &board, Region::Europe, None, None);
    let selected_only = render_region(&map, &layout, &board, Region::Europe, Some(italy), None);
    let with_coup = render_region(&map, &layout, &board, Region::Europe, Some(italy), Some(&op));

    assert_eq!(selected_only.height(), plain.height() + 2, "selection alone should still add exactly two rows");
    // Selection title, balance line, the target-number line, and the
    // odds line: three more than a bare selection's title + hint.
    assert_eq!(
        with_coup.height(),
        selected_only.height() + 3,
        "a coup with a country selected should add exactly three more rows than a bare selection"
    );
}

#[test]
fn a_couped_country_shows_its_badge() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);

    // The coup's `base` is captured here, before the board is mutated
    // below — exactly as a real attempt would leave it.
    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));
    board.set_influence(italy, Superpower::Us, 1); // as if the attempt just removed 2

    let canvas = render_region(&map, &layout, &board, Region::Europe, None, Some(&op));
    let text = canvas.render(ColorMode::Never);
    let italy_stats_line = line_after(&text, "Italy");
    assert!(italy_stats_line.contains("-2"), "Italy should show a -2 badge for the US influence it lost: {italy_stats_line:?}");
}

#[test]
fn a_country_with_no_opponent_influence_is_dimmed_during_a_coup() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    // On an empty board, the US has no influence anywhere in South
    // America, so every country there is an illegal coup target for the
    // USSR — a good, unambiguous "should be dimmed" case.
    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));

    let canvas = render_region(&map, &layout, &board, Region::SouthAmerica, None, Some(&op));
    let text = canvas.render(ColorMode::Always);
    let chile_line = line_containing(&text, "Chile");
    assert!(chile_line.contains("\x1b[2m") || chile_line.contains(";2m"), "an illegal target's name should be dimmed: {chile_line:?}");
}

#[test]
fn the_coup_hint_replaces_the_selection_hint() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap();
    board.set_influence(italy, Superpower::Us, 3);
    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));

    // The region screen no longer attempts the coup — it just points at
    // the country screen, which is where the attempt (and its hint) live.
    let with_coup = render_region(&map, &layout, &board, Region::Europe, Some(italy), Some(&op));
    let text = with_coup.render(ColorMode::Never);
    assert!(text.contains("Enter target"), "coup region hint should point at the country screen:\n{text}");
    assert!(text.contains("c done"), "coup hint missing 'c done':\n{text}");
    assert!(text.contains("abandon"), "coup hint missing 'abandon':\n{text}");
    assert!(!text.contains("r coup"), "couping no longer happens on the region screen:\n{text}");
    assert!(!text.contains("u undo"), "a coup hint shouldn't offer undo, which isn't possible:\n{text}");
    assert!(!text.contains("+ place"), "a coup hint shouldn't offer placement's '+ place':\n{text}");
    assert!(!text.contains("i influence"), "a coup in progress shouldn't offer to start another operation:\n{text}");

    let country_view = render_country(&map, &layout, &board, italy, Some(&op), ViewMode::Interactive);
    let country_text = country_view.render(ColorMode::Never);
    assert!(country_text.contains("r coup"), "country screen hint missing 'r coup':\n{country_text}");
    assert!(country_text.contains("c done"), "country screen hint missing 'c done':\n{country_text}");
    assert!(country_text.contains("abandon"), "country screen hint missing 'abandon':\n{country_text}");
    assert!(!country_text.contains("i influence"), "a coup in progress shouldn't offer to start another operation:\n{country_text}");
}

#[test]
fn the_coup_target_line_names_the_stability_and_ops() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let italy = map.id_by_name("Italy").unwrap(); // stability 2
    board.set_influence(italy, Superpower::Us, 3);
    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));

    let canvas = render_region(&map, &layout, &board, Region::Europe, Some(italy), Some(&op));
    let text = canvas.render(ColorMode::Never);
    assert!(text.contains("vs 4"), "expected the doubled-stability target number:\n{text}");
    assert!(text.contains("stability 2"), "expected the raw stability named:\n{text}");
}

#[test]
fn no_region_line_exceeds_the_canvas_width_with_a_coup_active() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let east_germany = map.id_by_name("East Germany").unwrap();
    board.set_influence(east_germany, Superpower::Us, 3);

    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));
    for &region in &Region::ALL {
        let canvas = render_region(&map, &layout, &board, region, Some(east_germany), Some(&op));
        let text = canvas.render(ColorMode::Never);
        for line in text.lines() {
            assert!(line.chars().count() <= canvas.width(), "region {region} coup view exceeded its own width: {line:?}");
        }
    }
}

#[test]
fn no_country_detail_line_exceeds_its_canvas_width_with_a_coup_active() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    board.set_influence(poland, Superpower::Us, 3);

    let op = Operation::Coup(Coup::new(Superpower::Ussr, 4, &board));
    let canvas = render_country(&map, &layout, &board, poland, Some(&op), ViewMode::Static);
    let text = canvas.render(ColorMode::Never);
    for line in text.lines() {
        assert!(line.chars().count() <= canvas.width(), "country detail view exceeded its own width: {line:?}");
    }
    assert!(text.contains("odds"), "expected the odds line to be present:\n{text}");
}

#[test]
fn country_detail_marks_the_neighbours_supplying_adjacent_controlled() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let east_germany = map.id_by_name("East Germany").unwrap();
    board.set_influence(east_germany, Superpower::Ussr, 3);

    let op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    let canvas = render_country(&map, &layout, &board, poland, Some(&op), ViewMode::Static);
    let text = canvas.render(ColorMode::Never);
    // A chip has no room for the marker text itself, so the controlled
    // neighbours supplying the modifier are named on their own footnote
    // line instead.
    let marker_line = line_containing(&text, "+1 realign");
    assert!(marker_line.contains("East Germany"), "the controlled neighbour should be named: {marker_line:?}");
    assert!(!marker_line.contains("Czechoslovakia"), "an uncontrolled neighbour shouldn't be named: {marker_line:?}");
}

#[test]
fn country_detail_matches_snapshot() {
    let (map, layout) = standard();
    let cards = cards();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let poland = map.id_by_name("Poland").unwrap();
    let canvas = render_country(&map, &layout, &scenario.board, poland, None, ViewMode::Static);
    let expected = include_str!("snapshots/country_poland.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn the_country_view_has_one_divider_with_no_operation_and_two_with_one() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();

    // No operation: just the Neighbours panel divider. With one open,
    // the Operation panel adds a second — regardless of which kind, since
    // every kind draws exactly one divider for its own panel.
    let plain = render_country(&map, &layout, &board, poland, None, ViewMode::Static);
    assert_eq!(plain.render(ColorMode::Never).matches('├').count(), 1, "no operation should draw just the Neighbours divider");

    for op in [
        Operation::Influence(InfluencePlacement::new(Superpower::Ussr, 5, &board)),
        Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board)),
        Operation::Coup(Coup::new(Superpower::Ussr, 5, &board)),
    ] {
        let canvas = render_country(&map, &layout, &board, poland, Some(&op), ViewMode::Static);
        assert_eq!(canvas.render(ColorMode::Never).matches('├').count(), 2, "an open operation should add its own panel divider");
    }
}

#[test]
fn no_country_detail_line_exceeds_its_canvas_width_for_any_country() {
    let (map, layout) = standard();
    let mut board = Board::new(&map);
    // Give every country influence on both sides, so a realignment or
    // coup on any of them has real modifier/odds lines to draw rather
    // than the short "no influence to remove" placeholder — the longer,
    // more likely-to-overflow case.
    for (id, _) in map.iter() {
        board.set_influence(id, Superpower::Us, 1);
        board.set_influence(id, Superpower::Ussr, 1);
    }

    let placement_op = Operation::Influence(InfluencePlacement::new(Superpower::Ussr, 5, &board));
    let realign_op = Operation::Realign(Realignment::new(Superpower::Ussr, 5, &board));
    let coup_op = Operation::Coup(Coup::new(Superpower::Ussr, 5, &board));

    for (id, country) in map.iter() {
        for op in [None, Some(&placement_op), Some(&realign_op), Some(&coup_op)] {
            let canvas = render_country(&map, &layout, &board, id, op, ViewMode::Interactive);
            let text = canvas.render(ColorMode::Never);
            for line in text.lines() {
                assert!(
                    line.chars().count() <= canvas.width(),
                    "{}: country detail view exceeded its own width: {line:?}",
                    country.name
                );
            }
        }
    }
}

#[test]
fn the_hint_row_only_appears_under_interactive_mode() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();

    let static_view = render_country(&map, &layout, &board, poland, None, ViewMode::Static);
    let static_text = static_view.render(ColorMode::Never);
    assert!(!static_text.contains("←→↑↓"), "a static print has no keys to hint at:\n{static_text}");

    let interactive_view = render_country(&map, &layout, &board, poland, None, ViewMode::Interactive);
    let interactive_text = interactive_view.render(ColorMode::Never);
    assert!(interactive_text.contains("←→↑↓"), "an interactive view should hint its keys:\n{interactive_text}");
    assert_eq!(interactive_view.height(), static_view.height() + 1, "the hint should add exactly one row");
}

#[test]
fn pending_influence_is_marked_in_the_country_view() {
    let (map, layout) = standard();
    let board = Board::new(&map);
    let poland = map.id_by_name("Poland").unwrap();
    let mut placement = InfluencePlacement::new(Superpower::Ussr, 5, &board);
    placement.place(&map, poland).unwrap();
    placement.place(&map, poland).unwrap();

    let op = Operation::Influence(placement);
    let canvas = render_country(&map, &layout, &board, poland, Some(&op), ViewMode::Static);
    let text = canvas.render(ColorMode::Never);
    // Poland's own influence line should read the speculative board, not
    // the caller's real (still-empty) one.
    let poland_stats_line = line_after(&text, "Poland");
    assert!(poland_stats_line.contains("USSR 2"), "Poland's USSR figure should be live: {poland_stats_line:?}");
    assert!(text.contains("2 placed here"), "the Operation panel should say how much is pending here:\n{text}");
}

#[test]
fn hand_strip_matches_snapshot() {
    let (map, _) = standard();
    let cards = cards();
    let scenario = Scenario::demo(&map, &cards).unwrap();
    let hand = scenario.hands.hand(Superpower::Ussr);
    let canvas = render_hand(&cards, hand, Some(scenario.status.china_card_face_up), Superpower::Ussr, None);
    let expected = include_str!("snapshots/hand_ussr.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}

#[test]
fn card_detail_matches_snapshot() {
    let cards = cards();
    let red_scare = cards.id_by_name("Red Scare/Purge").unwrap();
    let canvas = render_card(&cards, red_scare, None);
    let expected = include_str!("snapshots/card_red_scare.txt");
    assert_eq!(canvas.render(ColorMode::Never), expected.trim_end_matches('\n'));
}
