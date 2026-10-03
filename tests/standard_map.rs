use twilight_struggle::{Board, MapError, Region, SubRegion, Superpower, WorldMap};

#[test]
fn standard_map_loads_and_has_expected_shape() {
    let map = WorldMap::standard().expect("standard map should load");
    assert_eq!(map.len(), 84);

    // Every SE Asia sub-region country must be in the Asia region.
    for (_, country) in map.iter() {
        if country.is_in_sub_region(SubRegion::SoutheastAsia) {
            assert_eq!(country.region, Region::Asia, "{} is SE Asia but not Asia", country.name);
        }
        if country.is_in_sub_region(SubRegion::WesternEurope)
            || country.is_in_sub_region(SubRegion::EasternEurope)
        {
            assert_eq!(country.region, Region::Europe, "{} is in a Europe sub-region but not Europe", country.name);
        }
    }
}

/// Pins the per-region country counts against the real board, which this
/// crate's map data has previously drifted from (merged/extra countries,
/// wrong battlegrounds) without any test catching it.
#[test]
fn each_region_has_the_expected_country_count() {
    let map = WorldMap::standard().unwrap();
    let mut counts = std::collections::HashMap::new();
    for (_, country) in map.iter() {
        *counts.entry(country.region).or_insert(0) += 1;
    }
    assert_eq!(counts[&Region::Europe], 21);
    assert_eq!(counts[&Region::MiddleEast], 10);
    assert_eq!(counts[&Region::Africa], 18);
    assert_eq!(counts[&Region::CentralAmerica], 10);
    assert_eq!(counts[&Region::SouthAmerica], 10);
    assert_eq!(counts[&Region::Asia], 15);
}

/// Pins every battleground country, by region — the real board's own
/// list, not just "however many this crate's data happens to have".
#[test]
fn battlegrounds_match_the_real_board() {
    let map = WorldMap::standard().unwrap();
    let battlegrounds = |region: Region| -> Vec<&str> {
        let mut names: Vec<&str> =
            map.iter().filter(|(_, c)| c.region == region && c.battleground).map(|(_, c)| c.name.as_str()).collect();
        names.sort();
        names
    };

    let mut europe = battlegrounds(Region::Europe);
    europe.sort();
    let mut expected = vec!["France", "West Germany", "East Germany", "Poland", "Italy"];
    expected.sort();
    assert_eq!(europe, expected);

    let mut expected = vec!["Egypt", "Israel", "Iraq", "Iran", "Libya", "Saudi Arabia"];
    expected.sort();
    assert_eq!(battlegrounds(Region::MiddleEast), expected);

    let mut expected = vec!["North Korea", "South Korea", "Japan", "Pakistan", "India", "Thailand"];
    expected.sort();
    assert_eq!(battlegrounds(Region::Asia), expected);

    let mut expected = vec!["Algeria", "Nigeria", "Zaire", "Angola", "South Africa"];
    expected.sort();
    assert_eq!(battlegrounds(Region::Africa), expected);

    let mut expected = vec!["Mexico", "Cuba", "Panama"];
    expected.sort();
    assert_eq!(battlegrounds(Region::CentralAmerica), expected);

    let mut expected = vec!["Venezuela", "Chile", "Argentina", "Brazil"];
    expected.sort();
    assert_eq!(battlegrounds(Region::SouthAmerica), expected);
}

/// The specific countries with a direct superpower border, pinned so a
/// future map edit can't silently drop or add one.
#[test]
fn superpower_borders_match_the_real_board() {
    let map = WorldMap::standard().unwrap();
    let borders = |sp: Superpower| -> Vec<&str> {
        let mut names: Vec<&str> =
            map.iter().filter(|(_, c)| c.borders_superpower(sp)).map(|(_, c)| c.name.as_str()).collect();
        names.sort();
        names
    };

    let mut expected = vec!["Canada", "Mexico", "Cuba", "Japan"];
    expected.sort();
    assert_eq!(borders(Superpower::Us), expected);

    let mut expected = vec!["Finland", "Poland", "Romania", "Afghanistan", "North Korea"];
    expected.sort();
    assert_eq!(borders(Superpower::Ussr), expected);
}

#[test]
fn every_name_resolves_and_round_trips() {
    let map = WorldMap::standard().unwrap();
    for (id, country) in map.iter() {
        let looked_up = map.id_by_name(&country.name).expect("name should resolve");
        assert_eq!(looked_up, id);
    }
    assert!(map.id_by_name("Nowhere").is_none());
}

#[test]
fn superpower_adjacency_is_recorded() {
    let map = WorldMap::standard().unwrap();
    let cuba = map.id_by_name("Cuba").unwrap();
    assert!(map.country(cuba).borders_superpower(Superpower::Us));

    let finland = map.id_by_name("Finland").unwrap();
    assert!(map.country(finland).borders_superpower(Superpower::Ussr));

    let italy = map.id_by_name("Italy").unwrap();
    assert!(!map.country(italy).borders_superpower(Superpower::Us));
    assert!(!map.country(italy).borders_superpower(Superpower::Ussr));
}

#[test]
fn rejects_unknown_neighbor() {
    let json = r#"[
        { "name": "A", "stability": 2, "battleground": false, "region": "Europe", "adjacent": ["B"] }
    ]"#;
    match WorldMap::from_json(json) {
        Err(MapError::UnknownNeighbor { country, neighbor }) => {
            assert_eq!(country, "A");
            assert_eq!(neighbor, "B");
        }
        other => panic!("expected UnknownNeighbor, got {other:?}"),
    }
}

