use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;

use crate::country::{CountryId, Direction, Region, Superpower};
use crate::map::WorldMap;

const STANDARD_LAYOUT_JSON: &str = include_str!("../data/standard_layout.json");

/// A pre-rasterized ASCII/Unicode rendering of real landmass shape,
/// generated once from public-domain coastline data (Natural Earth's 110m
/// land polygons) and embedded verbatim — see the plan history for how it
/// was produced. Every country's `world_cell` is a position within this
/// same grid, so the two must be authored together.
const WORLD_BACKGROUND_TXT: &str = include_str!("../data/world_background.txt");

/// The maximum length of a country's short display name, set by the width
/// of the box drawn for it in a region zoom view.
const MAX_SHORT_NAME: usize = 11;

/// The maximum length of a country's code, set by the width of the chip
/// drawn for it on the world map. Codes are short English mnemonics (e.g.
/// `"Fra"` for France, `"WGe"` for West Germany), not ISO codes — chosen to
/// read naturally rather than to match a real-world standard.
const MAX_CODE: usize = 4;

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

/// A superpower's footprint on the world map: a labelled rectangle rather
/// than a point, since USA and USSR each border several countries spread
/// across regions — a sprawling track along the board's edge in the
/// physical game, not a single space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuperpowerBox {
    pub cell: Cell,
    pub rows: u8,
    pub cols: u8,
}

