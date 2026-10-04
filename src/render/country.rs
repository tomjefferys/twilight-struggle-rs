use std::collections::HashMap;

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::layout::{Cell, GuestEntity, MapLayout};
use crate::map::WorldMap;
use crate::ops::Operation;

use super::mode_only_hint;
use super::chip::{ChipGrid, ChipRole, CHIP_H, REGION_CHIP_W};
use super::{
    war_line,
    control_glyph, coup_odds_line, coup_target_line_with, modifier_line, nz, odds_line, operation_header, operation_touched_line,
    put_border_title, Canvas, Color, Style, ViewMode, BEGIN_HINT,
};

/// Shown below the box, only under [`ViewMode::Interactive`] — a static
/// print into scrollback has no keys to hint at. A function rather than a
/// plain `const` since it interpolates [`BEGIN_HINT`].
fn hint() -> String {
    format!("←→↑↓ select · {BEGIN_HINT} · Esc back")
}
const PLACEMENT_HINT: &str = "←→↑↓ select · +/= place · u undo · ⌫ abandon · c confirm · Esc back";
const REALIGN_HINT: &str = "←→↑↓ select · r roll · ⌫ abandon · c done · Esc back";
const WAR_HINT: &str = "←→↑↓ select · r declare war · ⌫ abandon · Esc back";
const COUP_HINT: &str = "←→↑↓ select · r coup · ⌫ abandon · c done · Esc back";
const EVENT_HINT: &str = "←→↑↓ select · + add · - remove · u undo · 1-9 mode · ⌫ abandon · c done";

/// What occupies one cell of the neighbourhood mini-map: either a country
/// (the one viewed, or one of its neighbours) or a superpower guest chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Occupant {
    Country(CountryId),
    Superpower(Superpower),
}

impl Occupant {
    fn label(self, map: &WorldMap) -> String {
        match self {
            Occupant::Country(id) => map.country(id).name.clone(),
            Occupant::Superpower(sp) => sp.to_string(),
        }
    }

    /// Whether `self` and `other` actually border each other on the game
    /// map — as opposed to merely sharing a grid-adjacent cell, which is
    /// all [`ChipGrid::draw_edge`] itself checks.
    fn really_adjacent(self, map: &WorldMap, other: Occupant) -> bool {
        match (self, other) {
            (Occupant::Country(a), Occupant::Country(b)) => map.country(a).adjacent.contains(&b),
            (Occupant::Country(a), Occupant::Superpower(sp)) | (Occupant::Superpower(sp), Occupant::Country(a)) => {
                map.country(a).adjacent_superpowers.contains(&sp)
            }
            (Occupant::Superpower(_), Occupant::Superpower(_)) => false,
        }
    }
}

/// Every chip the neighbourhood mini-map should draw for `id` — the
/// country itself plus each of its immediate neighbours — placed at the
/// cell (native, or a guest cell in `id`'s own region) that's actually
/// grid-adjacent to it, alongside whatever adjacency couldn't be placed
/// that way. Empty on the standard layout (every adjacency of every
/// country has a grid-adjacent cell somewhere in that country's own
/// region — see `tests/layout.rs::standard_layout_has_no_undrawn_links`,
/// which pins the same property for the region view), so this is the
/// fallback for a future layout edit that doesn't get every guest cell
/// right; see `every_country_places_all_its_neighbours_on_the_country_view_grid`.
fn neighbourhood(map: &WorldMap, layout: &MapLayout, id: CountryId) -> (Vec<(Cell, Occupant)>, Vec<Occupant>) {
    let country = map.country(id);
    let region = country.region;
    let here = layout.cell(id);
    let guests = layout.guests(region);

    let mut placed = vec![(here, Occupant::Country(id))];
    let mut unplaced = Vec::new();

    for &n in &country.adjacent {
        let neighbor = map.country(n);
        let found = if neighbor.region == region {
            here.is_adjacent(layout.cell(n)).then(|| layout.cell(n))
        } else {
            guests.iter().find(|g| g.entity == GuestEntity::Country(n) && here.is_adjacent(g.cell)).map(|g| g.cell)
        };
        match found {
            Some(cell) => placed.push((cell, Occupant::Country(n))),
            None => unplaced.push(Occupant::Country(n)),
        }
    }

    for &sp in &country.adjacent_superpowers {
        let found = guests.iter().find(|g| g.entity == GuestEntity::Superpower(sp) && here.is_adjacent(g.cell)).map(|g| g.cell);
        match found {
            Some(cell) => placed.push((cell, Occupant::Superpower(sp))),
            None => unplaced.push(Occupant::Superpower(sp)),
        }
    }

    (placed, unplaced)
}

