//! The title screen: block-letter title flanked by a US and a Soviet missile,
//! a DEFCON strip, and the start menu. A pure view — `splash.rs` (the binary's
//! menu state machine) supplies a [`SplashMenu`] and draws the result centred.

use super::{modal_box, Canvas, Color, Style};

/// Width of the splash canvas. Fixed, so the view (and its snapshot) doesn't
/// depend on the terminal; the caller centres it.
pub const SPLASH_WIDTH: usize = 72;

/// Width of the menu box.
const MENU_WIDTH: usize = 40;

/// One menu line.
pub struct SplashItem {
    pub label: String,
    /// A disabled item is shown muted (the menu skips it when moving).
    pub enabled: bool,
}

/// What the menu currently offers.
pub struct SplashMenu {
    pub title: String,
    pub items: Vec<SplashItem>,
    pub selected: usize,
}

// Five-row block letters, `#` for a filled cell.
fn glyph(ch: char) -> [&'static str; 5] {
    match ch {
        'T' => ["#####", "  #  ", "  #  ", "  #  ", "  #  "],
        'W' => ["#   #", "#   #", "# # #", "## ##", "#   #"],
        'I' => ["###", " # ", " # ", " # ", "###"],
        'L' => ["#    ", "#    ", "#    ", "#    ", "#####"],
        'G' => [" ####", "#    ", "#  ##", "#   #", " ### "],
        'H' => ["#   #", "#   #", "#####", "#   #", "#   #"],
        'S' => [" ####", "#    ", " ### ", "    #", "#### "],
        'R' => ["#### ", "#   #", "#### ", "#  # ", "#   #"],
        'U' => ["#   #", "#   #", "#   #", "#   #", " ### "],
        'E' => ["#####", "#    ", "#### ", "#    ", "#####"],
        _ => ["     "; 5],
    }
}

fn big_word(word: &str) -> [String; 5] {
    let mut rows: [String; 5] = Default::default();
    for (i, ch) in word.chars().enumerate() {
        for (r, row) in rows.iter_mut().enumerate() {
            if i > 0 {
                row.push(' ');
            }
            row.push_str(&glyph(ch)[r].replace('#', "█"));
        }
    }
    rows
}

const MISSILE: [&str; 11] = [
    "  ▲  ",
    " /█\\ ",
    " |█| ",
    " |█| ",
    " |█| ",
    " |█| ",
    " |█| ",
    "/|█|\\",
    "█████",
    " ▓▓▓ ",
    " ░░░ ",
];

fn put_centred(canvas: &mut Canvas, row: usize, text: &str, style: Style) {
    let len = text.chars().count();
    canvas.put(row, SPLASH_WIDTH.saturating_sub(len) / 2, text, style);
}

/// The whole title screen with `menu` below the art.
pub fn render_splash(menu: &SplashMenu) -> Canvas {
    let us = Style::color(Color::Us).bold();
    let ussr = Style::color(Color::Ussr).bold();
    // 15 rows of art, then the menu box: its items, a blank, two hint lines and the frame.
    let height = 15 + menu.items.len() + 5;
    let mut canvas = Canvas::new(SPLASH_WIDTH, height);

    // Title, flanked by the two missiles.
    let top = big_word("TWILIGHT");
    let bottom = big_word("STRUGGLE");
    for (i, line) in top.iter().enumerate() {
        let w = line.chars().count();
        canvas.put(i, SPLASH_WIDTH.saturating_sub(w) / 2, line, us);
    }
    for (i, line) in bottom.iter().enumerate() {
        let w = line.chars().count();
        canvas.put(6 + i, SPLASH_WIDTH.saturating_sub(w) / 2, line, ussr);
    }
    for (i, line) in MISSILE.iter().enumerate() {
        canvas.put(i, 3, line, us);
        canvas.put(i, SPLASH_WIDTH - 3 - 5, line, ussr);
    }
    canvas.put(0, 9, "★", Style::color(Color::Battleground).bold());
    canvas.put(0, SPLASH_WIDTH - 10, "☭", Style::color(Color::Battleground).bold());

    // The iron curtain, years, DEFCON strip and tagline.
    let mut row = 11;
    put_centred(&mut canvas, row, "─ ─ ─ ─ ─ ─ ─ ─  1945 — 1989  ─ ─ ─ ─ ─ ─ ─ ─", Style::color(Color::Muted));
    row += 1;
    let strip_col = (SPLASH_WIDTH - 37) / 2;
    canvas.put(row, strip_col, "DEFCON", Style::color(Color::Default).bold());
    let tint = [Color::SouthAmerica, Color::SouthAmerica, Color::Battleground, Color::Ussr, Color::Ussr];
    for (i, color) in tint.iter().enumerate() {
        let text = format!("[ {} ]", 5 - i);
        canvas.put(row, strip_col + 8 + i * 6, &text, Style::color(*color).bold());
    }
    row += 1;
    put_centred(&mut canvas, row, "The Cold War. Contain or expand.", Style::color(Color::Muted));
    row += 2;

    // The menu.
    let mut lines: Vec<(String, Style)> = Vec::new();
    for (i, item) in menu.items.iter().enumerate() {
        let (marker, style) = if !item.enabled {
            ("  ", Style::color(Color::Muted))
        } else if i == menu.selected {
            ("▶ ", Style::color(Color::Selected).bold())
        } else {
            ("  ", Style::default())
        };
        lines.push((format!("{marker}{}", item.label), style));
    }
    lines.push((String::new(), Style::default()));
    lines.push(("↑↓ move · Enter select".to_string(), Style::color(Color::Muted)));
    lines.push(("Esc back · q quit".to_string(), Style::color(Color::Muted)));
    let boxed = modal_box(&menu.title, "", MENU_WIDTH, Style::color(Color::Battleground), true, lines);
    canvas.blit(&boxed, row, (SPLASH_WIDTH - MENU_WIDTH) / 2);
    canvas
}
