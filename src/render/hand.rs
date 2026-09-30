//! One side's hand, drawn as a strip of mini-card boxes — meant to sit
//! under every map screen (world/region/country) so a player can browse
//! their cards and the board at once. Always [`HAND_ROWS`] tall and a
//! fixed width, the same "never shifts the screen above it" rule
//! [`super::statusbar`]'s bar follows, for the same reason: this is drawn
//! at a fixed position on every screen, not just one.
//!
//! No card behaviour lives here — this only ever shows what [`Game::hand`]
//! already holds; playing, drawing, or discarding a card doesn't exist
//! yet.

use crate::cards::{Card, CardCatalog, CardId, CHINA_CARD};
use crate::country::Superpower;

use super::{card_side_color, Canvas, Color, Style};

const HAND_COLS: usize = 5;
const HAND_VISIBLE_ROWS: usize = 2;
const SLOT_W: usize = 17;
const SLOT_H: usize = 4;
const SLOTS_PER_PAGE: usize = HAND_COLS * HAND_VISIBLE_ROWS;

pub const HAND_WIDTH: usize = HAND_COLS * SLOT_W;
/// The strip's fixed height: one header row plus two rows of boxes —
/// always this, whether the hand is empty or full, so the view above it
/// never shifts as a card is drawn or played.
pub const HAND_ROWS: usize = 1 + HAND_VISIBLE_ROWS * SLOT_H;

/// Draws `side`'s hand. `china`, when `Some`, appends the China Card as
/// the hand's final slot — `Some(face_up)` reports which side it's
/// showing; it's never taken from `hand` itself, since
/// [`crate::cards::Hands`] deliberately never carries it (see that type's
/// own doc). `selected`, when given, is an index into the *combined*
/// list (hand cards, then China if present) — the thick-bordered slot,
/// and what a page beyond the first 10 cards is centred on.
pub fn render_hand(cards: &CardCatalog, hand: &[CardId], china: Option<bool>, side: Superpower, selected: Option<usize>) -> Canvas {
    let mut items: Vec<CardId> = hand.to_vec();
    if china.is_some() {
        items.push(CHINA_CARD);
    }
    let total = items.len();

    let page = if total <= SLOTS_PER_PAGE {
        0
    } else {
        selected.unwrap_or(0).min(total - 1) / SLOTS_PER_PAGE
    };
    let start = page * SLOTS_PER_PAGE;
    let end = total.min(start + SLOTS_PER_PAGE);
    let page_items = &items[start..end];

    let mut header = format!("{side} hand · {} card{}", hand.len(), if hand.len() == 1 { "" } else { "s" });
    if china.is_some() {
        header.push_str(" + China");
    }
    if total > SLOTS_PER_PAGE {
        header.push_str(&format!(" · cards {}-{} of {total}", start + 1, end));
    }
    header.push_str(" — [ ] select · z zoom");

    let mut canvas = Canvas::new(HAND_WIDTH, HAND_ROWS);
    let side_style = match side {
        Superpower::Us => Style::color(Color::Us).bold(),
        Superpower::Ussr => Style::color(Color::Ussr).bold(),
    };
    canvas.put(0, 0, &header, side_style);

    for (idx, &id) in page_items.iter().enumerate() {
        let row = 1 + (idx / HAND_COLS) * SLOT_H;
        let col = (idx % HAND_COLS) * SLOT_W;
        let is_selected = selected == Some(start + idx);
        draw_slot(&mut canvas, row, col, cards.card(id), china, is_selected);
    }

    canvas
}

