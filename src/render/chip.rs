//! The country-box chip: a 4-row, region-tinted box showing a country's
//! flag, name, influence, control, and stability, plus the connector
//! glyphs drawn between grid-adjacent chips. Originally `region.rs`'s own
//! `draw_country_box`/`draw_guest_country_box`/`draw_superpower_box`
//! (which were ~95% duplicated with each other); pulled out here once
//! `country.rs`'s neighbourhood mini-map needed the exact same drawing —
//! just at a wider pitch, to fit a full country name rather than a short
//! one, and windowed onto a fixed 3×3 patch of cells centred on one
//! country rather than a whole region's grid.

use std::collections::HashMap;

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::layout::Cell;
use crate::map::WorldMap;
use crate::ops::Operation;

use super::{control_glyph, nz, region_color, Canvas, Color, Style};

/// The region view's own chip width — every chip on a region grid is
/// exactly this wide, since the region snapshot and width tests pin a
/// grid built at this pitch. The country view's neighbourhood mini-map
/// widens its chips to fit a full country name instead (see
/// `country.rs::neighbourhood`), so this constant belongs to `region.rs`
/// alone; nothing here assumes it.
pub(super) const REGION_CHIP_W: usize = 14;

/// Every chip is this many rows tall, on both the region grid and the
/// country view's mini-map.
pub(super) const CHIP_H: usize = 4;

/// How one chip is drawn. `Selected` marks the one country a screen has
/// picked as its current selection — the region view's own selected box,
/// or the country view's own centre chip (that view has no "selection"
/// distinct from the country it's showing). `Native` is an ordinary chip,
/// tinted for its own region, dimmed when `op` says it can't legally be
/// targeted. `Foreign` uses the same region tint but is never thick and
/// never dimmed, since it can't be the current selection or a target from
/// this screen at all — a region view's guest chip, and every neighbour
/// chip on the country view's mini-map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChipRole {
    Selected,
    Native,
    Foreign,
}

/// Places chips and the connectors between them on a [`Canvas`], at a
/// fixed pitch derived from `chip_w`, anchored so that cell `anchor`
/// lands at `origin`. `anchor` is a *signed* row/col pair in whatever
/// region grid the caller is drawing from — not a [`Cell`] — since the
/// country view's mini-map centres on a country's own cell, which needs
/// an anchor one row and one column above-left of it, and a cell in the
/// grid's own top or left edge has no such (non-negative) `Cell` to give.
/// The region view anchors at that grid's own `(0, 0)`, always
/// non-negative, since it never needs to draw anything above or left of
/// its own origin.
pub(super) struct ChipGrid {
    pub(super) origin: (usize, usize),
    pub(super) anchor: (i32, i32),
    pub(super) chip_w: usize,
}

impl ChipGrid {
    pub(super) fn pitch_row(&self) -> usize {
        CHIP_H + 1
    }

    pub(super) fn pitch_col(&self) -> usize {
        self.chip_w + 1
    }

    /// The top-left canvas position of the chip at `cell`. `cell` must not
    /// lie above or left of `anchor` — every caller crops or chooses its
    /// anchor so that's always true.
    fn pos(&self, cell: Cell) -> (usize, usize) {
        let row = self.origin.0 as i32 + (cell.row as i32 - self.anchor.0) * self.pitch_row() as i32;
        let col = self.origin.1 as i32 + (cell.col as i32 - self.anchor.1) * self.pitch_col() as i32;
        (row as usize, col as usize)
    }

