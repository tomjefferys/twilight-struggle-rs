//! A single card's full detail — the "zoom" view opened from the hand
//! strip ([`super::hand`]). A titled box like [`super::country::render_country`]'s
//! own outer box: the title sits on the top border via [`put_border_title`]
//! rather than a divider (its own ops value — or `Scoring` — on the left,
//! since the card's id is already the footer's own "card #N" line), the
//! body is the card's rules text word-wrapped to the box's own width, and
//! a flags line + that printed number fill out the bottom the same way
//! that view's panels do.

use crate::cards::{Card, CardCatalog, CardId, CHINA_CARD};

use super::{card_side_color, put_border_title, wrap, Canvas, Color, Style};

/// Fixed content width for every card — the physical card's own rules
/// text varies a lot in length, but nothing about a card's *width* should
/// change as a player zooms from one to the next.
const CARD_WIDTH: usize = 60;
/// Columns of padding inside the border on each side.
const PADDING: usize = 2;

/// `china_face_up`: only meaningful when `id` is [`CHINA_CARD`] — whether
/// it's currently showing its Operations side (face up) or sitting spent
/// (face down), shown in place of the side/phase flags every other card
/// gets, since the China Card's own side/phase don't mean much next to
/// that.
pub fn render_card(cards: &CardCatalog, id: CardId, china_face_up: Option<bool>) -> Canvas {
    let card = cards.card(id);
    let text_width = CARD_WIDTH - 2 - 2 * PADDING;

    let mut lines: Vec<(String, Style)> = wrap(&card.text, text_width).into_iter().map(|l| (l, Style::default())).collect();

    let flags = card_flags(card, id, china_face_up);
    if !flags.is_empty() {
        lines.push((String::new(), Style::default()));
        // Word-wrapped like the body text above — a card can carry several
        // flags at once (optional *and* removed-after-event *and* ongoing,
        // say), and the joined line can run well past the box's own width.
        for line in wrap(&flags.join(" · "), text_width) {
            lines.push((line, Style::color(Color::Muted)));
        }
    }
    lines.push((String::new(), Style::default()));
    let footer = format!("card #{}", card.id);
    lines.push((format!("{footer:>width$}", width = text_width), Style::color(Color::Muted)));

    let box_height = 2 + lines.len();
    let mut canvas = Canvas::new(CARD_WIDTH, box_height);

    let border_style = Style::color(card_side_color(card.side));
    canvas.draw_box(0, 0, CARD_WIDTH, box_height, border_style);

    // The ops value, not the card's id — that's already the footer's own
    // "card #N" line, and showing it again here just duplicated it
    // without saying anything about the card itself. A scoring card reads
    // "Scoring" instead of "Ops 0": its ops value is never spent as ops
    // (see `Card::scoring`'s own doc), so "Ops 0" would be technically
    // true but misleading.
    let title_left = if card.scoring { format!("Scoring · {}", card.name) } else { format!("Ops {} · {}", card.ops, card.name) };
    let title_right = format!("{} · {}", card.side, card.phase);
    put_border_title(&mut canvas, 0, 0, &title_left, Style::default().bold(), &title_right, border_style, CARD_WIDTH);

    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }

    canvas
}

