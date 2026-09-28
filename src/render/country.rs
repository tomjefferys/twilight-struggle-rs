use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::map::WorldMap;
use crate::ops::Operation;

use super::{control_glyph, modifier_line, nz, odds_line, Canvas, Color, Style};

/// A single country in detail: its own state plus every neighbour's, since
/// that's what deciding where to place, coup, or realign from actually
/// needs.
///
/// `op`, when it's a [`Realignment`](crate::ops::Realignment) in
/// progress, appends a block below everything else: both sides' itemised
/// modifiers for a roll on this country and the resulting odds. An
/// [`InfluencePlacement`](crate::ops::InfluencePlacement) adds nothing
/// here — its balance line has nowhere to go on this view either, so
/// `main.rs` prints it as a banner above, exactly as it already does for
/// the six-region dashboard.
pub fn render_country(map: &WorldMap, board: &Board, id: CountryId, op: Option<&Operation>) -> Canvas {
    let country = map.country(id);
    let neighbor_lines = country.adjacent.len() + country.adjacent_superpowers.len();
    let sub_region_line = if country.sub_regions.is_empty() { 0 } else { 1 };

    let realign_preview = match op {
        Some(Operation::Realign(r)) => Some((r.side(), r.preview(map, board, id))),
        _ => None,
    };
    // A blank separator row plus one line per side's modifiers and one
    // for the odds.
    let realign_rows = if realign_preview.is_some() { 4 } else { 0 };
    let height = 5 + sub_region_line + neighbor_lines + realign_rows;

    // This view was 60 columns fixed for as long as nothing on it could
    // run longer than that. The realignment odds line can, so the width
    // grows to fit it rather than risk the silent clipping every other
    // view in this crate has to fold for.
    let content_width = match &realign_preview {
        Some((side, (acting, opposing, odds))) => {
            let lines = [modifier_line(*side, acting), modifier_line(side.opponent(), opposing), odds_line(*side, odds)];
            lines.iter().map(|l| l.chars().count()).max().unwrap_or(60).max(60)
        }
        None => 60,
    };
    let mut canvas = Canvas::new(content_width, height.max(5));

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
        // Turn a realignment's "adjacent controlled" modifier from a bare
        // number into a visible derivation: mark exactly the neighbours
        // that are actually supplying it.
        if let Some((side, _)) = &realign_preview
            && board.is_controlled_by(map, neighbor_id, *side)
        {
            canvas.put(row, 42, "+1 realign", Style::color(Color::Selected));
        }
        row += 1;
    }
    for &sp in &country.adjacent_superpowers {
        canvas.put(row, 2, &format!("{sp} (superpower)"), Style::color(Color::Muted));
        row += 1;
    }

    if let Some((side, (acting, opposing, odds))) = &realign_preview {
        row += 1;
        canvas.put(row, 0, &modifier_line(*side, acting), Style::color(Color::Selected));
        row += 1;
        canvas.put(row, 0, &modifier_line(side.opponent(), opposing), Style::color(Color::Muted));
        row += 1;
        canvas.put(row, 0, &odds_line(*side, odds), Style::color(Color::Muted));
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
