use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::country::{CountryId, Region};
use crate::map::WorldMap;

const STANDARD_LAYOUT_JSON: &str = include_str!("../data/standard_layout.json");

/// The maximum length of a country's short display name, set by the width
/// of the box drawn for it in a region zoom view.
const MAX_SHORT_NAME: usize = 11;

/// A country's position on its region's display grid. Coordinates are
/// scoped to the region: two countries in different regions may share the
/// same cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub row: u8,
    pub col: u8,
}

impl Cell {
    /// Whether two cells are grid-neighbours: orthogonally or diagonally
    /// adjacent (Chebyshev distance of exactly 1).
    fn is_adjacent(self, other: Cell) -> bool {
        let dr = (self.row as i16 - other.row as i16).abs();
        let dc = (self.col as i16 - other.col as i16).abs();
        dr.max(dc) == 1
    }
}

#[derive(Debug)]
pub enum LayoutError {
    Json(serde_json::Error),
    /// A country in the map has no entry in the layout.
    MissingCountry(String),
    /// A layout entry names a country the map doesn't have.
    UnknownCountry(String),
    /// Two countries in the same region were placed on the same cell.
    DuplicateCell {
        region: Region,
        row: u8,
        col: u8,
        first: String,
        second: String,
    },
    /// An explicit `short` name is too long to fit a region-view box.
    ShortNameTooLong {
        country: String,
        name: String,
        max: usize,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::Json(e) => write!(f, "invalid layout JSON: {e}"),
            LayoutError::MissingCountry(name) => {
                write!(f, "{name} has no entry in the layout")
            }
            LayoutError::UnknownCountry(name) => {
                write!(f, "layout names {name:?}, which is not a country on this map")
            }
            LayoutError::DuplicateCell {
                region,
                row,
                col,
                first,
                second,
            } => write!(
                f,
                "{first} and {second} are both placed at ({row}, {col}) in {region}"
            ),
            LayoutError::ShortNameTooLong { country, name, max } => write!(
                f,
                "{country}'s short name {name:?} is {} characters, but the maximum is {max}",
                name.chars().count()
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl From<serde_json::Error> for LayoutError {
    fn from(e: serde_json::Error) -> Self {
        LayoutError::Json(e)
    }
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    cell: [u8; 2],
    #[serde(default)]
    short: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawLayout {
    region_order: Vec<Region>,
    countries: HashMap<String, RawEntry>,
}

#[derive(Debug, Clone)]
struct CountryLayout {
    cell: Cell,
    short: String,
}

/// Where each country is drawn: its region-view grid cell and its short
/// display name. Loaded once against a [`WorldMap`] and never changes.
///
/// A [`WorldMap`] can be laid out in more than one way (a custom map may
/// need its own layout file), so `MapLayout` is validated against, but
/// never bundled with, the map it describes.
#[derive(Debug)]
pub struct MapLayout {
    entries: Vec<CountryLayout>,
    region_order: Vec<Region>,
    undrawn: Vec<(CountryId, CountryId)>,
}

impl MapLayout {
    /// The layout for the standard Twilight Struggle map, embedded in the
    /// binary.
    pub fn standard(map: &WorldMap) -> Result<Self, LayoutError> {
        Self::load(map, STANDARD_LAYOUT_JSON)
    }

    pub fn load(map: &WorldMap, json: &str) -> Result<Self, LayoutError> {
        let raw: RawLayout = serde_json::from_str(json)?;

        for (_, country) in map.iter() {
            if !raw.countries.contains_key(&country.name) {
                return Err(LayoutError::MissingCountry(country.name.clone()));
            }
        }
        for name in raw.countries.keys() {
            if map.id_by_name(name).is_none() {
                return Err(LayoutError::UnknownCountry(name.clone()));
            }
        }

        let mut entries = Vec::with_capacity(map.len());
        for (_, country) in map.iter() {
            let entry = &raw.countries[&country.name];
            let cell = Cell {
                row: entry.cell[0],
                col: entry.cell[1],
            };
            let short = match &entry.short {
                Some(name) => {
                    if name.chars().count() > MAX_SHORT_NAME {
                        return Err(LayoutError::ShortNameTooLong {
                            country: country.name.clone(),
                            name: name.clone(),
                            max: MAX_SHORT_NAME,
                        });
                    }
                    name.clone()
                }
                None => country.name.chars().take(MAX_SHORT_NAME).collect(),
            };
            entries.push(CountryLayout { cell, short });
        }

        // Duplicate-cell check, scoped per region.
        let mut seen: HashMap<Region, HashMap<(u8, u8), CountryId>> = HashMap::new();
        for (id, country) in map.iter() {
            let cell = entries[id.index()].cell;
            let region_cells = seen.entry(country.region).or_default();
            if let Some(&first) = region_cells.get(&(cell.row, cell.col)) {
                return Err(LayoutError::DuplicateCell {
                    region: country.region,
                    row: cell.row,
                    col: cell.col,
                    first: map.country(first).name.clone(),
                    second: country.name.clone(),
                });
            }
            region_cells.insert((cell.row, cell.col), id);
        }

        let mut layout = MapLayout {
            entries,
            region_order: raw.region_order,
            undrawn: Vec::new(),
        };
        layout.undrawn = layout.compute_undrawn_links(map);
        Ok(layout)
    }

    fn compute_undrawn_links(&self, map: &WorldMap) -> Vec<(CountryId, CountryId)> {
        let mut undrawn = Vec::new();
        for (id, country) in map.iter() {
            for &neighbor_id in &country.adjacent {
                if neighbor_id <= id {
                    continue; // each undirected edge considered once
                }
                let neighbor = map.country(neighbor_id);
                if neighbor.region != country.region {
                    continue; // cross-region links are never drawn on a region grid
                }
                if !self.cell(id).is_adjacent(self.cell(neighbor_id)) {
                    undrawn.push((id, neighbor_id));
                }
            }
        }
        undrawn
    }

    pub fn cell(&self, id: CountryId) -> Cell {
        self.entries[id.index()].cell
    }

    pub fn short_name(&self, id: CountryId) -> &str {
        &self.entries[id.index()].short
    }

    /// Region panel order for the world dashboard.
    pub fn region_order(&self) -> &[Region] {
        &self.region_order
    }

    /// Every country in `region`, ordered by grid cell (row, then column)
    /// so reading order matches the geography of the zoom view.
    pub fn countries_in_region(&self, map: &WorldMap, region: Region) -> Vec<CountryId> {
        let mut ids: Vec<CountryId> = map
            .iter()
            .filter(|(_, c)| c.region == region)
            .map(|(id, _)| id)
            .collect();
        ids.sort_by_key(|&id| {
            let cell = self.cell(id);
            (cell.row, cell.col)
        });
        ids
    }

    /// In-region adjacencies whose endpoints are not grid-neighbours, so no
    /// connector is drawn between them in the region zoom view. Every such
    /// link is still real; it just isn't geometrically representable on
    /// this grid, and is instead footnoted below the map.
    pub fn undrawn_links(&self) -> &[(CountryId, CountryId)] {
        &self.undrawn
    }
}
