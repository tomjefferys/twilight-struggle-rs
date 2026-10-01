use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::country::{Country, CountryId, Region, SubRegion, Superpower};

const STANDARD_MAP_JSON: &str = include_str!("../data/standard_map.json");

#[derive(Debug)]
pub enum MapError {
    Json(serde_json::Error),
    DuplicateName(String),
    UnknownNeighbor { country: String, neighbor: String },
    DuplicateNeighbor { country: String, neighbor: String },
    AsymmetricAdjacency { country: String, neighbor: String },
    SelfAdjacency(String),
    ZeroStability(String),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::Json(e) => write!(f, "invalid map JSON: {e}"),
            MapError::DuplicateName(name) => {
                write!(f, "duplicate country name: {name}")
            }
            MapError::UnknownNeighbor { country, neighbor } => write!(
                f,
                "{country} lists unknown neighbor {neighbor:?} (not a country, USA, or USSR)"
            ),
            MapError::DuplicateNeighbor { country, neighbor } => {
                write!(f, "{country} lists {neighbor} as a neighbor more than once")
            }
            MapError::AsymmetricAdjacency { country, neighbor } => write!(
                f,
                "{country} lists {neighbor} as a neighbor, but {neighbor} does not list {country} back"
            ),
            MapError::SelfAdjacency(name) => {
                write!(f, "{name} lists itself as a neighbor")
            }
            MapError::ZeroStability(name) => {
                write!(f, "{name} has a stability of 0, which is not valid")
            }
        }
    }
}

impl std::error::Error for MapError {}

/// The result of a forgiving, interactive name lookup ([`WorldMap::find`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    One(CountryId),
    Ambiguous(Vec<CountryId>),
    None,
}

impl From<serde_json::Error> for MapError {
    fn from(e: serde_json::Error) -> Self {
        MapError::Json(e)
    }
}

#[derive(Debug, Deserialize)]
struct RawCountry {
    name: String,
    stability: u8,
    battleground: bool,
    region: Region,
    #[serde(default)]
    sub_regions: Vec<SubRegion>,
    #[serde(default)]
    adjacent: Vec<String>,
}

/// The full set of countries and their adjacencies for one game map.
///
/// Immutable once loaded: neighbour names have already been validated and
/// resolved to [`CountryId`]s, so every lookup through this type is
/// infallible.
#[derive(Debug)]
pub struct WorldMap {
    countries: Vec<Country>,
    by_name: HashMap<String, CountryId>,
}

impl WorldMap {
    /// The standard Twilight Struggle map, embedded in the binary.
    pub fn standard() -> Result<Self, MapError> {
        Self::from_json(STANDARD_MAP_JSON)
    }

    pub fn from_json(json: &str) -> Result<Self, MapError> {
        let raw: Vec<RawCountry> = serde_json::from_str(json)?;

        let mut by_name = HashMap::with_capacity(raw.len());
        for (index, entry) in raw.iter().enumerate() {
            if by_name
                .insert(entry.name.clone(), CountryId::new(index))
                .is_some()
            {
                return Err(MapError::DuplicateName(entry.name.clone()));
            }
        }

        let mut countries = Vec::with_capacity(raw.len());
        for entry in &raw {
            if entry.stability == 0 {
                return Err(MapError::ZeroStability(entry.name.clone()));
            }

            let mut adjacent = Vec::with_capacity(entry.adjacent.len());
            let mut adjacent_superpowers = Vec::new();
            for neighbor in &entry.adjacent {
                if neighbor == &entry.name {
                    return Err(MapError::SelfAdjacency(entry.name.clone()));
                }
                match neighbor.as_str() {
                    "USA" => {
                        if adjacent_superpowers.contains(&Superpower::Us) {
                            return Err(MapError::DuplicateNeighbor {
                                country: entry.name.clone(),
                                neighbor: neighbor.clone(),
                            });
                        }
                        adjacent_superpowers.push(Superpower::Us)
                    }
                    "USSR" => {
                        if adjacent_superpowers.contains(&Superpower::Ussr) {
                            return Err(MapError::DuplicateNeighbor {
                                country: entry.name.clone(),
                                neighbor: neighbor.clone(),
                            });
                        }
                        adjacent_superpowers.push(Superpower::Ussr)
                    }
                    _ => {
                        let id = by_name.get(neighbor).copied().ok_or_else(|| {
                            MapError::UnknownNeighbor {
                                country: entry.name.clone(),
                                neighbor: neighbor.clone(),
                            }
                        })?;
                        if adjacent.contains(&id) {
                            return Err(MapError::DuplicateNeighbor {
                                country: entry.name.clone(),
                                neighbor: neighbor.clone(),
                            });
                        }
                        adjacent.push(id);
                    }
                }
            }

            countries.push(Country {
                name: entry.name.clone(),
                stability: entry.stability,
                battleground: entry.battleground,
                region: entry.region,
                sub_regions: entry.sub_regions.clone(),
                adjacent,
                adjacent_superpowers,
            });
        }

        for entry in &raw {
            let id = by_name[&entry.name];
            for &neighbor_id in &countries[id.index()].adjacent {
                let neighbor = &countries[neighbor_id.index()];
                if !neighbor.adjacent.contains(&id) {
                    return Err(MapError::AsymmetricAdjacency {
                        country: entry.name.clone(),
                        neighbor: neighbor.name.clone(),
                    });
                }
            }
        }

        Ok(WorldMap { countries, by_name })
    }

    pub fn country(&self, id: CountryId) -> &Country {
        &self.countries[id.index()]
    }

    pub fn id_by_name(&self, name: &str) -> Option<CountryId> {
        self.by_name.get(name).copied()
    }

    /// A forgiving name lookup for interactive use: an exact match (any
    /// case) wins outright; otherwise every country whose name starts with
    /// `query` (any case) is a candidate.
    pub fn find(&self, query: &str) -> Found {
        let query_lower = query.to_lowercase();
        for country in &self.countries {
            if country.name.to_lowercase() == query_lower {
                return Found::One(self.by_name[&country.name]);
            }
        }
        let matches: Vec<CountryId> = self
            .countries
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name.to_lowercase().starts_with(&query_lower))
            .map(|(i, _)| CountryId::new(i))
            .collect();
        match matches.len() {
            0 => Found::None,
            1 => Found::One(matches[0]),
            _ => Found::Ambiguous(matches),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (CountryId, &Country)> {
        self.countries
            .iter()
            .enumerate()
            .map(|(i, c)| (CountryId::new(i), c))
    }

    pub fn len(&self) -> usize {
        self.countries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.countries.is_empty()
    }
}
