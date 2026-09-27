use twilight_struggle::{Direction, LayoutError, MapLayout, Region, WorldMap};

fn standard() -> (WorldMap, MapLayout) {
    let map = WorldMap::standard().expect("standard map should load");
    let layout = MapLayout::standard(&map).expect("standard layout should load");
    (map, layout)
}

#[test]
fn standard_layout_loads_against_standard_map() {
    let (map, layout) = standard();
    // Every country resolved to a cell without error; region order lists
    // exactly the six regions the dashboard cycles through.
    assert_eq!(layout.region_order().len(), 6);
    for (id, _) in map.iter() {
        let _ = layout.cell(id); // does not panic
        let _ = layout.world_cell(id); // does not panic
        assert!(!layout.short_name(id).is_empty());
        assert!(layout.short_name(id).chars().count() <= 11);
    }
}

#[test]
fn every_country_has_a_short_unique_code() {
    use std::collections::HashSet;
    let (map, layout) = standard();
    let mut seen = HashSet::new();
    for (id, country) in map.iter() {
        let code = layout.code(id);
        assert!(!code.is_empty(), "{} has an empty code", country.name);
        assert!(code.chars().count() <= 4, "{}'s code {code:?} is too long", country.name);
        assert!(
            seen.insert(code.to_uppercase()),
            "{}'s code {code:?} is not unique",
            country.name
        );
    }
}

#[test]
fn find_by_code_is_case_insensitive_and_unique() {
    let (map, layout) = standard();
    let france = map.id_by_name("France").unwrap();
    assert_eq!(layout.code(france), "Fra");
    assert_eq!(layout.find_by_code("fra"), Some(france));
    assert_eq!(layout.find_by_code("FRA"), Some(france));
    assert_eq!(layout.find_by_code("zz"), None);
}

#[test]
fn no_two_countries_share_a_world_cell() {
    use std::collections::HashMap;
    let (map, layout) = standard();
    let mut world_seen: HashMap<(u8, u8), &str> = HashMap::new();
    for (id, country) in map.iter() {
        let cell = layout.world_cell(id);
        if let Some(&first) = world_seen.get(&(cell.row, cell.col)) {
            panic!("{} and {} collide at world cell {:?}", first, country.name, cell);
        }
        world_seen.insert((cell.row, cell.col), &country.name);
    }
}

#[test]
fn every_world_cell_is_within_the_background_bounds() {
    let (map, layout) = standard();
    let bg = layout.background();
    let bg_rows = bg.len();
    let bg_cols = bg.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    for (id, country) in map.iter() {
        let cell = layout.world_cell(id);
        assert!(
            (cell.row as usize) < bg_rows && (cell.col as usize) < bg_cols,
            "{}'s world cell {:?} is outside the {}x{} background",
            country.name,
            cell,
            bg_rows,
            bg_cols
        );
    }
}

#[test]
fn standard_layout_has_no_undrawn_links() {
    // This is about the REGION view, not the world map: every in-region
    // adjacency in the standard map has a grid placement that draws a
    // connector for it — verified by an exact local-repair search, not
    // just eyeballing. If a future edit to the layout data reintroduces a
    // violation, this should fail loudly rather than quietly falling back
    // to a footnote.
    let (_, layout) = standard();
    assert!(
        layout.undrawn_links().is_empty(),
        "expected no undrawn links, found {:?}",
        layout.undrawn_links()
    );
}

#[test]
fn countries_in_region_are_ordered_by_cell() {
    let (map, layout) = standard();
    for &region in layout.region_order() {
        let ids = layout.countries_in_region(&map, region);
        let cells: Vec<(u8, u8)> = ids
            .iter()
            .map(|&id| {
                let cell = layout.cell(id);
                (cell.row, cell.col)
            })
            .collect();
        let mut sorted = cells.clone();
        sorted.sort();
        assert_eq!(cells, sorted, "{region} countries are not in row/col order");
    }
}

#[test]
fn rejects_missing_country() {
    let map = WorldMap::standard().unwrap();
    // Valid JSON shape, but only one entry: every other country is missing.
    let json = r#"{
        "region_order": ["Europe"],
        "superpowers": { "Us": { "cell": [0, 0], "size": [1, 1] }, "Ussr": { "cell": [0, 1], "size": [1, 1] } },
        "countries": { "Canada": { "cell": [0, 0], "code": "CA", "world_cell": [0, 0] } }
    }"#;
    match MapLayout::load(&map, json) {
        Err(LayoutError::MissingCountry(_)) => {}
        other => panic!("expected MissingCountry, got {other:?}"),
    }
}