fn draw_slot(canvas: &mut Canvas, row: usize, col: usize, card: &Card, china: Option<bool>, selected: bool) {
    let is_china = card.id == CHINA_CARD;
    let border_style = if selected { Style::color(Color::Selected) } else { Style::color(card_side_color(card.side)) };
    if selected {
        canvas.draw_thick_box(row, col, SLOT_W, SLOT_H, border_style);
    } else {
        canvas.draw_box(row, col, SLOT_W, SLOT_H, border_style);
    }

    let content_w = SLOT_W - 2;
    let ops_label = card.ops_label();
    let name = truncate(&card.name, content_w.saturating_sub(ops_label.chars().count() + 1));
    canvas.put(row + 1, col + 1, &format!("{ops_label} {name}"), Style::default());

    let line2 = if is_china {
        match china {
            Some(true) => "face up".to_string(),
            Some(false) => "face down".to_string(),
            None => String::new(),
        }
    } else {
        let side_short = match card.side {
            crate::cards::CardSide::Us => "US",
            crate::cards::CardSide::Ussr => "USSR",
            crate::cards::CardSide::Neutral => "Both",
        };
        let flag = if card.removed_after_event { "*" } else { "" };
        format!("{side_short:<4} {}{flag}", card.phase.short())
    };
    canvas.put(row + 2, col + 1, &line2, Style::color(Color::Muted));
}

/// Truncates `s` to at most `max` characters, replacing the last one with
/// `…` if it had to cut anything — never returns more than `max` chars.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max || max == 0 {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::ColorMode;

    fn catalog() -> CardCatalog {
        CardCatalog::standard().unwrap()
    }

    fn hand_of(cards: &CardCatalog, names: &[&str]) -> Vec<CardId> {
        names.iter().map(|n| cards.id_by_name(n).unwrap()).collect()
    }

    #[test]
    fn the_strip_is_always_hand_rows_tall() {
        let cards = catalog();
        for hand in [vec![], hand_of(&cards, &["Duck and Cover"])] {
            assert_eq!(render_hand(&cards, &hand, None, Superpower::Us, None).height(), HAND_ROWS);
        }
        let full = hand_of(&cards, &["Duck and Cover", "Five Year Plan", "Truman Doctrine", "NATO", "Independent Reds"]);
        assert_eq!(render_hand(&cards, &full, Some(true), Superpower::Us, None).height(), HAND_ROWS);
    }

    #[test]
    fn a_selected_slot_uses_thick_border_glyphs() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan"]);
        let text = render_hand(&cards, &hand, None, Superpower::Us, Some(0)).render(ColorMode::Never);
        assert!(text.contains('┏'), "expected a thick border on the selected slot:\n{text}");
    }

    #[test]
    fn china_is_always_the_last_slot() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover"]);
        let text = render_hand(&cards, &hand, Some(true), Superpower::Us, None).render(ColorMode::Never);
        assert!(text.contains("China C"), "expected the China Card slot:\n{text}");
        assert!(text.contains("face up"), "expected its face-up status:\n{text}");
        assert!(text.contains("+ China"), "expected the header to mention China:\n{text}");
    }

    #[test]
    fn more_than_ten_cards_paginates_around_the_selection() {
        let cards = catalog();
        let names = [
            "Duck and Cover", "Five Year Plan", "Truman Doctrine", "NATO", "Independent Reds", "Marshall Plan", "Containment", "CIA Created",
            "US/Japan Mutual Defense Pact", "Indo-Pakistani War", "Olympic Games", "SALT Negotiations",
        ];
        let hand = hand_of(&cards, &names);
        let text = render_hand(&cards, &hand, None, Superpower::Us, Some(11)).render(ColorMode::Never);
        assert!(text.contains("of 12"), "expected pagination info:\n{text}");
        assert!(text.contains("SALT Negotia"), "expected the selected card's page to be shown:\n{text}");
    }

    #[test]
    fn color_never_emits_no_escape_codes() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover"]);
        let text = render_hand(&cards, &hand, Some(true), Superpower::Us, Some(0)).render(ColorMode::Never);
        assert!(!text.contains('\x1b'), "ColorMode::Never should emit no escapes:\n{text}");
    }

    #[test]
    fn no_rendered_line_exceeds_the_hand_width() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan"]);
        let canvas = render_hand(&cards, &hand, Some(true), Superpower::Us, Some(1));
        let width = canvas.width();
        for line in canvas.render(ColorMode::Never).lines() {
            assert!(line.chars().count() <= width, "line {line:?} exceeds width {width}");
        }
    }
}
