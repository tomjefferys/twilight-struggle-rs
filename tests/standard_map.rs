use twilight_struggle::{Board, MapError, Region, SubRegion, Superpower, WorldMap};

#[test]
fn standard_map_loads_and_has_expected_shape() {
    let map = WorldMap::standard().expect("standard map should load");
    assert_eq!(map.len(), 86);

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
