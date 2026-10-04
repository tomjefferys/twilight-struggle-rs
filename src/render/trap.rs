//! The escape-attempt modals for a trapped action round (Bear Trap, Quagmire): the confirmation
//! before the discard and roll, and the result. Pure `Canvas` producers, in the style of
//! [`super::space`]'s.

use crate::cards::{Card, CardCatalog};
use crate::country::Superpower;
use crate::game::TrapResult;
use crate::ongoing::LastingEffect;

use super::{put_border_title, wrap, Canvas, Color, Style};

const WIDTH: usize = 60;
const PADDING: usize = 2;

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

fn finish(title: &str, border: Color, lines: &[(String, Style)]) -> Canvas {
    let height = 2 + lines.len();
    let mut canvas = Canvas::new(WIDTH, height);
    canvas.draw_thick_box(0, 0, WIDTH, height, Style::color(border));
    put_border_title(&mut canvas, 0, 0, title, Style::default().bold(), "", Style::default(), WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// Before the roll: what discarding `card` and rolling will do.
pub fn render_trap_confirm(effect: LastingEffect, side: Superpower, card: &Card) -> Canvas {
    let text_width = WIDTH - 2 - 2 * PADDING;
    let muted = Style::color(Color::Muted);
    let mut lines: Vec<(String, Style)> = Vec::new();
    for part in wrap(&format!("{side} is caught in {}. Discard {} ({} ops) and roll a die: 1-4 ends the trap.", effect.label(), card.name, card.ops), text_width) {
        lines.push((part, Style::color(side_color(side))));
    }
    lines.push((String::new(), Style::default()));
    for part in wrap("Either way this action round is spent.", text_width) {
        lines.push((part, muted));
    }
    lines.push((String::new(), Style::default()));
    lines.push((format!("{:>text_width$}", "Enter discard and roll · Esc cancel"), muted));
    finish(&format!("{} · escape attempt", effect.label()), side_color(side), &lines)
}

/// After the roll.
pub fn render_trap_result(cards: &CardCatalog, result: &TrapResult, queue_pos: Option<(usize, usize)>) -> Canvas {
    let side = result.side;
    let text_width = WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    lines.push((format!("{side} discards {}", cards.card(result.discarded).name), Style::color(side_color(side))));
    lines.push((format!("d6: {}   needs ≤4", result.roll), Style::color(side_color(side))));
    lines.push((String::new(), Style::default()));
    let name = &cards.card(result.trap).name;
    if result.escaped {
        lines.push((format!("{side} escapes — {name} ends"), Style::color(side_color(side)).bold()));
    } else {
        lines.push((format!("Still trapped — {name} holds {side}'s next action round too"), Style::color(Color::Muted).bold()));
    }
    lines.push((String::new(), Style::default()));
    let hint = match queue_pos {
        Some((n, total)) => format!("Enter to continue · {n} of {total}"),
        None => "Enter to continue".to_string(),
    };
    lines.push((format!("{hint:>text_width$}"), Style::color(Color::Muted)));
    finish(&format!("{name} · escape attempt"), if result.escaped { side_color(side) } else { Color::Muted }, &lines)
}
