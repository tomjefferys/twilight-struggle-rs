//! The `E` modal: every effect in force, spelled out. The status bar only names them.

use crate::space::{self, Perk};
use crate::status::GameStatus;

use super::space::side_color;
use super::{lasting_effect_line, modal_box, ongoing_effect_line, Color, Canvas, Style};

const WIDTH: usize = 76;

/// Game-long events, turn-long events and the space race perks, each with its full wording.
pub fn render_effects(status: &GameStatus, hint: &str) -> Canvas {
    let muted = Style::color(Color::Muted);
    let mut lines: Vec<(String, Style)> = Vec::new();
    let section = |lines: &mut Vec<(String, Style)>, title: &str, items: Vec<(String, String, crate::country::Superpower)>| {
        if items.is_empty() {
            return;
        }
        if !lines.is_empty() {
            lines.push((String::new(), Style::default()));
        }
        lines.push((title.to_string(), muted));
        for (name, text, side) in items {
            lines.push((format!("  {name}"), Style::color(side_color(side)).bold()));
            lines.push((format!("    {text}"), Style::default()));
        }
    };
    section(
        &mut lines,
        "Until the card says otherwise",
        status.lasting.active().iter().map(|e| (e.label().to_string(), lasting_effect_line(e), e.side())).collect(),
    );
    section(
        &mut lines,
        "Until the end of this turn",
        status.effects.active().iter().map(|e| (e.label().to_string(), ongoing_effect_line(e), e.side())).collect(),
    );
    section(
        &mut lines,
        "Space race perks",
        Perk::ALL
            .iter()
            .filter_map(|&p| space::perk_holder(status, p).map(|side| (format!("{} ({side})", p.name()), p.label().to_string(), side)))
            .collect(),
    );
    if lines.is_empty() {
        lines.push(("Nothing is in force.".to_string(), muted));
    }
    lines.push((String::new(), Style::default()));
    lines.push((hint.to_string(), muted));
    modal_box("Effects in force", "", WIDTH, Style::default(), false, lines)
}
