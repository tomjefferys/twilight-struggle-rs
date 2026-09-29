use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::layout::MapLayout;
use crate::map::WorldMap;
use crate::ops::Operation;

use super::{
    control_glyph, coup_odds_line, coup_target_line, modifier_line, nz, odds_line, operation_header, operation_touched_line,
    put_border_title, Canvas, Color, Style, ViewMode,
};

/// Shown below the box, only under [`ViewMode::Interactive`] — a static
/// print into scrollback has no keys to hint at.
const HINT: &str = "←→↑↓ select · Esc back";
const PLACEMENT_HINT: &str = "←→↑↓ select · + place · u undo · c confirm · Esc back";
const REALIGN_HINT: &str = "←→↑↓ select · r roll · c done · Esc back";
const COUP_HINT: &str = "←→↑↓ select · r coup · c done · Esc back";

/// A single country in detail, as a titled box with up to three panels:
/// the country itself, its neighbours, and — while an operation is open —
/// that operation's live preview. This is the interactive world map's
/// third screen (opened from the region view with `Enter` or `r`) and
/// also the REPL's `country <name>` / `/<name>` command.
///
/// `op`, when it's a [`Realignment`](crate::ops::Realignment) in
/// progress, adds an Operation panel with both sides' itemised modifiers
/// for a roll on this country and the resulting odds. A
/// [`Coup`](crate::ops::Coup) in progress adds the target number
/// (stability × 2) and success odds instead — a coup has no per-side
/// modifiers to itemise. An [`InfluencePlacement`](crate::ops::InfluencePlacement)
/// adds the cost of placing here and how much is already pending. Any
/// operation with a speculative board (a placement) is read through it,
/// the same substitution `render_region`/`render_world_map` already make,
/// so pending influence shows here too.
///
/// `mode` controls whether a key-hint row is drawn below the box —
/// [`ViewMode::Interactive`] only, since a one-shot REPL print has no
/// keyboard listening on the other end.
pub fn render_country(map: &WorldMap, layout: &MapLayout, board: &Board, id: CountryId, op: Option<&Operation>, mode: ViewMode) -> Canvas {
    let board = op.and_then(Operation::board).unwrap_or(board);
    let country = map.country(id);

    // --- Panel 1: Country. Its title lives on the box's own top border
    // rather than a divider, so it's built as a (left, right) pair. ---
    let title_left = if country.battleground { format!("* {}", country.name) } else { country.name.clone() };
    let title_right = format!("{} · stability {}", country.region, country.stability);
    let sub_region_line = if country.sub_regions.is_empty() {
        None
    } else {
        let names: Vec<String> = country.sub_regions.iter().map(|s| s.to_string()).collect();
        Some(format!("sub-regions: {}", names.join(", ")))
    };
    let country_row_count = 1 + sub_region_line.is_some() as usize;

    // --- Panel 2: Neighbours. Each neighbour line needs its own
    // multi-style influence readout, so these are drawn directly rather
    // than collected as (String, Style) pairs like the other panels. ---
    let realign_side = match op {
        Some(Operation::Realign(r)) => Some(r.side()),
        _ => None,
    };

    // --- Panel 3: Operation, only when one is open. Every one of these
    // rows is single-style, so — unlike the two panels above — they're
    // collected up front as plain (String, Style) pairs. ---
    let op_panel: Option<(String, Vec<(String, Style)>)> = op.map(|operation| {
        let mut rows = vec![(operation_touched_line(layout, board, operation), Style::color(Color::Muted))];
        if !operation.is_legal_target(map, board, id) {
            let reason = match operation {
                Operation::Influence(_) => "no presence or adjacency here".to_string(),
                Operation::Realign(_) | Operation::Coup(_) => {
                    format!("no {} influence to remove", operation.side().opponent())
                }
            };
            rows.push((reason, Style::color(Color::Muted)));
        }
        match operation {
            Operation::Influence(p) => {
                let cost = p.cost(map, id);
                rows.push((format!("{}  costs {cost} op{}", p.side(), if cost == 1 { "" } else { "s" }), Style::color(Color::Selected)));
                let pending = p.pending(id);
                if pending > 0 {
                    rows.push((format!("{pending} placed here"), Style::color(Color::Muted)));
                }
            }
            Operation::Realign(r) => {
                let (acting, opposing, odds) = r.preview(map, board, id);
                rows.push((modifier_line(r.side(), &acting), Style::color(Color::Selected)));
                rows.push((modifier_line(r.side().opponent(), &opposing), Style::color(Color::Muted)));
                rows.push((odds_line(r.side(), &odds), Style::color(Color::Muted)));
            }
            Operation::Coup(c) => {
                let (target_number, odds) = c.preview(map, board, id);
                rows.push((coup_target_line(c.side(), c.ops_total(), target_number, country.stability), Style::color(Color::Selected)));
                rows.push((coup_odds_line(c.side(), &odds), Style::color(Color::Muted)));
            }
        }
        (operation_header(operation), rows)
    });

    let hint = (mode == ViewMode::Interactive).then_some(match op {
        Some(Operation::Influence(_)) => PLACEMENT_HINT,
        Some(Operation::Realign(_)) => REALIGN_HINT,
        Some(Operation::Coup(_)) => COUP_HINT,
        None => HINT,
    });

    // --- Sizing. Every panel's content is already known above; take the
    // width the widest of it needs, since `Canvas` clips silently rather
    // than erroring on an under-sized write. ---
    let neighbour_rows = country.adjacent.len() + country.adjacent_superpowers.len();
    let box_height = 2 // top + bottom border
        + country_row_count
        + 1 + neighbour_rows // "Neighbours" divider + its rows
        + op_panel.as_ref().map_or(0, |(_, rows)| 1 + rows.len()); // "{header}" divider + its rows
    let height = box_height + hint.is_some() as usize;

    let neighbour_name_width = country.adjacent.iter().map(|&n| map.country(n).name.chars().count()).max().unwrap_or(0);
    // Left margin, the name field, the gap before it, and `draw_influence`'s
    // own fixed 19-column readout — then either a plain right margin, or,
    // while a realignment is open, room for the "+1 realign" marker too.
    const INFLUENCE_WIDTH: usize = 19;
    let neighbour_width = 2 + neighbour_name_width + 2 + INFLUENCE_WIDTH
        + if realign_side.is_some() { 2 + "+1 realign".chars().count() + 2 } else { 2 };
    let content_width = [
        // Left/right border margins (2 each), the space-padded left and
        // right title halves, and a one-column gap of bare dash between
        // them so the two never touch.
        4 + (title_left.chars().count() + 2) + (title_right.chars().count() + 2) + 1,
        sub_region_line.as_ref().map_or(0, |l| l.chars().count() + 4),
        neighbour_width,
        country
            .adjacent_superpowers
            .iter()
            .map(|sp| format!("{sp} (superpower)").chars().count() + 4)
            .max()
            .unwrap_or(0),
        op_panel.as_ref().map_or(0, |(title, rows)| {
            rows.iter().map(|(l, _)| l.chars().count() + 4).chain([title.chars().count() + 6]).max().unwrap_or(0)
        }),
        hint.map_or(0, |h| h.chars().count()),
        60,
    ]
    .into_iter()
    .max()
    .unwrap_or(60);

    let mut canvas = Canvas::new(content_width, height);

    canvas.draw_box(0, 0, content_width, box_height, Style::default());
    let title_style = if country.battleground { Style::color(Color::Battleground).bold() } else { Style::default().bold() };
    put_border_title(&mut canvas, 0, 0, &title_left, title_style, &title_right, Style::default(), content_width);

    let mut row = 1;
    let controller = board.controller(map, id);
    let after = draw_influence(&mut canvas, row, 2, board, id, controller);
    let control_str = match controller {
        Some(Superpower::Us) => "US control",
        Some(Superpower::Ussr) => "USSR control",
        None => "contested",
    };
    canvas.put(row, after + 3, control_str, Style::default());
    row += 1;
    if let Some(line) = &sub_region_line {
        canvas.put(row, 2, line, Style::color(Color::Muted));
        row += 1;
    }

    canvas.draw_divider(row, 0, content_width, "Neighbours", Style::default());
    row += 1;
    for &neighbor_id in &country.adjacent {
        let n = map.country(neighbor_id);
        let nctl = board.controller(map, neighbor_id);
        canvas.put(row, 2, &format!("{:<width$}", n.name, width = neighbour_name_width), Style::default());
        draw_influence(&mut canvas, row, 4 + neighbour_name_width, board, neighbor_id, nctl);
        // Turn a realignment's "adjacent controlled" modifier from a bare
        // number into a visible derivation: mark exactly the neighbours
        // that are actually supplying it.
        if let Some(side) = realign_side
            && board.is_controlled_by(map, neighbor_id, side)
        {
            let marker = "+1 realign";
            canvas.put(row, content_width - 2 - marker.chars().count(), marker, Style::color(Color::Selected));
        }
        row += 1;
    }
    for &sp in &country.adjacent_superpowers {
        canvas.put(row, 2, &format!("{sp} (superpower)"), Style::color(Color::Muted));
        row += 1;
    }

    if let Some((title, rows)) = &op_panel {
        canvas.draw_divider(row, 0, content_width, title, Style::color(Color::Selected));
        row += 1;
        for (line, style) in rows {
            canvas.put(row, 2, line, *style);
            row += 1;
        }
    }

    if let Some(hint) = hint {
        canvas.put(height - 1, 0, hint, Style::color(Color::Muted));
    }

    canvas
}

/// Draws `US <n>  <glyph>  USSR <n>` at `(row, col)`, colouring each part
/// by the same convention as the world and region views, and returns the
/// column just past the end of what it wrote.
fn draw_influence(canvas: &mut Canvas, row: usize, col: usize, board: &Board, id: CountryId, controller: Option<Superpower>) -> usize {
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
