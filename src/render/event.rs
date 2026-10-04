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
    if let Some(contest) = &result.contest {
        for part in wrap(&format!("Rolls: {}", crate::events::choice::describe_contest(contest)), text_width) {
            lines.push((part, Style::color(Color::Selected)));
        }
    }
    for &(side, card) in &result.discards {
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

/// A modal's width for an event played in a modal of its own (Summit, Olympic Games).
const SESSION_WIDTH: usize = 68;

/// The live modal for an event that runs in a modal of its own — Summit's and Olympic Games'
/// roll-offs. Before a roll: each side's bonus and the odds; after it: the dice, the winner and
/// what the chosen option does; for Olympic Games, first the choice between taking part and
/// boycotting. Redrawn from the open event on every frame, so it follows the choices as they're made.
pub fn render_event_session(cards: &CardCatalog, e: &crate::events::EventChoice, status: &crate::status::GameStatus) -> Canvas {
    let text_width = SESSION_WIDTH - 2 - 2 * PADDING;
    let muted = Style::color(Color::Muted);
    let mut lines: Vec<(String, Style)> = Vec::new();
    let mut border = Color::Muted;
    let hint;

    if let Some(contest) = e.contest() {
        for part in wrap(&format!("Rolls: {}", crate::events::choice::describe_contest(contest)), text_width) {
            lines.push((part, Style::color(Color::Selected)));
        }
        lines.push((String::new(), Style::default()));
        match contest.winner() {
            Some(winner) => {
                border = side_color(winner);
                if e.is_participation() {
                    lines.push((format!("{winner} wins the Olympic Games — 2 VP"), Style::color(side_color(winner)).bold()));
                } else {
                    lines.push((format!("{winner} wins — +2 VP, and chooses what happens to DEFCON ({}):", status.defcon), Style::color(side_color(winner)).bold()));
                    lines.push((String::new(), Style::default()));
                    push_modes(&mut lines, e, text_width, side_color(winner));
                }
            }
            None => lines.push(("A tie — nobody scores, and DEFCON stays where it is.".to_string(), Style::color(Color::Muted).bold())),
        }
        if e.mode().is_some() {
            push_result(&mut lines, e, status, text_width);
        }
        hint = match (e.mode(), contest.winner(), e.is_participation()) {
            (_, _, true) => "c confirm".to_string(),
            (Some(_), Some(_), _) => format!("c confirm · 1-{} change · ⌫ clear", e.modes().len()),
            (None, Some(_), _) => format!("1-{} choose", e.modes().len()),
            (_, None, _) => "c close".to_string(),
        };
    } else if e.is_participation() {
        // Olympic Games, before any roll: take part or boycott.
        for part in wrap(&format!("{} — choose:", e.context()), text_width) {
            lines.push((part, Style::default().bold()));
        }
        border = side_color(e.chooser());
        lines.push((String::new(), Style::default()));
        push_modes(&mut lines, e, text_width, side_color(e.chooser()));
        if e.needs_roll() {
            lines.push((String::new(), Style::default()));
            push_roll_block(&mut lines, e);
            hint = format!("r roll the dice · 1-{} change · ⌫ clear", e.modes().len());
        } else if e.mode().is_some() {
            push_result(&mut lines, e, status, text_width);
            hint = format!("c confirm · 1-{} change · ⌫ clear", e.modes().len());
        } else {
            hint = format!("1-{} choose", e.modes().len());
        }
    } else if e.needs_roll() {
        for part in wrap("Each superpower rolls a die and adds 1 for every region it Dominates or Controls.", text_width) {
            lines.push((part, Style::default()));
        }
        lines.push((String::new(), Style::default()));
        push_roll_block(&mut lines, e);
        lines.push((String::new(), Style::default()));
        for part in wrap("The winner gets 2 VP and may improve or degrade DEFCON by 1, or leave it. A tie does nothing.", text_width) {
            lines.push((part, muted));
        }
        hint = "r roll the dice · ⌫ cancel the event".to_string();
    } else {
        hint = String::new();
    }

    lines.push((String::new(), Style::default()));
    lines.push((format!("{hint:>text_width$}"), muted));

    let height = 2 + lines.len();
    let mut canvas = Canvas::new(SESSION_WIDTH, height);
    canvas.draw_thick_box(0, 0, SESSION_WIDTH, height, Style::color(border));
    let title = cards.card(e.card()).name.clone();
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), SESSION_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// The numbered options, the chosen one marked `▶`, long labels wrapped under their number.
fn push_modes(lines: &mut Vec<(String, Style)>, e: &crate::events::EventChoice, text_width: usize, side: Color) {
    for (i, mode) in e.modes().iter().enumerate() {
        let chosen = e.mode() == Some(i);
        let style = if chosen { Style::color(side).bold() } else { Style::default() };
        for (n, part) in wrap(&mode.label, text_width.saturating_sub(5)).into_iter().enumerate() {
            let lead = if n == 0 { format!("{} {}) ", if chosen { "▶" } else { " " }, i + 1) } else { "     ".to_string() };
            lines.push((format!("{lead}{part}"), style));
        }
    }
}

/// Each side's bonus and the odds of the roll still to be thrown, one row apiece.
fn push_roll_block(lines: &mut Vec<(String, Style)>, e: &crate::events::EventChoice) {
    let (Some((us, ussr)), Some((win_us, tie, win_ussr))) = (e.pending_bonuses(), e.roll_odds()) else { return };
    for (side, (bonus, note)) in [(Superpower::Us, us), (Superpower::Ussr, ussr)] {
        let why = if note.is_empty() { "no bonus".to_string() } else { note.clone() };
        lines.push((format!("{:<5} +{bonus}  {why}", side.to_string()), Style::color(side_color(side))));
    }
    lines.push((String::new(), Style::default()));
    let pct = |n: u8, of: u8| (n as u32 * 100 + of as u32 / 2) / of as u32;
    if e.rerolls_ties() {
        let decisive = win_us + win_ussr;
        lines.push((format!("Odds  US wins    {win_us:>2}/{decisive}  {:>3}%", pct(win_us, decisive)), Style::color(Color::Us)));
        lines.push((format!("      USSR wins  {win_ussr:>2}/{decisive}  {:>3}%", pct(win_ussr, decisive)), Style::color(Color::Ussr)));
        lines.push(("      (a tied roll is thrown again)".to_string(), Style::color(Color::Muted)));
    } else {
        lines.push((format!("Odds  US wins    {win_us:>2}/36  {:>3}%", pct(win_us, 36)), Style::color(Color::Us)));
        lines.push((format!("      tie        {tie:>2}/36  {:>3}%", pct(tie, 36)), Style::color(Color::Muted)));
        lines.push((format!("      USSR wins  {win_ussr:>2}/36  {:>3}%", pct(win_ussr, 36)), Style::color(Color::Ussr)));
    }
}

/// What the chosen option does: the DEFCON move, any operations it allows, and the VP.
fn push_result(lines: &mut Vec<(String, Style)>, e: &crate::events::EventChoice, status: &crate::status::GameStatus, text_width: usize) {
    let muted = Style::color(Color::Muted);
    let result = e.into_result(status);
    lines.push((String::new(), Style::default()));
    lines.push(("Result:".to_string(), muted));
    if let Some((before, after)) = result.defcon {
        lines.push((format!("  DEFCON {before} → {after}"), Style::default()));
    }
    if let Some(grant) = e.grant() {
        let sponsor = e.chooser().opponent();
        for part in wrap(&format!("{sponsor} may then conduct {}", grant.describe()), text_width.saturating_sub(2)) {
            lines.push((format!("  {part}"), Style::color(side_color(sponsor))));
        }
    }
    let vp_after = (status.vp as i16 + result.vp_delta as i16).clamp(-20, 20);
    match result.vp_delta.signum() {
        1 => lines.push((format!("  +{} VP to the US (now {vp_after})", result.vp_delta), Style::color(Color::Us))),
        -1 => lines.push((format!("  +{} VP to the USSR (now {vp_after})", -result.vp_delta), Style::color(Color::Ussr))),
        _ if e.contest().is_some() || e.grant().is_none() => lines.push((format!("  no VP change (still {vp_after})"), muted)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardId;
    use crate::ongoing::OngoingEffect;
    use crate::render::ColorMode;

    fn result(ongoing: Option<OngoingEffect>, vp_delta: i8) -> EffectResult {
        EffectResult { card: CardId(25), player: Superpower::Us, influence: Vec::new(), vp_delta, defcon: None, ongoing, lasting: None, cancels: None, china: None, space: None, mil_ops: 0, ends_game: false, reveals: None, discards: Vec::new(), contest: None }
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