#[test]
fn rejects_asymmetric_adjacency() {
    let json = r#"[
        { "name": "A", "stability": 2, "battleground": false, "region": "Europe", "adjacent": ["B"] },
        { "name": "B", "stability": 2, "battleground": false, "region": "Europe", "adjacent": [] }
    ]"#;
    assert!(matches!(
        WorldMap::from_json(json),
        Err(MapError::AsymmetricAdjacency { .. })
    ));
}

#[test]
fn rejects_duplicate_name() {
    let json = r#"[
        { "name": "A", "stability": 2, "battleground": false, "region": "Europe", "adjacent": [] },
        { "name": "A", "stability": 3, "battleground": false, "region": "Asia", "adjacent": [] }
    ]"#;
    match WorldMap::from_json(json) {
        Err(MapError::DuplicateName(name)) => assert_eq!(name, "A"),
        other => panic!("expected DuplicateName, got {other:?}"),
    }
}

#[test]
fn rejects_self_adjacency() {
    let json = r#"[
        { "name": "A", "stability": 2, "battleground": false, "region": "Europe", "adjacent": ["A"] }
    ]"#;
    assert!(matches!(WorldMap::from_json(json), Err(MapError::SelfAdjacency(name)) if name == "A"));
}

#[test]
fn rejects_duplicate_country_neighbor() {
    let json = r#"[
        { "name": "A", "stability": 2, "battleground": false, "region": "Europe", "adjacent": ["B", "B"] },
        { "name": "B", "stability": 2, "battleground": false, "region": "Europe", "adjacent": ["A"] }
    ]"#;
    assert!(matches!(
        WorldMap::from_json(json),
        Err(MapError::DuplicateNeighbor { country, neighbor })
            if country == "A" && neighbor == "B"
    ));
}

#[test]
fn rejects_duplicate_superpower_neighbor() {
    let json = r#"[
        { "name": "A", "stability": 2, "battleground": false, "region": "Europe", "adjacent": ["USSR", "USSR"] }
    ]"#;
    assert!(matches!(
        WorldMap::from_json(json),
        Err(MapError::DuplicateNeighbor { country, neighbor })
            if country == "A" && neighbor == "USSR"
    ));
}

#[test]
fn rejects_zero_stability() {
    let json = r#"[
        { "name": "A", "stability": 0, "battleground": false, "region": "Europe", "adjacent": [] }
    ]"#;
    assert!(matches!(WorldMap::from_json(json), Err(MapError::ZeroStability(name)) if name == "A"));
}

#[test]
fn control_requires_stability_margin() {
    let map = WorldMap::standard().unwrap();
    let italy = map.id_by_name("Italy").unwrap(); // stability 2
    let mut board = Board::new(&map);

    assert_eq!(board.controller(&map, italy), None);

    board.set_influence(italy, Superpower::Us, 2);
    assert_eq!(board.controller(&map, italy), Some(Superpower::Us));

    board.set_influence(italy, Superpower::Ussr, 1);
    assert_eq!(board.controller(&map, italy), None);

    board.set_influence(italy, Superpower::Ussr, 2);
    assert_eq!(board.controller(&map, italy), None);
    assert!(!board.is_controlled_by(&map, italy, Superpower::Us));
    assert!(!board.is_controlled_by(&map, italy, Superpower::Ussr));
}

#[test]
fn control_does_not_overflow_at_max_influence() {
    let map = WorldMap::standard().unwrap();
    let italy = map.id_by_name("Italy").unwrap(); // stability 2
    let mut board = Board::new(&map);

    board.set_influence(italy, Superpower::Us, 255);
    board.set_influence(italy, Superpower::Ussr, 255);
    assert_eq!(board.controller(&map, italy), None);

    board.set_influence(italy, Superpower::Us, 255);
    board.set_influence(italy, Superpower::Ussr, 253);
    assert_eq!(board.controller(&map, italy), Some(Superpower::Us));
}

#[test]
fn influence_mutators_saturate_and_accumulate() {
    let map = WorldMap::standard().unwrap();
    let poland = map.id_by_name("Poland").unwrap();
    let mut board = Board::new(&map);

    board.remove_influence(poland, Superpower::Us, 5);
    assert_eq!(board.influence(poland, Superpower::Us), 0);

    board.add_influence(poland, Superpower::Ussr, 3);
    board.add_influence(poland, Superpower::Ussr, 2);
    assert_eq!(board.influence(poland, Superpower::Ussr), 5);

    board.remove_influence(poland, Superpower::Ussr, 2);
    assert_eq!(board.influence(poland, Superpower::Ussr), 3);
}

/// Finland and Austria are in both Western and Eastern Europe, so cards
/// that name either (Comecon, Marshall Plan, …) can target them.
#[test]
fn finland_and_austria_count_as_both_western_and_eastern_europe() {
    let map = WorldMap::standard().unwrap();
    for name in ["Finland", "Austria"] {
        let c = map.country(map.id_by_name(name).unwrap());
        assert!(c.is_in_sub_region(SubRegion::WesternEurope), "{name} should be Western Europe");
        assert!(c.is_in_sub_region(SubRegion::EasternEurope), "{name} should be Eastern Europe");
    }
}