impl SuperpowerBox {
    fn contains(&self, cell: Cell) -> bool {
        cell.row >= self.cell.row
            && cell.row < self.cell.row + self.rows
            && cell.col >= self.cell.col
            && cell.col < self.cell.col + self.cols
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
    /// Two countries share the same code (case-insensitively).
    DuplicateCode {
        first: String,
        second: String,
        code: String,
    },
    /// A country's code is too long to fit the world-map chip.
    CodeTooLong {
        country: String,
        code: String,
        max: usize,
    },
    /// Two countries land on the same world-map cell.
    DuplicateWorldCell {
        first: String,
        second: String,
        row: u8,
        col: u8,
    },
    /// The `superpowers` object has no entry for this superpower.
    MissingSuperpowerBox(Superpower),
    /// A country's world cell falls inside a superpower's box footprint.
    SuperpowerBoxOverlap {
        superpower: Superpower,
        country: String,
        row: u8,
        col: u8,
    },
    /// A country's `world_cell` falls outside the background art's bounds.
    WorldCellOutOfBounds {
        country: String,
        row: u8,
        col: u8,
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
            LayoutError::DuplicateCode { first, second, code } => {
                write!(f, "{first} and {second} both use the code {code:?}")
            }
            LayoutError::CodeTooLong { country, code, max } => write!(
                f,
                "{country}'s code {code:?} is {} characters, but the maximum is {max}",
                code.chars().count()
            ),
            LayoutError::DuplicateWorldCell { first, second, row, col } => write!(
                f,
                "{first} and {second} are both placed at world cell ({row}, {col})"
            ),
            LayoutError::MissingSuperpowerBox(sp) => {
                write!(f, "{sp} has no entry in superpowers")
            }
            LayoutError::SuperpowerBoxOverlap { superpower, country, row, col } => write!(
                f,
                "{country}'s world cell ({row}, {col}) falls inside {superpower}'s box"
            ),
            LayoutError::WorldCellOutOfBounds { country, row, col } => write!(
                f,
                "{country}'s world cell ({row}, {col}) falls outside the background art's bounds"
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
    code: String,
    #[serde(default)]
    short: Option<String>,
    /// Where this country is hand-placed on the world map — chosen to
    /// resemble real geography, independent of the region-view `cell`.
    world_cell: [u8; 2],
}

#[derive(Debug, Deserialize)]
struct RawSuperpowerBox {
    cell: [u8; 2],
    size: [u8; 2],
}

#[derive(Debug, Deserialize)]
struct RawLayout {
    region_order: Vec<Region>,
    superpowers: HashMap<Superpower, RawSuperpowerBox>,
    countries: HashMap<String, RawEntry>,
}

#[derive(Debug, Clone)]
struct CountryLayout {
    cell: Cell,
    short: String,
    code: String,
    world_cell: Cell,
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
    superpower_boxes: HashMap<Superpower, SuperpowerBox>,
    background: Vec<String>,
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
            if entry.code.chars().count() > MAX_CODE {
                return Err(LayoutError::CodeTooLong {
                    country: country.name.clone(),
                    code: entry.code.clone(),
                    max: MAX_CODE,
                });
            }
            let world_cell = Cell {
                row: entry.world_cell[0],
                col: entry.world_cell[1],
            };
            entries.push(CountryLayout { cell, short, code: entry.code.clone(), world_cell });
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

        // Duplicate-code check, case-insensitive, global (codes are meant
        // to be unique identifiers across the whole map).
        let mut codes_seen: HashMap<String, CountryId> = HashMap::new();
        for (id, country) in map.iter() {
            let key = entries[id.index()].code.to_uppercase();
            if let Some(&first) = codes_seen.get(&key) {
                return Err(LayoutError::DuplicateCode {
                    first: map.country(first).name.clone(),
                    second: country.name.clone(),
                    code: entries[id.index()].code.clone(),
                });
            }
            codes_seen.insert(key, id);
        }

        // Every superpower needs a box, or render_world_map has nothing to
        // draw for it.
        for sp in [Superpower::Us, Superpower::Ussr] {
            if !raw.superpowers.contains_key(&sp) {
                return Err(LayoutError::MissingSuperpowerBox(sp));
            }
        }
        let superpower_boxes: HashMap<Superpower, SuperpowerBox> = raw
            .superpowers
            .iter()
            .map(|(&sp, b)| {
                (
                    sp,
                    SuperpowerBox {
                        cell: Cell { row: b.cell[0], col: b.cell[1] },
                        rows: b.size[0],
                        cols: b.size[1],
                    },
                )
            })
            .collect();

        // Duplicate-world-cell check, global.
        let mut world_seen: HashMap<(u8, u8), CountryId> = HashMap::new();
        for (id, country) in map.iter() {
            let cell = entries[id.index()].world_cell;
            if let Some(&first) = world_seen.get(&(cell.row, cell.col)) {
                return Err(LayoutError::DuplicateWorldCell {
                    first: map.country(first).name.clone(),
                    second: country.name.clone(),
                    row: cell.row,
                    col: cell.col,
                });
            }
            world_seen.insert((cell.row, cell.col), id);
        }

        // A country placed inside a superpower's box footprint would be
        // drawn over, or under, without either mistake being obvious.
        for (id, country) in map.iter() {
            let cell = entries[id.index()].world_cell;
            for (&sp, sbox) in &superpower_boxes {
                if sbox.contains(cell) {
                    return Err(LayoutError::SuperpowerBoxOverlap {
                        superpower: sp,
                        country: country.name.clone(),
                        row: cell.row,
                        col: cell.col,
                    });
                }
            }
        }

        // Every country's world_cell must land within the background art —
        // otherwise it would be drawn off the edge of the canvas, or (worse)
        // silently clipped by Canvas's own bounds-clipping with no
        // indication anything was wrong.
        let background: Vec<String> = WORLD_BACKGROUND_TXT.lines().map(str::to_string).collect();
        let bg_rows = background.len() as u8;
        let bg_cols = background.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u8;
        for (id, country) in map.iter() {
            let cell = entries[id.index()].world_cell;
            if cell.row >= bg_rows || cell.col >= bg_cols {
                return Err(LayoutError::WorldCellOutOfBounds {
                    country: country.name.clone(),
                    row: cell.row,
                    col: cell.col,
                });
            }
        }

        let mut layout = MapLayout {
            entries,
            region_order: raw.region_order,
            superpower_boxes,
            background,
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

    pub fn code(&self, id: CountryId) -> &str {
        &self.entries[id.index()].code
    }

    /// Where a country is hand-placed on the unified world map — chosen to
    /// resemble real geography, independent of its region-view `cell`.
    pub fn world_cell(&self, id: CountryId) -> Cell {
        self.entries[id.index()].world_cell
    }

    /// The pre-rasterized landmass art the world map draws underneath every
    /// country, one string per row. Every country's [`MapLayout::world_cell`]
    /// is a position within this same grid.
    pub fn background(&self) -> &[String] {
        &self.background
    }

    /// A superpower's box on the world map.
    ///
    /// Panics if `superpower` has no box — every [`Superpower`] is
    /// guaranteed one by [`MapLayout::load`]'s validation.
    pub fn superpower_box(&self, superpower: Superpower) -> SuperpowerBox {
        self.superpower_boxes[&superpower]
    }

    /// A case-insensitive exact match on a country's code (e.g. `"fr"` for
    /// France). Codes are short and unique, so unlike [`WorldMap::find`]
    /// there's no prefix matching or ambiguity to handle.
    pub fn find_by_code(&self, query: &str) -> Option<CountryId> {
        let query = query.to_uppercase();
        self.entries
            .iter()
            .position(|e| e.code.to_uppercase() == query)
            .map(CountryId::new)
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

    /// The country reached by moving `dir` from `from` on `region`'s
    /// display grid: the nearest other country in `region` that lies
    /// strictly on `dir`'s side of `from`, or `None` if there isn't one —
    /// the selection should then hold still, the same convention as
    /// [`Region::step`].
    ///
    /// The region grids are sparse (Africa is over half empty, with holes
    /// in the interior, not just the edges), so a bare row±1/col±1 step
    /// would often land on nothing. Picking the nearest candidate by
    /// (cross-axis distance, along-axis distance) instead reaches every
    /// country in every region with no dead ends — verified against the
    /// real grid in `data/standard_layout.json`.
    pub fn step_country(&self, map: &WorldMap, region: Region, from: CountryId, dir: Direction) -> Option<CountryId> {
        let from_cell = self.cell(from);
        self.countries_in_region(map, region)
            .into_iter()
            .filter(|&id| id != from)
            .filter_map(|id| {
                let cell = self.cell(id);
                let dr = cell.row as i16 - from_cell.row as i16;
                let dc = cell.col as i16 - from_cell.col as i16;
                let (cross, along) = match dir {
                    Direction::Left if dc < 0 => (dr.abs(), -dc),
                    Direction::Right if dc > 0 => (dr.abs(), dc),
                    Direction::Up if dr < 0 => (dc.abs(), -dr),
                    Direction::Down if dr > 0 => (dc.abs(), dr),
                    _ => return None,
                };
                Some((cross, along, cell.row, cell.col, id))
            })
            .min_by_key(|&(cross, along, row, col, _)| (cross, along, row, col))
            .map(|(.., id)| id)
    }
}
