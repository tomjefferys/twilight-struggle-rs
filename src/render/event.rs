//! The post-event result modal for a resolved fixed-effect card
//! (`events::effects`) — the same titled-box treatment
//! [`super::score::render_scoring_result`] gives a scoring card, since
//! these events are just as irreversible and just as easy to miss in a
//! one-line message: one row per influence change, the DEFCON move, and
//! the VP swing and new total. Blitted centred over whichever screen
//! `interactive.rs` is showing and dismissed with Enter.

use crate::cards::CardCatalog;
use crate::country::Superpower;
use crate::events::EffectResult;
use crate::game::Victory;
use crate::map::WorldMap;

use super::{game_over_line, ongoing_effect_line, put_border_title, wrap, Canvas, Color, Style};

const EVENT_WIDTH: usize = 56;
const PADDING: usize = 2;

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

/// Draws `result` as a titled box, bordered in the playing side's colour
/// (or the winner's, if the event just ended the game). `queue_pos` means
/// the same as for [`super::render_scoring_result`].
pub fn render_event_result(
    map: &WorldMap,
    cards: &CardCatalog,
    result: &EffectResult,
    vp_after: i8,
    winner: Option<Victory>,
    queue_pos: Option<(usize, usize)>,
) -> Canvas {
    let title = cards.card(result.card).name.clone();
    let text_width = EVENT_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();

    for c in &result.influence {
        let name = &map.country(c.country).name;
        lines.push((format!("{name}  {} influence {} → {}", c.side, c.before, c.after), Style::color(side_color(c.side))));
    }
    if let Some((before, after)) = result.defcon {
        lines.push((format!("DEFCON {before} → {after}"), Style::color(Color::Muted).bold()));
    }
    if result.mil_ops != 0 {
        lines.push((format!("{} Military Operations {:+}", result.player, result.mil_ops), Style::color(side_color(result.player))));
    }
    if let Some((side, card)) = result.discards {
        lines.push((format!("{side} discards {}", cards.card(card).name), Style::color(side_color(side))));
    }
    if let Some(reveal) = &result.reveals {
        let names: Vec<&str> = reveal.cards.iter().map(|&c| cards.card(c).name.as_str()).collect();
        let shown = if names.is_empty() { "(empty)".to_string() } else { names.join(", ") };
        for part in wrap(&format!("{} reveals their hand: {shown}", reveal.side), text_width) {
            lines.push((part, Style::color(side_color(reveal.side))));
        }
    }
    if let Some(t) = &result.china {
        lines.push((format!("China Card → {} ({})", t.to, if t.face_up { "face up" } else { "face down" }), Style::color(side_color(t.to)).bold()));
    }
    if let Some((side, from, to)) = result.space {
        lines.push((format!("{side} space race: box {from} → {to} ({})", crate::space::space_box(to).name), Style::color(side_color(side)).bold()));
    }
    if let Some(effect) = &result.ongoing {
        if !lines.is_empty() {
            lines.push((String::new(), Style::default()));
        }
        let style = Style::color(side_color(effect.side())).bold();
        // The box's title already names the card, so give just what it does.
        let line = ongoing_effect_line(effect);
        let what = line.split_once(": ").map_or(line.as_str(), |(_, rest)| rest);
        for part in wrap(&format!("In effect until the turn ends: {what}"), text_width) {
            lines.push((part, style));
        }
    }
    if !lines.is_empty() {
        lines.push((String::new(), Style::default()));
    }
    // A card that only starts a turn-long effect has no VP line to show.
    let nothing_else = result.ongoing.is_some() && result.vp_delta == 0;
    let (text, style) = match result.vp_delta.signum() {
        _ if nothing_else => (String::new(), Style::default()),
        1 => (format!("+{} VP to the US (now {vp_after})", result.vp_delta), Style::color(Color::Us).bold()),
        -1 => (format!("+{} VP to the USSR (now {vp_after})", -result.vp_delta), Style::color(Color::Ussr).bold()),
        _ => (format!("no VP change (still {vp_after})"), Style::color(Color::Muted)),
    };
    if !nothing_else {
        lines.push((text, style));
    } else {
        lines.pop();
    }

    let mut border = side_color(result.player);
    if let Some(victory) = winner {
        lines.push((String::new(), Style::default()));
        lines.push((game_over_line(victory), Style::color(side_color(victory.side)).bold()));
        border = side_color(victory.side);
    }

    lines.push((String::new(), Style::default()));
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));

    let height = 2 + lines.len();
    let mut canvas = Canvas::new(EVENT_WIDTH, height);
    canvas.draw_thick_box(0, 0, EVENT_WIDTH, height, Style::color(border));
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), EVENT_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardId;
    use crate::ongoing::OngoingEffect;
    use crate::render::ColorMode;

    fn result(ongoing: Option<OngoingEffect>, vp_delta: i8) -> EffectResult {
        EffectResult { card: CardId(25), player: Superpower::Us, influence: Vec::new(), vp_delta, defcon: None, ongoing, lasting: None, cancels: None, china: None, space: None, mil_ops: 0, ends_game: false, reveals: None, discards: None }
    }

    #[test]
    fn a_card_that_only_starts_an_effect_says_so_and_has_no_vp_line() {
        let map = WorldMap::standard().unwrap();
        let cards = CardCatalog::standard().unwrap();
        let text = render_event_result(&map, &cards, &result(Some(OngoingEffect::Containment), 0), 0, None, None).render(ColorMode::Never);
        assert!(text.contains("In effect until the turn ends: US ops +1"), "{text}");
        assert!(!text.contains("no VP change"), "{text}");
    }

    #[test]
    fn a_card_with_no_effect_and_no_vp_still_reports_no_vp_change() {
        let map = WorldMap::standard().unwrap();
        let cards = CardCatalog::standard().unwrap();
        let text = render_event_result(&map, &cards, &result(None, 0), 3, None, None).render(ColorMode::Never);
        assert!(text.contains("no VP change (still 3)"), "{text}");
    }
}
