use std::collections::{HashMap, HashSet};

use crate::board::Board;
use crate::country::{CountryId, Region, Superpower};
use crate::layout::{Cell, GuestEntity, LinkTarget, MapLayout};
use crate::map::WorldMap;
use crate::ops::Operation;

use super::{control_glyph, coup_odds_line, coup_target_line, modifier_line, nz, odds_line, operation_balance_line, region_color, Canvas, Color, Style};

/// Box dimensions and grid pitch for the region zoom view. A 1-column,
/// 1-row gap between boxes leaves room for connector glyphs.
const BOX_W: usize = 14;
const BOX_H: usize = 4;
const PITCH_COL: usize = BOX_W + 1;
const PITCH_ROW: usize = BOX_H + 1;
const LEFT_MARGIN: usize = 2;
const TOP_MARGIN: usize = 2; // title line + blank

/// `Enter` opens the selected country's own detail screen
/// (`render::render_country`) — the region view keeps no roll or odds
/// preview of its own worth a key for beyond that.
const SELECTION_HINT: &str = "←→↑↓ select · Enter open · Esc back";

/// Shown instead, once an [`InfluencePlacement`](crate::ops::InfluencePlacement) is in progress.
/// Placement stays bound here too — it's undoable, so it doesn't need the
/// country screen's confirmation step the way a roll does.
const PLACEMENT_HINT: &str = "←→↑↓ select · Enter open · + place · u undo · c confirm · Esc back";

/// Shown instead of [`SELECTION_HINT`], once a [`Realignment`](crate::ops::Realignment)
/// is in progress. No `r roll` here any more — a roll is irreversible the
/// instant it's made, so it only happens on the country screen `Enter`
/// opens, where the odds are the whole screen rather than a footer.
const REALIGN_HINT: &str = "←→↑↓ select · Enter target · c done · Esc back";

/// Shown instead of [`SELECTION_HINT`], once a [`Coup`](crate::ops::Coup) is
/// in progress. Same reasoning as [`REALIGN_HINT`] — the attempt itself
/// only happens on the country screen.
const COUP_HINT: &str = "←→↑↓ select · Enter target · c done · Esc back";

