use twilight_struggle::render::{render_country, render_region, render_world};
use twilight_struggle::{Board, ColorMode, MapLayout, Region, Scenario, Superpower, WorldMap};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().unwrap();
    let layout = MapLayout::standard(&map).unwrap();
    (map, layout)
}

const ALL_REGIONS: [Region; 6] = [
    Region::Europe,
    Region::Asia,
    Region::MiddleEast,
    Region::Africa,
    Region::CentralAmerica,
    Region::SouthAmerica,
];

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
    for &region in &ALL_REGIONS {
        let canvas = render_region(&map, &layout, &scenario.board, region);
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
    for &region in &ALL_REGIONS {
        let canvas = render_region(&map, &layout, &board, region);
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

    let canvas = render_region(&map, &layout, &board, Region::Europe);
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
