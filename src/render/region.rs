use std::collections::{HashMap, HashSet};

use crate::board::Board;
use crate::country::{CountryId, Region, Superpower};
use crate::layout::{GuestEntity, LinkTarget, MapLayout};
use crate::map::WorldMap;
use crate::ops::Operation;

use super::chip::{ChipGrid, ChipRole, REGION_CHIP_W};
use super::{coup_odds_line, coup_target_line_with, modifier_line, odds_line, operation_balance_line, war_line, Canvas, Color, Style, BEGIN_HINT};

const LEFT_MARGIN: usize = 2;
const TOP_MARGIN: usize = 2; // title line + blank

/// `Enter` opens the selected country's own detail screen
/// (`render::render_country`) — the region view keeps no roll or odds
/// preview of its own worth a key for beyond that.
const SELECTION_HINT: &str = "←→↑↓ select · Enter open · Esc back";

/// Shown instead, once an [`InfluencePlacement`](crate::ops::InfluencePlacement) is in progress.
/// Placement stays bound here too — it's undoable, so it doesn't need the
/// country screen's confirmation step the way a roll does.
const PLACEMENT_HINT: &str = "←→↑↓ select · Enter open · +/= place · u undo · ⌫ abandon · c confirm · Esc back";

/// Shown instead of [`SELECTION_HINT`], once a [`Realignment`](crate::ops::Realignment)
/// is in progress. No `r roll` here any more — a roll is irreversible the
/// instant it's made, so it only happens on the country screen `Enter`
/// opens, where the odds are the whole screen rather than a footer.
const REALIGN_HINT: &str = "←→↑↓ select · Enter target · ⌫ abandon · c done · Esc back";

/// Shown instead of [`SELECTION_HINT`], once a [`Coup`](crate::ops::Coup) is
/// in progress. Same reasoning as [`REALIGN_HINT`] — the attempt itself
/// only happens on the country screen.
const WAR_HINT: &str = "←→↑↓ select · Enter target · ⌫ abandon · Esc back";
const COUP_HINT: &str = "←→↑↓ select · Enter target · ⌫ abandon · c done · Esc back";

/// Shown while an event's choices are open: `+`/`-` act on the selected
/// country, `1`/`2` pick a mode on a card that has two.
/// What the small marks on a chip mean while an event's choices are open.
const WAR_LEGEND: &str = "double border: can be attacked · grey: not a target";
const EVENT_LEGEND: &str = "double border: can act here · dim: not eligible · ↑N/↓N: can add/remove N · +N/-N: staged";
const EVENT_HINT: &str = "←→↑↓ select · Enter open · + add · - remove · u undo · 1-9 mode · ⌫ abandon · c done";

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

    let grid = ChipGrid { origin: (TOP_MARGIN, LEFT_MARGIN), anchor: (0, 0), chip_w: REGION_CHIP_W };
    let grid_width = LEFT_MARGIN + (max_col as usize + 1) * grid.pitch_col();
    let grid_height = TOP_MARGIN + (max_row as usize + 1) * grid.pitch_row();

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
        let role = if selected == Some(id) { ChipRole::Selected } else { ChipRole::Native };
        grid.draw_country(&mut canvas, map, board, cell, id, layout.short_name(id), role, op);
    }

    for guest in guests {
        match guest.entity {
            GuestEntity::Country(id) => grid.draw_country(&mut canvas, map, board, guest.cell, id, layout.short_name(id), ChipRole::Foreign, op),
            GuestEntity::Superpower(sp) => grid.draw_superpower(&mut canvas, guest.cell, sp),
        }
    }

    draw_connectors(&mut canvas, &grid, map, layout, &ids, region);

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

/// Draws a connector glyph in the gap between every pair of grid-adjacent,
/// really-adjacent countries — in-region pairs as before, plus a native
/// country and any guest chip standing in for a cross-region or
/// superpower neighbour of its own. When two diagonal connectors would
/// land on the same corner cell, they're merged into `╳` rather than one
/// silently overwriting the other.
fn draw_connectors(canvas: &mut Canvas, grid: &ChipGrid, map: &WorldMap, layout: &MapLayout, ids: &[CountryId], region: Region) {
    let mut glyphs = HashMap::new();
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
                grid.draw_edge(&mut glyphs, a, layout.cell(neighbor_id));
            } else {
                for guest in guests.iter().filter(|g| g.entity == GuestEntity::Country(neighbor_id)) {
                    grid.draw_edge(&mut glyphs, a, guest.cell);
                }
            }
        }
        for &sp in &country.adjacent_superpowers {
            for guest in guests.iter().filter(|g| g.entity == GuestEntity::Superpower(sp)) {
                grid.draw_edge(&mut glyphs, a, guest.cell);
            }
        }
    }

    grid.flush_edges(canvas, &glyphs);
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
        if let (Operation::War(war), Some(id)) = (operation, selected)
            && war.is_legal_target(map, id)
        {
            lines.push((war_line(map, board, war, id), Style::color(Color::Selected)));
        }
        if let (Operation::Coup(coup), Some(id)) = (operation, selected) {
            let (target_number, odds) = coup.preview(map, board, id);
            lines.push((
                coup_target_line_with(
                    coup.side(),
                    coup.ops_for(map, id),
                    coup.roll_mod(map, id).map_or(0, |(_, m)| m),
                    target_number,
                    map.country(id).stability,
                ),
                Style::color(Color::Selected),
            ));
            lines.push((coup_odds_line(coup.side(), &odds), Style::color(Color::Muted)));
        }
    }
    if let Some(Operation::War(_)) = op {
        lines.push((WAR_LEGEND.to_string(), Style::color(Color::Muted)));
    }
    if let Some(Operation::Event(_)) = op {
        lines.push((EVENT_LEGEND.to_string(), Style::color(Color::Muted)));
    }
    if let (Some(Operation::Event(e)), Some(id)) = (op, selected) {
        lines.push((e.hint(map, id), Style::color(Color::Selected)));
    }
    if selected.is_some() || op.is_some() {
        let hint = match op {
            Some(Operation::Influence(_)) => PLACEMENT_HINT.to_string(),
            Some(Operation::Realign(_)) => REALIGN_HINT.to_string(),
            Some(Operation::Coup(_)) => COUP_HINT.to_string(),
            Some(Operation::Event(_)) => EVENT_HINT.to_string(),
            Some(Operation::War(_)) => WAR_HINT.to_string(),
            None => format!("{SELECTION_HINT} · {BEGIN_HINT}"),
        };
        lines.push((hint, Style::color(Color::Muted)));
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
