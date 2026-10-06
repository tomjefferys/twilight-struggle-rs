use twilight_struggle::{Direction, GuestEntity, LayoutError, MapLayout, Region, WorldMap};

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
    raw["countries"]["Canada"]["cell"] = serde_json::json!([3, 1]);
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
fn rejects_zero_sized_superpower_box() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["superpowers"]["Us"]["size"] = serde_json::json!([0, 1]);
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::ZeroSizedSuperpowerBox(_)) => {}
        other => panic!("expected ZeroSizedSuperpowerBox, got {other:?}"),
    }
}

#[test]
fn rejects_superpower_box_out_of_bounds() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["superpowers"]["Us"]["cell"] = serde_json::json!([255, 255]);
    raw["superpowers"]["Us"]["size"] = serde_json::json!([1, 1]);
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::SuperpowerBoxOutOfBounds { .. }) => {}
        other => panic!("expected SuperpowerBoxOutOfBounds, got {other:?}"),
    }
}

#[test]
fn rejects_region_order_missing_a_region() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["region_order"] = serde_json::json!(["Europe", "Asia", "MiddleEast", "Africa", "CentralAmerica"]);
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::InvalidRegionOrder) => {}
        other => panic!("expected InvalidRegionOrder, got {other:?}"),
    }
}

