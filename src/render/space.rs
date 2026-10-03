//! The Space Race views: [`render_space_track`], the whole track with both
//! markers and each box's perk, and [`render_space_result`], the modal for
//! one resolved attempt — the same titled-box treatment
//! [`super::war::render_war_result`] gives a war. Pure `Canvas` producers.

use crate::cards::{Card, CardCatalog, CHINA_CARD};
use crate::country::Superpower;
use crate::game::Victory;
use crate::space::{self, Perk, SpaceResult, MAX_BOX, TRACK};
use crate::status::GameStatus;

use super::{game_over_line, put_border_title, wrap, Canvas, Color, Style};

const TRACK_WIDTH: usize = 72;
const RESULT_WIDTH: usize = 56;
const CONFIRM_WIDTH: usize = 64;
const PADDING: usize = 2;
const US_COL: usize = 54;
const USSR_COL: usize = 59;

fn side_color(side: Superpower) -> Color {
    match side {
        Superpower::Us => Color::Us,
        Superpower::Ussr => Color::Ussr,
    }
}

/// Who a perk currently belongs to, in words.
pub fn perk_line(status: &GameStatus, perk: Perk) -> (String, Color) {
    let note = if perk.is_enforced() { "" } else { " (not enforced yet)" };
    match space::perk_holder(status, perk) {
        Some(side) => (format!("★ {} — {side}{note}", perk.label()), side_color(side)),
        None if space::position(status, Superpower::Us) >= perk.box_index() => (format!("★ {} — cancelled", perk.label()), Color::Muted),
        None => (format!("★ {} — nobody yet", perk.label()), Color::Muted),
    }
}

/// The whole track: box 0 to 8 with ops and die needed, VP for first and
/// second in, each side's marker and each perk's current holder.
pub fn render_space_track(status: &GameStatus) -> Canvas {
    let mut rows: Vec<(String, Style, Option<u8>)> = Vec::new();
    rows.push(("Start".to_string(), Style::default(), Some(0)));
    for (i, b) in TRACK.iter().enumerate() {
        let n = i as u8 + 1;
        rows.push((format!("{n}  {:<22} {}+ ops  d6≤{}  {}/{} VP", b.name, b.ops, b.max_roll, b.vp_first, b.vp_second), Style::default(), Some(n)));
        if let Some(perk) = b.perk {
            let (text, color) = perk_line(status, perk);
            rows.push((format!("     {text}"), Style::color(color), None));
        }
    }
    let height = rows.len() + 4;
    let mut canvas = Canvas::new(TRACK_WIDTH, height);
    canvas.draw_box(0, 0, TRACK_WIDTH, height, Style::default());
    put_border_title(&mut canvas, 0, 0, "Space Race", Style::default().bold(), "", Style::default(), TRACK_WIDTH);
    canvas.put(1, 1 + PADDING, "Box", Style::color(Color::Muted));
    canvas.put(1, US_COL, "USA", Style::color(Color::Us).bold());
    canvas.put(1, USSR_COL, "USSR", Style::color(Color::Ussr).bold());
    for (i, (text, style, box_n)) in rows.iter().enumerate() {
        let row = 2 + i;
        canvas.put(row, 1 + PADDING, text, *style);
        if let Some(n) = box_n {
            if space::position(status, Superpower::Us) == *n {
                canvas.put(row, US_COL, "◆", Style::color(Color::Us).bold());
            }
            if space::position(status, Superpower::Ussr) == *n {
                canvas.put(row, USSR_COL, "◆", Style::color(Color::Ussr).bold());
            }
        }
    }
    canvas.put(height - 2, 1 + PADDING, &format!("Attempts this turn: USA {}/{}  USSR {}/{}",
        space::attempts_used(status, Superpower::Us), space::attempts_allowed(status, Superpower::Us),
        space::attempts_used(status, Superpower::Ussr), space::attempts_allowed(status, Superpower::Ussr)), Style::color(Color::Muted));
    canvas
}

