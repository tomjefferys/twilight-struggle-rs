//! Turning game state into text.
//!
//! Every view here is a pure function from `(WorldMap, MapLayout, Board,
//! ...)` to a [`Canvas`], and a `Canvas` only ever turns into a `String` —
//! nothing in this module touches the terminal directly. That keeps every
//! view snapshot-testable, and leaves the choice of how it's actually
//! presented (a REPL printing to stdout today, perhaps a full-screen TUI
//! later) entirely outside this module.

pub mod country;
pub mod region;
pub mod world;
pub mod worldmap;

pub use country::render_country;
pub use region::render_region;
pub use world::render_world;
pub use worldmap::render_world_map;

/// A semantic colour. What each one actually looks like is up to the
/// [`Theme`] resolving it, so a colour-blind or light-background palette is
/// a data change here, not a rewrite of every view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Us,
    Ussr,
    Battleground,
    Muted,
    /// The six *Twilight Struggle* board regions, each matching that
    /// region's colour on the physical board — used to tint the world
    /// map's landmass so it reads like the real board at a glance.
    Europe,
    Asia,
    MiddleEast,
    Africa,
    CentralAmerica,
    SouthAmerica,
    /// A UI accent with no in-game meaning of its own — used to mark
    /// whatever is currently selected in interactive navigation, so it
    /// doesn't compete with (or get lost among) the colours above, which
    /// all mean something about the game state.
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub color: Color,
    pub bold: bool,
    pub dim: bool,
}

impl Style {
    pub fn color(color: Color) -> Style {
        Style { color, bold: false, dim: false }
    }

    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }

    /// A subdued version of the same colour — used for background
    /// shading, so it recedes behind the foreground content drawn on top.
    pub fn dim(mut self) -> Style {
        self.dim = true;
        self
    }
}

/// Whether [`Canvas::render`] emits ANSI colour codes at all. `Never`
/// guarantees the output contains no escape sequences, for piped output,
/// `NO_COLOR`, and snapshot tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Always,
    Never,
}

/// Resolves a [`Style`] to an ANSI SGR sequence.
struct Theme;

impl Theme {
    fn sgr(style: Style) -> Option<String> {
        // The base 16-colour palette can't cover orange/purple, so those
        // two reach for a 256-colour code; everything else stays a plain
        // SGR number for maximum terminal compatibility.
        let color_code: Option<&'static str> = match style.color {
            Color::Default => None,
            Color::Us => Some("94"),
            Color::Ussr => Some("91"),
            Color::Battleground => Some("93"),
            Color::Muted => None,
            Color::Europe => Some("38;5;140"),
            Color::Asia => Some("38;5;208"),
            Color::MiddleEast => Some("96"),
            Color::Africa => Some("33"),
            Color::CentralAmerica => Some("92"),
            Color::SouthAmerica => Some("32"),
            Color::Selected => Some("97"),
        };

        let mut parts: Vec<&str> = Vec::new();
        if style.bold {
            parts.push("1");
        }
        if style.dim || style.color == Color::Muted {
            parts.push("2");
        }
        if let Some(code) = color_code {
            parts.push(code);
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!("\x1b[{}m", parts.join(";")))
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CanvasCell {
    ch: char,
    style: Style,
}

impl Default for CanvasCell {
    fn default() -> Self {
        CanvasCell {
            ch: ' ',
            style: Style::default(),
        }
    }
}

/// A fixed-size character grid that views draw into. Out-of-bounds writes
/// are silently clipped rather than panicking, since a view's content can
/// legitimately vary in size (a wide token at high influence, say) without
/// that being worth treating as a bug.
pub struct Canvas {
    width: usize,
    rows: Vec<Vec<CanvasCell>>,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Canvas {
            width,
            rows: vec![vec![CanvasCell::default(); width]; height],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: char, style: Style) {
        if let Some(cell) = self.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
            *cell = CanvasCell { ch, style };
        }
    }

    pub fn put(&mut self, row: usize, col: usize, text: &str, style: Style) {
        for (i, ch) in text.chars().enumerate() {
            self.put_char(row, col + i, ch, style);
        }
    }

    pub fn draw_box(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style) {
        self.draw_box_glyphs(row, col, w, h, style, ['┌', '┐', '└', '┘', '─', '│']);
    }

    /// The same box as [`Canvas::draw_box`], but drawn with heavy
    /// box-drawing characters instead of thin ones — a shape change, not
    /// just a style change, so a selection drawn this way still stands
    /// out under [`ColorMode::Never`], where colour and boldness are both
    /// invisible.
    pub fn draw_thick_box(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style) {
        self.draw_box_glyphs(row, col, w, h, style, ['┏', '┓', '┗', '┛', '━', '┃']);
    }

