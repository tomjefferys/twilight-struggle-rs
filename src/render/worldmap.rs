use std::collections::HashMap;

use crate::board::Board;
use crate::country::Superpower;
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

    for (row, line) in background.iter().enumerate() {
        canvas.put(row, 0, line, Style::color(Color::Muted));
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