#[test]
fn rejects_region_order_with_a_duplicate() {
    let map = WorldMap::standard().unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(include_str!("../data/standard_layout.json")).unwrap();
    raw["region_order"] =
        serde_json::json!(["Europe", "Europe", "MiddleEast", "Africa", "CentralAmerica", "SouthAmerica"]);
    let json = serde_json::to_string(&raw).unwrap();
    match MapLayout::load(&map, &json) {
        Err(LayoutError::InvalidRegionOrder) => {}
        other => panic!("expected InvalidRegionOrder, got {other:?}"),
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
    //
    // `step_country` can now also land on a guest chip — a country
    // native to a *different* region — so a step's target is only
    // recursed into when it's still native to `region`; a guest is a
    // reachable leaf here, the same way `interactive.rs` would follow it
    // elsewhere rather than continuing to explore this grid from it.
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
                    if map.country(next).region == region {
                        frontier.push(next);
                    }
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

/// Starting from `start`'s first country, follows `step_country` alone —
/// jumping onto a guest chip the moment it's reached, exactly as
/// `interactive.rs` does — and returns every region reached this way.
fn regions_reachable_from(map: &WorldMap, layout: &MapLayout, start: Region) -> Vec<Region> {
    let mut seen_regions = vec![start];
    let mut frontier = vec![(start, layout.countries_in_region(map, start)[0])];
    let mut seen_countries = vec![frontier[0].1];
    while let Some((region, id)) = frontier.pop() {
        for &dir in &DIRECTIONS {
            if let Some(next) = layout.step_country(map, region, id, dir) {
                let next_region = map.country(next).region;
                if !seen_regions.contains(&next_region) {
                    seen_regions.push(next_region);
                }
                if !seen_countries.contains(&next) {
                    seen_countries.push(next);
                    frontier.push((next_region, next));
                }
            }
        }
    }
    seen_regions
}

#[test]
fn every_region_is_reachable_by_stepping_through_guest_chips_within_its_own_hemisphere() {
    // Guest chips only stand in for a *real* country-to-country border,
    // and on the standard map those never cross between the Old World
    // (Europe/Asia/MiddleEast/Africa, all connected to each other) and
    // the New World (CentralAmerica/SouthAmerica, connected only to each
    // other) — the two are joined solely through superpower home
    // territory, which isn't a steppable link. So this pins two
    // reachable clusters, not one: within each, the whole cluster is
    // walkable by stepping alone with no need to return to the world map.
    let (map, layout) = standard();
    let old_world = regions_reachable_from(&map, &layout, Region::Europe);
    for &region in &[Region::Europe, Region::Asia, Region::MiddleEast, Region::Africa] {
        assert!(old_world.contains(&region), "{region} is unreachable by stepping alone from Europe");
    }

    let new_world = regions_reachable_from(&map, &layout, Region::CentralAmerica);
    for &region in &[Region::CentralAmerica, Region::SouthAmerica] {
        assert!(new_world.contains(&region), "{region} is unreachable by stepping alone from Central America");
    }
}

#[test]
fn guests_never_collide_with_native_cells_or_each_other() {
    use std::collections::HashSet;
    let (map, layout) = standard();
    for &region in &Region::ALL {
        let mut cells = HashSet::new();
        for &id in &layout.countries_in_region(&map, region) {
            let cell = layout.cell(id);
            assert!(cells.insert((cell.row, cell.col)), "{} collides with another native cell in {region}", map.country(id).name);
        }
        for guest in layout.guests(region) {
            assert!(cells.insert((guest.cell.row, guest.cell.col)), "a guest cell collides in {region}: {guest:?}");
        }
    }
}

#[test]
fn a_guest_is_never_native_to_its_own_host_region() {
    let (map, layout) = standard();
    for &region in &Region::ALL {
        for guest in layout.guests(region) {
            if let GuestEntity::Country(id) = guest.entity {
                assert_ne!(map.country(id).region, region, "{} is a guest of its own region, {region}", map.country(id).name);
            }
        }
    }
}

#[test]
fn stepping_onto_a_guest_lands_in_a_different_region() {
    let (map, layout) = standard();
    for &region in &Region::ALL {
        for &id in &layout.countries_in_region(&map, region) {
            for &dir in &DIRECTIONS {
                if let Some(next) = layout.step_country(&map, region, id, dir)
                    && map.country(next).region != region
                {
                    // `next` must be exactly the country the guest chip
                    // stands for — a country actually native somewhere
                    // else, never a superpower (which is never a
                    // `step_country` candidate at all).
                    assert!(
                        layout.guests(region).iter().any(|g| g.entity == GuestEntity::Country(next)),
                        "{} in {region} stepped to {}, which isn't one of {region}'s guest chips",
                        map.country(id).name,
                        map.country(next).name
                    );
                }
            }
        }
    }
}

#[test]
fn every_region_has_a_guest_covering_its_superpower_adjacency() {
    // `step_country` itself can never return a superpower — its return
    // type is `CountryId` — so the exclusion is enforced by the type
    // system, not tested here. What's worth pinning is that every region
    // bordering a superpower actually has a guest chip for it, so the
    // superpower shows up on the grid rather than only in a footnote.
    let (map, layout) = standard();
    for (_, country) in map.iter() {
        for &sp in &country.adjacent_superpowers {
            assert!(
                layout.guests(country.region).iter().any(|g| g.entity == GuestEntity::Superpower(sp)),
                "{}'s region ({}) has no guest chip for {sp}",
                country.name,
                country.region
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

/// Every region grid places a bordering country on the side of its neighbour that the real
/// map does (judged by `world_cell`): if the world map clearly puts B north/south/east/west of
/// A, B's cell on the grid is never on the opposite side. "Clearly" is ≥3 rows or ≥6 columns
/// apart on the world map (a column is about half a row's size), so near-ties don't trip it.
/// Superpowers are skipped — they have no point on the map to compare against.
#[test]
fn bordering_countries_are_on_the_correct_side_of_each_other() {
    // One-step connectors can't give every Balkan border its true side: Greece, Bulgaria,
    // Romania and Turkey are all pulled between Italy/Yugoslavia and each other, so Romania
    // goes north-east of Turkey rather than north-west. The one known compromise.
    const ALLOWED: &[(&str, &str)] = &[("Romania", "Turkey")];
    let (map, layout) = standard();
    let mut inversions = Vec::new();
    for &region in &Region::ALL {
        let mut placed: Vec<(twilight_struggle::CountryId, twilight_struggle::Cell)> =
            layout.countries_in_region(&map, region).into_iter().map(|id| (id, layout.cell(id))).collect();
        for guest in layout.guests(region) {
            if let GuestEntity::Country(id) = guest.entity {
                placed.push((id, guest.cell));
            }
        }
        for &(a, a_cell) in &placed {
            for &(b, b_cell) in &placed {
                if a >= b || !map.country(a).adjacent.contains(&b) {
                    continue;
                }
                // Native-native, or a native with a guest: the pair's grid cells must be adjacent.
                let grid_dr = b_cell.row as i32 - a_cell.row as i32;
                let grid_dc = b_cell.col as i32 - a_cell.col as i32;
                if grid_dr.abs().max(grid_dc.abs()) != 1 {
                    continue; // a different chip stands for this link, checked on its own
                }
                let (wa, wb) = (layout.world_cell(a), layout.world_cell(b));
                let world_dr = wb.row as i32 - wa.row as i32;
                let world_dc = wb.col as i32 - wa.col as i32;
                let flipped = (world_dr.abs() >= 3 && grid_dr * world_dr < 0) || (world_dc.abs() >= 6 && grid_dc * world_dc < 0);
                if flipped {
                    let names = (map.country(a).name.as_str(), map.country(b).name.as_str());
                    let allowed = ALLOWED.iter().any(|&(x, y)| (x, y) == names || (y, x) == names);
                    if !allowed {
                        inversions.push(format!("{region}: {} / {}", names.0, names.1));
                    }
                }
            }
        }
    }
    assert!(inversions.is_empty(), "grid puts these bordering countries on the wrong side of each other: {inversions:#?}");
}

/// A border between two regions is drawn on both regions' screens (each as a guest chip), and
/// the two must agree: if B is up-left of A on A's screen, A is down-right of B on B's. This is
/// what makes stepping across a region boundary and back land where you started and feel like
/// one reversible move.
#[test]
fn cross_region_links_point_opposite_ways_on_their_two_screens() {
    use twilight_struggle::Cell;
    let (map, layout) = standard();
    let step = |from: Cell, to: Cell| (to.row as i32 - from.row as i32, to.col as i32 - from.col as i32);
    // Offsets from `a`'s cell to each guest chip standing for `b` that touches it.
    let offsets = |region: Region, a, b| -> Vec<(i32, i32)> {
        layout
            .guests(region)
            .iter()
            .filter(|g| g.entity == GuestEntity::Country(b))
            .map(|g| step(layout.cell(a), g.cell))
            .filter(|&(dr, dc)| dr.abs().max(dc.abs()) == 1)
            .collect()
    };
    let mut mismatches = Vec::new();
    for (a, country) in map.iter() {
        for &b in &country.adjacent {
            if a >= b || map.country(b).region == country.region {
                continue;
            }
            let from_a = offsets(country.region, a, b);
            let from_b = offsets(map.country(b).region, b, a);
            let agree = from_a.iter().any(|&(dr, dc)| from_b.contains(&(-dr, -dc)));
            if !agree {
                mismatches.push(format!("{} ({:?}) / {} ({:?})", country.name, from_a, map.country(b).name, from_b));
            }
        }
    }
    assert!(mismatches.is_empty(), "cross-region links disagree between their two screens: {mismatches:#?}");
}

/// Stepping onto a different region's country is only ever along a real border: from a country
/// to a guest it doesn't border (Zaire "north" to Libya, the UK "down" to Algeria) must not land.
#[test]
fn stepping_never_crosses_a_region_without_a_border() {
    let (map, layout) = standard();
    for &region in &Region::ALL {
        for &id in &layout.countries_in_region(&map, region) {
            for dir in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
                if let Some(next) = layout.step_country(&map, region, id, dir) {
                    if map.country(next).region != region {
                        assert!(
                            map.country(id).adjacent.contains(&next),
                            "{} stepped {dir:?} onto {}, which it doesn't border",
                            map.country(id).name,
                            map.country(next).name
                        );
                    }
                }
            }
        }
    }
}
