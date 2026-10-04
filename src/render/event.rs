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
    let title = result.name(cards).to_string();
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
    for &card in &result.pile_discards {
        lines.push((format!("{} discards {} (revealed)", Superpower::Us, cards.card(card).name), Style::color(side_color(Superpower::Us))));
    }
    if !result.returns.is_empty() {
        lines.push((format!("{} cards go back into the draw pile, which is reshuffled", result.returns.len()), Style::color(Color::Muted)));
    }
    if let Some((side, n)) = result.redraw {
        lines.push((format!("{side} draws {n} replacement{}", if n == 1 { "" } else { "s" }), Style::color(side_color(side))));
    }
    for &(side, card) in &result.takes {
        lines.push((format!("{side} takes {} from the discard pile (revealed)", cards.card(card).name), Style::color(side_color(side))));
    }
    if let Some(play) = result.plays {
        for part in wrap(&format!("{} {}", cards.card(play.id).name, play_next(&play)), text_width) {
            lines.push((part, Style::color(Color::Selected).bold()));
        }
        if play.exchange {
            for part in wrap(&format!("{} goes to {}'s hand, to be used for operations in their next action round", cards.card(result.card).name, result.player.opponent()), text_width) {
                lines.push((part, Style::default()));
            }
        }
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
        lines.push((game_over_line(victory), Style::color(victory.side.map_or(Color::Muted, side_color)).bold()));
        border = victory.side.map_or(Color::Muted, side_color);
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
            push_result(cards, &mut lines, e, status, text_width);
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
            push_result(cards, &mut lines, e, status, text_width);
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
    } else if e.is_pile_pick() && e.pile().is_empty() {
        border = side_color(e.chooser());
        let why = match e.pile_use() {
            crate::events::choice::PileUse::Take => "The discard pile is empty, so there is no card to take.",
            crate::events::choice::PileUse::Play => "No card in the discard pile has an event that can be played, so nothing happens.",
            crate::events::choice::PileUse::AskNot | crate::events::choice::PileUse::Tehran => "There are no cards to choose from, so nothing happens.",
        };
        for part in wrap(why, text_width) {
            lines.push((part, Style::default().bold()));
        }
        push_result(cards, &mut lines, e, status, text_width);
        hint = "c confirm · ⌫ take the card back".to_string();
    } else if e.is_pile_pick() && e.is_multi() {
        // Marking any number of cards (Ask Not What Your Country…, Our Man in Tehran).
        const WINDOW: usize = 9;
        let side = side_color(e.chooser());
        border = side;
        let intro = match e.pile_use() {
            crate::events::choice::PileUse::Tehran => format!("{} drew {} cards. Mark any to discard — the rest go back into the draw pile, which is reshuffled:", e.chooser(), e.pile().len()),
            _ => format!("{} may discard any of these {} cards (scoring cards too) and draws as many replacements:", e.chooser(), e.pile().len()),
        };
        for part in wrap(&intro, text_width) {
            lines.push((part, Style::default().bold()));
        }
        lines.push((String::new(), Style::default()));
        let total = e.pile().len();
        let start = e.cursor().saturating_sub(WINDOW / 2).min(total.saturating_sub(WINDOW));
        let end = (start + WINDOW).min(total);
        lines.push((if start > 0 { format!("   ↑ {start} more") } else { String::new() }, muted));
        for i in start..end {
            let (marked, here) = (e.is_marked(i), e.cursor() == i);
            let style = if marked { Style::color(side).bold() } else if here { Style::default().bold() } else { Style::default() };
            let card = cards.card(e.pile()[i]);
            let ops = if card.scoring { "S".to_string() } else { card.ops.to_string() };
            let text: String = format!("{ops} {}", card.name).chars().take(text_width.saturating_sub(6)).collect();
            lines.push((format!("{} {} {text}", if here { "›" } else { " " }, if marked { "☑" } else { "☐" }), style));
        }
        lines.push((if end < total { format!("   ↓ {} more", total - end) } else { String::new() }, muted));
        lines.push((String::new(), Style::default()));
        let summary = match e.pile_use() {
            crate::events::choice::PileUse::Tehran => format!("{} to discard, {} to return", e.marked_count(), total - e.marked_count()),
            _ => format!("{} to discard, {} to draw", e.marked_count(), e.marked_count()),
        };
        lines.push((summary, Style::color(side).bold()));
        hint = "↑↓ move · Enter mark/unmark · c confirm · ⌫ clear marks".to_string();
    } else if e.is_pile_pick() {
        // A discard-pile pick: a scrolling list, the highlight on `cursor`, the choice marked ▶.
        const WINDOW: usize = 9;
        let side = side_color(e.chooser());
        border = side;
        let intro = match e.pile_use() {
            crate::events::choice::PileUse::Take => format!("{} may take one non-scoring card from the discard pile ({} there):", e.chooser(), e.pile().len()),
            crate::events::choice::PileUse::Play => format!("{} picks a non-scoring card from the discard pile ({} playable) and plays it as an event:", e.chooser(), e.pile().len()),
            crate::events::choice::PileUse::AskNot | crate::events::choice::PileUse::Tehran => unreachable!("marking picks have their own branch"),
        };
        for part in wrap(&intro, text_width) {
            lines.push((part, Style::default().bold()));
        }
        lines.push((String::new(), Style::default()));
        let total = e.modes().len();
        let start = e.cursor().saturating_sub(WINDOW / 2).min(total.saturating_sub(WINDOW));
        let end = (start + WINDOW).min(total);
        lines.push((if start > 0 { format!("   ↑ {start} more") } else { String::new() }, muted));
        for i in start..end {
            let (chosen, here) = (e.mode() == Some(i), e.cursor() == i);
            let style = if chosen { Style::color(side).bold() } else if here { Style::default().bold() } else { Style::default() };
            let label = &e.modes()[i].label;
            let text: String = label.chars().take(text_width.saturating_sub(5)).collect();
            lines.push((format!("{}{} {text}", if here { "›" } else { " " }, if chosen { "▶" } else { " " }), style));
        }
        lines.push((if end < total { format!("   ↓ {} more", total - end) } else { String::new() }, muted));
        if e.mode().is_some() {
            push_result(cards, &mut lines, e, status, text_width);
            hint = "↑↓ move · Enter change · c confirm · ⌫ clear".to_string();
        } else {
            hint = "↑↓ / [ ] move · Enter choose".to_string();
        }
    } else {
        hint = String::new();
    }

    lines.push((String::new(), Style::default()));
    lines.push((format!("{hint:>text_width$}"), muted));

    let height = 2 + lines.len();
    let mut canvas = Canvas::new(SESSION_WIDTH, height);
    canvas.draw_thick_box(0, 0, SESSION_WIDTH, height, Style::color(border));
    let title = e.title().map_or_else(|| cards.card(e.card()).name.clone(), str::to_string);
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), SESSION_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// What happens to a card another event has put into play.
fn play_next(play: &crate::events::PlayCard) -> &'static str {
    match play.how {
        crate::events::PlayAs::Event => "has to be played as an event (press e)",
        crate::events::PlayAs::Either => "goes into play: play it as an event (e) or for operations (i/a/o)",
        crate::events::PlayAs::Ops => "goes into play: an opponent's event, so only its operations are used (i/a/o)",
    }
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
fn push_result(cards: &CardCatalog, lines: &mut Vec<(String, Style)>, e: &crate::events::EventChoice, status: &crate::status::GameStatus, text_width: usize) {
    let muted = Style::color(Color::Muted);
    let result = e.into_result(status);
    lines.push((String::new(), Style::default()));
    lines.push(("Result:".to_string(), muted));
    if let Some((before, after)) = result.defcon {
        lines.push((format!("  DEFCON {before} → {after}"), Style::default()));
    }
    for &(side, card) in &result.takes {
        lines.push((format!("  {side} takes {} (revealed)", cards.card(card).name), Style::color(side_color(side))));
    }
    if let Some(play) = result.plays {
        lines.push((format!("  {} {}", cards.card(play.id).name, play_next(&play)), Style::color(side_color(e.chooser()))));
    }
    if let Some(grant) = e.grant() {
        let sponsor = grant.side.unwrap_or(status.active);
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
        EffectResult { card: CardId(25), player: Superpower::Us, influence: Vec::new(), vp_delta, defcon: None, ongoing, lasting: None, cancels: None, china: None, space: None, mil_ops: 0, ends_game: false, reveals: None, discards: Vec::new(), takes: Vec::new(), plays: None, contest: None, title: None, redraw: None, pile_discards: Vec::new(), returns: Vec::new() }
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
