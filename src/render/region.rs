use crate::board::Board;
use crate::country::{CountryId, Region, Superpower};
use crate::layout::MapLayout;
use crate::map::WorldMap;

use super::{control_glyph, nz, Canvas, Color, Style};

/// Box dimensions and grid pitch for the region zoom view. A 1-column,
/// 1-row gap between boxes leaves room for connector glyphs.
const BOX_W: usize = 14;
const BOX_H: usize = 4;
const PITCH_COL: usize = BOX_W + 1;
const PITCH_ROW: usize = BOX_H + 1;
const LEFT_MARGIN: usize = 2;
const TOP_MARGIN: usize = 2; // title line + blank

/// A geographic zoom into one region: every country in it drawn as a box
/// on its layout grid cell, connected to its in-region neighbours by line
/// glyphs. Adjacencies that leave the region (to another region, or to a
/// superpower) are footnoted below the map, along with any in-region
/// adjacency the grid couldn't draw a connector for.
pub fn render_region(map: &WorldMap, layout: &MapLayout, board: &Board, region: Region) -> Canvas {
    let ids = layout.countries_in_region(map, region);
    let (max_row, max_col) = ids.iter().fold((0u8, 0u8), |(mr, mc), &id| {
        let cell = layout.cell(id);
        (mr.max(cell.row), mc.max(cell.col))
    });

    let grid_width = LEFT_MARGIN + (max_col as usize + 1) * PITCH_COL;
    let grid_height = TOP_MARGIN + (max_row as usize + 1) * PITCH_ROW;

    let off_region_links = collect_off_region_links(map, &ids, region);
    let undrawn = undrawn_links_for(map, layout, region);

    let mut extra_lines = 0;
    if !off_region_links.is_empty() {
        extra_lines += 1 + off_region_links.len();
    }
    if !undrawn.is_empty() {
        extra_lines += 1 + undrawn.len();
    }
    let height = grid_height + if extra_lines > 0 { extra_lines + 1 } else { 0 };

    let bg_us = ids
        .iter()
        .filter(|&&id| map.country(id).battleground && board.is_controlled_by(map, id, Superpower::Us))
        .count();
    let bg_ussr = ids
        .iter()
        .filter(|&&id| map.country(id).battleground && board.is_controlled_by(map, id, Superpower::Ussr))
        .count();
    let ctry_us = ids.iter().filter(|&&id| board.is_controlled_by(map, id, Superpower::Us)).count();
    let ctry_ussr = ids.iter().filter(|&&id| board.is_controlled_by(map, id, Superpower::Ussr)).count();
    let title = format!(
        "{} — battlegrounds US {}/USSR {}   countries US {}/USSR {}",
        region, bg_us, bg_ussr, ctry_us, ctry_ussr
    );

    // Never shrink the canvas below what the title or footnotes need just
    // because the grid itself is narrower (South America, at 3 columns
    // wide, is narrower than its own title line).
    let content_width = off_region_links
        .iter()
        .chain(undrawn.iter())
        .map(|line| line.chars().count())
        .chain([title.chars().count(), grid_width])
        .max()
        .unwrap_or(grid_width);

    let mut canvas = Canvas::new(content_width, height);
    canvas.put(0, 0, &title, Style::default());

    for &id in &ids {
        let cell = layout.cell(id);
        let y = TOP_MARGIN + cell.row as usize * PITCH_ROW;
        let x = LEFT_MARGIN + cell.col as usize * PITCH_COL;
        draw_country_box(&mut canvas, y, x, map, layout, board, id);
    }

    draw_connectors(&mut canvas, map, layout, &ids, region);

    let mut row = grid_height;
    if !off_region_links.is_empty() {
        row += 1;
        for line in &off_region_links {
            canvas.put(row, LEFT_MARGIN, line, Style::color(Color::Muted));
            row += 1;
        }
    }
    if !undrawn.is_empty() {
        row += 1;
        for line in &undrawn {
            canvas.put(row, LEFT_MARGIN, line, Style::color(Color::Muted));
            row += 1;
        }
    }

    canvas
}

