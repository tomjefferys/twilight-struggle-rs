//! One side's hand, drawn as a strip of mini-card boxes — meant to sit
//! under every map screen (world/region/country) so a player can browse
//! their cards and the board at once. Always [`HAND_ROWS`] tall and a
//! fixed width, the same "never shifts the screen above it" rule
//! [`super::statusbar`]'s bar follows, for the same reason: this is drawn
//! at a fixed position on every screen, not just one.
//!
//! Still no card *event* behaviour here — this only ever shows what
//! [`Game::hand`] and [`Game::card_in_play_slot`] already hold. A played
//! card (`in_play`) is spliced back into its old spot in the strip rather
//! than just vanishing, since a keypress earlier isn't something a player
//! reliably remembers — and drawn unmistakably: a bright, bold, thick
//! border and an `IN PLAY` label in place of its usual side/phase line,
//! with every *other* slot dimmed so it can't compete for attention.
//! Nothing here mutates `Game` or decides what counts as "in play" — that
//! judgment, and the hand-index bookkeeping behind it, belongs to
//! [`Game::play_card`]/[`Game::return_card`] alone.

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

/// How a slot is drawn — at most one slot is ever [`SlotRole::Played`]
/// (there's only ever one card in play), and once there is, every other
/// slot is [`SlotRole::Dimmed`] rather than [`SlotRole::Selected`]: the
/// browsing cursor stops competing with the one card that actually
/// matters right now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotRole {
    Played,
    Selected,
    Dimmed,
    Normal,
}

/// Draws `side`'s hand. `china`, when `Some`, appends the China Card as
/// the hand's final slot — `Some(face_up)` reports which side it's
/// showing; it's never taken from `hand` itself, since
/// [`crate::cards::Hands`] deliberately never carries it (see that type's
/// own doc). `selected`, when given, is an index into the *combined*
/// list (hand cards, then China if present) — the thick-bordered slot
/// (while nothing's in play), and what a page beyond the first 10 cards
/// is centred on.
///
/// `in_play`, when given, is [`Game::card_in_play_slot`]'s own pair: the
/// played card's id, and the index in `hand` it should be spliced back
/// into for display (its original position, or `None` for the China
/// Card, which is never spliced into `hand` — it's already drawn via
/// `china` whether or not it's the one in play). The splice keeps the
/// strip from losing a card the moment it's played, and keeps the
/// remaining cards from shifting left into its old spot.
pub fn render_hand(cards: &CardCatalog, hand: &[CardId], china: Option<bool>, side: Superpower, selected: Option<usize>, in_play: Option<(CardId, Option<usize>)>) -> Canvas {
    let mut items: Vec<CardId> = hand.to_vec();
    if let Some((id, Some(index))) = in_play {
        items.insert(index.min(items.len()), id);
    }
    if china.is_some() {
        items.push(CHINA_CARD);
    }
    let total = items.len();
    let played_id = in_play.map(|(id, _)| id);

    let page = if total <= SLOTS_PER_PAGE {
        0
    } else {
        selected.unwrap_or(0).min(total - 1) / SLOTS_PER_PAGE
    };
    let start = page * SLOTS_PER_PAGE;
    let end = total.min(start + SLOTS_PER_PAGE);
    let page_items = &items[start..end];

    let hand_count = total - china.is_some() as usize;
    let mut header = format!("{side} hand · {hand_count} card{}", if hand_count == 1 { "" } else { "s" });
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
        let role = if Some(id) == played_id {
            SlotRole::Played
        } else if played_id.is_some() {
            SlotRole::Dimmed
        } else if selected == Some(start + idx) {
            SlotRole::Selected
        } else {
            SlotRole::Normal
        };
        draw_slot(&mut canvas, row, col, cards.card(id), china, role);
    }

    canvas
}