#[test]
fn rejects_unknown_country() {
    let map = WorldMap::standard().unwrap();
    // Reuse the real standard layout file's content plus a bogus extra
    // entry, so every real country is still present and only the unknown
    // one trips the check.
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["countries"]["Narnia"] = serde_json::json!({ "cell": [0, 0], "code": "NR", "world_cell": [99, 99] });
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::UnknownCountry(name)) => assert_eq!(name, "Narnia"),
        other => panic!("expected UnknownCountry, got {other:?}"),
    }
}

#[test]
fn rejects_duplicate_cell_within_a_region() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    // UK and Canada are both Europe; force Canada onto UK's region-view
    // cell (keeping Canada's own code/world_cell intact, so only the cell
    // collision trips).
    raw["countries"]["Canada"]["cell"] = serde_json::json!([1, 1]);
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::DuplicateCell { .. }) => {}
        other => panic!("expected DuplicateCell, got {other:?}"),
    }
}

#[test]
fn rejects_duplicate_code() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    // France's code, reused (case-differently) on Italy.
    raw["countries"]["Italy"]["code"] = serde_json::json!("fra");
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::DuplicateCode { .. }) => {}
        other => panic!("expected DuplicateCode, got {other:?}"),
    }
}

#[test]
fn rejects_code_too_long() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["countries"]["Canada"]["code"] = serde_json::json!("TOOLONG");
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::CodeTooLong { country, .. }) => assert_eq!(country, "Canada"),
        other => panic!("expected CodeTooLong, got {other:?}"),
    }
}

#[test]
fn rejects_duplicate_world_cell() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    // Force Canada onto UK's world cell directly.
    let uk_world_cell = raw["countries"]["UK"]["world_cell"].clone();
    raw["countries"]["Canada"]["world_cell"] = uk_world_cell;
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::DuplicateWorldCell { .. }) => {}
        other => panic!("expected DuplicateWorldCell, got {other:?}"),
    }
}

#[test]
fn rejects_world_cell_out_of_bounds() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["countries"]["Canada"]["world_cell"] = serde_json::json!([255, 255]);
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::WorldCellOutOfBounds { country, .. }) => assert_eq!(country, "Canada"),
        other => panic!("expected WorldCellOutOfBounds, got {other:?}"),
    }
}

#[test]
fn rejects_missing_superpower_box() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["superpowers"].as_object_mut().unwrap().remove("Ussr");
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::MissingSuperpowerBox(_)) => {}
        other => panic!("expected MissingSuperpowerBox, got {other:?}"),
    }
}

#[test]
fn rejects_superpower_box_overlap() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    // Put Canada's world cell inside the USA box's footprint.
    let us_cell = raw["superpowers"]["Us"]["cell"].clone();
    raw["countries"]["Canada"]["world_cell"] = us_cell;
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::SuperpowerBoxOverlap { country, .. }) => assert_eq!(country, "Canada"),
        other => panic!("expected SuperpowerBoxOverlap, got {other:?}"),
    }
}

#[test]
fn rejects_short_name_too_long() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["countries"]["Canada"]["short"] = serde_json::json!("Way Too Long A Name");
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::ShortNameTooLong { country, .. }) => assert_eq!(country, "Canada"),
        other => panic!("expected ShortNameTooLong, got {other:?}"),
    }
}

const DIRECTIONS: [Direction; 4] = [Direction::Up, Direction::Down, Direction::Left, Direction::Right];

#[test]
fn every_country_is_reachable_by_stepping_within_its_region() {
    // The region display grids are sparse, with interior holes as well as
    // edge ones (Africa is over half empty) — a plain row±1/col±1 step
    // would frequently land on nothing. `step_country` instead has to
    // reach every country in a region by some sequence of arrow presses
    // starting from that region's first country in grid order.
    let (map, layout) = standard();
    for &region in &Region::ALL {
        let ids = layout.countries_in_region(&map, region);
        let start = ids[0];
        let mut seen = vec![start];
        let mut frontier = vec![start];
        while let Some(id) = frontier.pop() {
            for &dir in &DIRECTIONS {
                if let Some(next) = layout.step_country(&map, region, id, dir)
                    && !seen.contains(&next)
                {
                    seen.push(next);
                    frontier.push(next);
                }
            }
        }
        for &id in &ids {
            assert!(
                seen.contains(&id),
                "{} in {region} is unreachable by stepping from {}",
                map.country(id).name,
                map.country(start).name
            );
        }
    }
}

#[test]
fn step_country_never_returns_the_starting_country() {
    let (map, layout) = standard();
    for &region in &Region::ALL {
        for &id in &layout.countries_in_region(&map, region) {
            for &dir in &DIRECTIONS {
                let next = layout.step_country(&map, region, id, dir);
                assert_ne!(
                    next,
                    Some(id),
                    "{} stepping {dir:?} in {region} returned itself",
                    map.country(id).name
                );
            }
        }
    }
}