fn draw_country_box(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    id: CountryId,
) {
    let country = map.country(id);
    canvas.draw_box(row, col, BOX_W, BOX_H, Style::default());

    let flag_style = if country.battleground {
        Style::color(Color::Battleground)
    } else {
        Style::default()
    };
    canvas.put_char(row + 1, col + 1, if country.battleground { '*' } else { ' ' }, flag_style);
    canvas.put(row + 1, col + 2, layout.short_name(id), Style::default());

    let us = board.influence(id, Superpower::Us);
    let ussr = board.influence(id, Superpower::Ussr);
    let controller = board.controller(map, id);
    let sep = control_glyph(controller);
    let sep_style = match controller {
        Some(Superpower::Us) => Style::color(Color::Us),
        Some(Superpower::Ussr) => Style::color(Color::Ussr),
        None => Style::default(),
    };
    let us_style = if us > 0 { Style::color(Color::Us) } else { Style::color(Color::Muted) };
    let ussr_style = if ussr > 0 { Style::color(Color::Ussr) } else { Style::color(Color::Muted) };

    canvas.put(row + 2, col + 1, &format!("{:>2}", nz(us)), us_style);
    canvas.put_char(row + 2, col + 3, sep, sep_style);
    canvas.put(row + 2, col + 4, &format!("{:<2}", nz(ussr)), ussr_style);
    canvas.put(row + 2, col + 8, &format!("st{}", country.stability), Style::color(Color::Muted));
}

/// Draws a connector glyph in the gap between every pair of grid-adjacent,
/// really-adjacent countries. When two diagonal connectors would land on
/// the same corner cell, they're merged into `╳` rather than one silently
/// overwriting the other.
fn draw_connectors(canvas: &mut Canvas, map: &WorldMap, layout: &MapLayout, ids: &[CountryId], region: Region) {
    use std::collections::HashMap;
    let mut glyphs: HashMap<(usize, usize), char> = HashMap::new();

    let mut put_glyph = |pos: (usize, usize), glyph: char| {
        glyphs
            .entry(pos)
            .and_modify(|existing| {
                *existing = match (*existing, glyph) {
                    ('╲', '╱') | ('╱', '╲') => '╳',
                    (a, _) => a,
                };
            })
            .or_insert(glyph);
    };

    let mut done = std::collections::HashSet::new();
    for &id in ids {
        let country = map.country(id);
        for &neighbor_id in &country.adjacent {
            if map.country(neighbor_id).region != region {
                continue;
            }
            let key = (id.min(neighbor_id), id.max(neighbor_id));
            if !done.insert(key) {
                continue;
            }
            let a = layout.cell(id);
            let b = layout.cell(neighbor_id);
            let (dr, dc) = (
                b.row as isize - a.row as isize,
                b.col as isize - a.col as isize,
            );
            if dr.abs().max(dc.abs()) != 1 {
                continue; // not grid-adjacent; footnoted separately
            }
            let ay = TOP_MARGIN + a.row as usize * PITCH_ROW;
            let ax = LEFT_MARGIN + a.col as usize * PITCH_COL;
            let by = TOP_MARGIN + b.row as usize * PITCH_ROW;
            let bx = LEFT_MARGIN + b.col as usize * PITCH_COL;
            if dr == 0 {
                // horizontal neighbours: connector in the column gap
                put_glyph((ay + 2, ax.min(bx) + BOX_W), '─');
            } else if dc == 0 {
                // vertical neighbours: connector in the row gap
                put_glyph((ay.min(by) + BOX_H, ax + BOX_W / 2), '│');
            } else {
                // diagonal neighbours: connector at the shared corner
                let glyph = if (dr > 0) == (dc > 0) { '╲' } else { '╱' };
                put_glyph((ay.min(by) + BOX_H, ax.min(bx) + BOX_W), glyph);
            }
        }
    }

    for (&(y, x), &glyph) in &glyphs {
        canvas.put_char(y, x, glyph, Style::color(Color::Muted));
    }
}

fn collect_off_region_links(map: &WorldMap, ids: &[CountryId], region: Region) -> Vec<String> {
    let mut lines = Vec::new();
    for &id in ids {
        let country = map.country(id);
        for &neighbor_id in &country.adjacent {
            let neighbor = map.country(neighbor_id);
            if neighbor.region != region {
                lines.push(format!(
                    "  ↔ {} – {} ({})",
                    country.name, neighbor.name, neighbor.region
                ));
            }
        }
        for &sp in &country.adjacent_superpowers {
            lines.push(format!("  ↔ {} – {}", country.name, sp));
        }
    }
    lines
}

fn undrawn_links_for(map: &WorldMap, layout: &MapLayout, region: Region) -> Vec<String> {
    layout
        .undrawn_links()
        .iter()
        .filter(|(a, _)| map.country(*a).region == region)
        .map(|(a, b)| {
            format!(
                "  (not drawn on this grid: {} – {})",
                map.country(*a).name,
                map.country(*b).name
            )
        })
        .collect()
}
