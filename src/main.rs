use std::collections::BTreeMap;

use twilight_struggle::{Region, WorldMap};

fn main() {
    let map = WorldMap::standard().expect("standard map should be valid");

    println!("Loaded {} countries.\n", map.len());

    let mut by_region: BTreeMap<Region, (usize, usize)> = BTreeMap::new();
    for (_, country) in map.iter() {
        let entry = by_region.entry(country.region).or_default();
        entry.0 += 1;
        if country.battleground {
            entry.1 += 1;
        }
    }

    println!("{:<20} {:>10} {:>13}", "Region", "Countries", "Battlegrounds");
    for (region, (count, battlegrounds)) in &by_region {
        println!("{:<20} {:>10} {:>13}", region.to_string(), count, battlegrounds);
    }
}
