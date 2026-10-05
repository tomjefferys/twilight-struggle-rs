//! The post-roll modal for a resolved war card (`events::war`) — the same
//! titled-box treatment [`super::roll::render_roll_result`] gives a coup:
//! the die and what lowered it, whether it won, the influence replaced,
//! VP and Military Operations. Blitted centred by `interactive.rs` and
//! dismissed with Enter.

use crate::cards::CardCatalog;
use crate::country::Superpower;
use crate::events::WarResult;
use crate::game::Victory;
use crate::map::WorldMap;

use super::{game_over_line, war_reasons, Canvas, Color, Style, modal_box};

const WAR_WIDTH: usize = 56;
const PADDING: usize = 2;

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

/// Draws `result` as a titled box, bordered in the winner's colour (muted
/// for a failed war, or the game's winner if it ended the game).
pub fn render_war_result(
    map: &WorldMap,
    cards: &CardCatalog,
    result: &WarResult,
    vp_after: i8,
    winner: Option<Victory>,
    queue_pos: Option<(usize, usize)>,
) -> Canvas {
    let target = &map.country(result.target).name;
    let title = format!("{} · {target}", cards.card(result.card).name);
    let text_width = WAR_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    let side = result.side;

    lines.push((format!("{side}  d6: {}   mod: {:+}   = {}", result.die, result.modifier.total(), result.modified()), Style::color(side_color(side))));
    lines.push((format!("  ({})", war_reasons(map, &result.modifier)), Style::color(Color::Muted)));
    lines.push((format!("needs {}+ to win", result.success_min), Style::color(Color::Muted)));
    lines.push((String::new(), Style::default()));

    let outcome = if result.success {
        (format!("{side} WINS — the invasion of {target} succeeds"), Style::color(side_color(side)).bold())
    } else {
        (format!("The invasion of {target} FAILS"), Style::color(Color::Muted).bold())
    };
    lines.push(outcome);
    for c in &result.influence {
        lines.push((format!("{target}  {} influence {} → {}", c.side, c.before, c.after), Style::color(side_color(c.side))));
    }
    lines.push((String::new(), Style::default()));
    if result.vp_delta != 0 {
        let (who, n) = if result.vp_delta > 0 { ("US", result.vp_delta) } else { ("USSR", -result.vp_delta) };
        lines.push((format!("+{n} VP to the {who} (now {vp_after})"), Style::color(side_color(side)).bold()));
    } else {
        lines.push((format!("no VP change (still {vp_after})"), Style::color(Color::Muted)));
    }
    lines.push((format!("{side} Military Operations +{}", result.mil_ops), Style::color(Color::Muted)));

    let mut border = if result.success { side_color(side) } else { Color::Muted };
    if let Some(victory) = winner {
        lines.push((String::new(), Style::default()));
        lines.push((game_over_line(victory), Style::color(victory.side.map_or(Color::Muted, side_color)).bold()));
        border = victory.side.map_or(Color::Muted, side_color);
    }
    lines.push((String::new(), Style::default()));
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));

    modal_box(&title, "", WAR_WIDTH, Style::color(border), true, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::cards::CardId;
    use crate::render::ColorMode;

    #[test]
    fn a_failed_war_says_so_and_a_win_shows_the_replaced_influence() {
        let map = WorldMap::standard().unwrap();
        let cards = CardCatalog::standard().unwrap();
        let sk = map.id_by_name("South Korea").unwrap();
        let mut board = Board::new(&map);
        board.set_influence(sk, Superpower::Us, 2);
        let lose = crate::events::war::resolve(&map, &board, CardId(11), sk, Superpower::Ussr, 3).unwrap();
        let text = render_war_result(&map, &cards, &lose, 0, None, None).render(ColorMode::Never);
        assert!(text.contains("FAILS") && text.contains("Korean War · South Korea"), "{text}");
        let win = crate::events::war::resolve(&map, &board, CardId(11), sk, Superpower::Ussr, 6).unwrap();
        let text = render_war_result(&map, &cards, &win, -2, None, Some((1, 2))).render(ColorMode::Never);
        assert!(text.contains("USA influence 2 → 0") && text.contains("+2 VP to the USSR (now -2)") && text.contains("1 of 2"), "{text}");
    }
}