/// The flags row under a card's text: whichever of optional/removed/
/// ongoing/scoring actually apply, in the order a player would care about
/// them — plus, for the China Card specifically, whether it's currently
/// face up or down in place of the others (none of the normal flags say
/// anything useful about it).
fn card_flags(card: &Card, id: CardId, china_face_up: Option<bool>) -> Vec<String> {
    if id == CHINA_CARD {
        return match china_face_up {
            Some(true) => vec!["face up".to_string()],
            Some(false) => vec!["face down".to_string()],
            None => Vec::new(),
        };
    }
    let mut flags = Vec::new();
    if card.scoring {
        flags.push("Scoring card — may not be held".to_string());
    }
    if card.removed_after_event {
        flags.push("Removed from play once played as an event".to_string());
    }
    if card.ongoing {
        flags.push("Ongoing effect".to_string());
    }
    if card.optional {
        flags.push("Optional card".to_string());
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::ColorMode;

    fn catalog() -> CardCatalog {
        CardCatalog::standard().unwrap()
    }

    #[test]
    fn every_card_in_the_catalog_renders_without_exceeding_its_own_width() {
        let cards = catalog();
        for number in 1..=cards.len() as u8 {
            let id = CardId(number);
            let china_face_up = (id == CHINA_CARD).then_some(true);
            let canvas = render_card(&cards, id, china_face_up);
            let width = canvas.width();
            for line in canvas.render(ColorMode::Never).lines() {
                assert!(line.chars().count() <= width, "card {number} line {line:?} exceeds width {width}");
            }
        }
    }

    #[test]
    fn the_china_card_shows_face_up_or_down_instead_of_its_other_flags() {
        let cards = catalog();
        let up = render_card(&cards, CHINA_CARD, Some(true)).render(ColorMode::Never);
        assert!(up.contains("face up"), "expected face up in:\n{up}");
        let down = render_card(&cards, CHINA_CARD, Some(false)).render(ColorMode::Never);
        assert!(down.contains("face down"), "expected face down in:\n{down}");
    }

    #[test]
    fn a_scoring_card_names_itself_in_its_flags() {
        let cards = catalog();
        let text = render_card(&cards, CardId(1), None).render(ColorMode::Never);
        assert!(text.contains("Scoring card"), "expected a scoring flag in:\n{text}");
    }

    #[test]
    fn the_title_shows_ops_not_a_second_copy_of_the_card_number() {
        let cards = catalog();
        let text = render_card(&cards, CardId(31), None).render(ColorMode::Never);
        let title = text.lines().next().unwrap();
        assert!(title.contains("Ops 4"), "expected the ops value in the title: {title:?}");
        // "card #31" belongs only on the footer line — the title used to
        // repeat the bare id instead of saying anything about the card.
        assert_eq!(text.matches("31").count(), 1, "the card's number should appear exactly once:\n{text}");
    }

    #[test]
    fn a_scoring_cards_title_says_scoring_instead_of_ops_0() {
        let cards = catalog();
        let text = render_card(&cards, CardId(1), None).render(ColorMode::Never);
        let title = text.lines().next().unwrap();
        assert!(title.contains("Scoring"), "expected 'Scoring' in the title: {title:?}");
        assert!(!title.contains("Ops"), "a scoring card's title shouldn't say 'Ops 0': {title:?}");
    }

    #[test]
    fn a_long_flags_line_wraps_instead_of_overrunning_the_border() {
        // "Warsaw Pact Formed" carries both the removed-after-event and
        // ongoing flags, whose joined line used to run past the box's own
        // width and overwrite the right border instead of wrapping.
        let cards = catalog();
        let id = cards.id_by_name("Warsaw Pact Formed").unwrap();
        let text = render_card(&cards, id, None).render(ColorMode::Never);
        let lines: Vec<&str> = text.lines().collect();
        for line in &lines[1..lines.len() - 1] {
            assert!(line.ends_with('│'), "lost the right border: {line:?}\nfull card:\n{text}");
        }
    }

    #[test]
    fn every_interior_row_keeps_its_right_border() {
        let cards = catalog();
        for number in 1..=cards.len() as u8 {
            let id = CardId(number);
            let china_face_up = (id == CHINA_CARD).then_some(true);
            let text = render_card(&cards, id, china_face_up).render(ColorMode::Never);
            let lines: Vec<&str> = text.lines().collect();
            for line in &lines[1..lines.len() - 1] {
                assert!(line.ends_with('│'), "card {number} lost its right border: {line:?}");
            }
        }
    }

    #[test]
    fn color_never_emits_no_escape_codes() {
        let cards = catalog();
        let text = render_card(&cards, CardId(31), None).render(ColorMode::Never);
        assert!(!text.contains('\x1b'), "ColorMode::Never should emit no escapes:\n{text}");
    }
}
