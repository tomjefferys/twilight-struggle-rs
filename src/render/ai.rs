//! The modals shown while the AI plays a turn step by step (interactive mode's default; `fast`
//! mode skips them). Each one is a pause before something the human would otherwise only see
//! the aftermath of: a card announced, a die about to be thrown, a realignment's rolls as they
//! land, where influence went. Pure `Canvas` producers, like every other view.

use crate::board::Board;
use crate::country::{CountryId, Superpower};
use crate::game::RollOutcome;
use crate::map::WorldMap;
use crate::ops::Operation;
use crate::status::GameStatus;

use super::roll::{report_body, side_color, target_preview, RollReport};
use super::{modal_box, Color, Style, Canvas};

const AI_WIDTH: usize = 60;
const PADDING: usize = 2;

fn hint_line(hint: &str) -> (String, Style) {
    let text_width = AI_WIDTH - 2 - 2 * PADDING;
    (format!("{hint:>text_width$}"), Style::color(Color::Muted))
}

/// One announcement or note: a title naming the AI side, some plain lines, and the key hint.
/// Used for a card being played, a die about to be thrown that has no preview of its own, the
/// influence a placement put down, and a pass.
pub fn render_ai_step(side: Superpower, title: &str, body: &[String], hint: &str) -> Canvas {
    let mut lines: Vec<(String, Style)> = Vec::new();
    for (i, line) in body.iter().enumerate() {
        let style = if i == 0 { Style::default().bold() } else { Style::default() };
        lines.push((line.clone(), style));
    }
    lines.push((String::new(), Style::default()));
    lines.push(hint_line(hint));
    modal_box(&format!("{side} (AI) · {title}"), "", AI_WIDTH, Style::color(side_color(side)), true, lines)
}

/// One finished realignment roll on a single line, for the tally above the latest roll.
fn compact_roll(map: &WorldMap, report: &RollReport) -> String {
    let RollOutcome::Realign(r) = &report.outcome else { return String::new() };
    let name = &map.country(r.target).name;
    let acting = report.side;
    let a = r.acting_die as i32 + r.acting_mods.total() as i32;
    let o = r.opposing_die as i32 + r.opposing_mods.total() as i32;
    let result = match r.loser {
        None => "tie".to_string(),
        Some(loser) if loser == acting => format!("{acting} loses {}", r.removed),
        Some(loser) => format!("{loser} loses {}", r.removed),
    };
    format!("{name}: {acting} {a} vs {} {o} — {result}", acting.opponent())
}

/// The single modal an AI realignment lives in: earlier rolls as one-line entries, the latest
/// in full, then either the next target's odds (Enter rolls it) or, once the operation is
/// closed (`done`), a prompt to continue. `card` names what funded it.
#[allow(clippy::too_many_arguments)]
pub fn render_ai_realign(
    map: &WorldMap,
    status: &GameStatus,
    board: &Board,
    op: Option<&Operation>,
    side: Superpower,
    card: &str,
    history: &[RollReport],
    next: Option<CountryId>,
    done: bool,
) -> Canvas {
    let mut lines: Vec<(String, Style)> = Vec::new();
    let mut border = side_color(side);
    let mut title = format!("Realignment · {card}");
    lines.push((format!("{side} (AI) realigns with {card}"), Style::color(side_color(side)).bold()));
    lines.push((String::new(), Style::default()));
    if let Some((last, earlier)) = history.split_last() {
        for report in earlier {
            lines.push((compact_roll(map, report), Style::color(Color::Muted)));
        }
        if !earlier.is_empty() {
            lines.push((String::new(), Style::default()));
        }
        let (result_title, body, color) = report_body(map, last);
        title = result_title;
        border = color;
        lines.extend(body);
    }
    if let (Some(id), Some(op)) = (next, op) {
        if !history.is_empty() {
            lines.push((String::new(), Style::default()));
            lines.push((format!("Next: {}", map.country(id).name), Style::default().bold()));
        } else {
            title = format!("Realignment · {}", map.country(id).name);
        }
        lines.extend(target_preview(map, status, board, op, id).0);
    }
    if done {
        let spent = history.len();
        lines.push((String::new(), Style::default()));
        lines.push((format!("{spent} roll{} made — operation complete", if spent == 1 { "" } else { "s" }), Style::color(Color::Muted)));
    }
    lines.push((String::new(), Style::default()));
    lines.push(hint_line(if next.is_some() { "Enter to roll" } else { "Enter to continue" }));
    modal_box(&title, "", AI_WIDTH + 4, Style::color(border), true, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Found;
    use crate::ops::{Modifiers, RollResult};
    use crate::render::ColorMode;

    fn none() -> Modifiers {
        Modifiers { adjacent_controlled: 0, more_influence: false, superpower_adjacent: false, iran_contra: false }
    }

    fn report(map: &WorldMap, name: &str, acting_die: u8, opposing_die: u8, loser: Option<Superpower>, removed: u8) -> RollReport {
        let Found::One(target) = map.find(name) else { panic!("{name}") };
        let result = RollResult { target, acting_die, acting_mods: none(), opposing_die, opposing_mods: none(), loser, removed };
        RollReport { side: Superpower::Ussr, outcome: RollOutcome::Realign(result), before: (2, 1), aftermath: None }
    }

    #[test]
    fn a_step_names_the_ai_side_and_says_what_enter_does() {
        let text = render_ai_step(Superpower::Ussr, "plays Comecon", &["USSR plays Comecon for its event".to_string()], "Enter to resolve").render(ColorMode::Never);
        assert!(text.contains("USSR (AI) · plays Comecon"), "{text}");
        assert!(text.contains("Enter to resolve"), "{text}");
    }

    #[test]
    fn a_realignment_keeps_earlier_rolls_to_one_line_and_the_latest_in_full() {
        let map = WorldMap::standard().unwrap();
        let status = GameStatus::default();
        let board = Board::new(&map);
        let history = vec![report(&map, "Iran", 5, 2, Some(Superpower::Us), 2), report(&map, "Iraq", 1, 4, Some(Superpower::Ussr), 1)];
        let text = render_ai_realign(&map, &status, &board, None, Superpower::Ussr, "Comecon", &history, None, true).render(ColorMode::Never);
        assert!(text.contains("Iran: USSR 5 vs USA 2"), "{text}");
        assert!(text.contains("Realignment · Iraq"), "{text}");
        assert!(text.contains("2 rolls made"), "{text}");
        assert!(text.contains("Enter to continue"), "{text}");
        for line in text.lines() {
            assert!(line.chars().count() <= AI_WIDTH + 4, "line too wide: {line:?}");
        }
    }
}