/// A geographic zoom into one region: every country in it drawn as a box
/// on its layout grid cell, connected to its in-region neighbours by line
/// glyphs. Adjacencies that leave the region (to another region, or to a
/// superpower) are drawn too, as guest chips — a foreign country tinted
/// with its own region's colour, a superpower tinted like its own box —
/// so the whole map is reachable by stepping alone; whatever a guest chip
/// couldn't be placed for is footnoted below the map instead, the same as
/// any in-region adjacency the grid couldn't draw a connector for.
///
/// `selected`, when set, is the country currently picked in interactive
/// navigation: its box border and name are drawn bold, and a two-line
/// footer below everything else names it and gives the key hints.
/// Passing `None` reproduces the plain, static view exactly — no extra
/// rows, no style changes — so every existing caller is unaffected.
///
/// `op`, when set, is an operation in progress. An
/// [`InfluencePlacement`](crate::ops::InfluencePlacement) shows each
/// pending country with a `+N` badge and the live (speculative) influence
/// numbers, dims a country that isn't currently a legal target, and adds
/// a footer line with the side, its ops balance, and where it's placed so
/// far. A [`Realignment`](crate::ops::Realignment) shows a `+N`/`-N` net
/// badge from the real board's own live numbers (there's no speculative
/// board — see the module's own doc), dims a country with no opponent
/// influence to remove, and — while a country is also selected — adds a
/// modifier and odds breakdown for the roll that country would resolve
/// next. A [`Coup`](crate::ops::Coup) behaves the same way — same badge,
/// same dimming rule — but its footer breakdown shows the target number
/// (stability × 2) and success odds instead of realignment's modifiers.
pub fn render_region(
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    region: Region,
    selected: Option<CountryId>,
    op: Option<&Operation>,
) -> Canvas {
    // While a placement is in progress, every reader — influence numbers,
    // control glyphs, tallies — should see its speculative board rather
    // than the caller's, so the view always reflects pending state live.
    // A realignment or coup has no speculative board of its own: its
    // rolls are already on the real one, which is exactly what `board`
    // already is.
    let board = op.and_then(Operation::board).unwrap_or(board);
    let ids = layout.countries_in_region(map, region);
    let guests = layout.guests(region);
    let (max_row, max_col) = ids
        .iter()
        .map(|&id| layout.cell(id))
        .chain(guests.iter().map(|g| g.cell))
        .fold((0u8, 0u8), |(mr, mc), cell| (mr.max(cell.row), mc.max(cell.col)));

    let grid_width = LEFT_MARGIN + (max_col as usize + 1) * PITCH_COL;
    let grid_height = TOP_MARGIN + (max_row as usize + 1) * PITCH_ROW;

    let undrawn = undrawn_links_for(map, layout, region);

    let mut extra_lines = 0;
    if !undrawn.is_empty() {
        extra_lines += 1 + undrawn.len();
    }
    let footer_lines = build_footer_lines(map, board, layout, selected, op);
    let footer_rows = footer_lines.len();
    let height = grid_height + if extra_lines > 0 { extra_lines + 1 } else { 0 } + footer_rows;

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

    // Never shrink the canvas below what the title, footnotes, or the
    // selection footer need just because the grid itself is narrower
    // (South America, at 3 columns wide, is narrower than its own title
    // line) — `Canvas` silently clips writes past its width, so an
    // under-sized canvas would truncate text rather than erroring.
    let content_width = undrawn
        .iter()
        .map(|line| line.chars().count())
        .chain(footer_lines.iter().map(|(line, _)| line.chars().count()))
        .chain([title.chars().count(), grid_width])
        .max()
        .unwrap_or(grid_width);

    let mut canvas = Canvas::new(content_width, height);
    canvas.put(0, 0, &title, Style::default());

    for &id in &ids {
        let cell = layout.cell(id);
        let y = TOP_MARGIN + cell.row as usize * PITCH_ROW;
        let x = LEFT_MARGIN + cell.col as usize * PITCH_COL;
        draw_country_box(&mut canvas, y, x, map, layout, board, id, selected == Some(id), op);
    }

    for guest in guests {
        let y = TOP_MARGIN + guest.cell.row as usize * PITCH_ROW;
        let x = LEFT_MARGIN + guest.cell.col as usize * PITCH_COL;
        match guest.entity {
            GuestEntity::Country(id) => draw_guest_country_box(&mut canvas, y, x, map, layout, board, id, op),
            GuestEntity::Superpower(sp) => draw_superpower_box(&mut canvas, y, x, sp),
        }
    }

    draw_connectors(&mut canvas, map, layout, &ids, region);

    for (i, (line, style)) in footer_lines.iter().enumerate() {
        canvas.put(height - footer_rows + i, 0, line, *style);
    }

    let mut row = grid_height;
    if !undrawn.is_empty() {
        row += 1;
        for line in &undrawn {
            canvas.put(row, LEFT_MARGIN, line, Style::color(Color::Muted));
            row += 1;
        }
    }

    canvas
}