/// The strip while a card another event put into play waits to have its event played: just that
/// card, so nothing else in the hand competes with it, and a header saying where it came from.
pub fn render_forced_card(cards: &CardCatalog, card: CardId, host: CardId, side: Superpower) -> Canvas {
    let mut canvas = Canvas::new(HAND_WIDTH, HAND_ROWS);
    let side_style = match side {
        Superpower::Us => Style::color(Color::Us).bold(),
        Superpower::Ussr => Style::color(Color::Ussr).bold(),
    };
    canvas.put(0, 0, &format!("{} puts this card in play — its event has to be played now (e)  ·  z zoom", cards.card(host).name), side_style);
    draw_slot(&mut canvas, 1, 0, cards.card(card), None, SlotRole::Played);
    canvas.put(2, SLOT_W + 2, &format!("{} · {} ops", cards.card(card).name, cards.card(card).ops), Style::default().bold());
    for (i, line) in crate::render::wrap(&cards.card(card).text, 70).iter().take(5).enumerate() {
        canvas.put(3 + i, SLOT_W + 2, line, Style::default());
    }
    canvas
}

fn draw_slot(canvas: &mut Canvas, row: usize, col: usize, card: &Card, china: Option<bool>, role: SlotRole) {
    let is_china = card.id == CHINA_CARD;
    let (border_style, thick) = match role {
        SlotRole::Played => (Style::color(Color::Selected).bold(), true),
        SlotRole::Selected => (Style::color(Color::Selected), true),
        SlotRole::Dimmed => (Style::color(card_side_color(card.side)).dim(), false),
        SlotRole::Normal => (Style::color(card_side_color(card.side)), false),
    };
    if thick {
        canvas.draw_thick_box(row, col, SLOT_W, SLOT_H, border_style);
    } else {
        canvas.draw_box(row, col, SLOT_W, SLOT_H, border_style);
    }

    let content_style = match role {
        SlotRole::Played => Style::default().bold(),
        SlotRole::Dimmed => Style::default().dim(),
        SlotRole::Selected | SlotRole::Normal => Style::default(),
    };

    let content_w = SLOT_W - 2;
    let ops_label = card.ops_label();
    let name = truncate(&card.name, content_w.saturating_sub(ops_label.chars().count() + 1));
    canvas.put(row + 1, col + 1, &format!("{ops_label} {name}"), content_style);

    let line2 = if role == SlotRole::Played {
        "IN PLAY".to_string()
    } else if is_china {
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
    let line2_style = match role {
        SlotRole::Played => Style::color(Color::Selected).bold(),
        // `Color::Muted` already renders dim on its own (see `Theme::sgr`),
        // so a dimmed slot's line2 needs no extra `.dim()` beyond that.
        SlotRole::Dimmed | SlotRole::Selected | SlotRole::Normal => Style::color(Color::Muted),
    };
    canvas.put(row + 2, col + 1, &line2, line2_style);
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
            assert_eq!(render_hand(&cards, &hand, None, Superpower::Us, None, None).height(), HAND_ROWS);
        }
        let full = hand_of(&cards, &["Duck and Cover", "Five Year Plan", "Truman Doctrine", "NATO", "Independent Reds"]);
        assert_eq!(render_hand(&cards, &full, Some(true), Superpower::Us, None, None).height(), HAND_ROWS);
    }

    #[test]
    fn a_selected_slot_uses_thick_border_glyphs() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan"]);
        let text = render_hand(&cards, &hand, None, Superpower::Us, Some(0), None).render(ColorMode::Never);
        assert!(text.contains('┏'), "expected a thick border on the selected slot:\n{text}");
    }

    #[test]
    fn china_is_always_the_last_slot() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover"]);
        let text = render_hand(&cards, &hand, Some(true), Superpower::Us, None, None).render(ColorMode::Never);
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
        let text = render_hand(&cards, &hand, None, Superpower::Us, Some(11), None).render(ColorMode::Never);
        assert!(text.contains("of 12"), "expected pagination info:\n{text}");
        assert!(text.contains("SALT Negotia"), "expected the selected card's page to be shown:\n{text}");
    }

    #[test]
    fn color_never_emits_no_escape_codes() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover"]);
        let text = render_hand(&cards, &hand, Some(true), Superpower::Us, Some(0), None).render(ColorMode::Never);
        assert!(!text.contains('\x1b'), "ColorMode::Never should emit no escapes:\n{text}");
    }

    #[test]
    fn no_rendered_line_exceeds_the_hand_width() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan"]);
        let canvas = render_hand(&cards, &hand, Some(true), Superpower::Us, Some(1), None);
        let width = canvas.width();
        for line in canvas.render(ColorMode::Never).lines() {
            assert!(line.chars().count() <= width, "line {line:?} exceeds width {width}");
        }
    }

    #[test]
    fn a_played_card_stays_in_its_original_slot_instead_of_vanishing() {
        let cards = catalog();
        let full_hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan", "Truman Doctrine"]);
        let played = full_hand[1]; // Five Year Plan, originally index 1
        // Once played, `Game::hand` no longer contains it.
        let remaining: Vec<CardId> = full_hand.iter().copied().filter(|&id| id != played).collect();

        let text = render_hand(&cards, &remaining, None, Superpower::Us, None, Some((played, Some(1)))).render(ColorMode::Never);
        assert!(text.contains("Five Year"), "the played card should still be drawn, not disappear:\n{text}");
        assert!(text.contains("IN PLAY"), "the played slot should say so:\n{text}");
        // Still at its original position: Duck and Cover, then Five Year
        // Plan, then Truman Doctrine, left to right.
        let duck_pos = text.find("Duck").unwrap();
        let five_pos = text.find("Five Year").unwrap();
        let truman_pos = text.find("Truman").unwrap();
        assert!(duck_pos < five_pos && five_pos < truman_pos, "the played card should stay at its original index:\n{text}");
    }

    #[test]
    fn a_played_card_gets_a_thick_bright_border_and_every_other_slot_is_dimmed() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan"]);
        let played = hand[0];
        let remaining = vec![hand[1]];

        let played_text =
            render_hand(&cards, &remaining, None, Superpower::Us, None, Some((played, Some(0)))).render(ColorMode::Always);
        let none_played_text = render_hand(&cards, &hand, None, Superpower::Us, None, None).render(ColorMode::Always);
        assert_ne!(played_text, none_played_text, "a card in play should render differently from no card in play");

        let no_color = render_hand(&cards, &remaining, None, Superpower::Us, None, Some((played, Some(0)))).render(ColorMode::Never);
        assert!(no_color.contains('┏'), "the played slot should use a thick border:\n{no_color}");
    }

    #[test]
    fn the_browsing_cursor_is_suppressed_once_a_card_is_in_play() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan", "Truman Doctrine"]);
        let played = hand[0];
        let remaining: Vec<CardId> = hand[1..].to_vec();

        // `selected` points at Truman Doctrine, but with a card in play
        // every non-played slot is dimmed instead of thick-bordered —
        // only one slot (the played one) should use a thick border.
        let text = render_hand(&cards, &remaining, None, Superpower::Us, Some(1), Some((played, Some(0)))).render(ColorMode::Never);
        assert_eq!(text.matches('┏').count(), 1, "exactly the played slot should be thick-bordered:\n{text}");
    }

    #[test]
    fn the_header_card_count_includes_the_played_card_spliced_back_in() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover", "Five Year Plan"]);
        let played = hand[0];
        let remaining = vec![hand[1]];

        let text = render_hand(&cards, &remaining, None, Superpower::Us, None, Some((played, Some(0)))).render(ColorMode::Never);
        assert!(text.contains("2 cards"), "the header should count the played card too, matching what's drawn:\n{text}");
    }

    #[test]
    fn the_china_card_can_be_the_one_in_play_without_a_hand_index() {
        let cards = catalog();
        let hand = hand_of(&cards, &["Duck and Cover"]);
        let text = render_hand(&cards, &hand, Some(true), Superpower::Us, None, Some((CHINA_CARD, None))).render(ColorMode::Never);
        assert!(text.contains("IN PLAY"), "the China Card's slot should show it's in play:\n{text}");
        assert!(text.contains("Duck and"), "the rest of the hand should still be drawn, just dimmed:\n{text}");
    }
}
