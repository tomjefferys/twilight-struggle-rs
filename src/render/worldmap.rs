use std::collections::HashMap;

use crate::board::Board;
use crate::country::{Region, Superpower};
use crate::layout::MapLayout;
use crate::map::WorldMap;
use crate::ops::InfluencePlacement;

use super::{placement_balance_line, region_tally, Canvas, Color, Style};

/// Shown below the legend when a region is selected but no placement is
/// in progress.
const WORLD_HINT: &str = "←→↑↓ select · Enter open · Esc back";

/// Shown instead once an [`InfluencePlacement`] is in progress.
const WORLD_PLACEMENT_HINT: &str = "←→↑↓ select · Enter open · u undo · c confirm · Esc back";

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
///
/// `selected`, when set, is the region currently picked in interactive
/// navigation: its landmass and country chips are drawn bold and at full
/// brightness rather than dimmed, and two extra lines below the legend
/// name it and give the key hints. Passing `None` reproduces the plain,
/// static view exactly — no extra rows, no style changes — so every
/// existing caller is unaffected.
///
/// `placement`, when set, is an influence placement in progress: every
/// country with pending influence shows a `+` in place of its flag, and
/// an extra footer line shows the side, its ops balance, and where it's
/// placed so far.
pub fn render_world_map(
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    selected: Option<Region>,
    placement: Option<&InfluencePlacement>,
) -> Canvas {
    // While a placement is in progress, every reader should see its
    // speculative board rather than the caller's, so control colours and
    // (were they ever added) influence figures stay live as points are
    // staged.
    let board = placement.map_or(board, InfluencePlacement::board);
    let background = layout.background();
    let height = background.len();
    let width = background.iter().map(|l| l.chars().count()).max().unwrap_or(0);

    let legend = superpower_legend(map);
    let balance_line = placement.map(|p| placement_balance_line(layout, p));
    // A selected region's footer is normally 2 rows (title + hint); an
    // active placement adds the balance line to it. With no region
    // selected there's no title to show, so an active placement (an edge
    // case outside interactive use, where the two always go together)
    // just gets the one balance row.
    let footer_rows = if selected.is_some() {
        2 + usize::from(balance_line.is_some())
    } else {
        usize::from(balance_line.is_some())
    };
    // Unlike `render_region`, this view's width previously left the
    // selection footer's own strings out of the fold entirely — harmless
    // while that footer never grew, but the placement balance line can be
    // long, so it (and the hints) must be included here or `Canvas` will
    // silently clip them.
    let content_width = legend
        .iter()
        .map(|l| l.chars().count())
        .chain(balance_line.iter().map(|l| l.chars().count()))
        .chain([width, WORLD_HINT.chars().count(), WORLD_PLACEMENT_HINT.chars().count()])
        .max()
        .unwrap_or(width);

    let mut canvas = Canvas::new(content_width, height + 1 + legend.len() + footer_rows);

    // Tint each patch of land by whichever zone is closest to it, so the
    // landmass reads like the physical board's coloured areas rather than
    // a flat grey silhouette. Sea is left untinted. The selected region's
    // own land is drawn bold and undimmed so it visibly stands out; every
    // other zone keeps today's dim treatment.
    let zones = tint_zones(map, layout);
    for (row, line) in background.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let zone = home_territory(row, col).unwrap_or_else(|| nearest_zone(&zones, row, col));
            let style = match zone {
                Zone::Region(r) if selected == Some(r) => Style::color(zone_color(zone)).bold(),
                _ => Style::color(zone_color(zone)).dim(),
            };
            canvas.put_char(row, col, ch, style);
        }
    }

    for sp in [Superpower::Us, Superpower::Ussr] {
        let b = layout.superpower_box(sp);
        draw_superpower_box(&mut canvas, (b.cell.row as usize, b.cell.col as usize), (b.rows, b.cols), sp);
    }

    for (id, country) in map.iter() {
        let cell = layout.world_cell(id);
        let bold = selected == Some(country.region);
        let pending = placement.is_some_and(|p| p.pending(id) > 0);
        draw_chip(&mut canvas, cell.row as usize, cell.col as usize, map, layout, board, id, bold, pending);
    }

    for (i, line) in legend.iter().enumerate() {
        canvas.put(height + 1 + i, 0, line, Style::color(Color::Muted));
    }

    let frow = height + 1 + legend.len();
    if let Some(region) = selected {
        draw_selection_footer(&mut canvas, frow, map, layout, board, region, placement);
    } else if let Some(line) = &balance_line {
        canvas.put(frow, 0, line, Style::color(Color::Selected));
    }

    canvas
}

/// A patch of the background art: either a scoring region (tinted by that
/// region's board colour) or a superpower's own home territory (tinted by
/// that superpower's colour, matching its box).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Region(Region),
    Superpower(Superpower),
}