/// `selected` draws the border in heavy line-work and a bright accent
/// colour, and bolds the name — a shape change as well as a colour one,
/// so the selection still reads clearly even under `ColorMode::Never` or
/// on a terminal where bold text barely differs from regular weight.
/// Unselected, the border and name are tinted with `region_color` for
/// this grid's own region — the same tint a guest chip elsewhere uses for
/// *its* home region — so a region view reads as one coherent colour, and
/// a guest chip's different colour stands out as visibly foreign at a
/// glance rather than only by its short-name and stats matching a
/// different country.
#[allow(clippy::too_many_arguments)]
fn draw_country_box(
    canvas: &mut Canvas,
    row: usize,
    col: usize,
    map: &WorldMap,
    layout: &MapLayout,
    board: &Board,
    id: CountryId,
    selected: bool,
    op: Option<&Operation>,
) {
    let country = map.country(id);
    let tint = Style::color(region_color(country.region));
    let frame_style = if selected { Style::color(Color::Selected).bold() } else { tint };
    if selected {
        canvas.draw_thick_box(row, col, BOX_W, BOX_H, frame_style);
    } else {
        canvas.draw_box(row, col, BOX_W, BOX_H, frame_style);
    }

    let flag_style = if country.battleground { Style::color(Color::Battleground) } else { frame_style };
    canvas.put_char(row + 1, col + 1, if country.battleground { '*' } else { ' ' }, flag_style);
    // A country that can't legally receive the next placement (or roll)
    // is dimmed — the rule is visible up front, not just enforced on a
    // failed attempt.
    let legal = op.is_none_or(|o| o.is_legal_target(map, board, id));
    let name_style = if legal { frame_style } else { Style::color(Color::Muted) };
    canvas.put(row + 1, col + 2, layout.short_name(id), name_style);

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

    if let Some(operation) = op {
        draw_operation_badge(canvas, row, col, operation, board, id);
    }
}

/// A country whose home region isn't the one currently on screen — a
/// guest chip. Drawn with the same influence, control, stability, and
/// operation badges as a native box, but tinted with `region_color` for
/// its *own* region instead of the default/selected styling: never
/// thick-bordered (a guest can't be the current selection — arrowing onto
/// one jumps the whole screen there instead), and never dimmed for
/// illegality (the tint is the signal here, not a target-eligibility one
/// — a guest isn't a target from this screen at all).
#[allow(clippy::too_many_arguments)]
fn draw_guest_country_box(canvas: &mut Canvas, row: usize, col: usize, map: &WorldMap, layout: &MapLayout, board: &Board, id: CountryId, op: Option<&Operation>) {
    let country = map.country(id);
    let tint = Style::color(region_color(country.region));
    canvas.draw_box(row, col, BOX_W, BOX_H, tint);

    let flag_style = if country.battleground { Style::color(Color::Battleground) } else { tint };
    canvas.put_char(row + 1, col + 1, if country.battleground { '*' } else { ' ' }, flag_style);
    canvas.put(row + 1, col + 2, layout.short_name(id), tint);

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

    if let Some(operation) = op {
        draw_operation_badge(canvas, row, col, operation, board, id);
    }
}

/// A superpower's own chip on the region grid — the same frame size as a
/// country box but with no stats rows, just its name tinted the same
/// colour used for it everywhere else. Never selectable: there's no
/// superpower screen to jump to, so [`MapLayout::step_country`] never
/// returns one, and the arrow-key cursor simply steps past it.
fn draw_superpower_box(canvas: &mut Canvas, row: usize, col: usize, superpower: Superpower) {
    let style = Style::color(match superpower {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    });
    canvas.draw_box(row, col, BOX_W, BOX_H, style);
    let name = superpower.to_string();
    let start_col = col + 1 + (BOX_W - 2).saturating_sub(name.chars().count()) / 2;
    canvas.put(row + 1, start_col, &name, style);
}

/// Cols 11-12 are free on the stats row for the operation's headline
/// number (1-5 is the influence pair, 8-10 is `st<n>`); cols 6-7 are also
/// free, used only by a realignment that's *also* cost the acting side
/// its own influence there. Clamped so a badge can never run into the
/// box's right border at col 13. Shared by both a native and a guest
/// country box — a guest touched by the operation shows the same badge a
/// native one would, since a placement made while you were in its home
/// region is exactly as pending here.
fn draw_operation_badge(canvas: &mut Canvas, row: usize, col: usize, operation: &Operation, board: &Board, id: CountryId) {
    match operation {
        Operation::Influence(p) => {
            let pending = p.pending(id) as i8;
            if pending > 0 {
                canvas.put(row + 2, col + 11, &format_delta(pending), Style::color(Color::Selected).bold());
            }
        }
        Operation::Realign(_) | Operation::Coup(_) => {
            let side = operation.side();
            let opponent = side.opponent();
            let opp_delta = operation.delta(board, id, opponent);
            let own_delta = operation.delta(board, id, side);
            if opp_delta != 0 {
                canvas.put(row + 2, col + 11, &format_delta(opp_delta), Style::color(Color::Selected).bold());
            }
            if own_delta != 0 {
                canvas.put(row + 2, col + 6, &format_delta(own_delta), Style::color(Color::Muted));
            }
        }
    }
}

