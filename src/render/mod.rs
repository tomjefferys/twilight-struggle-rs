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
        if w < 2 || h < 2 {
            return;
        }
        self.put_char(row, col, '┌', style);
        self.put_char(row, col + w - 1, '┐', style);
        self.put_char(row + h - 1, col, '└', style);
        self.put_char(row + h - 1, col + w - 1, '┘', style);
        for c in (col + 1)..(col + w - 1) {
            self.put_char(row, c, '─', style);
            self.put_char(row + h - 1, c, '─', style);
        }
        for r in (row + 1)..(row + h - 1) {
            self.put_char(r, col, '│', style);
            self.put_char(r, col + w - 1, '│', style);
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
