//! The card piles: [`render_piles`], a tabbed, scrolling view of the discard pile, the
//! removed-from-play pile and the draw deck, and [`piles_text`], the same lists as plain
//! text for the REPL. The deck is hidden information, so it only ever shows a count.

use crate::cards::{Card, CardCatalog, CardSide, Hands};

use super::{Canvas, Color, Style, modal_box};

const WIDTH: usize = 60;
const PADDING: usize = 2;
/// Rows of the list visible at once.
const WINDOW: usize = 12;

/// Which pile a view is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PileTab {
    Discard,
    Removed,
    Deck,
}

impl PileTab {
    pub const ALL: [PileTab; 3] = [PileTab::Discard, PileTab::Removed, PileTab::Deck];

    pub fn label(self) -> &'static str {
        match self {
            PileTab::Discard => "Discard",
            PileTab::Removed => "Removed",
            PileTab::Deck => "Deck",
        }
    }

    /// The next tab, wrapping.
    pub fn next(self) -> PileTab {
        Self::ALL[(self.index() + 1) % 3]
    }

    /// The previous tab, wrapping.
    pub fn prev(self) -> PileTab {
        Self::ALL[(self.index() + 2) % 3]
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&t| t == self).expect("every tab is in ALL")
    }
}

/// The cards in `tab`, in pile order. The deck lists nothing: it's face down.
pub fn pile_cards(hands: &Hands, tab: PileTab) -> &[crate::cards::CardId] {
    match tab {
        PileTab::Discard => hands.discards(),
        PileTab::Removed => hands.removed(),
        PileTab::Deck => &[],
    }
}

/// How many cards `tab` holds (the deck's count is shown, its contents never).
pub fn pile_len(hands: &Hands, tab: PileTab) -> usize {
    match tab {
        PileTab::Deck => hands.deck().len(),
        _ => pile_cards(hands, tab).len(),
    }
}

fn card_row(card: &Card) -> (String, Color) {
    let ops = if card.scoring { "S".to_string() } else { card.ops.to_string() };
    let color = match card.side {
        CardSide::Us => Color::Us,
        CardSide::Ussr => Color::Ussr,
        CardSide::Neutral => Color::Muted,
    };
    let star = if card.removed_after_event { "*" } else { "" };
    (format!("{:>3}  {ops}  {}{star}", card.id.0, card.name), color)
}

/// One pile as a plain line-per-card list: `#  ops  name`, then a count.
pub fn piles_text(cards: &CardCatalog, hands: &Hands, tab: PileTab) -> String {
    if tab == PileTab::Deck {
        return format!("Deck: {} cards, face down", hands.deck().len());
    }
    let list = pile_cards(hands, tab);
    let mut out = format!("{} pile: {} card{}", tab.label(), list.len(), if list.len() == 1 { "" } else { "s" });
    for &id in list {
        out.push_str(&format!("\n  {}", card_row(cards.card(id)).0));
    }
    out
}

/// The tabbed pile view with a scrolling list on `tab`, the highlight on `cursor`.
pub fn render_piles(cards: &CardCatalog, hands: &Hands, tab: PileTab, cursor: usize) -> Canvas {
    let muted = Style::color(Color::Muted);
    let mut lines: Vec<(String, Style)> = Vec::new();
    let header: Vec<String> = PileTab::ALL
        .iter()
        .map(|&t| {
            let text = format!("{} {}", t.label(), pile_len(hands, t));
            if t == tab { format!("[{text}]") } else { format!(" {text} ") }
        })
        .collect();
    lines.push((header.join("  "), Style::default().bold()));
    lines.push((String::new(), Style::default()));

    let hint = match tab {
        PileTab::Deck => {
            lines.push((format!("{} cards are face down in the deck.", hands.deck().len()), Style::default()));
            lines.push(("Their order and identity are hidden information.".to_string(), muted));
            lines.push(("When it runs out the discard pile is shuffled into it".to_string(), muted));
            lines.push(("(the removed pile never comes back).".to_string(), muted));
            "←→/[ ] tab · Enter/Esc/D close"
        }
        _ => {
            let list = pile_cards(hands, tab);
            if list.is_empty() {
                lines.push((format!("The {} pile is empty.", tab.label().to_lowercase()), muted));
            }
            let cursor = cursor.min(list.len().saturating_sub(1));
            let start = cursor.saturating_sub(WINDOW / 2).min(list.len().saturating_sub(WINDOW));
            let end = (start + WINDOW).min(list.len());
            if start > 0 {
                lines.push((format!("   ↑ {start} more"), muted));
            }
            for (i, &id) in list.iter().enumerate().take(end).skip(start) {
                let (text, color) = card_row(cards.card(id));
                let style = if i == cursor { Style::color(color).bold() } else { Style::color(color) };
                lines.push((format!("{} {text}", if i == cursor { "›" } else { " " }), style));
            }
            if end < list.len() {
                lines.push((format!("   ↓ {} more", list.len() - end), muted));
            }
            if tab == PileTab::Removed || tab == PileTab::Discard {
                lines.push((String::new(), Style::default()));
                lines.push(("* removed from the game when played as an event".to_string(), muted));
            }
            "↑↓ move · z zoom · ←→ tab · Enter/Esc/D close"
        }
    };
    lines.push((String::new(), Style::default()));
    let text_width = WIDTH - 2 - 2 * PADDING;
    lines.push((format!("{hint:>text_width$}"), muted));

    modal_box("Card piles", "", WIDTH, Style::default(), false, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardId;

    fn cards() -> CardCatalog {
        CardCatalog::standard().unwrap()
    }

    #[test]
    fn the_deck_tab_shows_a_count_and_never_a_card() {
        let hands = Hands::default().with_deck(vec![CardId(4), CardId(5)]);
        let shown = render_piles(&cards(), &hands, PileTab::Deck, 0).render(crate::ColorMode::Never);
        assert!(shown.contains("2 cards are face down"));
        assert!(!shown.contains("Duck and Cover"));
        assert_eq!(piles_text(&cards(), &hands, PileTab::Deck), "Deck: 2 cards, face down");
    }

    #[test]
    fn the_discard_tab_lists_cards_and_scrolls() {
        let catalog = cards();
        let hands = Hands::with_piles(vec![], vec![], (7..27).map(CardId).collect(), vec![CardId(4)]);
        let top = render_piles(&catalog, &hands, PileTab::Discard, 0).render(crate::ColorMode::Never);
        assert!(top.contains("[Discard 20]") && top.contains("Removed 1") && top.contains("↓ 8 more"));
        let bottom = render_piles(&catalog, &hands, PileTab::Discard, 19).render(crate::ColorMode::Never);
        assert!(bottom.contains("↑ 8 more") && bottom.matches(" more").count() == 1, "scrolled to the end: only the 'more above' marker remains");
        assert!(piles_text(&catalog, &hands, PileTab::Removed).contains("Duck and Cover"));
    }

    #[test]
    fn tabs_wrap_in_both_directions() {
        assert_eq!(PileTab::Deck.next(), PileTab::Discard);
        assert_eq!(PileTab::Discard.prev(), PileTab::Deck);
    }
}