/// A signed delta clamped to a single digit of magnitude, so it always
/// fits the two-character badge slot in a country box.
fn format_delta(delta: i8) -> String {
    let sign = if delta < 0 { '-' } else { '+' };
    format!("{sign}{}", delta.unsigned_abs().min(9))
}

/// Draws a connector glyph in the gap between every pair of grid-adjacent,
/// really-adjacent countries — in-region pairs as before, plus a native
/// country and any guest chip standing in for a cross-region or
/// superpower neighbour of its own. When two diagonal connectors would
/// land on the same corner cell, they're merged into `╳` rather than one
/// silently overwriting the other.
fn draw_connectors(canvas: &mut Canvas, map: &WorldMap, layout: &MapLayout, ids: &[CountryId], region: Region) {
    let mut glyphs: HashMap<(usize, usize), char> = HashMap::new();
    let guests = layout.guests(region);
    let mut done = HashSet::new();

    for &id in ids {
        let country = map.country(id);
        let a = layout.cell(id);
        for &neighbor_id in &country.adjacent {
            let neighbor = map.country(neighbor_id);
            if neighbor.region == region {
                let key = (id.min(neighbor_id), id.max(neighbor_id));
                if !done.insert(key) {
                    continue;
                }
                draw_edge(&mut glyphs, a, layout.cell(neighbor_id));
            } else {
                for guest in guests.iter().filter(|g| g.entity == GuestEntity::Country(neighbor_id)) {
                    draw_edge(&mut glyphs, a, guest.cell);
                }
            }
        }
        for &sp in &country.adjacent_superpowers {
            for guest in guests.iter().filter(|g| g.entity == GuestEntity::Superpower(sp)) {
                draw_edge(&mut glyphs, a, guest.cell);
            }
        }
    }

    for (&(y, x), &glyph) in &glyphs {
        canvas.put_char(y, x, glyph, Style::color(Color::Muted));
    }
}

/// Draws one connector glyph between two grid-adjacent cells, merging a
/// `╲`/`╱` collision into `╳` rather than letting one silently overwrite
/// the other. A pair that isn't actually grid-adjacent (Chebyshev
/// distance > 1) draws nothing — footnoted separately as undrawn.
fn draw_edge(glyphs: &mut HashMap<(usize, usize), char>, a: Cell, b: Cell) {
    let (dr, dc) = (b.row as isize - a.row as isize, b.col as isize - a.col as isize);
    if dr.abs().max(dc.abs()) != 1 {
        return;
    }
    let ay = TOP_MARGIN + a.row as usize * PITCH_ROW;
    let ax = LEFT_MARGIN + a.col as usize * PITCH_COL;
    let by = TOP_MARGIN + b.row as usize * PITCH_ROW;
    let bx = LEFT_MARGIN + b.col as usize * PITCH_COL;
    let (pos, glyph) = if dr == 0 {
        // horizontal neighbours: connector in the column gap
        ((ay + 2, ax.min(bx) + BOX_W), '─')
    } else if dc == 0 {
        // vertical neighbours: connector in the row gap
        ((ay.min(by) + BOX_H, ax + BOX_W / 2), '│')
    } else {
        // diagonal neighbours: connector at the shared corner
        ((ay.min(by) + BOX_H, ax.min(bx) + BOX_W), if (dr > 0) == (dc > 0) { '╲' } else { '╱' })
    };
    glyphs
        .entry(pos)
        .and_modify(|existing| {
            *existing = match (*existing, glyph) {
                ('╲', '╱') | ('╱', '╲') => '╳',
                (a, _) => a,
            };
        })
        .or_insert(glyph);
}