/// The confirmation modal shown before a space attempt is rolled: the next
/// box (what it needs, the die roll that succeeds, what reaching it pays
/// right now and its perk), the card about to be spent, and — when
/// [`space::check`] refuses — why the roll is unavailable. The modal is
/// shown either way; only the hint row changes, so the player can still
/// read the box while seeing that Enter will do nothing.
pub fn render_space_confirm(status: &GameStatus, card: &Card) -> Canvas {
    let side = status.active;
    let at = space::position(status, side);
    let (ops, _) = status.effects.card_ops(card.ops, side);
    let check = space::check(status, side, card.id == CHINA_CARD, card.scoring, ops);
    let text_width = CONFIRM_WIDTH - 2 - 2 * PADDING;
    let muted = Style::color(Color::Muted);
    let mut lines: Vec<(String, Style)> = Vec::new();

    let title = match space::next_box(status, side) {
        Some(b) => {
            let n = at + 1;
            lines.push((format!("{side}: box {at} of {MAX_BOX} → box {n}, {}", b.name), Style::color(side_color(side)).bold()));
            lines.push((String::new(), Style::default()));
            lines.push((format!("Needs a card of {}+ ops", b.ops), Style::default()));
            lines.push((format!("Roll d6 ≤ {} to succeed ({} in 6)", b.max_roll, b.max_roll), Style::default()));
            let vp = space::arrival_vp(status, side, n).abs();
            let place = if space::position(status, side.opponent()) >= n { "second in" } else { "first in" };
            lines.push((format!("Reaching it now pays {vp} VP ({place}; {}/{} first/second)", b.vp_first, b.vp_second), Style::default()));
            if let Some(perk) = b.perk {
                let (text, style) = if space::position(status, side.opponent()) >= n {
                    ("no longer available — the opponent is already there".to_string(), muted)
                } else {
                    (format!("yours until the opponent arrives{}", if perk.is_enforced() { "" } else { " (not enforced yet)" }), Style::color(side_color(side)))
                };
                lines.push((format!("Perk: {} — {text}", perk.label()), style));
            }
            format!("Space Race · {}", b.name)
        }
        None => {
            lines.push((format!("{side} is already in the last box ({MAX_BOX} of {MAX_BOX})"), Style::color(side_color(side)).bold()));
            "Space Race".to_string()
        }
    };

    lines.push((String::new(), Style::default()));
    lines.push((format!("Card: {} ({ops} ops)", card.name), Style::color(side_color(side))));
    lines.push((
        format!("Attempts this turn: {}/{}", space::attempts_used(status, side), space::attempts_allowed(status, side)),
        muted,
    ));

    lines.push((String::new(), Style::default()));
    let (hint, border) = match check {
        Ok(()) => ("Enter roll the die · Esc cancel".to_string(), side_color(side)),
        Err(e) => {
            for part in wrap(&format!("Can't roll: {e}"), text_width) {
                lines.push((part, Style::color(Color::Muted).bold()));
            }
            lines.push((String::new(), Style::default()));
            ("Roll unavailable · Esc close".to_string(), Color::Muted)
        }
    };
    lines.push((format!("{hint:>text_width$}"), muted));

    let height = 2 + lines.len();
    let mut canvas = Canvas::new(CONFIRM_WIDTH, height);
    canvas.draw_thick_box(0, 0, CONFIRM_WIDTH, height, Style::color(border));
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), CONFIRM_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

