use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::map::WorldMap;

use super::{control_glyph, nz, Canvas, Color, Style};

/// A single country in detail: its own state plus every neighbour's, since
/// that's what deciding where to place, coup, or realign from actually
/// needs.
pub fn render_country(map: &WorldMap, board: &Board, id: CountryId) -> Canvas {
    let country = map.country(id);
    let neighbor_lines = country.adjacent.len() + country.adjacent_superpowers.len();
    let sub_region_line = if country.sub_regions.is_empty() { 0 } else { 1 };
    let height = 5 + sub_region_line + neighbor_lines;
    let mut canvas = Canvas::new(60, height.max(5));

    let title_style = if country.battleground {
        Style::color(Color::Battleground).bold()
    } else {
        Style::default().bold()
    };
    canvas.put(
        0,
        0,
        &format!(
            "{}{}",
            if country.battleground { "* " } else { "" },
            country.name
        ),
        title_style,
    );
    canvas.put(1, 0, &format!("{}   stability {}", country.region, country.stability), Style::default());

    let mut row = 2;
    if !country.sub_regions.is_empty() {
        let names: Vec<String> = country.sub_regions.iter().map(|s| s.to_string()).collect();
        canvas.put(row, 0, &format!("sub-regions: {}", names.join(", ")), Style::color(Color::Muted));
        row += 1;
    }

    let controller = board.controller(map, id);
    let control_str = match controller {
        Some(Superpower::Us) => "US control".to_string(),
        Some(Superpower::Ussr) => "USSR control".to_string(),
        None => "contested".to_string(),
    };
    canvas.put(row, 0, "influence   ", Style::default());
    let after = draw_influence(&mut canvas, row, 12, board, id, controller);
    canvas.put(row, after + 3, &control_str, Style::default());
    row += 2;

    canvas.put(row - 1, 0, "neighbours:", Style::default());
    for &neighbor_id in &country.adjacent {
        let n = map.country(neighbor_id);
        let nctl = board.controller(map, neighbor_id);
        canvas.put(row, 2, &format!("{:<20}", n.name), Style::default());
        draw_influence(&mut canvas, row, 22, board, neighbor_id, nctl);
        row += 1;
    }
    for &sp in &country.adjacent_superpowers {
        canvas.put(row, 2, &format!("{sp} (superpower)"), Style::color(Color::Muted));
        row += 1;
    }

    canvas
}

/// Draws `US <n>  <glyph>  USSR <n>` at `(row, col)`, colouring each part
/// by the same convention as the world and region views, and returns the
/// column just past the end of what it wrote.
fn draw_influence(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    board: &Board,
    id: CountryId,
    controller: Option<Superpower>,
) -> usize {
    let us = board.influence(id, Superpower::Us);
    let ussr = board.influence(id, Superpower::Ussr);
    let us_style = if us > 0 { Style::color(Color::Us) } else { Style::color(Color::Muted) };
    let ussr_style = if ussr > 0 { Style::color(Color::Ussr) } else { Style::color(Color::Muted) };
    let sep_style = match controller {
        Some(Superpower::Us) => Style::color(Color::Us),
        Some(Superpower::Ussr) => Style::color(Color::Ussr),
        None => Style::default(),
    };

    canvas.put(row, col, "US ", Style::default());
    canvas.put(row, col + 3, &format!("{:>2}", nz(us)), us_style);
    canvas.put(row, col + 6, "  ", Style::default());
    canvas.put_char(row, col + 8, control_glyph(controller), sep_style);
    canvas.put(row, col + 10, "  USSR ", Style::default());
    canvas.put(row, col + 17, &format!("{:<2}", nz(ussr)), ussr_style);
    col + 19
}