    /// Draws one country's chip at `cell`, labelled `label` — the region
    /// view passes [`crate::layout::MapLayout::short_name`], the country
    /// view's mini-map passes the full country name, since a chip's width
    /// is no longer fixed at [`REGION_CHIP_W`] there.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_country(
        &self,
        canvas: &mut Canvas,
        map: &WorldMap,
        board: &Board,
        cell: Cell,
        id: CountryId,
        label: &str,
        role: ChipRole,
        op: Option<&Operation>,
    ) {
        let (row, col) = self.pos(cell);
        let country = map.country(id);
        let tint = Style::color(region_color(country.region));
        let frame_style = if role == ChipRole::Selected { Style::color(Color::Selected).bold() } else { tint };
        if role == ChipRole::Selected {
            canvas.draw_thick_box(row, col, self.chip_w, CHIP_H, frame_style);
        } else {
            canvas.draw_box(row, col, self.chip_w, CHIP_H, frame_style);
        }

        let flag_style = if country.battleground { Style::color(Color::Battleground) } else { frame_style };
        canvas.put_char(row + 1, col + 1, if country.battleground { '*' } else { ' ' }, flag_style);
        // A `Foreign` chip is never a target from whichever screen is
        // showing it (a region view's guest, or any neighbour on the
        // country view's mini-map), so illegality never dims its name —
        // only a `Native` chip's can be.
        let legal = role != ChipRole::Native || op.is_none_or(|o| o.is_legal_target(map, board, id));
        let name_style = if legal { frame_style } else { Style::color(Color::Muted) };
        canvas.put(row + 1, col + 2, label, name_style);

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
        canvas.put(row + 2, col + self.chip_w - 6, &format!("st{}", country.stability), Style::color(Color::Muted));

        if let Some(operation) = op {
            draw_operation_badge(canvas, row, col, self.chip_w, operation, map, board, id);
        }
    }

    /// A superpower's own chip — the same frame size as a country chip but
    /// with no stats rows, just its name tinted the same colour used for
    /// it everywhere else. Never selectable: there's no superpower screen
    /// to jump to.
    pub(super) fn draw_superpower(&self, canvas: &mut Canvas, cell: Cell, superpower: Superpower) {
        let (row, col) = self.pos(cell);
        let style = Style::color(match superpower {
            Superpower::Us => Color::Us,
            Superpower::Ussr => Color::Ussr,
        });
        canvas.draw_box(row, col, self.chip_w, CHIP_H, style);
        let name = superpower.to_string();
        let start_col = col + 1 + (self.chip_w - 2).saturating_sub(name.chars().count()) / 2;
        canvas.put(row + 1, start_col, &name, style);
    }

    /// Records the connector glyph between two grid-adjacent cells into
    /// `glyphs`, merging a `╲`/`╱` collision into `╳` rather than letting
    /// one silently overwrite the other. A pair that isn't actually
    /// grid-adjacent (Chebyshev distance > 1) draws nothing — the caller's
    /// job is to only call this for pairs that are.
    pub(super) fn draw_edge(&self, glyphs: &mut HashMap<(usize, usize), char>, a: Cell, b: Cell) {
        let dr = b.row as isize - a.row as isize;
        let dc = b.col as isize - a.col as isize;
        if dr.abs().max(dc.abs()) != 1 {
            return;
        }
        let (ay, ax) = self.pos(a);
        let (by, bx) = self.pos(b);
        let (pos, glyph) = if dr == 0 {
            // horizontal neighbours: connector in the column gap
            ((ay + 2, ax.min(bx) + self.chip_w), '─')
        } else if dc == 0 {
            // vertical neighbours: connector in the row gap
            ((ay.min(by) + CHIP_H, ax + self.chip_w / 2), '│')
        } else {
            // diagonal neighbours: connector at the shared corner
            ((ay.min(by) + CHIP_H, ax.min(bx) + self.chip_w), if (dr > 0) == (dc > 0) { '╲' } else { '╱' })
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

    /// Draws every glyph `draw_edge` recorded.
    pub(super) fn flush_edges(&self, canvas: &mut Canvas, glyphs: &HashMap<(usize, usize), char>) {
        for (&(y, x), &glyph) in glyphs {
            canvas.put_char(y, x, glyph, Style::color(Color::Muted));
        }
    }
}

/// A chip's stats-row badge for the operation in progress, positioned
/// relative to `chip_w` so it works at the region view's fixed width and
/// the country view's wider ones alike: a 2-column slot flush against the
/// right border (`col + chip_w - 3`, clamped clear of the border itself)
/// for the operation's headline number, and — only for a realignment that
/// has *also* cost the acting side its own influence there — a second,
/// left-relative slot at `col + 6` (free on every chip, since influence
/// and the control glyph occupy cols 1-5 and `st<n>` sits flush right).
#[allow(clippy::too_many_arguments)]
fn draw_operation_badge(canvas: &mut Canvas, row: usize, col: usize, chip_w: usize, operation: &Operation, map: &WorldMap, board: &Board, id: CountryId) {
    let right_slot = col + chip_w - 3;
    match operation {
        Operation::Influence(p) => {
            let pending = p.pending(id) as i8;
            if pending > 0 {
                canvas.put(row + 2, right_slot, &format_delta(pending), Style::color(Color::Selected).bold());
            }
        }
        Operation::Event(e) => {
            // What the event has changed here (`+N`/`-N`, in the moved side's
            // own colour); otherwise (dim `↑N`/`↓N`) the step `+`/`-` would take next, so
            // every country the chooser can use shows how much it can
            // take before a pick is made.
            let (us, ussr) = (operation.delta(board, id, Superpower::Us), operation.delta(board, id, Superpower::Ussr));
            let shown = if us != 0 { Some((us, Superpower::Us)) } else if ussr != 0 { Some((ussr, Superpower::Ussr)) } else { None };
            match shown {
                Some((d, side)) => {
                    let style = Style::color(if side == Superpower::Us { Color::Us } else { Color::Ussr }).bold();
                    canvas.put(row + 2, right_slot, &format_delta(d), style);
                }
                None => {
                    if let Some((sign, n)) = e.suggestion(map, id) {
                        // `↑N` / `↓N` rather than `+N` / `-N`, so what a chip
                        // *can* take never reads as what's been staged on it.
                        let arrow = if sign == crate::events::choice::Sign::Plus { '↑' } else { '↓' };
                        canvas.put(row + 2, right_slot, &format!("{arrow}{}", n.min(9)), Style::color(Color::Muted));
                    }
                }
            }
        }
        Operation::Realign(_) | Operation::Coup(_) => {
            let side = operation.side();
            let opponent = side.opponent();
            let opp_delta = operation.delta(board, id, opponent);
            let own_delta = operation.delta(board, id, side);
            if opp_delta != 0 {
                canvas.put(row + 2, right_slot, &format_delta(opp_delta), Style::color(Color::Selected).bold());
            }
            if own_delta != 0 {
                canvas.put(row + 2, col + 6, &format_delta(own_delta), Style::color(Color::Muted));
            }
        }
    }
}

/// A signed delta clamped to a single digit of magnitude, so it always
/// fits the two-character badge slot in a chip.
fn format_delta(delta: i8) -> String {
    let sign = if delta < 0 { '-' } else { '+' };
    format!("{sign}{}", delta.unsigned_abs().min(9))
}