/// Every footer line this view might draw, in draw order — built once so
/// the row count (`.len()`) and the drawing loop can never drift apart,
/// unlike the hand-maintained parallel tallies this replaced.
///
/// A selection title shows whenever a country is selected; the operation
/// balance line shows whenever one is open; a realignment *also* adds its
/// modifier and odds breakdown (4 rows total with the balance line), and a
/// coup adds its target-number and odds breakdown (3 rows total) — either
/// way only once a country is selected too, since gating that on
/// `op.is_some()` alone would change a placement's row count, which
/// `tests/render.rs` pins exactly. A hint line closes it off whenever
/// either a selection or an operation is present, naming whichever of the
/// two applies.
fn build_footer_lines(
    map: &WorldMap,
    board: &Board,
    layout: &MapLayout,
    selected: Option<CountryId>,
    op: Option<&Operation>,
) -> Vec<(String, Style)> {
    let mut lines = Vec::new();
    if let Some(id) = selected {
        lines.push((selection_title(map, board, id), Style::color(Color::Selected).bold()));
    }
    if let Some(operation) = op {
        lines.push((operation_balance_line(layout, board, operation), Style::color(Color::Selected)));
        if let (Operation::Realign(realignment), Some(id)) = (operation, selected) {
            let (acting, opposing, odds) = realignment.preview(map, board, id);
            lines.push((modifier_line(realignment.side(), &acting), Style::color(Color::Selected)));
            lines.push((modifier_line(realignment.side().opponent(), &opposing), Style::color(Color::Muted)));
            lines.push((odds_line(realignment.side(), &odds), Style::color(Color::Muted)));
        }
        if let (Operation::Coup(coup), Some(id)) = (operation, selected) {
            let (target_number, odds) = coup.preview(map, board, id);
            lines.push((
                coup_target_line(coup.side(), coup.ops_total(), target_number, map.country(id).stability),
                Style::color(Color::Selected),
            ));
            lines.push((coup_odds_line(coup.side(), &odds), Style::color(Color::Muted)));
        }
    }
    if selected.is_some() || op.is_some() {
        let hint = match op {
            Some(Operation::Influence(_)) => PLACEMENT_HINT,
            Some(Operation::Realign(_)) => REALIGN_HINT,
            Some(Operation::Coup(_)) => COUP_HINT,
            None => SELECTION_HINT,
        };
        lines.push((hint.to_string(), Style::color(Color::Muted)));
    }
    lines
}

/// The selected country's footer line: name, battleground flag, stability,
/// and control — the same wording `render_country` uses for control.
fn selection_title(map: &WorldMap, board: &Board, id: CountryId) -> String {
    let country = map.country(id);
    let control = match board.controller(map, id) {
        Some(Superpower::Us) => "US control",
        Some(Superpower::Ussr) => "USSR control",
        None => "contested",
    };
    let mut parts = Vec::new();
    if country.battleground {
        parts.push("battleground".to_string());
    }
    parts.push(format!("stability {}", country.stability));
    parts.push(control.to_string());
    format!("▸ {} ◂  {}", country.name, parts.join(" · "))
}

/// The "not drawn on this grid" footnote for whatever `region`'s grid
/// couldn't place a connector for — an in-region pair too far apart, or a
/// cross-region/superpower link with no guest chip. Country-country and
/// superpower guest coverage is nearly total on the standard map (see
/// `tests/layout.rs::standard_layout_has_no_undrawn_links`), so this is
/// normally empty; it's the fallback for a future layout edit that
/// doesn't get every guest cell right.
fn undrawn_links_for(map: &WorldMap, layout: &MapLayout, region: Region) -> Vec<String> {
    layout
        .undrawn_links()
        .iter()
        .filter(|link| link.region == region)
        .map(|link| {
            let from = &map.country(link.from).name;
            let to = match link.to {
                LinkTarget::Country(id) => map.country(id).name.clone(),
                LinkTarget::Superpower(sp) => sp.to_string(),
            };
            format!("  (not drawn on this grid: {from} – {to})")
        })
        .collect()
}