    fn draw_box_glyphs(&mut self, row: usize, col: usize, w: usize, h: usize, style: Style, glyphs: [char; 6]) {
        if w < 2 || h < 2 {
            return;
        }
        let [top_left, top_right, bottom_left, bottom_right, horizontal, vertical] = glyphs;
        self.put_char(row, col, top_left, style);
        self.put_char(row, col + w - 1, top_right, style);
        self.put_char(row + h - 1, col, bottom_left, style);
        self.put_char(row + h - 1, col + w - 1, bottom_right, style);
        for c in (col + 1)..(col + w - 1) {
            self.put_char(row, c, horizontal, style);
            self.put_char(row + h - 1, c, horizontal, style);
        }
        for r in (row + 1)..(row + h - 1) {
            self.put_char(r, col, vertical, style);
            self.put_char(r, col + w - 1, vertical, style);
        }
    }

    /// Renders the canvas as text, one line per row, with trailing spaces
    /// on each line trimmed. `mode` controls whether ANSI colour codes are
    /// emitted at all — `Never` guarantees no `\x1b` appears anywhere in
    /// the output.
    pub fn render(&self, mode: ColorMode) -> String {
        self.rows
            .iter()
            .map(|row| render_line(row, mode))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn render_line(cells: &[CanvasCell], mode: ColorMode) -> String {
    let mut end = cells.len();
    while end > 0 && cells[end - 1].ch == ' ' {
        end -= 1;
    }
    let cells = &cells[..end];

    if mode == ColorMode::Never {
        return cells.iter().map(|c| c.ch).collect();
    }

    let mut out = String::new();
    let mut i = 0;
    while i < cells.len() {
        let style = cells[i].style;
        let mut j = i + 1;
        while j < cells.len() && cells[j].style == style {
            j += 1;
        }
        let text: String = cells[i..j].iter().map(|c| c.ch).collect();
        match Theme::sgr(style) {
            Some(code) => {
                out.push_str(&code);
                out.push_str(&text);
                out.push_str("\x1b[0m");
            }
            None => out.push_str(&text),
        }
        i = j;
    }
    out
}

/// Battleground and total country counts for one region, US then USSR —
/// the numbers shown in a region's title, whether that's a dashboard panel
/// or the world map's selection line.
pub(crate) struct RegionTally {
    pub bg_us: usize,
    pub bg_ussr: usize,
    pub ctry_us: usize,
    pub ctry_ussr: usize,
}

pub(crate) fn region_tally(
    map: &crate::map::WorldMap,
    layout: &crate::layout::MapLayout,
    board: &crate::board::Board,
    region: crate::country::Region,
) -> RegionTally {
    use crate::country::Superpower;

    let ids = layout.countries_in_region(map, region);
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

    RegionTally { bg_us, bg_ussr, ctry_us, ctry_ussr }
}

/// `"-"` for zero, otherwise the number — so an occupied country stands out
/// from an empty one at a glance.
pub(crate) fn nz(value: u8) -> String {
    if value == 0 {
        "-".to_string()
    } else {
        value.to_string()
    }
}

/// The control-marker glyph: `<` for US, `>` for USSR, `:` for contested —
/// doubling as the separator between the two influence numbers.
pub(crate) fn control_glyph(controller: Option<crate::country::Superpower>) -> char {
    match controller {
        Some(crate::country::Superpower::Us) => '<',
        Some(crate::country::Superpower::Ussr) => '>',
        None => ':',
    }
}

/// The ops balance line shown wherever an operation is in progress: the
/// region view's footer, the world map's footer, and — since neither the
/// six-region dashboard nor the country detail view have room to show
/// pending state inline — a banner `main.rs` prints above them instead.
/// Worded the same way for every operation (just naming its own verb),
/// so switching views mid-action reads as the same session, not several.
///
/// For a realignment, the number shown per country is primarily the
/// *opponent's* net change there — the operation's own core metric,
/// mirroring how a placement's number is always the placing side's own
/// points added. A country that's also cost the acting side its own
/// influence (a later roll there went the other way) gets that named
/// too; a pure tie reads as "tied" rather than a bare `+0`.
pub fn operation_balance_line(layout: &crate::layout::MapLayout, board: &crate::board::Board, op: &crate::ops::Operation) -> String {
    let touched = op.touched();
    let where_touched = if touched.is_empty() {
        "nothing yet".to_string()
    } else {
        touched
            .iter()
            .map(|&id| touched_country_summary(layout, board, op, id))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{} {} · {} of {} ops left · {where_touched}",
        op.side(),
        op.verb(),
        op.remaining(),
        op.ops_total(),
    )
}

fn touched_country_summary(
    layout: &crate::layout::MapLayout,
    board: &crate::board::Board,
    op: &crate::ops::Operation,
    id: crate::country::CountryId,
) -> String {
    let name = layout.short_name(id);
    match op {
        crate::ops::Operation::Influence(p) => format!("{name} +{}", p.pending(id)),
        crate::ops::Operation::Realign(_) | crate::ops::Operation::Coup(_) => {
            let side = op.side();
            let opponent = side.opponent();
            let opp_delta = op.delta(board, id, opponent);
            let own_delta = op.delta(board, id, side);
            // A pure (0, 0) reads differently depending on how it can
            // happen: a realignment roll actually contested the two
            // totals and came out level, so "tied" fits; a coup with
            // nothing changed only ever means its one attempt fell short
            // of the target number, so "failed" is the honest word.
            let no_change = match op {
                crate::ops::Operation::Coup(_) => format!("{name} failed"),
                _ => format!("{name} tied"),
            };
            match (opp_delta, own_delta) {
                (0, 0) => no_change,
                (o, 0) => format!("{name} {opponent}{o:+}"),
                (0, s) => format!("{name} {side}{s:+}"),
                (o, s) => format!("{name} {opponent}{o:+}/{side}{s:+}"),
            }
        }
    }
}

/// One side's realignment modifier breakdown for the currently selected
/// country — shared by the region footer, the country detail view, and
/// the REPL's own `target`/`roll` output, so all three read identically.
pub fn modifier_line(side: crate::country::Superpower, mods: &crate::ops::Modifiers) -> String {
    let reasons = mods.reasons();
    let detail = if reasons.is_empty() { "no modifiers".to_string() } else { reasons.join(" · ") };
    format!("{side}  d6 {:+}   ({detail})", mods.total())
}

/// The outcome of one resolved realignment roll, in a single line — used
/// by both the REPL's own `roll` command and the interactive footer's
/// sticky roll message, so the wording is identical either way.
pub fn roll_result_line(map: &crate::map::WorldMap, side: crate::country::Superpower, result: &crate::ops::RollResult) -> String {
    let opponent = side.opponent();
    let country = &map.country(result.target).name;
    let acting_total = result.acting_die as i8 + result.acting_mods.total();
    let opposing_total = result.opposing_die as i8 + result.opposing_mods.total();
    let dice = format!(
        "{side} {}{:+}={acting_total} · {opponent} {}{:+}={opposing_total}",
        result.acting_die, result.acting_mods.total(), result.opposing_die, result.opposing_mods.total(),
    );
    let outcome = match result.loser {
        None => format!("a tie — no influence removed in {country}"),
        Some(loser) if loser == opponent => format!("{side} removes {} {opponent} influence from {country}", result.removed),
        Some(_) => format!("{opponent} wins the roll — {side} loses {} of its own influence in {country}", result.removed),
    };
    format!("{dice} → {outcome}")
}

/// The realignment odds line: each side's win/draw/loss share out of 36,
/// plus the expected influence swing on each side.
pub fn odds_line(side: crate::country::Superpower, odds: &crate::ops::Odds) -> String {
    let opponent = side.opponent();
    format!(
        "odds  {side} {}/36 · draw {}/36 · {opponent} {}/36   avg  {opponent} -{:.1} / {side} -{:.1}",
        odds.win,
        odds.draw,
        odds.loss,
        odds.removed_36ths as f32 / 36.0,
        odds.lost_36ths as f32 / 36.0,
    )
}

/// A coup's target number for the currently selected country, alongside
/// what's rolled against it — the `modifier_line` analogue for a coup,
/// shared by the region footer, the country detail view, and the REPL.
pub fn coup_target_line(side: crate::country::Superpower, ops: u8, target_number: u8, stability: u8) -> String {
    format!("{side}  d6 +{ops} vs {target_number}   (stability {stability} ×2)")
}

/// The coup odds line: success/failure share out of 6, plus the expected
/// influence swing on each side.
pub fn coup_odds_line(side: crate::country::Superpower, odds: &crate::ops::CoupOdds) -> String {
    let opponent = side.opponent();
    format!(
        "odds  {side} {}/6 · {opponent} {}/6   avg  {opponent} -{:.1} / {side} +{:.1}",
        odds.success,
        odds.failure,
        odds.removed_6ths as f32 / 6.0,
        odds.added_6ths as f32 / 6.0,
    )
}

/// The outcome of a resolved coup attempt, in a single line — used by
/// both the REPL's own `roll` command and the interactive footer's
/// sticky roll message, so the wording is identical either way.
pub fn coup_result_line(map: &crate::map::WorldMap, side: crate::country::Superpower, result: &crate::ops::CoupResult) -> String {
    let opponent = side.opponent();
    let country = &map.country(result.target).name;
    let modified = result.die as u16 + result.ops as u16;
    let dice = format!("{side} {}+{}={modified} vs {}", result.die, result.ops, result.target_number);
    let outcome = if !result.success() {
        format!("no better than {} — the coup fails in {country}", result.target_number)
    } else if result.added == 0 {
        format!("{side} removes {} {opponent} influence from {country}", result.removed)
    } else if result.removed == 0 {
        format!("{side} adds {} of its own influence to {country}", result.added)
    } else {
        format!(
            "{side} removes {} {opponent} influence from {country} and adds {} of its own",
            result.removed, result.added,
        )
    };
    format!("{dice} → {outcome}")
}