/// Draws `result` as a titled modal box, bordered in the side's colour on
/// a success (muted on a miss, or the game's winner if it ended the game).
pub fn render_space_result(
    cards: &CardCatalog,
    result: &SpaceResult,
    vp_after: i8,
    winner: Option<Victory>,
    queue_pos: Option<(usize, usize)>,
) -> Canvas {
    let side = result.side;
    let target = result.target();
    let title = format!("Space Race · {}", target.name);
    let text_width = RESULT_WIDTH - 2 - 2 * PADDING;
    let mut lines: Vec<(String, Style)> = Vec::new();
    lines.push((format!("{side} spends {}", cards.card(result.card).name), Style::color(side_color(side))));
    lines.push((format!("d6: {}   needs ≤{}", result.roll, result.max_roll), Style::color(side_color(side))));
    lines.push((String::new(), Style::default()));
    if result.success {
        lines.push((format!("{side} reaches box {}/{MAX_BOX}: {}", result.from + 1, target.name), Style::color(side_color(side)).bold()));
    } else {
        lines.push((format!("The attempt FAILS — {side} stays at box {}", result.from), Style::color(Color::Muted).bold()));
    }
    if result.vp_delta != 0 {
        let (who, n) = if result.vp_delta > 0 { ("US", result.vp_delta) } else { ("USSR", -result.vp_delta) };
        lines.push((format!("+{n} VP to the {who} (now {vp_after})"), Style::color(side_color(side)).bold()));
    } else if result.success {
        lines.push((format!("no VP change (still {vp_after})"), Style::color(Color::Muted)));
    }

    let mut border = if result.success { side_color(side) } else { Color::Muted };
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
    let mut canvas = Canvas::new(RESULT_WIDTH, height);
    canvas.draw_thick_box(0, 0, RESULT_WIDTH, height, Style::color(border));
    put_border_title(&mut canvas, 0, 0, &title, Style::default().bold(), "", Style::default(), RESULT_WIDTH);
    for (i, (line, style)) in lines.iter().enumerate() {
        canvas.put(1 + i, 1 + PADDING, line, *style);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::CardId;
    use crate::render::ColorMode;

    #[test]
    fn the_track_lists_every_box_and_who_holds_each_perk() {
        let status = GameStatus { space_race_us: 2, space_race_ussr: 1, ..GameStatus::default() };
        let text = render_space_track(&status).render(ColorMode::Never);
        for b in TRACK {
            assert!(text.contains(b.name), "{text}");
        }
        assert!(text.contains("2 space attempts a turn — USA"), "{text}");
        assert!(text.contains("8 action rounds — nobody yet"), "{text}");
        let both = GameStatus { space_race_us: 2, space_race_ussr: 3, ..GameStatus::default() };
        assert!(render_space_track(&both).render(ColorMode::Never).contains("cancelled"));
    }

    fn card(cards: &CardCatalog, name: &str) -> Card {
        cards.card(cards.id_by_name(name).unwrap()).clone()
    }

    #[test]
    fn the_confirmation_explains_the_next_box_and_enables_the_roll() {
        let cards = CardCatalog::standard().unwrap();
        let status = GameStatus::default();
        let text = render_space_confirm(&status, &card(&cards, "Duck and Cover")).render(ColorMode::Never);
        for want in ["Earth Satellite", "Needs a card of 2+ ops", "d6 ≤ 3", "3 in 6", "Reaching it now pays 2 VP (first in", "Enter roll the die"] {
            assert!(text.contains(want), "missing {want:?}:\n{text}");
        }
    }

    #[test]
    fn the_confirmation_is_shown_but_disabled_when_the_roll_is_not_allowed() {
        let cards = CardCatalog::standard().unwrap();
        let status = GameStatus::default();
        let weak = render_space_confirm(&status, &card(&cards, "Romanian Abdication")).render(ColorMode::Never);
        assert!(weak.contains("Can't roll: Earth Satellite needs a card of at least 2 ops") && weak.contains("Roll unavailable") && !weak.contains("Enter roll"), "{weak}");
        let used = GameStatus { space_attempts_ussr: 1, ..status };
        let spent = render_space_confirm(&used, &card(&cards, "Duck and Cover")).render(ColorMode::Never);
        assert!(spent.contains("already made this turn's space attempt") && spent.contains("Attempts this turn: 1/1") && spent.contains("Roll unavailable"), "{spent}");
        let done = GameStatus { space_race_ussr: 8, ..status };
        assert!(render_space_confirm(&done, &card(&cards, "Duck and Cover")).render(ColorMode::Never).contains("last box"));
    }

    #[test]
    fn the_modal_reports_a_miss_and_a_first_in_win() {
        let cards = CardCatalog::standard().unwrap();
        let status = GameStatus::default();
        let hit = space::resolve(&status, Superpower::Ussr, CardId(4), 2);
        let text = render_space_result(&cards, &hit, -2, None, Some((1, 2))).render(ColorMode::Never);
        assert!(text.contains("Earth Satellite") && text.contains("+2 VP to the USSR (now -2)") && text.contains("1 of 2"), "{text}");
        let miss = space::resolve(&status, Superpower::Us, CardId(4), 6);
        let text = render_space_result(&cards, &miss, 0, None, None).render(ColorMode::Never);
        assert!(text.contains("FAILS"), "{text}");
    }
}