/// The board colour for a zone — a scoring region's matches the physical
/// *Twilight Struggle* board, independent of any country's control state;
/// a superpower's matches its own box.
fn zone_color(zone: Zone) -> Color {
    match zone {
        Zone::Region(Region::Europe) => Color::Europe,
        Zone::Region(Region::Asia) => Color::Asia,
        Zone::Region(Region::MiddleEast) => Color::MiddleEast,
        Zone::Region(Region::Africa) => Color::Africa,
        Zone::Region(Region::CentralAmerica) => Color::CentralAmerica,
        Zone::Region(Region::SouthAmerica) => Color::SouthAmerica,
        Zone::Superpower(Superpower::Us) => Color::Us,
        Zone::Superpower(Superpower::Ussr) => Color::Ussr,
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

/// The strip of land directly south of the USSR box, east of
/// Romania/Bulgaria (column 103) and north of Turkey/Iraq (row 11) —
/// roughly where real-world Kazakhstan sits. It has no country of its
/// own, and the nearest real country to it is usually Iraq or Turkey, so
/// a plain Voronoi tessellation tinted it Middle East — visually reading
/// as the Middle East abutting the USSR box directly, with no Asia in
/// between. Since this land sits right against USSR territory rather
/// than the Middle East's own cluster, it reads better as Asia. Bounded
/// to two rows (9–10) so it stops before Turkey's own row (11).
const CENTRAL_ASIA_GAP: ((usize, usize), (usize, usize)) = ((9, 10), (104, usize::MAX));

fn home_territory(row: usize, col: usize) -> Option<Zone> {
    let in_rect = |((r0, r1), (c0, c1)): ((usize, usize), (usize, usize))| {
        row >= r0 && row <= r1 && col >= c0 && col <= c1
    };
    if in_rect(CANADA_TERRITORY) {
        Some(Zone::Region(Region::Europe))
    } else if in_rect(USA_TERRITORY) {
        Some(Zone::Superpower(Superpower::Us))
    } else if in_rect(USSR_TERRITORY) {
        Some(Zone::Superpower(Superpower::Ussr))
    } else if in_rect(CENTRAL_ASIA_GAP) {
        Some(Zone::Region(Region::Asia))
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
fn tint_zones(map: &WorldMap, layout: &MapLayout) -> Vec<(isize, isize, Zone)> {
    map.iter()
        .map(|(id, country)| {
            let cell = layout.world_cell(id);
            (cell.row as isize, cell.col as isize, Zone::Region(country.region))
        })
        .collect()
}

/// The zone of whichever anchor is geographically closest to `(row, col)`.
fn nearest_zone(zones: &[(isize, isize, Zone)], row: usize, col: usize) -> Zone {
    zones
        .iter()
        .min_by_key(|&&(r, c, _)| {
            let dr = r - row as isize;
            let dc = c - col as isize;
            dr * dr + dc * dc
        })
        .map(|&(_, _, zone)| zone)
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

/// `bold` is set for a country in the currently-selected region, so its
/// chip stays legible against the brightened land beneath it. `pending`
/// marks a country with staged (uncommitted) influence: since a chip is
/// only ever `flag + code`, there's no spare column for a count, so the
/// flag slot is overridden with `+` — trading away the battleground flag
/// for the duration of the placement, which is an acceptable loss on this
/// low-detail overview (the region view keeps both).
#[allow(clippy::too_many_arguments)]
fn draw_chip(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    id: crate::country::CountryId,
    bold: bool,
    pending: bool,
) {
    let country = map.country(id);
    let controller = board.controller(map, id);
    let mut style = match controller {
        Some(Superpower::Us) => Style::color(Color::Us),
        Some(Superpower::Ussr) => Style::color(Color::Ussr),
        None if country.battleground => Style::color(Color::Battleground),
        None => Style::default(),
    };
    if bold || pending {
        style = style.bold();
    }
    // Centre the chip (flag + code) on its geographic point rather than
    // left-aligning from it — left-aligned, every label reads as sitting
    // to the right of where it's actually placed, which is most obvious
    // where countries are dense (Europe).
    let code = layout.code(id);
    let total = code.chars().count() + 1;
    let start = col.saturating_sub(total / 2);
    let flag = if pending {
        '+'
    } else if country.battleground {
        '*'
    } else {
        ' '
    };
    canvas.put_char(row, start, flag, style);
    canvas.put(row, start + 1, code, style);
}

/// The two lines shown below the legend once a region is selected: its
/// name (upper-cased, bold, in its own region colour) with the same
/// battleground/country tallies a dashboard panel shows, then the key
/// hints for interactive navigation.
#[allow(clippy::too_many_arguments)]
fn draw_selection_footer(
    canvas: &mut Canvas,
    row: usize,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    region: Region,
    placement: Option<&InfluencePlacement>,
) {
    let tally = region_tally(map, layout, board, region);
    let name = region.to_string().to_uppercase();
    let title = format!(
        "▸ {name} ◂   bg {}-{}  ctry {}-{}",
        tally.bg_us, tally.bg_ussr, tally.ctry_us, tally.ctry_ussr
    );
    let mut r = row;
    canvas.put(r, 0, &title, Style::color(zone_color(Zone::Region(region))).bold());
    r += 1;
    if let Some(p) = placement {
        canvas.put(r, 0, &placement_balance_line(layout, p), Style::color(Color::Selected));
        r += 1;
    }
    let hint = if placement.is_some() { WORLD_PLACEMENT_HINT } else { WORLD_HINT };
    canvas.put(r, 0, hint, Style::color(Color::Muted));
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
