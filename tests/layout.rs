use twilight_struggle::{LayoutError, MapLayout, WorldMap};

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
        assert!(!layout.short_name(id).is_empty());
        assert!(layout.short_name(id).chars().count() <= 11);
    }
}

#[test]
fn standard_layout_has_no_undrawn_links() {
    // Every in-region adjacency in the standard map has a grid placement
    // that draws a connector for it — verified by an exact local-repair
    // search, not just eyeballing. If a future edit to the layout data
    // reintroduces a violation, this should fail loudly rather than
    // quietly falling back to a footnote.
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
    let json = r#"{ "region_order": ["Europe"], "countries": { "Canada": { "cell": [0, 0] } } }"#;
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
    raw["countries"]["Narnia"] = serde_json::json!({ "cell": [0, 0] });
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
    // UK and Canada are both Europe; force them onto the same cell.
    raw["countries"]["Canada"] = serde_json::json!({ "cell": [1, 1] });
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::DuplicateCell { .. }) => {}
        other => panic!("expected DuplicateCell, got {other:?}"),
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