/// A single country in detail, as a titled box with up to three panels:
/// the country itself, a neighbourhood mini-map, and — while an operation
/// is open — that operation's live preview. This is the interactive world
/// map's third screen (opened from the region view with `Enter` or `r`)
/// and also the REPL's `country <name>` / `/<name>` command.
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
    // rather than a divider, so it's built as a (left, right) pair. The
    // sub-regions row is always reserved, blank when there are none,
    // rather than only present sometimes — so the box is exactly the
    // same height for every country, the same reasoning behind the
    // Neighbours mini-map's own fixed size below. ---
    let title_left = if country.battleground { format!("* {}", country.name) } else { country.name.clone() };
    let title_right = format!("{} · stability {}", country.region, country.stability);
    let sub_region_line = if country.sub_regions.is_empty() {
        None
    } else {
        let names: Vec<String> = country.sub_regions.iter().map(|s| s.to_string()).collect();
        Some(format!("sub-regions: {}", names.join(", ")))
    };
    let country_row_count = 2;

    // --- Panel 2: Neighbours, drawn as a mini-map — a fixed 3×3 window
    // of the region view's own grid, centred on the viewed country
    // rather than cropped to whoever's actually adjacent, so it's always
    // the same size and the country itself is always the middle chip,
    // even sitting on a region's own edge with no neighbour on one or
    // more sides. Each immediate neighbour is placed at whichever cell
    // (native, or a guest cell in this country's own region) is really
    // grid-adjacent to it. A neighbour is tinted with its own region's
    // colour exactly like a region view's guest chip, so where a
    // neighbour sits — and whether it's a visibly different colour — is
    // exactly where this screen's arrow keys go.
    let realign_side = match op {
        Some(Operation::Realign(r)) => Some(r.side()),
        _ => None,
    };
    let (placed, unplaced) = neighbourhood(map, layout, id);
    // Turn a realignment's "adjacent controlled" modifier from a bare
    // number into a visible derivation: name exactly the neighbours that
    // are actually supplying it. This can't be a marker on the chip
    // itself — "+1 realign" is wider than a chip's only free stats-row
    // slot — so it's footnoted below the grid instead.
    let realign_markers: Vec<CountryId> = match realign_side {
        Some(side) => placed
            .iter()
            .filter_map(|&(_, occ)| match occ {
                Occupant::Country(n) if n != id && board.is_controlled_by(map, n, side) => Some(n),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    };

    // Always a fixed 3×3 window centred on `here` — every country's
    // neighbours (native or guest) land within one cell of it in every
    // direction, so this always has room for all of them — rather than
    // cropped to whichever cells happen to be occupied, so the viewed
    // country is always in the middle cell, even directly on a region
    // grid's own edge with no neighbour at all on one or more sides.
    let here = layout.cell(id);
    let grid_rows = 3;
    let grid_cols = 3;
    let anchor = (here.row as i32 - 1, here.col as i32 - 1);
    // Wide enough for the longest country name in the whole game — a
    // full name, not `MapLayout::short_name`, since this view has the
    // room and a neighbour list previously named each one in full — not
    // just the longest label actually shown here: what varies while
    // arrowing between countries is which neighbours are shown, not how
    // much room a name might need, so sizing off only the current
    // neighbourhood would make the box jump width on every move. Never
    // narrower than the region view's own chip width either.
    let chip_w = map.iter().map(|(_, c)| c.name.chars().count() + 3).max().unwrap_or(REGION_CHIP_W).max(REGION_CHIP_W);
    let grid_height = grid_rows * (CHIP_H + 1) - 1;
    let grid_width = grid_cols * (chip_w + 1) - 1;

    let mut footnotes: Vec<(String, Style)> = unplaced
        .iter()
        .map(|occ| (format!("(not shown on this grid: {})", occ.label(map)), Style::color(Color::Muted)))
        .collect();
    if !realign_markers.is_empty() {
        let names: Vec<&str> = realign_markers.iter().map(|&n| map.country(n).name.as_str()).collect();
        footnotes.push((format!("+1 realign from {}", names.join(", ")), Style::color(Color::Selected)));
    }

    // --- Panel 3: Operation, only when one is open. Every one of these
    // rows is single-style, so — unlike the panels above — they're
    // collected up front as plain (String, Style) pairs. ---
    let op_panel: Option<(String, Vec<(String, Style)>)> = op.map(|operation| {
        let mut rows = vec![(operation_touched_line(layout, board, operation), Style::color(Color::Muted))];
        if !operation.is_legal_target(map, board, id) {
            let reason = match operation {
                Operation::Influence(_) => "no presence or adjacency here".to_string(),
                Operation::Realign(_) | Operation::Coup(_) => {
                    format!("no {} influence to remove", operation.side().opponent())
                }
                Operation::Event(e) => e.hint(map, id),
                Operation::War(_) => "not a legal target for this war".to_string(),
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
            Operation::Event(e) => {
                rows.push((e.prompt(), Style::color(Color::Selected)));
                if operation.is_legal_target(map, board, id) {
                    rows.push((e.hint(map, id), Style::color(Color::Muted)));
                }
            }
            Operation::Realign(r) => {
                let (acting, opposing, odds) = r.preview(map, board, id);
                rows.push((modifier_line(r.side(), &acting), Style::color(Color::Selected)));
                rows.push((modifier_line(r.side().opponent(), &opposing), Style::color(Color::Muted)));
                rows.push((odds_line(r.side(), &odds), Style::color(Color::Muted)));
            }
            Operation::War(w) => {
                if operation.is_legal_target(map, board, id) {
                    rows.push((war_line(map, board, w, id), Style::color(Color::Selected)));
                }
            }
            Operation::Coup(c) => {
                let (target_number, odds) = c.preview(map, board, id);
                rows.push((coup_target_line_with(c.side(), c.ops_for(map, id), c.roll_mod(map, id).map_or(0, |(_, m)| m), target_number, country.stability), Style::color(Color::Selected)));
                rows.push((coup_odds_line(c.side(), &odds), Style::color(Color::Muted)));
            }
        }
        (operation_header(operation), rows)
    });

    let hint = (mode == ViewMode::Interactive).then(|| match op {
        Some(Operation::Influence(_)) => PLACEMENT_HINT.to_string(),
        Some(Operation::Realign(_)) => REALIGN_HINT.to_string(),
        Some(Operation::Coup(_)) => COUP_HINT.to_string(),
        Some(Operation::Event(e)) if e.is_mode_only() => mode_only_hint(e),
        Some(Operation::Event(_)) => EVENT_HINT.to_string(),
        Some(Operation::War(_)) => WAR_HINT.to_string(),
        None => hint(),
    });

    // --- Sizing. Every panel's content is already known above; take the
    // width the widest of it needs, since `Canvas` clips silently rather
    // than erroring on an under-sized write. ---
    let box_height = 2 // top + bottom border
        + country_row_count
        + 1 + grid_height + footnotes.len() // "Neighbours" divider + its grid + footnotes
        + op_panel.as_ref().map_or(0, |(_, rows)| 1 + rows.len()); // "{header}" divider + its rows
    let height = box_height + hint.is_some() as usize;

    let content_width = [
        // Left/right border margins (2 each), the space-padded left and
        // right title halves, and a one-column gap of bare dash between
        // them so the two never touch.
        4 + (title_left.chars().count() + 2) + (title_right.chars().count() + 2) + 1,
        sub_region_line.as_ref().map_or(0, |l| l.chars().count() + 4),
        4 + grid_width,
        footnotes.iter().map(|(l, _)| l.chars().count() + 4).max().unwrap_or(0),
        op_panel.as_ref().map_or(0, |(title, rows)| {
            rows.iter().map(|(l, _)| l.chars().count() + 4).chain([title.chars().count() + 6]).max().unwrap_or(0)
        }),
        hint.as_deref().map_or(0, |h| h.chars().count()),
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
    // Always reserved, left blank with no sub-regions, so this row's
    // presence never changes the box's height.
    if let Some(line) = &sub_region_line {
        canvas.put(row, 2, line, Style::color(Color::Muted));
    }
    row += 1;

    canvas.draw_divider(row, 0, content_width, "Neighbours", Style::default());
    row += 1;

    // Centred inside whatever width the box ended up — the grid is
    // usually what drives `content_width`, but a wide title or Operation
    // panel can leave it with room either side.
    let grid_col = 2 + content_width.saturating_sub(4).saturating_sub(grid_width) / 2;
    let grid = ChipGrid { origin: (row, grid_col), anchor, chip_w };
    for &(cell, occ) in &placed {
        match occ {
            Occupant::Country(n) => {
                let role = if n == id { ChipRole::Selected } else { ChipRole::Foreign };
                grid.draw_country(&mut canvas, map, board, cell, n, &occ.label(map), role, op);
            }
            Occupant::Superpower(sp) => grid.draw_superpower(&mut canvas, cell, sp),
        }
    }
    let mut glyphs = HashMap::new();
    for i in 0..placed.len() {
        for j in (i + 1)..placed.len() {
            let (cell_a, occ_a) = placed[i];
            let (cell_b, occ_b) = placed[j];
            if occ_a.really_adjacent(map, occ_b) {
                grid.draw_edge(&mut glyphs, cell_a, cell_b);
            }
        }
    }
    grid.flush_edges(&mut canvas, &glyphs);
    row += grid_height;

    for (line, style) in &footnotes {
        canvas.put(row, 2, line, *style);
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

    if let Some(hint) = &hint {
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
