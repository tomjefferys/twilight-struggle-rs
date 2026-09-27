use std::collections::HashMap;

use crate::board::Board;
use crate::country::{Region, Superpower};
use crate::layout::MapLayout;
use crate::map::WorldMap;

use super::{Canvas, Color, Style};

/// The whole world on one grid, drawn to actually look like a map: real
/// landmass shading underneath (rasterized once from public-domain
/// coastline data — see [`MapLayout::background`]), with every country a
/// single-line chip placed at its real approximate position on top. USA
/// and USSR are labelled boxes rather than points, since each borders
/// several countries spread across regions.
///
/// Deliberately no connectors of any kind — a line here would invite
/// reading it as a real *Twilight Struggle* adjacency, which is exactly
/// the confusion this view exists to avoid. That precision lives
/// exclusively in `render_region`; this is the low-detail, geographic
/// overview.
pub fn render_world_map(map: &WorldMap, layout: &MapLayout, board: &Board) -> Canvas {
    let background = layout.background();
    let height = background.len();
    let width = background.iter().map(|l| l.chars().count()).max().unwrap_or(0);

    let legend = superpower_legend(map);
    let content_width = legend.iter().map(|l| l.chars().count()).chain([width]).max().unwrap_or(width);

    let mut canvas = Canvas::new(content_width, height + 1 + legend.len());

    // Tint each patch of land by whichever colour zone is closest to it,
    // so the landmass reads like the physical board's coloured areas
    // rather than a flat grey silhouette. Sea is left untinted.
    let zones = tint_zones(map, layout);
    for (row, line) in background.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let color = home_territory(row, col).unwrap_or_else(|| nearest_zone(&zones, row, col));
            canvas.put_char(row, col, ch, Style::color(color).dim());
        }
    }

    for sp in [Superpower::Us, Superpower::Ussr] {
        let b = layout.superpower_box(sp);
        draw_superpower_box(&mut canvas, (b.cell.row as usize, b.cell.col as usize), (b.rows, b.cols), sp);
    }

    for (id, _) in map.iter() {
        let cell = layout.world_cell(id);
        draw_chip(&mut canvas, cell.row as usize, cell.col as usize, map, layout, board, id);
    }

    for (i, line) in legend.iter().enumerate() {
        canvas.put(height + 1 + i, 0, line, Style::color(Color::Muted));
    }

    canvas
}

/// The board colour for a scoring region — matches the physical
/// *Twilight Struggle* board, independent of any country's control state.
fn region_color(region: Region) -> Color {
    match region {
        Region::Europe => Color::Europe,
        Region::Asia => Color::Asia,
        Region::MiddleEast => Color::MiddleEast,
        Region::Africa => Color::Africa,
        Region::CentralAmerica => Color::CentralAmerica,
        Region::SouthAmerica => Color::SouthAmerica,
    }
}

/// Canada's landmass on the background art: real Canada is a single point
/// (`world_cell` row 5), but the raster draws a landmass far wider than
/// that one point can dominate by nearest-neighbour alone — e.g. the
/// eastern coastline is column-close enough to the Caribbean's countries
/// that a plain Voronoi tessellation tinted it Central America instead.
/// Since Canada is the only real country anywhere in this rectangle and
/// it's scored as Europe, the whole area is forced to Europe outright
/// rather than left to compete. Bounds checked against every country's
/// `world_cell` in `data/standard_layout.json`.
const CANADA_TERRITORY: ((usize, usize), (usize, usize)) = ((0, 9), (0, 72));

/// The USA's own landmass on the background art, south of Canada: this
/// land belongs to no scoring region at all, so it's carved out as USA
/// territory rather than left to compete in the region Voronoi below.
/// Bounds checked against every country's `world_cell` in
/// `data/standard_layout.json`: nothing else on the map falls inside this
/// rectangle.
const USA_TERRITORY: ((usize, usize), (usize, usize)) = ((10, 15), (0, 72));

/// The same idea for USSR: the background art draws a huge stretch of
/// undifferentiated Siberian landmass around and below the USSR box that
/// has no country of its own, and left to a plain Voronoi tessellation it
/// reads as whichever real region happens to have the nearest country
/// (Middle East, usually, since Turkey/Iraq/Iran sit at a similar
/// longitude) rather than as USSR territory. Bounded to stop just short
/// of Finland (column 98, the westernmost real country anywhere near this
/// band) and well above Romania/Bulgaria/Turkey (row 9 onward), so no
/// real region's country is ever inside this rectangle.
const USSR_TERRITORY: ((usize, usize), (usize, usize)) = ((0, 8), (100, usize::MAX));

