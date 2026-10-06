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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    pub row: u8,
    pub col: u8,
}

impl Cell {
    /// Whether two cells are grid-neighbours: orthogonally or diagonally
    /// adjacent (Chebyshev distance of exactly 1).
    pub(crate) fn is_adjacent(self, other: Cell) -> bool {
        let dr = (self.row as i16 - other.row as i16).abs();
        let dc = (self.col as i16 - other.col as i16).abs();
        dr.max(dc) == 1
    }
}

/// What a [`Guest`] chip stands for: a country whose own `region` is
/// somewhere else, or a superpower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestEntity {
    Country(CountryId),
    Superpower(Superpower),
}

/// A country or superpower drawn on a region's display grid even though
/// it doesn't belong there — the region-view analogue of the world map's
/// chips, placed so every cross-region and superpower adjacency is a real
/// box you can step onto rather than a footnote below the grid.
/// Coordinates share the host region's cell space, so a guest cell must
/// not collide with a native one or another guest there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guest {
    pub entity: GuestEntity,
    pub cell: Cell,
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
    /// Two countries in the same region were placed on the same cell — or
    /// a guest chip collided with a native country's cell or another
    /// guest's, in the region hosting it.
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
    /// A superpower's box has zero rows or columns, so nothing would ever
    /// be drawn inside it.
    ZeroSizedSuperpowerBox(Superpower),
    /// A superpower's box footprint extends past the background art's
    /// bounds, so the renderer would silently clip it.
    SuperpowerBoxOutOfBounds {
        superpower: Superpower,
        row: u16,
        col: u16,
    },
    /// `region_order` isn't exactly one entry per region — `world.rs`
    /// trusts it to be a full permutation of the six regions.
    InvalidRegionOrder,
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
    /// A `guests` entry names neither a country nor a superpower, names
    /// both, or names a country the map doesn't have.
    UnknownGuest {
        region: Region,
        detail: String,
    },
    /// A `guests` entry names a country that's actually native to its own
    /// host region — it belongs on the grid as a real box, not a guest.
    GuestInOwnRegion {
        region: Region,
        country: String,
    },
    /// A guest cell in `region` isn't grid-adjacent to any native country
    /// there that actually borders it — most likely a stale or mistyped
    /// cell, since a guest that draws no connector isn't earning its
    /// place on the grid.
    UnusedGuest {
        region: Region,
        entity: String,
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
            LayoutError::ZeroSizedSuperpowerBox(sp) => {
                write!(f, "{sp}'s box has zero rows or columns")
            }
            LayoutError::SuperpowerBoxOutOfBounds { superpower, row, col } => write!(
                f,
                "{superpower}'s box extends to ({row}, {col}), outside the background art's bounds"
            ),
            LayoutError::InvalidRegionOrder => {
                write!(f, "region_order must list each of the six regions exactly once")
            }
            LayoutError::SuperpowerBoxOverlap { superpower, country, row, col } => write!(
                f,
                "{country}'s world cell ({row}, {col}) falls inside {superpower}'s box"
            ),
            LayoutError::WorldCellOutOfBounds { country, row, col } => write!(
                f,
                "{country}'s world cell ({row}, {col}) falls outside the background art's bounds"
            ),
            LayoutError::UnknownGuest { region, detail } => {
                write!(f, "{region}'s guests: {detail}")
            }
            LayoutError::GuestInOwnRegion { region, country } => write!(
                f,
                "{country} is listed as a guest of {region}, which is already its own region"
            ),
            LayoutError::UnusedGuest { region, entity } => write!(
                f,
                "{entity}'s guest cell in {region} is not grid-adjacent to any country there that borders it"
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

/// One `guests` list entry: exactly one of `country`/`superpower` names
/// the entity, `cell` places it on the host region's grid.
#[derive(Debug, Deserialize)]
struct RawGuest {
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    superpower: Option<Superpower>,
    cell: [u8; 2],
}

#[derive(Debug, Deserialize)]
struct RawLayout {
    region_order: Vec<Region>,
    superpowers: HashMap<Superpower, RawSuperpowerBox>,
    countries: HashMap<String, RawEntry>,
    #[serde(default)]
    guests: HashMap<Region, Vec<RawGuest>>,
}

#[derive(Debug, Clone)]
struct CountryLayout {
    cell: Cell,
    short: String,
    code: String,
    world_cell: Cell,
}

/// The far end of an [`UndrawnLink`] — whatever a region-view grid failed
/// to draw a connector for: another country, or a superpower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkTarget {
    Country(CountryId),
    Superpower(Superpower),
}

/// An adjacency `region`'s display grid has no connector for: either two
/// in-region countries too far apart on the grid to draw a line between,
/// or a cross-region/superpower adjacency with no guest chip standing in
/// for the far side. Every such link is still real; it's footnoted below
/// the map instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndrawnLink {
    pub region: Region,
    pub from: CountryId,
    pub to: LinkTarget,
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
    guests: HashMap<Region, Vec<Guest>>,
    undrawn: Vec<UndrawnLink>,
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

        // Resolve and validate `guests`: each entry names exactly one of
        // a country or a superpower, a named country must not be native
        // to the region hosting it as a guest, and no guest cell may
        // collide with a native cell (checked against `seen` above) or
        // another guest's in the same region.
        let mut guests: HashMap<Region, Vec<Guest>> = HashMap::new();
        for (&region, list) in &raw.guests {
            let mut region_guests: Vec<Guest> = Vec::with_capacity(list.len());
            for raw_guest in list {
                let entity = match (&raw_guest.country, raw_guest.superpower) {
                    (Some(name), None) => {
                        let id = map.id_by_name(name).ok_or_else(|| LayoutError::UnknownGuest {
                            region,
                            detail: format!("{name:?} is not a country on this map"),
                        })?;
                        if map.country(id).region == region {
                            return Err(LayoutError::GuestInOwnRegion { region, country: name.clone() });
                        }
                        GuestEntity::Country(id)
                    }
                    (None, Some(sp)) => GuestEntity::Superpower(sp),
                    (None, None) => {
                        return Err(LayoutError::UnknownGuest {
                            region,
                            detail: "a guest entry names neither a country nor a superpower".to_string(),
                        });
                    }
                    (Some(_), Some(_)) => {
                        return Err(LayoutError::UnknownGuest {
                            region,
                            detail: "a guest entry names both a country and a superpower".to_string(),
                        });
                    }
                };
                let cell = Cell { row: raw_guest.cell[0], col: raw_guest.cell[1] };
                let label = guest_label(map, entity);
                if let Some(&first) = seen.get(&region).and_then(|m| m.get(&(cell.row, cell.col))) {
                    return Err(LayoutError::DuplicateCell {
                        region,
                        row: cell.row,
                        col: cell.col,
                        first: map.country(first).name.clone(),
                        second: label,
                    });
                }
                if let Some(existing) = region_guests.iter().find(|g| g.cell == cell) {
                    return Err(LayoutError::DuplicateCell {
                        region,
                        row: cell.row,
                        col: cell.col,
                        first: guest_label(map, existing.entity),
                        second: label,
                    });
                }
                region_guests.push(Guest { entity, cell });
            }
            guests.insert(region, region_guests);
        }

        // Every guest chip must actually be grid-adjacent to a native
        // country in its host region that borders it — otherwise it
        // draws no connector, and its cell is just a stale or mistyped
        // placeholder no one would ever reach by stepping.
        for (&region, list) in &guests {
            for guest in list {
                let used = map.iter().filter(|(_, c)| c.region == region).any(|(id, country)| {
                    entries[id.index()].cell.is_adjacent(guest.cell)
                        && match guest.entity {
                            GuestEntity::Country(target) => country.adjacent.contains(&target),
                            GuestEntity::Superpower(sp) => country.adjacent_superpowers.contains(&sp),
                        }
                });
                if !used {
                    return Err(LayoutError::UnusedGuest { region, entity: guest_label(map, guest.entity) });
                }
            }
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

        let background: Vec<String> = WORLD_BACKGROUND_TXT.lines().map(str::to_string).collect();
        let bg_rows = background.len() as u16;
        let bg_cols = background.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;

        // A zero-sized box would never draw anything, and a box whose
        // footprint runs off the background art would be silently clipped
        // by Canvas's own bounds-clipping with no indication anything was
        // wrong — the same failure mode a country's own world_cell is
        // checked for below.
        for (&sp, sbox) in &superpower_boxes {
            if sbox.rows == 0 || sbox.cols == 0 {
                return Err(LayoutError::ZeroSizedSuperpowerBox(sp));
            }
            let last_row = sbox.cell.row as u16 + sbox.rows as u16 - 1;
            let last_col = sbox.cell.col as u16 + sbox.cols as u16 - 1;
            if last_row >= bg_rows || last_col >= bg_cols {
                return Err(LayoutError::SuperpowerBoxOutOfBounds {
                    superpower: sp,
                    row: last_row,
                    col: last_col,
                });
            }
        }

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
        for (id, country) in map.iter() {
            let cell = entries[id.index()].world_cell;
            if cell.row as u16 >= bg_rows || cell.col as u16 >= bg_cols {
                return Err(LayoutError::WorldCellOutOfBounds {
                    country: country.name.clone(),
                    row: cell.row,
                    col: cell.col,
                });
            }
        }

        // world.rs trusts region_order to be a full permutation of the six
        // regions when laying out the dashboard.
        let mut sorted_region_order = raw.region_order.clone();
        sorted_region_order.sort();
        let mut sorted_all_regions = Region::ALL.to_vec();
        sorted_all_regions.sort();
        if sorted_region_order != sorted_all_regions {
            return Err(LayoutError::InvalidRegionOrder);
        }

        let mut layout = MapLayout {
            entries,
            region_order: raw.region_order,
            superpower_boxes,
            background,
            guests,
            undrawn: Vec::new(),
        };
        layout.undrawn = layout.compute_undrawn_links(map);
        Ok(layout)
    }

    /// Every adjacency a region's grid has no connector for: an in-region
    /// pair too far apart on the grid (as before), plus — now that guest
    /// chips exist — any cross-region or superpower adjacency with no
    /// guest chip standing in for the far side in that region's `guests`
    /// list. Run after `guests` is populated, since it reads that list.
    fn compute_undrawn_links(&self, map: &WorldMap) -> Vec<UndrawnLink> {
        let mut undrawn = Vec::new();
        for (id, country) in map.iter() {
            let region = country.region;
            let from_cell = self.cell(id);
            for &neighbor_id in &country.adjacent {
                let neighbor = map.country(neighbor_id);
                if neighbor.region == region {
                    if neighbor_id <= id {
                        continue; // each undirected in-region edge considered once
                    }
                    if !from_cell.is_adjacent(self.cell(neighbor_id)) {
                        undrawn.push(UndrawnLink { region, from: id, to: LinkTarget::Country(neighbor_id) });
                    }
                } else if !self.has_guest_neighbor(region, from_cell, GuestEntity::Country(neighbor_id)) {
                    undrawn.push(UndrawnLink { region, from: id, to: LinkTarget::Country(neighbor_id) });
                }
            }
            for &sp in &country.adjacent_superpowers {
                if !self.has_guest_neighbor(region, from_cell, GuestEntity::Superpower(sp)) {
                    undrawn.push(UndrawnLink { region, from: id, to: LinkTarget::Superpower(sp) });
                }
            }
        }
        undrawn
    }

    /// Whether `region`'s guest list has a chip for `entity` that's
    /// grid-adjacent to `from_cell` — i.e. whether a connector would
    /// actually be drawn for it.
    fn has_guest_neighbor(&self, region: Region, from_cell: Cell, entity: GuestEntity) -> bool {
        self.guests(region).iter().any(|g| g.entity == entity && from_cell.is_adjacent(g.cell))
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

    /// Adjacencies `region`'s grid has no connector for: an in-region pair
    /// too far apart on the grid, or a cross-region/superpower adjacency
    /// with no guest chip standing in for the far side. Every such link is
    /// still real; it's just footnoted below the map instead.
    pub fn undrawn_links(&self) -> &[UndrawnLink] {
        &self.undrawn
    }

    /// Every country or superpower drawn on `region`'s grid as a guest —
    /// a box standing in for an adjacency that leaves the region, so the
    /// whole map can be walked by stepping alone. Empty for a region with
    /// no cross-region or superpower neighbours at all.
    pub fn guests(&self, region: Region) -> &[Guest] {
        self.guests.get(&region).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The country reached by moving `dir` from `from` on `region`'s
    /// display grid: the nearest other country — native to `region`, or
    /// one of its guest chips — that lies strictly on `dir`'s side of
    /// `from`, or `None` if there isn't one — the selection should then
    /// hold still, the same convention as [`Region::step`]. A returned
    /// country may belong to a different region than `region` itself,
    /// when it was reached via a guest chip; the caller (`interactive.rs`)
    /// is what follows the jump by rewriting which region is on screen.
    /// A guest superpower is never a candidate — there's no screen to
    /// jump to for one — and a guest country is one only when `from`
    /// really borders it, so no step crosses a region without a border.
    ///
    /// The region grids are sparse (Africa is over half empty, with holes
    /// in the interior, not just the edges), so a bare row±1/col±1 step
    /// would often land on nothing. Picking the nearest candidate by
    /// (cross-axis distance, along-axis distance) instead reaches every
    /// country in every region with no dead ends — verified against the
    /// real grid in `data/standard_layout.json`.
    pub fn step_country(&self, map: &WorldMap, region: Region, from: CountryId, dir: Direction) -> Option<CountryId> {
        let from_cell = self.cell(from);
        let native = self.countries_in_region(map, region).into_iter().filter(|&id| id != from).map(|id| (self.cell(id), id));
        // A guest chip is only a way across if `from` really borders it: it may sit near other
        // countries on the grid without being their neighbour (Libya next to Zaire's column).
        let from_country = map.country(from);
        let guest_countries = self.guests(region).iter().filter_map(|g| match g.entity {
            GuestEntity::Country(id) if from_country.adjacent.contains(&id) => Some((g.cell, id)),
            _ => None,
        });
        native
            .chain(guest_countries)
            .filter_map(|(cell, id)| {
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

/// A guest's display label for an error message: the country's name, or
/// the superpower's own `Display`.
fn guest_label(map: &WorldMap, entity: GuestEntity) -> String {
    match entity {
        GuestEntity::Country(id) => map.country(id).name.clone(),
        GuestEntity::Superpower(sp) => sp.to_string(),
    }
}
