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

use super::{game_over_line, put_border_title, Canvas, Color, Style};

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
    if !lines.is_empty() {
        lines.push((String::new(), Style::default()));
    }
    let (text, style) = match result.vp_delta.signum() {
        1 => (format!("+{} VP to the US (now {vp_after})", result.vp_delta), Style::color(Color::Us).bold()),
        -1 => (format!("+{} VP to the USSR (now {vp_after})", -result.vp_delta), Style::color(Color::Ussr).bold()),
        _ => (format!("no VP change (still {vp_after})"), Style::color(Color::Muted)),
    };
    lines.push((text, style));

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