fn home_territory(row: usize, col: usize) -> Option<Color> {
    let in_rect = |((r0, r1), (c0, c1)): ((usize, usize), (usize, usize))| {
        row >= r0 && row <= r1 && col >= c0 && col <= c1
    };
    if in_rect(CANADA_TERRITORY) {
        Some(Color::Europe)
    } else if in_rect(USA_TERRITORY) {
        Some(Color::Us)
    } else if in_rect(USSR_TERRITORY) {
        Some(Color::Ussr)
    } else {
        None
    }
}

/// One anchor point per country, at its own `world_cell`, coloured by its
/// *scoring* region rather than by geography — this is what keeps a
/// country like Turkey (geographically in the Middle East, but scored as
/// Europe in *Twilight Struggle*) tinted as its own real region right at
/// its own position: nearest-neighbour always resolves to a country's own
/// point first, at zero distance, before it ever reaches for a
/// neighbour's. A single per-region centroid can't offer that guarantee
/// — Turkey/Bulgaria/Romania all sit geographically closer to the Middle
/// East cluster than to the bulk of Europe, so a centroid-only lookup
/// tinted them Middle East despite their region being Europe. Only the
/// genuinely empty land between countries is left to a nearest-neighbour
/// guess, which is a far smaller and less consequential source of error.
fn tint_zones(map: &WorldMap, layout: &MapLayout) -> Vec<(isize, isize, Color)> {
    map.iter()
        .map(|(id, country)| {
            let cell = layout.world_cell(id);
            (cell.row as isize, cell.col as isize, region_color(country.region))
        })
        .collect()
}

/// The colour of whichever zone anchor is geographically closest to
/// `(row, col)`.
fn nearest_zone(zones: &[(isize, isize, Color)], row: usize, col: usize) -> Color {
    zones
        .iter()
        .min_by_key(|&&(r, c, _)| {
            let dr = r - row as isize;
            let dc = c - col as isize;
            dr * dr + dc * dc
        })
        .map(|&(_, _, color)| color)
        .expect("at least one tint zone exists")
}

fn draw_superpower_box(canvas: &mut Canvas, (y, x): (usize, usize), (rows, cols): (u8, u8), sp: Superpower) {
    let h = (rows as usize).max(3);
    let w = (cols as usize).max(sp.to_string().chars().count() + 2);
    let style = Style::color(match sp {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    });
    canvas.draw_box(y, x, w, h, style);
    let label = sp.to_string();
    let label_x = x + w.saturating_sub(label.chars().count()) / 2;
    let label_y = y + h / 2;
    canvas.put(label_y, label_x, &label, style);
}

fn draw_chip(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    id: crate::country::CountryId,
) {
    let country = map.country(id);
    let controller = board.controller(map, id);
    let style = match controller {
        Some(Superpower::Us) => Style::color(Color::Us),
        Some(Superpower::Ussr) => Style::color(Color::Ussr),
        None if country.battleground => Style::color(Color::Battleground),
        None => Style::default(),
    };
    // Centre the chip (flag + code) on its geographic point rather than
    // left-aligning from it — left-aligned, every label reads as sitting
    // to the right of where it's actually placed, which is most obvious
    // where countries are dense (Europe).
    let code = layout.code(id);
    let total = code.chars().count() + 1;
    let start = col.saturating_sub(total / 2);
    canvas.put_char(row, start, if country.battleground { '*' } else { ' ' }, style);
    canvas.put(row, start + 1, code, style);
}

/// One line per superpower listing its real bordering countries — with no
/// connectors anywhere on this view, this is the only place either
/// superpower's actual adjacency is represented.
fn superpower_legend(map: &WorldMap) -> Vec<String> {
    let mut borders: HashMap<Superpower, Vec<&str>> = HashMap::new();
    for (_, country) in map.iter() {
        for &sp in &country.adjacent_superpowers {
            borders.entry(sp).or_default().push(&country.name);
        }
    }
    let mut lines = Vec::new();
    for sp in [Superpower::Us, Superpower::Ussr] {
        if let Some(names) = borders.get(&sp) {
            lines.push(format!("{sp}: {}", names.join(", ")));
        }
    }
    lines
}
